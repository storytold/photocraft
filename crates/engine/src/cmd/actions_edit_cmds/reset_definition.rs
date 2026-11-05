//! Reset's original factory definitions and one definition-only undo value.
//! The caller owns the single latest undo slot; no recording/playback/editor state is copied.
use std::collections::BTreeMap;

use crate::{
    Result, Session,
    actions_cmds::{Action, ActionState, library},
    atn_cmds::sidecar::SetRowMetadata,
};
use serde_json::{Value, json};

use super::{bad, defaults, not_playing};

/// Only definitions and both shortcut maps belong to a Reset undo.
#[derive(Clone, Debug, PartialEq)]
pub struct ResetUndo {
    list: Vec<Action>,
    sets: Vec<String>,
    library: library::Library,
    rows: Vec<SetRowMetadata>,
    shortcuts: BTreeMap<String, String>,
    alternate_shortcuts: BTreeMap<String, Vec<String>>,
    after_shortcuts: BTreeMap<String, String>,
    after_alternate_shortcuts: BTreeMap<String, Vec<String>>,
    revision: u64,
}

/// Materialize a fresh parsed archive on each call, so equal factory captions have fresh RowOrigin.
fn factory() -> Result<ActionState> {
    let seed = ActionState { list: defaults::actions(), sets: vec![defaults::NAME.into()], ..Default::default() };
    let bytes = library::encode(&seed, &[]).map_err(|error| bad("actions.reset", error))?;
    library::decode(&bytes).map_err(|error| bad("actions.reset", error))
}

/// Keep definition changes outside active or suspended command ownership.
fn idle(s: &Session, command: &str) -> Result<()> {
    not_playing(s, command)?;
    if s.actions.recording_active() {
        return Err(bad(command, "stop recording before resetting actions"));
    }
    Ok(())
}

/// Validate a complete proposed definition before replacing any live field.
/// Return the one undo value to the caller's latest-only slot after the atomic publication.
pub(super) fn apply(s: &mut Session, replace: bool) -> Result<(Value, ResetUndo)> {
    let command = "actions.reset";
    idle(s, command)?;
    let fresh = factory()?;
    let mut proposed = library::definition_copy(&s.actions);
    if replace {
        proposed = library::definition_copy(&fresh);
        // Preserve the existing exact legacy-retirement receipt, not the discarded row sources.
        proposed.library.legacy = s.actions.library.legacy.clone();
    } else {
        let offset = proposed.sets.len();
        for (index, name) in fresh.sets.iter().enumerate() {
            let slot = proposed.sets.len();
            proposed.sets.push(name.clone());
            library::import_row(&mut proposed, &fresh, index, slot);
        }
        for row in &fresh.atn_set_rows {
            let mut row = row.clone();
            row.slot = row.slot.checked_add(offset).ok_or_else(|| bad(command, "Actions folder count overflow"))?;
            proposed.atn_set_rows.push(row);
        }
        proposed.list.extend(fresh.list);
    }
    let mut shortcuts = s.prefs().shortcuts.clone();
    let mut alternate = s.prefs().alternate_shortcuts.clone();
    if replace {
        shortcuts.retain(|key, _| !key.starts_with(crate::actions_cmds::PLAY_SHORTCUT_PREFIX));
        alternate.retain(|key, _| !key.starts_with(crate::actions_cmds::PLAY_SHORTCUT_PREFIX));
    }
    library::check_action_count(&proposed, &proposed.list).map_err(|error| bad(command, error))?;
    library::check_definition(&proposed, &shortcuts).map_err(|error| bad(command, error))?;
    let undo = ResetUndo {
        list: std::mem::replace(&mut s.actions.list, proposed.list),
        sets: std::mem::replace(&mut s.actions.sets, proposed.sets),
        library: std::mem::replace(&mut s.actions.library, proposed.library),
        rows: std::mem::replace(&mut s.actions.atn_set_rows, proposed.atn_set_rows),
        shortcuts: s.prefs().shortcuts.clone(),
        alternate_shortcuts: s.prefs().alternate_shortcuts.clone(),
        after_shortcuts: shortcuts.clone(),
        after_alternate_shortcuts: alternate.clone(),
        revision: 0,
    };
    s.edit_prefs(|prefs| {
        prefs.shortcuts = shortcuts;
        prefs.alternate_shortcuts = alternate;
    });
    s.actions.touch();
    let undo = ResetUndo { revision: s.actions.rev, ..undo };
    Ok((json!({"set":defaults::NAME,"slot":s.actions.sets.len().saturating_sub(1),"appended":!replace,"actions":2}), undo))
}

/// Check the latest slot by reference, so a refused undo does not consume its value.
pub(super) fn can_restore(s: &Session, undo: &ResetUndo) -> Result<()> {
    let command = "actions.undoReset";
    idle(s, command)?;
    if s.actions.rev != undo.revision || s.prefs().shortcuts != undo.after_shortcuts || s.prefs().alternate_shortcuts != undo.after_alternate_shortcuts {
        return Err(bad(command, "the actions or shortcuts changed after Reset Actions"));
    }
    Ok(())
}

/// Restore only the latest unchanged Reset definition, retaining all live runtime ownership.
pub(super) fn restore(s: &mut Session, undo: ResetUndo) -> Result<Value> {
    let command = "actions.undoReset";
    can_restore(s, &undo)?;
    let proposed = ActionState { list: undo.list, sets: undo.sets, library: undo.library, atn_set_rows: undo.rows, ..Default::default() };
    library::check_definition(&proposed, &undo.shortcuts).map_err(|error| bad(command, error))?;
    s.actions.list = proposed.list;
    s.actions.sets = proposed.sets;
    s.actions.library = proposed.library;
    s.actions.atn_set_rows = proposed.atn_set_rows;
    s.edit_prefs(|prefs| {
        prefs.shortcuts = undo.shortcuts;
        prefs.alternate_shortcuts = undo.alternate_shortcuts;
    });
    s.actions.touch();
    Ok(json!({"undone":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions_edit_cmds::folders::set_occurrences;

    /// Repeated factory loads keep physical rows and undo never restores runtime counters.
    #[test]
    fn append_keeps_same_caption_occurrences_and_latest_undo_only() {
        let mut s = Session::new();
        s.actions.take = 17;
        s.actions.playback_nonce = 41;
        s.actions.playback_steps = 23;
        let (_, first) = apply(&mut s, false).unwrap();
        let (_, latest) = apply(&mut s, false).unwrap();
        let rows = set_occurrences(&s).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(), [defaults::NAME, defaults::NAME]);
        assert!(rows.iter().all(|row| row.actions.len() == 2 && row.origin.is_some()));
        assert_ne!(rows[0].origin, rows[1].origin);
        let restored = library::decode(&library::encode(&s.actions, &[]).unwrap()).unwrap();
        assert_eq!(restored.sets, [defaults::NAME, defaults::NAME]);
        assert_eq!(restored.list.len(), 4);
        restore(&mut s, latest).unwrap();
        assert_eq!(s.actions.sets, [defaults::NAME]);
        assert_eq!((s.actions.take, s.actions.playback_nonce, s.actions.playback_steps), (17, 41, 23));
        assert!(restore(&mut s, first).is_err(), "an older Reset is not a second panel undo");
        assert_eq!(s.actions.list.len(), 2);
    }

    /// Replace and its undo own both shortcut maps and leave the document/history untouched.
    #[test]
    fn replace_undo_restores_definitions_empty_rows_and_both_shortcut_maps() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":8})).unwrap();
        apply(&mut s, false).unwrap();
        crate::atn_cmds::sidecar::install_row(&mut s.actions, "Empty", None);
        let prefix = crate::actions_cmds::PLAY_SHORTCUT_PREFIX;
        s.edit_prefs(|prefs| {
            prefs.shortcuts.insert(format!("{prefix}Invert Colors"), "F2".into());
            prefs.alternate_shortcuts.insert(format!("{prefix}Invert Colors"), vec!["F3".into()]);
            prefs.shortcuts.insert("file.save".into(), "Ctrl+S".into());
        });
        let keys = s.prefs().shortcuts.clone();
        let alternate = s.prefs().alternate_shortcuts.clone();
        let before = library::encode(&s.actions, &[]).unwrap();
        let document = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let (_, undo) = apply(&mut s, true).unwrap();
        assert_eq!(s.actions.sets, [defaults::NAME]);
        assert!(s.prefs().shortcuts.keys().all(|key| !key.starts_with(prefix)));
        assert!(s.prefs().alternate_shortcuts.keys().all(|key| !key.starts_with(prefix)));
        restore(&mut s, undo).unwrap();
        assert_eq!(library::encode(&s.actions, &[]).unwrap(), before);
        assert_eq!(s.prefs().shortcuts, keys);
        assert_eq!(s.prefs().alternate_shortcuts, alternate);
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &document));
        assert_eq!(s.active().unwrap().revision, revision);
    }

    /// Refuse invalid proposed definitions, active recorders, and changed shortcuts before mutation.
    #[test]
    fn reset_and_undo_refusals_do_not_publish_partial_state() {
        let mut s = Session::new();
        // Canonical kept-native storage preserves unknown commands without executing them.
        let unknown = Action::new("Unknown", vec![("not.a.command".into(), json!({}))]);
        s.actions.list.push(unknown.clone());
        let (_, undo) = apply(&mut s, false).unwrap();
        assert_eq!(s.actions.list[0], unknown);
        assert_eq!(s.actions.list.len(), 3);
        restore(&mut s, undo).unwrap();
        assert_eq!(s.actions.list, [unknown]);

        // A color outside both supported and preserved ATN colors is not durably representable.
        s.actions.list[0].color = "not-a-library-color".into();
        let before = s.actions.list.clone();
        let revision = s.actions.rev;
        assert!(library::check_definition(&s.actions, &s.prefs().shortcuts).is_err());
        assert!(apply(&mut s, false).is_err());
        assert_eq!(s.actions.list, before);
        assert!(s.actions.sets.is_empty());
        assert_eq!(s.actions.rev, revision);
        s.actions.list.clear();
        let (_, undo) = apply(&mut s, false).unwrap();
        let current = library::encode(&s.actions, &[]).unwrap();
        s.edit_prefs(|prefs| {
            prefs.alternate_shortcuts.insert("file.save".into(), vec!["F4".into()]);
        });
        assert!(restore(&mut s, undo).is_err());
        assert_eq!(library::encode(&s.actions, &[]).unwrap(), current);
        s.execute("actions.record", json!({"action":0})).unwrap();
        assert!(apply(&mut s, true).is_err());
        assert_eq!(s.actions.list.len(), 2);
        s.execute("actions.stop", json!({})).unwrap();
        s.actions.held_playback = Some(9);
        assert!(apply(&mut s, true).is_err());
    }
}
