//! Local-library identity metadata. Portable ATN files never inherit these identities.
use std::collections::HashSet;

use craft_psd::atn;
use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

use super::super::{Action, ActionState, identity};

type Result<T> = std::result::Result<T, String>;

/// Physical ATN records and logical runtime folders deliberately have different indices.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum GroupKey {
    Folder(usize),
    Legacy(String, usize),
    Implicit(usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IdentityRow {
    ordinal: usize,
    wire_digest: String,
    action_id: Option<String>,
    legacy_shortcut_owner: bool,
    set_slot: Option<usize>,
}

/// No steps or editor metadata are duplicated here: the existing ATN codec owns definitions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IdentityManifest {
    version: u32,
    #[serde(deserialize_with = "action_rows")]
    actions: Vec<IdentityRow>,
    #[serde(deserialize_with = "names")]
    sets: Vec<String>,
    #[serde(deserialize_with = "names")]
    set_rows: Vec<String>,
    #[serde(deserialize_with = "group_rows")]
    pub(crate) groups: Vec<GroupKey>,
}

/// Stop a hostile sequence at the existing archive count ceiling, before growing an unbounded Vec.
fn sequence<'de, D, T>(d: D, limit: usize) -> std::result::Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Bounded<T>(usize, std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Bounded<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a bounded library identity sequence")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Vec<T>, A::Error> {
            let mut out = Vec::new();
            while let Some(value) = seq.next_element()? {
                if out.len() == self.0 {
                    return Err(de::Error::custom("too many library identity records"));
                }
                out.push(value);
            }
            Ok(out)
        }
    }
    d.deserialize_seq(Bounded(limit, std::marker::PhantomData))
}
fn action_rows<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<IdentityRow>, D::Error> {
    sequence(d, atn::MAX_TOTAL_ACTIONS)
}
fn names<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<String>, D::Error> {
    sequence(d, atn::MAX_SETS)
}
fn group_rows<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<GroupKey>, D::Error> {
    sequence(d, atn::MAX_SETS)
}

impl IdentityManifest {
    pub(crate) fn group_name<'a>(&'a self, key: &'a GroupKey) -> Option<&'a str> {
        match key {
            GroupKey::Folder(slot) => self.set_rows.get(*slot).map(String::as_str),
            GroupKey::Legacy(name, _) => Some(name),
            GroupKey::Implicit(_) => None,
        }
    }

    pub(crate) fn capture(state: &ActionState, groups: Vec<GroupKey>, wire_actions: &[&Action], fingerprints: &[String]) -> Result<Self> {
        if wire_actions.len() != state.list.len() || fingerprints.len() != wire_actions.len() {
            return Err("local library identity selection is incomplete".into());
        }
        let positions: std::collections::HashMap<_, _> = state.list.iter().enumerate().map(|(i, action)| (std::ptr::from_ref(action), i)).collect();
        let mut actions = Vec::with_capacity(wire_actions.len());
        for (action, fingerprint) in wire_actions.iter().zip(fingerprints) {
            let ordinal = positions.get(&std::ptr::from_ref(*action)).copied().ok_or("identity row is not in the library")?;
            actions.push(IdentityRow {
                ordinal,
                wire_digest: fingerprint.clone(),
                action_id: action.action_id.clone(),
                legacy_shortcut_owner: action.legacy_shortcut_owner,
                set_slot: action.set_slot,
            });
        }
        let out = Self { version: 1, actions, sets: state.sets.clone(), set_rows: state.set_rows.clone(), groups };
        out.validate(wire_actions.len())?;
        Ok(out)
    }

    fn validate(&self, count: usize) -> Result<()> {
        if self.version != 1 || self.actions.len() != count || count > atn::MAX_TOTAL_ACTIONS || self.groups.len() > atn::MAX_SETS {
            return Err("invalid local library identity version or count".into());
        }
        if self.sets.len() > atn::MAX_SETS || self.set_rows.len() > atn::MAX_SETS {
            return Err("too many local library folders".into());
        }
        if self.sets.iter().chain(&self.set_rows).any(|name| name.chars().count() > crate::atn_cmds::MAX_NAME_CHARS) {
            return Err("local library folder name exceeds its limit".into());
        }
        let mut ordinals = HashSet::new();
        for row in &self.actions {
            if row.wire_digest.len() != 64 || !row.wire_digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
                return Err("invalid native identity fingerprint".into());
            }
            if row.ordinal >= count || !ordinals.insert(row.ordinal) || row.set_slot.is_some_and(|slot| slot >= self.set_rows.len()) {
                return Err("invalid local library action ordinal or folder".into());
            }
            if row.action_id.as_ref().is_some_and(|id| !id.is_empty() && !identity::valid_action_id(id)) {
                return Err("invalid local library action identity".into());
            }
        }
        let mut folders = HashSet::new();
        let mut implicit = HashSet::new();
        let mut legacy = HashSet::new();
        for group in &self.groups {
            match group {
                GroupKey::Folder(slot) if *slot < self.set_rows.len() && folders.insert(*slot) => {}
                GroupKey::Legacy(name, row)
                    if *row <= atn::MAX_SETS && name.chars().count() <= crate::atn_cmds::MAX_NAME_CHARS && legacy.insert((name, row)) => {}
                GroupKey::Implicit(slot) if *slot <= atn::MAX_SETS && implicit.insert(*slot) => {}
                _ => return Err("invalid or repeated local physical folder identity".into()),
            }
        }
        if folders.len() != self.set_rows.len() {
            return Err("local identity manifest omits an explicit folder".into());
        }
        Ok(())
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        self.validate(self.actions.len())?;
        struct Bounded(Vec<u8>);
        impl std::io::Write for Bounded {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if self.0.len().checked_add(bytes.len()).is_none_or(|size| size > super::MAX_BYTES) {
                    return Err(std::io::Error::other("library identity metadata exceeds action-file byte limit"));
                }
                self.0.try_reserve(bytes.len()).map_err(std::io::Error::other)?;
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut bytes = Bounded(Vec::new());
        serde_json::to_writer(&mut bytes, self).map_err(|error| error.to_string())?;
        Ok(bytes.0)
    }

    pub(crate) fn decode(bytes: &[u8], count: usize, physical_rows: usize) -> Result<Self> {
        if bytes.len() > super::MAX_BYTES {
            return Err("library identity metadata exceeds action-file byte limit".into());
        }
        let out: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        out.validate(count)?;
        if out.groups.len() != physical_rows {
            return Err("local identity manifest does not match the physical archive".into());
        }
        Ok(out)
    }

    /// Restore only a trusted local library. Portable import does not call this method.
    pub(crate) fn restore(&self, state: &mut ActionState, fingerprints: &[String]) -> Result<()> {
        self.validate(state.list.len())?;
        if fingerprints.len() != self.actions.len() || self.actions.iter().zip(fingerprints).any(|(row, fingerprint)| row.wire_digest != *fingerprint) {
            return Err("local identity metadata no longer matches its native records".into());
        }
        if state.list.iter().zip(&self.actions).any(|(action, row)| action.set_slot != row.set_slot) {
            return Err("local identity folder does not match its physical record".into());
        }
        let mut ordered = Vec::with_capacity(state.list.len());
        for (mut action, row) in std::mem::take(&mut state.list).into_iter().zip(&self.actions) {
            action.action_id = row.action_id.clone();
            action.legacy_shortcut_owner = row.legacy_shortcut_owner;
            action.set_slot = row.set_slot;
            if let Some(slot) = row.set_slot {
                action.set = Some(self.set_rows[slot].clone());
            }
            ordered.push((row.ordinal, action));
        }
        ordered.sort_unstable_by_key(|(ordinal, _)| *ordinal);
        state.list = ordered.into_iter().map(|(_, action)| action).collect();
        identity::invalidate_duplicate_ids(&mut state.list);
        state.sets = self.sets.clone();
        state.set_rows = self.set_rows.clone();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_manifest_keeps_same_caption_folders_and_global_order() {
        let mut state = ActionState { set_rows: vec!["Same".into(), "Same".into()], sets: vec!["Legacy empty".into()], ..Default::default() };
        for (name, slot, id) in [("First", 0, "11111111111111111111111111111111"), ("Second", 1, "22222222222222222222222222222222")] {
            let mut action = Action::new(name, Vec::new());
            action.set = Some("Same".into());
            action.set_slot = Some(slot);
            action.action_id = Some(id.into());
            state.list.push(action);
        }
        state.list[1].legacy_shortcut_owner = true;
        let groups = vec![GroupKey::Folder(1), GroupKey::Implicit(0), GroupKey::Legacy("Legacy empty".into(), atn::MAX_SETS), GroupKey::Folder(0)];
        let captured = IdentityManifest::capture(&state, groups.clone(), &[&state.list[1], &state.list[0]], &["1".repeat(64), "2".repeat(64)]).unwrap();
        let bytes = captured.encode().unwrap();
        let manifest = IdentityManifest::decode(&bytes, 2, 4).unwrap();
        let mut loaded = ActionState { list: vec![state.list[1].clone(), state.list[0].clone()], ..Default::default() };
        for action in &mut loaded.list {
            action.action_id = None;
            action.legacy_shortcut_owner = false;
        }
        manifest.restore(&mut loaded, &["1".repeat(64), "2".repeat(64)]).unwrap();
        assert_eq!(loaded.list, state.list);
        assert_eq!(loaded.set_rows, state.set_rows);
        assert_eq!(loaded.sets, state.sets);
        assert_eq!(manifest.groups, groups);
    }

    #[test]
    fn invalid_or_duplicate_manifest_positions_cannot_publish_partial_identity() {
        let state = ActionState { list: vec![Action::new("One", Vec::new()), Action::new("Two", Vec::new())], ..Default::default() };
        let mut captured =
            IdentityManifest::capture(&state, vec![GroupKey::Implicit(0)], &[&state.list[0], &state.list[1]], &["1".repeat(64), "2".repeat(64)]).unwrap();
        captured.actions[1].ordinal = 0;
        assert!(captured.encode().is_err());
        captured.actions[1].ordinal = 1;
        captured.actions[1].set_slot = Some(0);
        assert!(captured.encode().is_err());
    }
}
