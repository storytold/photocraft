//! Synchronous engine boundary for asynchronous browser storage. No browser handles cross the
//! engine's `Send` boundary. Byte buffers are shared with the single in-flight transaction.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use photocraft_engine::{
    Session,
    preset_store::{self, PresetBackend, *},
};

pub type Files = BTreeMap<String, Option<Arc<[u8]>>>;
#[derive(Clone)]
pub struct Migration {
    pub legacy: Arc<[u8]>,
    pub atn: Arc<[u8]>,
    pub published: bool,
}
impl Migration {
    /// Called only with bytes read under the writer's single readwrite transaction.
    pub fn already_committed(&self, atn: Option<&[u8]>, legacy: Option<&[u8]>, backup: Option<&[u8]>) -> Result<bool, String> {
        if atn == Some(self.atn.as_ref()) && legacy.is_none() && backup == Some(self.legacy.as_ref()) {
            return Ok(true);
        }
        if legacy == Some(self.legacy.as_ref()) && backup.is_none() && atn == self.published.then_some(self.atn.as_ref()) {
            return Ok(false);
        }
        Err("Actions migration records changed; original, ATN and backup retained unchanged".into())
    }
}
#[derive(Clone)]
pub struct Batch {
    pub files: Files,
    pub migration: Option<Migration>,
}
impl std::ops::Deref for Batch {
    type Target = Files;
    fn deref(&self) -> &Files {
        &self.files
    }
}
impl<'a> IntoIterator for &'a Batch {
    type Item = (&'a String, &'a Option<Arc<[u8]>>);
    type IntoIter = std::collections::btree_map::Iter<'a, String, Option<Arc<[u8]>>>;
    fn into_iter(self) -> Self::IntoIter {
        self.files.iter()
    }
}
/// Three sampled images per preset (tip, dual tip, texture), plus group/index/action/pattern records.
pub const MAX_RECORDS: usize = MAX_GROUPS + MAX_PRESETS_PER_GROUP * 3 + 5;

#[derive(Default)]
struct State {
    files: BTreeMap<String, Arc<[u8]>>,
    bytes: u64,
    dirty: BTreeSet<String>,
    writing: bool,
    migration: Option<Migration>,
    error: Option<String>,
    rejected: BTreeMap<String, String>,
    preserve_tips: bool,
}

#[derive(Clone, Default)]
pub struct Bridge(Arc<Mutex<State>>);

pub fn session(backend: Bridge) -> (Session, Vec<String>) {
    let mut session = Session::new();
    let opened = preset_store::open(Box::new(backend.clone()));
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
        s.bytes = size;
        Ok(())
    }

    pub fn load(&self, name: &str, bytes: Vec<u8>) -> Result<(), String> {
        Self::insert(&mut self.state(), name, &bytes)
    }

    pub fn begin(&self) -> Option<Batch> {
        let mut s = self.state();
        if s.writing || s.error.is_some() || s.dirty.is_empty() {
            return None;
        }
        s.writing = true;
        let mut files: Files = std::mem::take(&mut s.dirty)
            .into_iter()
            .map(|name| {
                let bytes = s.files.get(&name).cloned();
                (name, bytes)
            })
            .collect();
        if let Some(migration) = s.migration.clone() {
            // Every ambiguous retry has the same exact identity. Newer live edits wait
            // for this migration's acknowledgement, then get their own ordinary batch.
            if s.files.get(ACTIONS_FILE).map(AsRef::as_ref) != Some(migration.atn.as_ref()) {
                s.dirty.insert(ACTIONS_FILE.into());
            }
            files.insert(ACTIONS_FILE.into(), Some(migration.atn));
            files.insert(MIGRATED_ACTIONS_FILE.into(), Some(migration.legacy));
            files.insert(LEGACY_ACTIONS_FILE.into(), None);
        }
        Some(Batch { files, migration: s.migration.clone() })
    }

    pub fn finish(&self, batch: Batch, result: Result<(), String>) {
        let mut s = self.state();
        s.writing = false;
        if let Err(error) = result {
            // Requeue the whole migration and current bytes, including edits made in flight.
            s.dirty.extend(batch.files.into_keys());
            s.error = Some(error);
        } else if batch.migration.is_some() {
            // Only transaction completion retires the pending migration. Any newer ATN edit
            // remains in dirty and gets its own acknowledged transaction.
            s.migration = None;
        }
    }

    pub fn unavailable(&self, error: String) {
        self.state().error = Some(error);
    }

    pub fn retry(&self) {
        self.state().error = None;
    }

    pub fn error(&self) -> Option<String> {
        let s = self.state();
        s.rejected.values().next().cloned().or_else(|| s.error.clone())
    }

    pub fn unsaved(&self) -> bool {
        let s = self.state();
        s.writing || !s.dirty.is_empty() || !s.rejected.is_empty()
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
            let migration = Migration { legacy: Arc::from(legacy), atn: Arc::from(atn), published };
            // One lock, with all validation above mutation. add(), not put(), protects the backup
            // and a first ATN publication in the actual IndexedDB transaction as well.
            s.files.insert(ACTIONS_FILE.into(), migration.atn.clone());
            s.files.insert(MIGRATED_ACTIONS_FILE.into(), migration.legacy.clone());
            s.files.remove(LEGACY_ACTIONS_FILE);
            s.bytes = size;
            s.dirty.extend([ACTIONS_FILE.into(), LEGACY_ACTIONS_FILE.into(), MIGRATED_ACTIONS_FILE.into()]);
            s.migration = Some(migration);
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
        if s.files.get(name).is_some_and(|old| old.as_ref() == bytes) {
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
    use photocraft_psd::abr::{AbrSample, LegacyBrush, LegacyTip, write_v12};
    use serde_json::json;

    #[test]
    fn actions_migration_is_one_staged_batch_and_waits_for_completion() {
        let bridge = Bridge::default();
        bridge.load(LEGACY_ACTIONS_FILE, b"exact legacy".to_vec()).unwrap();
        assert_eq!(bridge.migrate_actions(b"exact legacy", b"complete ATN", false).unwrap(), ActionsMigration::Staged);
        assert!(bridge.unsaved());
        let batch = bridge.begin().unwrap();
        assert!(batch.migration.is_some());
        assert_eq!(batch.len(), 3);
        assert!(matches!(batch.get(LEGACY_ACTIONS_FILE), Some(None)));
        assert_eq!(batch.get(MIGRATED_ACTIONS_FILE).unwrap().as_deref(), Some(&b"exact legacy"[..]));
        assert!(bridge.begin().is_none());
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
        assert_eq!(batch.get(ACTIONS_FILE).unwrap().as_deref(), Some(&b"first ATN"[..]));
        bridge.write(ACTIONS_FILE, b"edited ATN").unwrap();
        bridge.finish(batch, Err("transaction aborted".into()));
        assert!(bridge.unsaved());
        assert!(bridge.begin().is_none());
        bridge.retry();
        let batch = bridge.begin().unwrap();
        assert!(batch.migration.is_some());
        assert_eq!(batch.len(), 3);
        assert_eq!(batch.get(ACTIONS_FILE).unwrap().as_deref(), Some(&b"first ATN"[..]));
        assert_eq!(batch.get(MIGRATED_ACTIONS_FILE).unwrap().as_deref(), Some(&b"legacy"[..]));
        bridge.finish(batch, Ok(()));
        assert!(bridge.unsaved());
        let next = bridge.begin().unwrap();
        assert!(next.migration.is_none());
        assert_eq!(next.len(), 1);
        assert_eq!(next.get(ACTIONS_FILE).unwrap().as_deref(), Some(&b"edited ATN"[..]));
        bridge.finish(next, Ok(()));
        assert!(!bridge.unsaved());
    }

    #[test]
    fn migration_recognizes_only_the_exact_fresh_or_committed_tuple() {
        for published in [false, true] {
            let migration = Migration { legacy: Arc::from(&b"legacy"[..]), atn: Arc::from(&b"ATN0"[..]), published };
            assert!(!migration.already_committed(published.then_some(&b"ATN0"[..]), Some(b"legacy"), None).unwrap());
            assert!(migration.already_committed(Some(b"ATN0"), None, Some(b"legacy")).unwrap());
            for (atn, legacy, backup) in [
                (Some(&b"ATN1"[..]), None, Some(&b"legacy"[..])),
                (Some(&b"ATN0"[..]), Some(&b"change"[..]), None),
                (Some(&b"ATN0"[..]), None, Some(&b"change"[..])),
                (Some(&b"ATN0"[..]), Some(&b"legacy"[..]), Some(&b"legacy"[..])),
                (None, None, Some(&b"legacy"[..])),
                (Some(&b""[..]), None, Some(&b"legacy"[..])),
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
        assert!(bridge.unsaved(), "a rejected migration remains visible until repaired or reloaded");
    }

    #[test]
    fn acknowledged_migration_keeps_an_inflight_newer_edit_pending() {
        let bridge = Bridge::default();
        bridge.load(LEGACY_ACTIONS_FILE, b"legacy".to_vec()).unwrap();
        bridge.migrate_actions(b"legacy", b"first ATN", false).unwrap();
        let batch = bridge.begin().unwrap();
        bridge.write(ACTIONS_FILE, b"newer ATN").unwrap();
        bridge.finish(batch, Ok(()));
        assert!(bridge.unsaved());
        let next = bridge.begin().unwrap();
        assert!(next.migration.is_none());
        assert_eq!(next.len(), 1);
        assert_eq!(next.get(ACTIONS_FILE).unwrap().as_deref(), Some(&b"newer ATN"[..]));
        bridge.finish(next, Ok(()));
        assert!(!bridge.unsaved());
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
        assert_eq!(batch.get("index.json").unwrap().as_deref(), Some(&b"two"[..]));
        b.remove("index.json").unwrap();
        assert!(b.begin().is_none());
        assert!(b.unsaved());
        b.finish(batch, Ok(()));
        let batch = b.begin().unwrap();
        assert!(batch.get("index.json").unwrap().is_none());
        b.finish(batch, Ok(()));
        assert!(!b.unsaved());
    }

    #[test]
    fn failed_transaction_retains_latest_writes_until_explicit_retry() {
        let b = Bridge::default();
        b.write("index.json", b"old").unwrap();
        let batch = b.begin().unwrap();
        b.write("index.json", b"new").unwrap();
        b.finish(batch, Err("QuotaExceededError".into()));
        assert!(b.unsaved());
        assert!(b.error().unwrap().contains("QuotaExceeded"));
        assert!(b.begin().is_none());
        b.retry();
        let batch = b.begin().unwrap();
        assert_eq!(batch.get("index.json").unwrap().as_deref(), Some(&b"new"[..]));
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
        for (key, value) in &persisted {
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
        assert!(b.error().unwrap().contains("SecurityError"));
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
        use photocraft_psd::grd::{GrdColor, GrdGradient, GrdStop, write};
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
}
