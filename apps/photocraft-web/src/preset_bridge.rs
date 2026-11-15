//! Synchronous engine boundary for asynchronous browser storage. No browser handles cross the
//! engine's `Send` boundary. Byte buffers are shared with the single in-flight transaction.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use photocraft_engine::{
    Session,
    actions_cmds::{ActionState, is_action_key},
    preset_store::{
        self, ACTIONS_FILE, ActionsMigration, INDEX_FILE, LEGACY_ACTIONS_FILE, MAX_ACTIONS_BYTES, MAX_GROUP_BYTES, MAX_GROUPS, MAX_INDEX_BYTES,
        MAX_PATTERNS_BYTES, MAX_PRESETS_PER_GROUP, MAX_STORE_BYTES, MAX_TIP_BYTES, MIGRATED_ACTIONS_FILE, PATTERNS_FILE, PresetBackend,
    },
};
use serde_json::{Value, json};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActionKeys(BTreeMap<String, String>, BTreeMap<String, Vec<String>>);

impl ActionKeys {
    pub fn from_preferences(prefs: &photocraft_engine::prefs::Preferences) -> Self {
        Self(
            prefs.shortcuts.iter().filter(|(key, _)| is_action_key(key)).map(|(key, value)| (key.clone(), value.clone())).collect(),
            prefs.alternate_shortcuts.iter().filter(|(key, _)| is_action_key(key)).map(|(key, value)| (key.clone(), value.clone())).collect(),
        )
    }

    pub fn from_legacy(prefs: &Value) -> Self {
        Self(
            action_entries(prefs, "shortcuts").filter_map(|(key, value)| Some((key.clone(), value.as_str()?.into()))).collect(),
            action_entries(prefs, "alternateShortcuts").filter_map(|(key, value)| Some((key.clone(), serde_json::from_value(value.clone()).ok()?))).collect(),
        )
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(&(&self.0, &self.1)).map_err(|e| e.to_string())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let (primary, alternate) = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let keys = Self(primary, alternate);
        if !keys.0.keys().chain(keys.1.keys()).all(|key| is_action_key(key)) {
            return Err("Invalid browser Action shortcuts".into());
        }
        Ok(keys)
    }

    pub fn overlay(&self, prefs: &mut Value) -> Result<(), String> {
        replace_action_keys(prefs, &json!({"shortcuts": self.0, "alternateShortcuts": self.1}))
    }
}

fn action_entries<'a>(prefs: &'a Value, section: &str) -> impl Iterator<Item = (&'a String, &'a Value)> {
    prefs.get(section).and_then(Value::as_object).into_iter().flatten().filter(|(key, _)| is_action_key(key))
}

/// Keep the pre-migration Action keys in localStorage while ordinary preferences are saved.
pub fn replace_action_keys(prefs: &mut Value, source: &Value) -> Result<(), String> {
    let object = prefs.as_object_mut().ok_or("Invalid browser preferences")?;
    for section in ["shortcuts", "alternateShortcuts"] {
        let bindings = object.entry(section).or_insert_with(|| json!({}));
        let bindings = bindings.as_object_mut().ok_or("Invalid browser shortcut map")?;
        bindings.retain(|key, _| !is_action_key(key));
        bindings.extend(action_entries(source, section).map(|(key, value)| (key.clone(), value.clone())));
    }
    Ok(())
}

/// Exact definitions and optional v2 paired keys whose retirement one transaction owns.
#[derive(Clone, Debug)]
pub struct Migration {
    pub legacy: Arc<[u8]>,
    pub atn: Arc<[u8]>,
    pub published: bool,
    pub legacy_pair: Option<(u64, ActionKeys)>,
    pub keys: ActionKeys,
}
impl Migration {
    /// Byte identity alone is insufficient for a legacy v2 paired record.
    pub fn validate_pairs(
        &self,
        action: &ActionWrite,
        committed: bool,
        atn: Option<(u64, Option<ActionKeys>)>,
        legacy: Option<(u64, ActionKeys)>,
        backup: Option<(u64, ActionKeys)>,
    ) -> Result<(), WriteError> {
        let matching = if committed {
            atn == Some((action.revision, Some(action.keys.clone()))) && backup == self.legacy_pair
        } else {
            legacy == self.legacy_pair
                && atn.map_or(action.base == 0, |(revision, keys)| revision == action.base && keys.as_ref().is_none_or(|keys| keys == &self.keys))
        };
        if !matching {
            return Err(WriteError::Stale("Actions migration generation or shortcuts changed; all records retained unchanged. Reopen this tab.".into()));
        }
        Ok(())
    }
    pub fn already_committed(&self, atn: Option<&[u8]>, legacy: Option<&[u8]>, backup: Option<&[u8]>) -> Result<bool, WriteError> {
        if atn == Some(self.atn.as_ref()) && legacy.is_none() && backup == Some(self.legacy.as_ref()) {
            return Ok(true);
        }
        if legacy == Some(self.legacy.as_ref()) && backup.is_none() && atn == self.published.then_some(self.atn.as_ref()) {
            return Ok(false);
        }
        Err(WriteError::Stale("Actions migration records changed; original, ATN and backup retained unchanged. Reopen this tab before saving Actions.".into()))
    }
}

#[derive(Clone)]
pub struct ActionWrite {
    pub base: u64,
    pub revision: u64,
    pub definitions: Option<Arc<[u8]>>,
    pub keys: ActionKeys,
    pub migration: Option<Migration>,
}

#[derive(Clone)]
pub enum Batch {
    Files(BTreeMap<String, Option<Arc<[u8]>>>),
    Actions(ActionWrite),
}

#[derive(Clone, Debug)]
pub enum WriteError {
    Aborted(String),
    Stale(String),
    Uncertain(String),
    Blocked(String),
}

impl WriteError {
    pub fn message(&self) -> &str {
        match self {
            Self::Aborted(message) | Self::Stale(message) | Self::Uncertain(message) | Self::Blocked(message) => message,
        }
    }
}

impl From<&str> for WriteError {
    fn from(message: &str) -> Self {
        Self::Aborted(message.into())
    }
}

impl From<String> for WriteError {
    fn from(message: String) -> Self {
        Self::Aborted(message)
    }
}

pub enum ActionRecovery {
    Reopen,
    Preserve,
}

pub enum SaveWarning {
    Actions(String, Option<ActionRecovery>),
    Presets(String),
}

/// Three sampled images per preset (tip, dual tip, texture), plus group/index/action/pattern records.
pub const MAX_RECORDS: usize = MAX_GROUPS + MAX_PRESETS_PER_GROUP * 3 + 5;

#[derive(Default)]
struct State {
    files: BTreeMap<String, Arc<[u8]>>,
    bytes: u64,
    dirty: BTreeSet<String>,
    writing: bool,
    unavailable: bool,
    actions_unsynced: bool,
    verified_actions_rev: Option<u64>,
    action_quarantine: Option<String>,
    action_error: Option<WriteError>,
    action_revision: u64,
    legacy_pair: Option<(u64, ActionKeys)>,
    migration: Option<Migration>,
    committed_keys: ActionKeys,
    current_keys: ActionKeys,
    error: Option<String>,
    rejected: BTreeMap<String, String>,
    preserve_tips: bool,
}

#[derive(Clone, Default)]
pub struct Bridge(Arc<Mutex<State>>);

pub fn session(backend: Bridge) -> (Session, Vec<String>) {
    let mut session = Session::new();
    let opened = preset_store::open(Box::new(backend.clone()));
    if opened.store.actions_load_failed() {
        backend.quarantine_actions("Browser Actions definitions could not be loaded".into());
    }
    // An unreadable group may reference any tip, including one shared with a good group.
    // Keep all tips until the damaged library can be recovered; unrelated edits still save.
    backend.state().preserve_tips = !opened.warnings.is_empty();
    let warnings = session.attach_preset_store(opened);
    (session, warnings)
}

/// Check metadata before copying a browser record into Rust memory.
pub fn record_limit(name: &str) -> Result<u64, String> {
    let leaf = name.strip_prefix("tips/").unwrap_or(name);
    if leaf.is_empty() || leaf.len() > 128 || leaf.starts_with('.') || !leaf.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.')) {
        return Err("invalid preset record name".into());
    }
    match name {
        INDEX_FILE => Ok(MAX_INDEX_BYTES),
        ACTIONS_FILE | LEGACY_ACTIONS_FILE | MIGRATED_ACTIONS_FILE => Ok(MAX_ACTIONS_BYTES),
        PATTERNS_FILE => Ok(MAX_PATTERNS_BYTES),
        _ if name.starts_with("tips/") && leaf.ends_with(".pctip") => Ok(MAX_TIP_BYTES),
        _ if !name.contains('/') && name.ends_with(".pcbrushes") => Ok(MAX_GROUP_BYTES),
        _ => Err("unknown preset record type".into()),
    }
}

impl Bridge {
    fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn insert(s: &mut State, name: &str, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() as u64 > record_limit(name)? {
            return Err(format!("{name}: preset record too large"));
        }
        let old = s.files.get(name).map_or(0, |b| b.len() as u64);
        let size = s.bytes.saturating_sub(old).saturating_add(bytes.len() as u64);
        if size > MAX_STORE_BYTES || (!s.files.contains_key(name) && s.files.len() >= MAX_RECORDS) {
            return Err("browser preset store is full".into());
        }
        s.files.insert(name.to_string(), Arc::from(bytes));
        if name == ACTIONS_FILE {
            s.verified_actions_rev = None;
        }
        s.bytes = size;
        Ok(())
    }

    pub fn load(&self, name: &str, bytes: Vec<u8>) -> Result<(), String> {
        Self::insert(&mut self.state(), name, &bytes)
    }

    pub fn load_action_keys(&self, revision: u64, keys: ActionKeys) {
        let mut s = self.state();
        s.action_revision = revision;
        s.committed_keys = keys.clone();
        s.current_keys = keys.clone();
        if revision == 0
            && !s.writing
            && let Some(migration) = &mut s.migration
        {
            migration.keys = keys;
        }
    }

    pub fn load_legacy_action_keys(&self, revision: u64, keys: ActionKeys) {
        let mut s = self.state();
        s.legacy_pair = Some((revision, keys.clone()));
        if s.action_revision == 0 {
            s.current_keys = keys.clone();
            s.committed_keys = keys;
        }
    }

    pub fn committed_action_keys(&self) -> Option<ActionKeys> {
        let s = self.state();
        (s.action_quarantine.is_none() && (s.action_revision != 0 || s.legacy_pair.is_some())).then(|| s.committed_keys.clone())
    }

    pub fn quarantine_actions(&self, error: String) {
        let mut s = self.state();
        s.action_quarantine.get_or_insert(error);
        s.committed_keys = ActionKeys::default();
        s.current_keys = ActionKeys::default();
    }

    pub fn stage_action_keys(&self, keys: ActionKeys) {
        self.state().current_keys = keys;
    }

    pub fn verify_action_definitions(&self, actions: &ActionState) {
        let mut s = self.state();
        if s.verified_actions_rev == Some(actions.rev) {
            return;
        }
        s.actions_unsynced = match s.files.get(ACTIONS_FILE) {
            Some(bytes) => !photocraft_engine::actions_cmds::matches_library_definition(bytes, actions),
            None => !(actions.list.is_empty() && actions.sets.is_empty() && actions.set_rows.is_empty()),
        };
        if !s.actions_unsynced {
            s.verified_actions_rev = Some(actions.rev);
        }
    }

    pub fn begin(&self) -> Option<Batch> {
        let mut s = self.state();
        if s.writing || s.unavailable {
            return None;
        }
        // A stale or unconfirmed Action generation must not hold brush and pattern edits.
        if s.error.is_none() && s.dirty.iter().any(|name| !matches!(name.as_str(), ACTIONS_FILE | LEGACY_ACTIONS_FILE | MIGRATED_ACTIONS_FILE)) {
            let files = s
                .dirty
                .iter()
                .filter(|name| !matches!(name.as_str(), ACTIONS_FILE | LEGACY_ACTIONS_FILE | MIGRATED_ACTIONS_FILE))
                .map(|name| (name.clone(), s.files.get(name).cloned()))
                .collect();
            s.dirty.retain(|name| matches!(name.as_str(), ACTIONS_FILE | LEGACY_ACTIONS_FILE | MIGRATED_ACTIONS_FILE));
            s.writing = true;
            return Some(Batch::Files(files));
        }
        // A refused definition write must never be paired with newer bindings and old definitions.
        if s.actions_unsynced
            || s.action_quarantine.is_some()
            || s.action_error.is_some()
            || s.rejected.contains_key(ACTIONS_FILE)
            || (!s.dirty.contains(ACTIONS_FILE) && s.current_keys == s.committed_keys)
        {
            return None;
        }
        let Some(revision) = s.action_revision.checked_add(1) else {
            s.action_error = Some(WriteError::Blocked("Browser Action revision limit reached".into()));
            return None;
        };
        s.dirty.remove(ACTIONS_FILE);
        s.writing = true;
        let migration = s.migration.clone();
        let definitions = migration.as_ref().map(|migration| migration.atn.clone()).or_else(|| s.files.get(ACTIONS_FILE).cloned());
        let keys = migration.as_ref().map(|migration| migration.keys.clone()).unwrap_or_else(|| s.current_keys.clone());
        if migration.is_some() {
            s.dirty.remove(LEGACY_ACTIONS_FILE);
            s.dirty.remove(MIGRATED_ACTIONS_FILE);
            if definitions != s.files.get(ACTIONS_FILE).cloned() {
                s.dirty.insert(ACTIONS_FILE.into());
            }
        }
        Some(Batch::Actions(ActionWrite { base: s.action_revision, revision, definitions, keys, migration }))
    }

    pub fn finish(&self, batch: Batch, result: Result<(), WriteError>) {
        let mut s = self.state();
        s.writing = false;
        match (batch, result) {
            (Batch::Files(files), Err(error)) => {
                // Requeue names, never old bytes: edits/deletes made during the write win.
                s.dirty.extend(files.into_keys());
                s.error = Some(error.message().into());
            }
            (Batch::Actions(_), Err(error)) => {
                s.dirty.insert(ACTIONS_FILE.into());
                s.action_error = Some(error);
            }
            (Batch::Actions(actions), Ok(())) => {
                s.action_revision = actions.revision;
                s.committed_keys = actions.keys;
                if actions.migration.is_some() {
                    s.migration = None;
                    s.legacy_pair = None;
                }
            }
            (Batch::Files(_), Ok(())) => {}
        }
    }

    pub fn unavailable(&self, error: String) {
        let mut s = self.state();
        s.unavailable = true;
        s.error = Some(error);
    }

    pub fn retry(&self) {
        let mut s = self.state();
        if !s.unavailable {
            s.error = None;
        }
        // An abort has no durable effect. Stale and uncertain writes require a fresh read.
        if matches!(s.action_error.as_ref(), Some(WriteError::Aborted(_))) {
            s.action_error = None;
        }
    }

    pub fn error(&self) -> Option<SaveWarning> {
        let s = self.state();
        s.rejected
            .iter()
            .next()
            .map(
                |(name, error)| {
                    if name == ACTIONS_FILE { SaveWarning::Actions(error.clone(), Some(ActionRecovery::Preserve)) } else { SaveWarning::Presets(error.clone()) }
                },
            )
            .or_else(|| s.error.clone().map(SaveWarning::Presets))
            .or_else(|| {
                s.action_error.as_ref().map(|error| {
                    let recovery = match error {
                        WriteError::Aborted(_) => None,
                        WriteError::Stale(_) | WriteError::Uncertain(_) => Some(ActionRecovery::Reopen),
                        WriteError::Blocked(_) => Some(ActionRecovery::Preserve),
                    };
                    SaveWarning::Actions(error.message().to_owned(), recovery)
                })
            })
            .or_else(|| s.action_quarantine.clone().map(|error| SaveWarning::Actions(error, Some(ActionRecovery::Preserve))))
            .or_else(|| {
                s.actions_unsynced.then(|| SaveWarning::Actions("Browser Actions definitions could not be staged".into(), Some(ActionRecovery::Preserve)))
            })
    }

    pub fn unsaved(&self) -> bool {
        let s = self.state();
        s.writing || s.actions_unsynced || !s.dirty.is_empty() || !s.rejected.is_empty() || s.current_keys != s.committed_keys
    }
}

impl PresetBackend for Bridge {
    fn list(&self) -> Result<Vec<(String, u64)>, String> {
        Ok(self.state().files.iter().map(|(k, b)| (k.clone(), b.len() as u64)).collect())
    }

    fn read(&self, name: &str, max: u64) -> Result<Vec<u8>, String> {
        let s = self.state();
        let bytes = s.files.get(name).ok_or_else(|| format!("{name}: not found"))?;
        if bytes.len() as u64 > max {
            return Err(format!("{name}: preset record too large"));
        }
        Ok(bytes.to_vec())
    }

    fn migrate_actions(&self, legacy: &[u8], atn: &[u8], published: bool) -> Result<ActionsMigration, String> {
        let mut s = self.state();
        let result = (|| -> Result<ActionsMigration, String> {
            if legacy.len() as u64 > MAX_ACTIONS_BYTES || atn.len() as u64 > MAX_ACTIONS_BYTES {
                return Err("Actions migration exceeds file limit".into());
            }
            if s.migration.is_some() {
                return Err("an Actions migration is already pending".into());
            }
            if s.files.get(LEGACY_ACTIONS_FILE).map(AsRef::as_ref) != Some(legacy) {
                return Err("legacy Actions input changed before migration".into());
            }
            match s.files.get(ACTIONS_FILE) {
                Some(bytes) if published && bytes.as_ref() == atn => {}
                None if !published => {}
                _ => return Err("Actions ATN changed before migration".into()),
            }
            if s.files.contains_key(MIGRATED_ACTIONS_FILE) {
                return Err("legacy backup already exists; original and backup retained unchanged".into());
            }
            let size = s.bytes.checked_add(if published { 0 } else { atn.len() as u64 }).ok_or("store size overflow")?;
            let records = s.files.len() + usize::from(!published);
            let pending = [ACTIONS_FILE, LEGACY_ACTIONS_FILE, MIGRATED_ACTIONS_FILE].iter().filter(|name| !s.dirty.contains(**name)).count();
            if size > MAX_STORE_BYTES || records > MAX_RECORDS || s.dirty.len() + pending > MAX_RECORDS {
                return Err("browser preset store is full".into());
            }
            let migration =
                Migration { legacy: Arc::from(legacy), atn: Arc::from(atn), published, legacy_pair: s.legacy_pair.clone(), keys: s.current_keys.clone() };
            // One lock, with all validation above mutation. add(), not put(), protects the backup
            // and a first ATN publication in the actual IndexedDB transaction as well.
            s.files.insert(ACTIONS_FILE.into(), migration.atn.clone());
            s.files.insert(MIGRATED_ACTIONS_FILE.into(), migration.legacy.clone());
            s.files.remove(LEGACY_ACTIONS_FILE);
            s.bytes = size;
            s.dirty.extend([ACTIONS_FILE.into(), LEGACY_ACTIONS_FILE.into(), MIGRATED_ACTIONS_FILE.into()]);
            s.migration = Some(migration);
            s.actions_unsynced = false;
            s.verified_actions_rev = None;
            Ok(ActionsMigration::Staged)
        })();
        if let Err(error) = &result {
            s.rejected.insert(ACTIONS_FILE.into(), error.clone());
        } else {
            s.rejected.remove(ACTIONS_FILE);
        }
        result
    }

    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        if matches!(name, LEGACY_ACTIONS_FILE | MIGRATED_ACTIONS_FILE) {
            return Err("legacy Actions records are migration-only".into());
        }
        let mut s = self.state();
        if name == ACTIONS_FILE
            && let Some(error) = s.action_quarantine.clone()
        {
            s.rejected.insert(name.into(), error.clone());
            return Err(error);
        }
        if s.files.get(name).is_some_and(|old| old.as_ref() == bytes) {
            if name == ACTIONS_FILE {
                s.actions_unsynced = false;
                s.verified_actions_rev = None;
            }
            s.rejected.remove(name);
            return Ok(());
        }
        if !s.dirty.contains(name) && s.dirty.len() >= MAX_RECORDS {
            let e = "too many pending preset records".to_string();
            s.rejected.insert(name.to_string(), e.clone());
            return Err(e);
        }
        if let Err(e) = Self::insert(&mut s, name, bytes) {
            s.rejected.insert(name.to_string(), e.clone());
            return Err(e);
        }
        s.dirty.insert(name.to_string());
        if name == ACTIONS_FILE {
            s.actions_unsynced = false;
        }
        // This current replacement resolves only its own omitted edit. Pending transactions
        // still keep unsaved() true; Retry or an older successful batch cannot clear it.
        s.rejected.remove(name);
        Ok(())
    }

    fn remove(&self, name: &str) -> Result<(), String> {
        if matches!(name, LEGACY_ACTIONS_FILE | MIGRATED_ACTIONS_FILE) {
            return Err("legacy Actions records are migration-only".into());
        }
        record_limit(name)?;
        let mut s = self.state();
        if name == ACTIONS_FILE
            && let Some(error) = s.action_quarantine.clone()
        {
            s.rejected.insert(name.into(), error.clone());
            return Err(error);
        }
        if s.preserve_tips && name.starts_with("tips/") {
            s.rejected.remove(name);
            return Ok(());
        }
        if !s.dirty.contains(name) && s.dirty.len() >= MAX_RECORDS {
            let e = "too many pending preset records".to_string();
            s.rejected.insert(name.to_string(), e.clone());
            return Err(e);
        }
        if let Some(bytes) = s.files.remove(name) {
            s.bytes = s.bytes.saturating_sub(bytes.len() as u64);
            s.dirty.insert(name.to_string());
        }
        s.rejected.remove(name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_psd::abr::{AbrSample, LegacyBrush, LegacyTip, write_v12};
    use serde_json::json;

    fn action_batch(batch: &Batch) -> &ActionWrite {
        let Batch::Actions(action) = batch else { panic!("expected an Actions transaction") };
        action
    }

    fn file_batch(batch: &Batch) -> &BTreeMap<String, Option<Arc<[u8]>>> {
        let Batch::Files(files) = batch else { panic!("expected an ordinary preset transaction") };
        files
    }

    #[test]
    fn action_pair_acknowledges_only_the_completed_snapshot() {
        let bridge = Bridge::default();
        let legacy = ActionKeys::from_legacy(&json!({"shortcuts":{"actions.play:Target":"F6","edit.fill":"F7"}}));
        let replacement = ActionKeys::default();
        bridge.load_action_keys(0, legacy.clone());
        assert!(!bridge.unsaved());
        bridge.write(ACTIONS_FILE, b"new library").unwrap();
        bridge.stage_action_keys(replacement.clone());
        let batch = bridge.begin().unwrap();
        let action = action_batch(&batch);
        assert_eq!((action.base, action.revision), (0, 1));
        assert_eq!(action.definitions.as_deref(), Some(&b"new library"[..]));
        assert_eq!(action.keys, replacement);
        assert!(bridge.unsaved());
        bridge.finish(batch, Err(WriteError::Aborted("transaction failed".into())));
        assert!(bridge.unsaved());
        assert_eq!(bridge.committed_action_keys(), None);
        bridge.retry();
        let batch = bridge.begin().unwrap();
        bridge.write(ACTIONS_FILE, b"newer library").unwrap();
        bridge.stage_action_keys(legacy.clone());
        bridge.finish(batch, Ok(()));
        assert_eq!(bridge.committed_action_keys(), Some(replacement));
        let batch = bridge.begin().unwrap();
        let action = action_batch(&batch);
        assert_eq!((action.base, action.revision), (1, 2));
        assert_eq!(action.definitions.as_deref(), Some(&b"newer library"[..]));
        assert_eq!(action.keys, legacy);
        bridge.finish(batch, Ok(()));
        assert!(!bridge.unsaved());
    }

    #[test]
    fn authoritative_action_keys_clear_stale_legacy_values_only() {
        let legacy = json!({
            "shortcuts":{"actions.play:Target":"F6", "edit.fill":"F7"},
            "alternateShortcuts":{"actions.play:Target":["F3"], "edit.fill":["F8"]},
            "shortcutSets":{"Mine":{"shortcuts":{"edit.fill":"F5"}}},
        });
        let mut prefs = legacy.clone();
        let cleared = ActionKeys::decode(&ActionKeys::default().encode().unwrap()).unwrap();
        assert!(ActionKeys::decode(br#"[{"edit.fill":"F6"},{}]"#).is_err());
        cleared.overlay(&mut prefs).unwrap();
        assert_eq!(prefs["shortcuts"], json!({"edit.fill":"F7"}));
        assert_eq!(prefs["alternateShortcuts"], json!({"edit.fill":["F8"]}));
        assert_eq!(prefs["shortcutSets"], legacy["shortcutSets"]);
        replace_action_keys(&mut prefs, &legacy).unwrap();
        assert_eq!(prefs, legacy);

        let mut session = Session::new();
        session.load_prefs_json(&legacy.to_string()).unwrap();
        let bridge = Bridge::default();
        bridge.stage_action_keys(ActionKeys::from_preferences(session.prefs()));
        let batch = bridge.begin().unwrap();
        assert_eq!(action_batch(&batch).keys, ActionKeys::from_legacy(&legacy));
        bridge.finish(batch, Ok(()));
    }

    #[test]
    fn unmatched_definitions_cannot_commit_new_keys() {
        let bridge = Bridge::default();
        let mut actions = ActionState::default();
        actions.list.push(photocraft_engine::actions_cmds::Action::new("New action", Vec::new()));
        bridge.verify_action_definitions(&actions);
        bridge.stage_action_keys(ActionKeys::from_legacy(&json!({"shortcuts":{"actions.play:New action":"F6"}})));
        bridge.write("index.json", b"unrelated").unwrap();
        let batch = bridge.begin().unwrap();
        assert!(file_batch(&batch).contains_key("index.json"));
        bridge.finish(batch, Ok(()));
        assert!(bridge.unsaved());
        assert!(bridge.begin().is_none());
    }

    #[test]
    fn stale_or_uncertain_actions_do_not_block_ordinary_presets() {
        for error in [WriteError::Stale("stale".into()), WriteError::Uncertain("unknown commit".into())] {
            let reason = error.message().to_owned();
            let bridge = Bridge::default();
            bridge.load_action_keys(5, ActionKeys::default());
            bridge.write(ACTIONS_FILE, b"local actions").unwrap();
            bridge.write(INDEX_FILE, b"ordinary index").unwrap();
            let batch = bridge.begin().unwrap();
            assert_eq!(file_batch(&batch).len(), 1);
            bridge.finish(batch, Ok(()));
            let batch = bridge.begin().unwrap();
            assert_eq!(action_batch(&batch).base, 5);
            bridge.write(INDEX_FILE, b"new brush index").unwrap();
            bridge.write(PATTERNS_FILE, b"new patterns").unwrap();
            bridge.finish(batch, Err(error));
            bridge.retry();
            let batch = bridge.begin().unwrap();
            assert_eq!(file_batch(&batch)[INDEX_FILE].as_deref(), Some(&b"new brush index"[..]));
            assert_eq!(file_batch(&batch)[PATTERNS_FILE].as_deref(), Some(&b"new patterns"[..]));
            bridge.finish(batch, Ok(()));
            assert!(bridge.unsaved(), "the Action intent is still pending");
            assert!(bridge.begin().is_none(), "retry must wait for a fresh Action generation");
            assert!(matches!(bridge.error(), Some(SaveWarning::Actions(message, Some(ActionRecovery::Reopen))) if message == reason));
        }
    }

    #[test]
    fn exhausted_action_revisions_warn_to_preserve_actions_without_promising_a_reopen_fix() {
        let bridge = Bridge::default();
        bridge.load_action_keys(u64::MAX, ActionKeys::default());
        bridge.write(ACTIONS_FILE, b"local actions").unwrap();
        assert!(bridge.begin().is_none());
        assert!(matches!(bridge.error(), Some(SaveWarning::Actions(_, Some(ActionRecovery::Preserve)))));
        bridge.retry();
        assert!(bridge.begin().is_none());
    }

    #[test]
    fn malformed_paired_definitions_quarantine_only_actions() {
        let bridge = Bridge::default();
        let (mut seed, _) = session(bridge.clone());
        import(&mut seed);
        bridge.finish(bridge.begin().unwrap(), Ok(()));
        let malformed = b"damaged Action definitions";
        bridge.load(ACTIONS_FILE, malformed.to_vec()).unwrap();
        let keys = ActionKeys::from_legacy(&json!({"shortcuts":{"actions.play:Target":"F6"}}));
        bridge.load_action_keys(2, keys.clone());
        let (session, warnings) = session(bridge.clone());
        assert!(!warnings.is_empty());
        assert!(session.actions.list.is_empty());
        assert!(session.tools.presets.iter().any(|preset| preset.name == "Browser seam brush"));
        assert_eq!(bridge.committed_action_keys(), None);
        assert_eq!(bridge.read(ACTIONS_FILE, 1024).unwrap(), malformed);
        assert!(bridge.write(ACTIONS_FILE, b"would overwrite damaged data").is_err());
        assert!(matches!(bridge.error(), Some(SaveWarning::Actions(_, Some(ActionRecovery::Preserve)))));
        assert!(bridge.remove(ACTIONS_FILE).is_err());
        bridge.stage_action_keys(keys);
        bridge.write(INDEX_FILE, b"unrelated index").unwrap();
        let batch = bridge.begin().unwrap();
        assert_eq!(file_batch(&batch)[INDEX_FILE].as_deref(), Some(&b"unrelated index"[..]));
        bridge.finish(batch, Ok(()));
        assert!(bridge.begin().is_none());
    }

    fn import(s: &mut Session) {
        let abr = write_v12(
            2,
            &[LegacyBrush {
                name: "Browser seam brush".into(),
                spacing: 25,
                anti_alias: true,
                tip: LegacyTip::Sampled(AbrSample { id: String::new(), width: 6, height: 4, depth: 8, data: vec![255; 24] }),
            }],
            false,
        )
        .unwrap();
        s.execute("brush.presets.importAbr", json!({"data": photocraft_paint::tile::b64_encode(&abr), "group": "Browser samples"})).unwrap();
    }

    #[test]
    fn browser_session_reloads_imported_tip_settings_group_and_order() {
        let backend = Bridge::default();
        let (mut first, _) = session(backend.clone());
        import(&mut first);
        let (second, warnings) = session(backend);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(first.tools.presets, second.tools.presets);
    }
    #[test]
    fn pending_writes_coalesce_and_only_one_transaction_runs() {
        let b = Bridge::default();
        b.write("index.json", b"one").unwrap();
        b.write("index.json", b"two").unwrap();
        let batch = b.begin().unwrap();
        assert_eq!(file_batch(&batch).get("index.json").unwrap().as_deref(), Some(&b"two"[..]));
        b.remove("index.json").unwrap();
        assert!(b.begin().is_none());
        assert!(b.unsaved());
        b.finish(batch, Ok(()));
        let batch = b.begin().unwrap();
        assert!(file_batch(&batch).get("index.json").unwrap().is_none());
        b.finish(batch, Ok(()));
        assert!(!b.unsaved());
    }

    #[test]
    fn failed_transaction_retains_latest_writes_until_explicit_retry() {
        let b = Bridge::default();
        b.write("index.json", b"old").unwrap();
        let batch = b.begin().unwrap();
        b.write("index.json", b"new").unwrap();
        b.finish(batch, Err(WriteError::Aborted("QuotaExceededError".into())));
        assert!(b.unsaved());
        assert!(matches!(b.error(), Some(SaveWarning::Presets(message)) if message == "QuotaExceededError"));
        assert!(b.begin().is_none());
        b.retry();
        let batch = b.begin().unwrap();
        assert_eq!(file_batch(&batch).get("index.json").unwrap().as_deref(), Some(&b"new"[..]));
        b.finish(batch, Ok(()));
        assert!(!b.unsaved());
        assert!(b.error().is_none());
    }

    #[test]
    fn browser_deletion_and_reordering_survive_reload() {
        let b = Bridge::default();
        let (mut s, _) = session(b.clone());
        import(&mut s);
        let deleted = s.tools.presets[0].name.clone();
        s.execute("brush.presets.delete", json!({"name": deleted})).unwrap();
        s.execute("brush.presets.moveGroup", json!({"group":"Browser samples", "index":0})).unwrap();
        let persisted = b.begin().unwrap();
        let restored = Bridge::default();
        for (key, value) in file_batch(&persisted) {
            if let Some(bytes) = value {
                restored.load(key, bytes.to_vec()).unwrap();
            }
        }
        b.finish(persisted, Ok(()));
        let (loaded, warnings) = session(restored);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(s.tools.presets, loaded.tools.presets);
        s.execute("brush.presets.delete", json!({"name":"Browser seam brush"})).unwrap();
        let (loaded, _) = session(b);
        assert_eq!(s.tools.presets, loaded.tools.presets);
    }

    #[test]
    fn corrupt_record_is_skipped_with_warning_and_kept_for_recovery() {
        let b = Bridge::default();
        b.load("bad.pcbrushes", b"corrupt".to_vec()).unwrap();
        let (s, warnings) = session(b.clone());
        assert_eq!(s.tools.presets, Session::new().tools.presets);
        assert!(warnings.iter().any(|w| w.contains("bad.pcbrushes")));
        assert_eq!(b.read("bad.pcbrushes", 100).unwrap(), b"corrupt");
    }

    #[test]
    fn rejects_invalid_and_oversized_records_before_copying() {
        assert!(record_limit("../index.json").is_err());
        assert!(record_limit("unknown.bin").is_err());
        assert_eq!(record_limit("index.json").unwrap(), MAX_INDEX_BYTES);
        assert_eq!(record_limit("tips/a.pctip").unwrap(), MAX_TIP_BYTES);
        let b = Bridge::default();
        b.load("index.json", b"initial".to_vec()).unwrap();
        assert!(!b.unsaved(), "hydration is not a user edit");
        assert!(b.read("index.json", 2).is_err());
        assert!(b.write("index.json", &vec![0; MAX_INDEX_BYTES as usize + 1]).is_err());
        assert!(b.error().is_some());
        b.retry();
        assert!(b.error().is_some(), "staging errors cannot be cleared by retrying older writes");
        assert_eq!(b.read("index.json", 100).unwrap(), b"initial");
    }

    #[test]
    fn accepted_replacement_resolves_only_its_own_staging_rejection() {
        let b = Bridge::default();
        b.load("index.json", b"initial".to_vec()).unwrap();
        let oversized = vec![0; MAX_INDEX_BYTES as usize + 1];
        assert!(b.write("index.json", &oversized).is_err());
        b.retry();
        assert!(b.error().is_some(), "retrying older bytes cannot save the rejected edit");
        b.write("index.json", b"repaired").unwrap();
        assert!(b.unsaved(), "accepted replacement still needs its transaction");
        b.finish(b.begin().unwrap(), Ok(()));
        assert!(b.error().is_none());
        assert!(!b.unsaved());
        assert_eq!(b.read("index.json", 100).unwrap(), b"repaired");

        assert!(b.write("index.json", &oversized).is_err());
        assert!(b.write("other.pcbrushes", &vec![0; MAX_GROUP_BYTES as usize + 1]).is_err());
        b.write("index.json", b"repaired").unwrap();
        assert!(b.error().is_some(), "an unchanged accepted record cannot clear another record's rejection");
        assert!(b.unsaved());
        b.remove("other.pcbrushes").unwrap();
        assert!(b.error().is_none());
        assert!(!b.unsaved(), "discarding the rejected absent record resolves its pending edit");
    }

    #[test]
    fn older_successful_transaction_does_not_resolve_an_omitted_edit() {
        let b = Bridge::default();
        b.write("index.json", b"older").unwrap();
        let older = b.begin().unwrap();
        assert!(b.write("index.json", &vec![0; MAX_INDEX_BYTES as usize + 1]).is_err());
        b.finish(older, Ok(()));
        assert!(b.error().is_some());
        assert!(b.unsaved());
        b.write("index.json", b"older").unwrap();
        assert!(b.error().is_none(), "explicitly restoring the already saved bytes resolves the omitted edit");
        assert!(!b.unsaved());
    }

    #[test]
    fn denied_storage_keeps_imports_in_memory_and_marks_them_unsaved() {
        let b = Bridge::default();
        b.unavailable("SecurityError".into());
        let (mut s, _) = session(b.clone());
        assert!(!b.unsaved(), "nothing is lost before the first change, so the web shell shows no banner");
        import(&mut s);
        assert!(s.tools.presets.iter().any(|p| p.name == "Browser seam brush"));
        assert!(b.unsaved());
        assert!(matches!(b.error(), Some(SaveWarning::Presets(message)) if message == "SecurityError"));
        assert!(b.begin().is_none());
    }
    #[test]
    fn corrupt_group_and_shared_tip_survive_unrelated_save_delete_and_reload() {
        let b = Bridge::default();
        let (mut s, _) = session(b.clone());
        import(&mut s);
        let tip = b.list().unwrap().into_iter().find(|(n, _)| n.starts_with("tips/")).unwrap().0;
        b.load("damaged.pcbrushes", b"recoverable broken group".to_vec()).unwrap();
        let (mut s, warnings) = session(b.clone());
        assert!(!warnings.is_empty());
        s.execute("brush.presets.save", json!({"name":"Unrelated brush"})).unwrap();
        s.execute("brush.presets.delete", json!({"name":"Browser seam brush"})).unwrap();
        assert_eq!(b.read("damaged.pcbrushes", 100).unwrap(), b"recoverable broken group");
        assert!(b.read(&tip, MAX_TIP_BYTES).is_ok());
        let (reloaded, warnings) = session(b.clone());
        assert!(!warnings.is_empty());
        assert!(reloaded.tools.presets.iter().any(|p| p.name == "Unrelated brush"));
        assert!(!reloaded.tools.presets.iter().any(|p| p.name == "Browser seam brush"));
        assert_eq!(b.read("damaged.pcbrushes", 100).unwrap(), b"recoverable broken group");
        assert!(b.read(&tip, MAX_TIP_BYTES).is_ok());
    }
    #[test]
    fn indexed_brush_store_coexists_with_imported_gradient_preferences() {
        use craft_psd::grd::{GrdColor, GrdGradient, GrdStop, write};
        let b = Bridge::default();
        let (mut s, _) = session(b.clone());
        import(&mut s);
        let data = write(&[GrdGradient {
            name: "Browser gradient".into(),
            noise: false,
            stops: vec![
                GrdStop { location: 0.0, midpoint: 0.5, color: GrdColor::Foreground },
                GrdStop { location: 1.0, midpoint: 0.5, color: GrdColor::Rgb([1.0, 0.0, 0.0]) },
            ],
            opacity: vec![(0.0, 1.0, 0.5), (1.0, 0.5, 0.5)],
        }]);
        s.execute("gradient.presets.importGrd", json!({"data":photocraft_paint::tile::b64_encode(&data),"group":"Browser gradients"})).unwrap();
        let prefs = s.prefs_to_json();
        let (mut loaded, _) = session(b);
        loaded.load_prefs_json(&prefs).unwrap();
        assert_eq!(loaded.tools.presets, s.tools.presets);
        assert_eq!(loaded.presets.gradients, s.presets.gradients);
        assert!(loaded.presets.gradients.iter().any(|g| g.name == "Browser gradients"));
    }
    #[test]
    fn actions_migration_is_one_staged_batch_and_waits_for_completion() {
        let bridge = Bridge::default();
        bridge.load(LEGACY_ACTIONS_FILE, b"exact legacy".to_vec()).unwrap();
        let keys = ActionKeys::from_legacy(
            &json!({"shortcuts":{"actions.playById:11111111111111111111111111111111":"F6"},"alternateShortcuts":{"actions.play:Target":["F7"]}}),
        );
        bridge.load_legacy_action_keys(7, keys.clone());
        assert_eq!(bridge.migrate_actions(b"exact legacy", b"complete ATN", false).unwrap(), ActionsMigration::Staged);
        let batch = bridge.begin().unwrap();
        let action = action_batch(&batch);
        let migration = action.migration.as_ref().unwrap();
        assert_eq!(action.definitions.as_deref(), Some(&b"complete ATN"[..]));
        assert_eq!(action.keys, keys);
        assert_eq!(migration.legacy_pair, Some((7, keys)));
        assert_eq!(bridge.read(MIGRATED_ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), b"exact legacy");
        assert!(bridge.begin().is_none());
        assert!(bridge.unsaved());
        bridge.finish(batch, Ok(()));
        assert!(!bridge.unsaved());
    }

    #[test]
    fn actions_migration_abort_retries_fixed_bytes_before_the_latest_atn() {
        let bridge = Bridge::default();
        bridge.load(LEGACY_ACTIONS_FILE, b"legacy".to_vec()).unwrap();
        bridge.migrate_actions(b"legacy", b"first ATN", false).unwrap();
        bridge.write(ACTIONS_FILE, b"edited before begin").unwrap();
        let batch = bridge.begin().unwrap();
        assert_eq!(action_batch(&batch).definitions.as_deref(), Some(&b"first ATN"[..]));
        bridge.write(ACTIONS_FILE, b"edited ATN").unwrap();
        bridge.finish(batch, Err(WriteError::Aborted("transaction aborted".into())));
        assert!(bridge.begin().is_none());
        bridge.retry();
        let batch = bridge.begin().unwrap();
        assert!(action_batch(&batch).migration.is_some());
        assert_eq!(action_batch(&batch).definitions.as_deref(), Some(&b"first ATN"[..]));
        bridge.finish(batch, Ok(()));
        assert!(bridge.unsaved());
        let next = bridge.begin().unwrap();
        assert!(action_batch(&next).migration.is_none());
        assert_eq!(action_batch(&next).definitions.as_deref(), Some(&b"edited ATN"[..]));
        bridge.finish(next, Ok(()));
        assert!(!bridge.unsaved());
    }

    #[test]
    fn acknowledged_migration_keeps_an_inflight_newer_edit_pending() {
        let bridge = Bridge::default();
        bridge.load(LEGACY_ACTIONS_FILE, b"legacy".to_vec()).unwrap();
        bridge.migrate_actions(b"legacy", b"first ATN", false).unwrap();
        let batch = bridge.begin().unwrap();
        let newer = ActionKeys::from_legacy(&json!({"shortcuts":{"actions.play:Newer":"F9"}}));
        bridge.write(ACTIONS_FILE, b"newer ATN").unwrap();
        bridge.stage_action_keys(newer.clone());
        bridge.finish(batch, Ok(()));
        assert!(bridge.unsaved());
        let next = bridge.begin().unwrap();
        assert!(action_batch(&next).migration.is_none());
        assert_eq!(action_batch(&next).keys, newer);
        assert_eq!(action_batch(&next).definitions.as_deref(), Some(&b"newer ATN"[..]));
        bridge.finish(next, Ok(()));
        assert!(!bridge.unsaved());
    }

    #[test]
    fn migration_recognizes_only_the_exact_fresh_or_committed_tuple() {
        for published in [false, true] {
            let migration =
                Migration { legacy: Arc::from(&b"legacy"[..]), atn: Arc::from(&b"ATN0"[..]), published, legacy_pair: None, keys: ActionKeys::default() };
            assert!(!migration.already_committed(published.then_some(&b"ATN0"[..]), Some(b"legacy"), None).unwrap());
            assert!(migration.already_committed(Some(b"ATN0"), None, Some(b"legacy")).unwrap());
            for (atn, legacy, backup) in [
                (Some(&b"ATN1"[..]), None, Some(&b"legacy"[..])),
                (Some(&b"ATN0"[..]), Some(&b"change"[..]), None),
                (Some(&b"ATN0"[..]), None, Some(&b"change"[..])),
                (Some(&b"ATN0"[..]), Some(&b"legacy"[..]), Some(&b"legacy"[..])),
                (None, None, Some(&b"legacy"[..])),
            ] {
                assert!(migration.already_committed(atn, legacy, backup).is_err());
            }
        }
    }

    #[test]
    fn actions_migration_validates_all_bindings_and_limits_before_mutation() {
        let bridge = Bridge::default();
        bridge.load(LEGACY_ACTIONS_FILE, b"legacy".to_vec()).unwrap();
        let before = bridge.list().unwrap();
        assert!(bridge.migrate_actions(b"changed", b"ATN", false).is_err());
        assert_eq!(bridge.list().unwrap(), before);
        bridge.state().bytes = MAX_STORE_BYTES;
        assert!(bridge.migrate_actions(b"legacy", b"ATN", false).is_err());
        assert_eq!(bridge.list().unwrap(), before);
        bridge.state().bytes = 6;
        bridge.load(MIGRATED_ACTIONS_FILE, b"other backup".to_vec()).unwrap();
        assert!(bridge.migrate_actions(b"legacy", b"ATN", false).is_err());
        assert_eq!(bridge.read(MIGRATED_ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), b"other backup");
        assert_eq!(bridge.read(LEGACY_ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), b"legacy");
        assert!(bridge.unsaved());
    }

    #[test]
    fn paired_migration_refuses_key_only_or_revision_only_changes_and_retains_retirement_identity() {
        let old = ActionKeys::from_legacy(&json!({"shortcuts":{"actions.play:Target":"F6"}}));
        let changed = ActionKeys::from_legacy(&json!({"shortcuts":{"actions.play:Target":"F7"}}));
        let migration = Migration {
            legacy: Arc::from(&b"legacy"[..]),
            atn: Arc::from(&b"ATN"[..]),
            published: false,
            legacy_pair: Some((4, old.clone())),
            keys: old.clone(),
        };
        let write = ActionWrite { base: 0, revision: 1, definitions: Some(migration.atn.clone()), keys: old.clone(), migration: Some(migration.clone()) };
        assert!(migration.validate_pairs(&write, false, None, Some((4, old.clone())), None).is_ok());
        assert!(migration.validate_pairs(&write, false, None, Some((4, changed.clone())), None).is_err());
        assert!(migration.validate_pairs(&write, false, None, Some((5, old.clone())), None).is_err());
        assert!(migration.validate_pairs(&write, true, Some((1, Some(old.clone()))), None, Some((4, old.clone()))).is_ok());
        assert!(migration.validate_pairs(&write, true, Some((1, Some(changed))), None, Some((4, old.clone()))).is_err());
        assert!(migration.validate_pairs(&write, true, Some((1, Some(old.clone()))), None, Some((5, old))).is_err());
    }
}
