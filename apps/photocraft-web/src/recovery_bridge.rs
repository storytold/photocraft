//! Crash recovery for the browser build (#6615): the platform-agnostic half.
//!
//! The UI already schedules autosaves (timer, dirty tracking, one write in flight per document)
//! and recovers at launch through `Services::{autosave, autosave_results, discard_autosave,
//! recover, adopt_autosave}`. The desktop backs them with a recovery folder (`RecoveryStore`,
//! threads and files). This is the same contract over browser storage: a snapshot is the
//! document's `.pcraft` bytes from the incremental writer (unchanged tiles are not compressed
//! again), stored as `<key>.pcraft` plus a `<key>.json` [`RecoveryInfo`] in one IndexedDB
//! transaction (`indexed_presets::write`, shared with the presets). Nothing here touches the
//! browser, so it is unit tested natively.
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use photocraft_doc::Document;
use photocraft_format::autosave::RecoveryInfo;
use photocraft_format::{PcraftWriter, SaveOptions};
use photocraft_ui_egui::Recoverable;

use crate::preset_bridge::Batch;

/// Largest single snapshot. Bigger documents are not autosaved (with a message) instead of
/// exhausting the browser's per-origin quota or the tab's memory.
pub const MAX_SNAPSHOT_BYTES: usize = 256 << 20;
/// Most recovery data kept at once, across documents.
pub const MAX_TOTAL_BYTES: u64 = 768 << 20;
/// Most entries read back at launch.
pub const MAX_ENTRIES: usize = 64;
const SIDECAR_MAX: usize = 1 << 20;

fn bundle_name(key: &str) -> String {
    format!("{key}.pcraft")
}
fn info_name(key: &str) -> String {
    format!("{key}.json")
}
fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 96 && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

struct State {
    /// Unique to this page load: document ids restart every load, and a new document must not
    /// overwrite an entry an earlier load left.
    session: String,
    max_snapshot: usize,
    max_total: u64,
    /// The entry each open document writes to (a recovered document adopts its entry's key).
    keys: HashMap<u64, String>,
    writers: HashMap<u64, PcraftWriter>,
    /// Stored bytes per entry, for the total cap.
    sizes: HashMap<String, u64>,
    /// Records read at launch, by name.
    loaded: BTreeMap<String, Arc<[u8]>>,
    done: Vec<(u64, u64, Result<(), String>)>,
}

/// Shared by the service closures and the write tasks (single-threaded, like the page).
#[derive(Clone)]
pub struct Recovery(Rc<RefCell<State>>);

impl Recovery {
    pub fn new(session: String) -> Self {
        Self::with_limits(session, MAX_SNAPSHOT_BYTES, MAX_TOTAL_BYTES)
    }

    pub fn with_limits(session: String, max_snapshot: usize, max_total: u64) -> Self {
        let session = session.chars().filter(|c| c.is_ascii_alphanumeric()).take(32).collect();
        Recovery(Rc::new(RefCell::new(State {
            session,
            max_snapshot,
            max_total,
            keys: HashMap::new(),
            writers: HashMap::new(),
            sizes: HashMap::new(),
            loaded: BTreeMap::new(),
            done: Vec::new(),
        })))
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> Result<R, String> {
        let mut s = self.0.try_borrow_mut().map_err(|_| "crash recovery is busy".to_string())?;
        Ok(f(&mut s))
    }

    /// A record read from storage at launch.
    pub fn load(&self, name: &str, bytes: Vec<u8>) {
        let _ = self.with(|s| {
            if s.loaded.len() < MAX_ENTRIES * 2 {
                s.sizes
                    .entry(name.rsplit_once('.').map_or(name, |(k, _)| k).to_string())
                    .and_modify(|n| *n += bytes.len() as u64)
                    .or_insert(bytes.len() as u64);
                s.loaded.insert(name.to_string(), bytes.into());
            }
        });
    }

    /// Serialise `doc` as the batch that replaces its entry. Errors (too large, over the cap,
    /// cannot encode) are shown by the UI as "Autosave failed"; nothing is written.
    pub fn snapshot(&self, doc: &Arc<Document>, revision: u64, path: Option<&str>, saved_at: u64) -> Result<Batch, String> {
        self.with(|s| {
            let id = doc.id.0;
            let key = s.keys.entry(id).or_insert_with(|| format!("web-{}-{id}", s.session)).clone();
            let writer = s.writers.entry(id).or_insert_with(PcraftWriter::new);
            let (bytes, _) = writer.save_zip(doc, &SaveOptions::default()).map_err(|e| e.to_string())?;
            if bytes.len() > s.max_snapshot {
                return Err(format!(
                    "this document is too large to autosave in the browser ({} MB; the limit is {} MB)",
                    bytes.len() >> 20,
                    s.max_snapshot >> 20
                ));
            }
            let others: u64 = s.sizes.iter().filter(|(k, _)| **k != key).map(|(_, n)| *n).sum();
            if others.saturating_add(bytes.len() as u64) > s.max_total {
                return Err("the browser's recovery storage limit is reached; save or close a document".into());
            }
            let info = RecoveryInfo { key: key.clone(), document_name: doc.name.clone(), original_path: path.map(str::to_string), saved_at, revision };
            let info = serde_json::to_vec(&info).map_err(|e| e.to_string())?;
            s.sizes.insert(key.clone(), bytes.len() as u64);
            let mut batch = Batch::new();
            batch.insert(bundle_name(&key), Some(bytes.into()));
            batch.insert(info_name(&key), Some(info.into()));
            Ok(batch)
        })?
    }

    /// A write for `doc_id`'s `revision` finished (or failed): reported through
    /// `Services::autosave_results`.
    pub fn finish(&self, doc_id: u64, revision: u64, result: Result<(), String>) {
        let _ = self.with(|s| s.done.push((doc_id, revision, result)));
    }

    pub fn take_completed(&self) -> Vec<(u64, u64, Result<(), String>)> {
        self.with(|s| std::mem::take(&mut s.done)).unwrap_or_default()
    }

    /// A recovered document takes over its entry.
    pub fn adopt(&self, doc_id: u64, key: &str) {
        if valid_key(key) {
            let _ = self.with(|s| s.keys.insert(doc_id, key.to_string()));
        }
    }

    /// The document was saved or closed: the batch deleting its entry (if it has one).
    pub fn discard(&self, doc_id: u64) -> Option<Batch> {
        self.with(|s| {
            s.writers.remove(&doc_id);
            let key = s.keys.remove(&doc_id)?;
            s.sizes.remove(&key);
            s.loaded.remove(&bundle_name(&key));
            s.loaded.remove(&info_name(&key));
            Some(Batch::from([(bundle_name(&key), None), (info_name(&key), None)]))
        })
        .ok()
        .flatten()
    }

    /// The entries read at launch, newest first. A `.pcraft` without a readable sidecar is not
    /// offered (and stays in storage).
    pub fn recoverable(&self) -> Vec<Recoverable> {
        let mut found: Vec<(RecoveryInfo, Arc<[u8]>)> = self
            .with(|s| {
                s.loaded
                    .iter()
                    .filter(|(_, b)| b.len() <= SIDECAR_MAX)
                    .filter_map(|(name, bytes)| {
                        let key = name.strip_suffix(".json")?;
                        let info = serde_json::from_slice::<RecoveryInfo>(bytes).ok().filter(|i| valid_key(key) && i.key == key)?;
                        Some((info, s.loaded.get(&bundle_name(key))?.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        found.sort_by(|a, b| b.0.saved_at.cmp(&a.0.saved_at).then(a.0.key.cmp(&b.0.key)));
        found
            .into_iter()
            .map(|(info, bytes)| Recoverable {
                key: info.key,
                name: info.document_name,
                path: info.original_path,
                load: Box::new(move || photocraft_format::load_from_bytes(&bytes).map_err(|e| e.to_string())),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{Color, ColorMode, SampleType};
    use photocraft_geom::Size;

    fn doc(name: &str) -> Arc<Document> {
        Arc::new(Document::with_background(name, Size::new(64, 48), ColorMode::Rgb, SampleType::U8, Color::WHITE))
    }

    /// Stores a batch like the IndexedDB write does, and hands it to a fresh page load.
    fn reload(batch: &Batch) -> Recovery {
        let next = Recovery::new("next".into());
        for (name, bytes) in batch {
            if let Some(b) = bytes {
                next.load(name, b.to_vec());
            }
        }
        next
    }

    #[test]
    fn snapshot_round_trips_through_a_new_page_load() {
        let r = Recovery::new("a1".into());
        let d = doc("Sketch");
        let batch = r.snapshot(&d, 7, Some("sketch.pcraft"), 1234).unwrap();
        let mut found = reload(&batch).recoverable();
        assert_eq!(found.len(), 1);
        let e = found.remove(0);
        assert_eq!((e.name.as_str(), e.path.as_deref()), ("Sketch", Some("sketch.pcraft")));
        assert_eq!(e.key, format!("web-a1-{}", d.id.0));
        assert_eq!((e.load)().unwrap(), *d);
    }

    #[test]
    fn a_second_snapshot_replaces_the_same_entry() {
        let r = Recovery::new("s".into());
        let d = doc("Doc");
        let a = r.snapshot(&d, 1, None, 1).unwrap();
        let b = r.snapshot(&d, 2, None, 2).unwrap();
        assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
    }

    #[test]
    fn recovered_document_adopts_its_entry_and_discard_deletes_it() {
        let old = Recovery::new("old".into());
        let batch = old.snapshot(&doc("Lost"), 1, None, 5).unwrap();
        let now = reload(&batch);
        let key = now.recoverable().remove(0).key;
        let open = doc("Lost");
        now.adopt(open.id.0, &key);
        let again = now.snapshot(&open, 2, None, 6).unwrap();
        assert!(again.contains_key(&bundle_name(&key)), "autosaves replace the recovered entry");
        let del = now.discard(open.id.0).unwrap();
        assert!(del.values().all(Option::is_none) && del.len() == 2);
        assert!(now.recoverable().is_empty());
        assert!(now.discard(open.id.0).is_none(), "nothing left to delete");
    }

    #[test]
    fn oversize_snapshot_is_refused_with_a_message() {
        let r = Recovery::with_limits("s".into(), 16, MAX_TOTAL_BYTES);
        let e = r.snapshot(&doc("Big"), 1, None, 1).unwrap_err();
        assert!(e.contains("too large"), "{e}");
    }

    #[test]
    fn total_cap_counts_other_documents_but_not_the_one_being_replaced() {
        let probe = Recovery::new("s".into()).snapshot(&doc("A"), 1, None, 1).unwrap();
        let size = probe.values().flatten().map(|b| b.len() as u64).max().unwrap_or(1);
        let r = Recovery::with_limits("s".into(), MAX_SNAPSHOT_BYTES, size + size / 2);
        let (a, b) = (doc("A"), doc("B"));
        r.snapshot(&a, 1, None, 1).unwrap();
        r.snapshot(&a, 2, None, 2).unwrap();
        assert!(r.snapshot(&b, 1, None, 3).unwrap_err().contains("limit"));
        r.discard(a.id.0);
        r.snapshot(&b, 1, None, 4).unwrap();
    }

    #[test]
    fn write_outcomes_are_reported_once() {
        let r = Recovery::new("s".into());
        r.finish(3, 9, Ok(()));
        r.finish(3, 10, Err("QuotaExceededError".into()));
        assert_eq!(r.take_completed(), vec![(3, 9, Ok(())), (3, 10, Err("QuotaExceededError".into()))]);
        assert!(r.take_completed().is_empty());
    }

    #[test]
    fn damaged_or_foreign_records_are_not_offered() {
        let r = Recovery::new("s".into());
        r.load("web-x-1.json", b"not json".to_vec());
        r.load("web-x-1.pcraft", vec![1, 2, 3]);
        r.load("../evil.json", br#"{"key":"../evil","document_name":"x","original_path":null,"saved_at":1,"revision":1}"#.to_vec());
        r.load("../evil.pcraft", vec![0]);
        r.load("orphan.pcraft", vec![0]);
        assert!(r.recoverable().is_empty());
        // A sidecar whose bundle is garbage is offered, and fails to load without a panic.
        r.load("web-y-2.json", br#"{"key":"web-y-2","document_name":"Y","original_path":null,"saved_at":1,"revision":1}"#.to_vec());
        r.load("web-y-2.pcraft", vec![1, 2, 3]);
        let mut found = r.recoverable();
        assert_eq!(found.len(), 1);
        assert!((found.remove(0).load)().is_err());
    }
}
