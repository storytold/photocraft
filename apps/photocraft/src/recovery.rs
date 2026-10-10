//! Desktop recovery ownership. Reading a snapshot never retires it; only a
//! explicit document close/discard asks the manager to retire it.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use photocraft_doc::Document;
use photocraft_format::Autosaver;
use photocraft_ops::HistoryCheckpoint;
use photocraft_ui_egui::{AutosaveCompletion, RecoveredDocument};

#[cfg(test)]
use photocraft_ui_egui::RecoveryBatch;

static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

struct Owned {
    key: String,
    saver: Option<Autosaver>,
    retiring: bool,
}

pub(crate) struct RecoveryManager {
    dir: Option<PathBuf>,
    prefix: String,
    next_key: u64,
    owned: HashMap<u64, Owned>,
    locks: HashMap<String, std::fs::File>,
}

impl RecoveryManager {
    pub(crate) fn new(dir: Option<PathBuf>) -> Self {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let serial = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
        Self { dir, prefix: format!("session-{}-{stamp}-{serial}", std::process::id()), next_key: 0, owned: HashMap::new(), locks: HashMap::new() }
    }

    #[cfg(test)]
    pub(crate) fn discover(&mut self) -> (Vec<photocraft_ui_egui::Recoverable>, Vec<String>) {
        let Some(dir) = self.dir.clone() else { return (Vec::new(), Vec::new()) };
        let (entries, errors) = photocraft_format::list_recovery_checked(&dir);
        self.claim_discovery(entries, errors)
    }
    pub(crate) fn claim_discovery(
        &mut self,
        entries: Vec<photocraft_format::RecoveryEntry>,
        mut errors: Vec<String>,
    ) -> (Vec<photocraft_ui_egui::Recoverable>, Vec<String>) {
        let mut documents = Vec::new();
        for entry in entries {
            if let Err(error) = self.lock_key(&entry.info.key) {
                errors.push(format!("Could not claim {}: {error}", entry.info.document_name));
                continue;
            }
            documents.push(photocraft_ui_egui::Recoverable {
                key: entry.info.key.clone(),
                name: entry.info.document_name.clone(),
                path: entry.info.original_path.clone(),
                load: Box::new(move || {
                    let (document, history, context) = photocraft_format::recover_checkpoint_with_context(&entry).map_err(|e| e.to_string())?;
                    Ok(RecoveredDocument { key: entry.info.key, path: entry.info.original_path, document, history: Some(history), context })
                }),
            });
        }
        (documents, errors)
    }
    #[cfg(test)]
    pub(crate) fn recover(&mut self) -> RecoveryBatch {
        let (entries, mut errors) = self.discover();
        let mut documents = Vec::new();
        for entry in entries {
            match (entry.load)() {
                Ok(document) => documents.push(document),
                Err(error) => errors.push(error),
            }
        }
        RecoveryBatch { documents, errors }
    }

    /// OS locks disappear on crash; a second running app cannot overwrite this checkpoint.
    fn lock_key(&mut self, key: &str) -> Result<(), String> {
        if self.locks.contains_key(key) {
            return Ok(());
        }
        let Some(dir) = &self.dir else {
            return Ok(());
        };
        let locks = dir.join(".locks");
        std::fs::create_dir_all(&locks).map_err(|e| e.to_string())?;
        if std::fs::symlink_metadata(&locks).map_err(|e| e.to_string())?.file_type().is_symlink() {
            return Err("Recovery lock directory is a symlink".into());
        }
        let path = locks.join(format!("{key}.lock"));
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("Recovery lock file is a symlink".into());
        }
        let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path).map_err(|e| e.to_string())?;
        file.try_lock().map_err(|e| format!("Recovery checkpoint is in use or cannot be locked: {e}"))?;
        self.locks.insert(key.to_owned(), file);
        Ok(())
    }

    fn claim(&mut self, id: u64, inherited: Option<&str>) -> Result<(), String> {
        if let Some(owner) = self.owned.get(&id) {
            if inherited.is_some_and(|key| key != owner.key) {
                return Err("Recovery ownership changed unexpectedly".into());
            }
            return Ok(());
        }
        let key = match inherited {
            Some(key) => {
                if key.is_empty() || key.len() > 200 || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    return Err("Invalid recovery key".into());
                }
                if self.owned.values().any(|owner| owner.key == key) {
                    return Err("Recovery snapshot already belongs to another document".into());
                }
                key.to_owned()
            }
            None => {
                self.next_key = self.next_key.checked_add(1).ok_or("Recovery key counter exhausted")?;
                format!("{}-{}", self.prefix, self.next_key)
            }
        };
        self.lock_key(&key)?;
        self.owned.insert(id, Owned { key, saver: None, retiring: false });
        Ok(())
    }

    pub(crate) fn queue(
        &mut self,
        doc: &Arc<Document>,
        history: HistoryCheckpoint,
        revision: u64,
        path: Option<&str>,
        key: Option<&str>,
        context: serde_json::Value,
    ) -> Result<(), String> {
        let dir = self.dir.clone().ok_or("No recovery directory available")?;
        self.claim(doc.id.0, key)?;
        let owner = self.owned.get_mut(&doc.id.0).ok_or("Recovery ownership unavailable")?;
        if owner.retiring {
            return Err("Recovery cleanup is still finishing".into());
        }
        if owner.saver.as_ref().is_some_and(Autosaver::is_finished) {
            owner.saver = None;
        }
        let saver = owner.saver.get_or_insert_with(|| Autosaver::new(&dir, &owner.key));
        saver.request_checkpoint_with_context(doc.clone(), history, revision, path.map(str::to_owned), Default::default(), context)
    }

    pub(crate) fn poll(&mut self) -> Vec<AutosaveCompletion> {
        let mut out = Vec::new();
        let mut retired = Vec::new();
        for (&id, owner) in &mut self.owned {
            if let Some(saver) = &owner.saver {
                if owner.retiring {
                    if saver.is_finished() {
                        let result = saver
                            .take_completion()
                            .map(|completion| completion.result.map(|_| ()))
                            .unwrap_or_else(|| Err("Recovery cleanup worker stopped without a result".into()));
                        if result.is_ok() {
                            retired.push(id);
                        } else {
                            owner.saver = None;
                            owner.retiring = false;
                        }
                        out.push(AutosaveCompletion { document_id: id, revision: 0, result, retired: true });
                    }
                } else {
                    while let Some(completion) = saver.take_completion() {
                        out.push(AutosaveCompletion { document_id: id, revision: completion.revision, result: completion.result.map(|_| ()), retired: false });
                    }
                }
            }
        }
        for id in retired {
            if let Some(owner) = self.owned.remove(&id) {
                self.locks.remove(&owner.key);
            }
        }
        out
    }

    pub(crate) fn discard(&mut self, id: u64, key: Option<&str>) -> Result<(), String> {
        let dir = self.dir.clone().ok_or("No recovery directory available")?;
        self.claim(id, key)?;
        let owner = self.owned.get_mut(&id).ok_or("Recovery ownership unavailable")?;
        if owner.retiring {
            return Ok(());
        }
        if owner.saver.as_ref().is_some_and(Autosaver::is_finished) {
            owner.saver = None;
        }
        let saver = owner.saver.get_or_insert_with(|| Autosaver::new(dir, &owner.key));
        saver.begin_discard()?;
        owner.retiring = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Color, ColorMode, SampleType, Size};
    use photocraft_ops::History;
    use std::time::{Duration, Instant};

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let serial = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("photocraft-recovery-manager-{}-{serial}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn document() -> Arc<Document> {
        Arc::new(Document::with_background("Recovery", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE))
    }
    fn completed(manager: &mut RecoveryManager, retiring: bool) -> AutosaveCompletion {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(completion) = manager.poll().into_iter().find(|completion| completion.retired == retiring) {
                return completion;
            }
            assert!(Instant::now() < deadline, "background recovery operation did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn startup_keeps_snapshot_for_second_crash_and_reports_corruption() {
        let dir = Temp::new();
        let doc = document();
        let mut writer = RecoveryManager::new(Some(dir.0.clone()));
        writer.queue(&doc, History::default().checkpoint(), 7, None, None, serde_json::Value::Null).unwrap();
        assert!(completed(&mut writer, false).result.is_ok());
        drop(writer);
        let mut first = RecoveryManager::new(Some(dir.0.clone()));
        let recovered = first.recover();
        assert_eq!(recovered.documents.len(), 1);
        assert_eq!(recovered.documents[0].document.name, "Recovery");
        let mut second = RecoveryManager::new(Some(dir.0.clone()));
        let in_use = second.recover();
        assert!(in_use.documents.is_empty(), "running instances must not share recovery ownership");
        assert!(!in_use.errors.is_empty());
        drop(first);
        assert_eq!(second.recover().documents.len(), 1);
        std::fs::write(dir.0.join("broken.json"), b"invalid-json").unwrap();
        assert!(!second.recover().errors.is_empty());
        assert!(dir.0.join("broken.json").exists());
    }
    #[test]
    fn inherited_key_belongs_to_admitted_document_and_retires_only_on_close() {
        let dir = Temp::new();
        let doc = document();
        let mut writer = RecoveryManager::new(Some(dir.0.clone()));
        writer.queue(&doc, History::default().checkpoint(), 1, None, None, serde_json::Value::Null).unwrap();
        assert!(completed(&mut writer, false).result.is_ok());
        drop(writer);
        let mut manager = RecoveryManager::new(Some(dir.0.clone()));
        let recovered = manager.recover().documents.pop().unwrap();
        let key = recovered.key;
        let admitted = doc.id.0 + 1000;
        manager.claim(admitted, Some(&key)).unwrap();
        assert!(manager.discard(admitted + 1, Some(&key)).is_err());
        assert_eq!(manager.recover().documents.len(), 1, "claiming must not delete durable state");
        manager.discard(admitted, Some(&key)).unwrap();
        assert!(completed(&mut manager, true).result.is_ok());
        assert!(manager.recover().documents.is_empty());
    }
    #[test]
    fn stopped_storage_worker_is_restarted_on_retry() {
        let dir = Temp::new();
        let mut doc = document();
        let mut manager = RecoveryManager::new(Some(dir.0.clone()));
        manager.queue(&doc, History::default().checkpoint(), 1, None, None, serde_json::Value::Null).unwrap();
        assert!(completed(&mut manager, false).result.is_ok());
        // Model a stopped worker without retiring the manager's document ownership.
        let saver = manager.owned.get(&doc.id.0).unwrap().saver.as_ref().unwrap();
        saver.begin_discard().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !saver.is_finished() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        Arc::make_mut(&mut doc).name = "After worker retry".into();
        manager.queue(&doc, History::default().checkpoint(), 2, None, None, serde_json::Value::Null).unwrap();
        assert!(completed(&mut manager, false).result.is_ok());
        assert_eq!(manager.recover().documents.pop().unwrap().document, *doc);
    }

    #[test]
    fn independent_managers_never_reuse_fresh_keys_and_invalid_inheritance_fails() {
        let mut first = RecoveryManager::new(None);
        let mut second = RecoveryManager::new(None);
        first.claim(1, None).unwrap();
        second.claim(1, None).unwrap();
        assert_ne!(first.owned.get(&1).unwrap().key, second.owned.get(&1).unwrap().key);
        for key in ["../escape", "/absolute", "", "bad/key", "bad\\key"] {
            assert!(first.claim(2, Some(key)).is_err());
        }
    }
}
