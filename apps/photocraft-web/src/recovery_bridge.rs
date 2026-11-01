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
use std::collections::{BTreeMap, HashMap, HashSet};
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

/// The page-load session that wrote an entry (`web-<session>-<document id>`). Sessions are
/// alphanumeric, so the last `-` always separates the id.
pub fn session_of(key: &str) -> Option<&str> {
    key.strip_prefix("web-")?.rsplit_once('-').map(|(session, _)| session).filter(|s| !s.is_empty())
}

/// Whether the stored preferences (the UI's JSON text) leave "Recover on launch" on. Unreadable
/// or missing preferences keep the default, which is on.
pub fn recover_on_launch(prefs: Option<&str>) -> bool {
    prefs
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .and_then(|v| v.pointer("/fileHandling/recoverOnLaunch").and_then(serde_json::Value::as_bool))
        .unwrap_or(true)
}

struct State {
    /// Unique to this page load: document ids restart every load, and a new document must not
    /// overwrite an entry an earlier load left.
    session: String,
    max_snapshot: usize,
    /// The entry each open document writes to (a recovered document adopts its entry's key).
    keys: HashMap<u64, String>,
    writers: HashMap<u64, PcraftWriter>,
    /// Sessions of earlier page loads whose entries this page owns: it holds their lock (the
    /// web layer takes it, so only one page wins). Entries of any other session belong to a
    /// page that may still be open and are neither listed nor deleted.
    claimed: HashSet<String>,
    /// Records of claimed sessions read at launch, by name.
    loaded: BTreeMap<String, Vec<u8>>,
    done: Vec<(u64, u64, Result<(), String>)>,
}

/// Shared by the service closures and the write tasks (single-threaded, like the page).
#[derive(Clone)]
pub struct Recovery(Rc<RefCell<State>>);

impl Recovery {
    pub fn new(session: String) -> Self {
        Self::with_max_snapshot(session, MAX_SNAPSHOT_BYTES)
    }

    pub fn with_max_snapshot(session: String, max_snapshot: usize) -> Self {
        let session = session.chars().filter(|c| c.is_ascii_alphanumeric()).take(32).collect();
        Recovery(Rc::new(RefCell::new(State {
            session,
            max_snapshot,
            keys: HashMap::new(),
            writers: HashMap::new(),
            claimed: HashSet::new(),
            loaded: BTreeMap::new(),
            done: Vec::new(),
        })))
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> Result<R, String> {
        let mut s = self.0.try_borrow_mut().map_err(|_| "crash recovery is busy".to_string())?;
        Ok(f(&mut s))
    }

    pub fn session(&self) -> String {
        self.with(|s| s.session.clone()).unwrap_or_default()
    }

    /// This page now owns `session`'s entries (it holds that session's lock).
    pub fn claim(&self, session: &str) {
        let _ = self.with(|s| s.claimed.insert(session.to_string()));
    }

    fn owns(s: &State, key: &str) -> bool {
        session_of(key).is_some_and(|k| k == s.session || s.claimed.contains(k))
    }

    /// A record of a claimed session read from storage at launch.
    pub fn load(&self, name: &str, bytes: Vec<u8>) {
        let _ = self.with(|s| {
            if s.loaded.len() < MAX_ENTRIES * 2 {
                s.loaded.insert(name.to_string(), bytes);
            }
        });
    }

    /// Serialise `doc` as the batch that replaces its entry. Errors (too large, cannot encode)
    /// are shown by the UI as "Autosave failed"; nothing is written. The total cap is not
    /// checked here: this tab cannot see what other tabs stored or deleted since launch, so
    /// the only check is the one inside the write transaction (`indexed_presets::write_capped`).
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
            let info = RecoveryInfo { key: key.clone(), document_name: doc.name.clone(), original_path: path.map(str::to_string), saved_at, revision };
            let info = serde_json::to_vec(&info).map_err(|e| e.to_string())?;
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
            let _ = self.with(|s| if Self::owns(s, key) { s.keys.insert(doc_id, key.to_string()) } else { None });
        }
    }

    /// The document was saved or closed: the batch deleting its entry (if it has one).
    pub fn discard(&self, doc_id: u64) -> Option<Batch> {
        self.with(|s| {
            s.writers.remove(&doc_id);
            let key = s.keys.remove(&doc_id)?;
            // Never delete what another page owns.
            if !Self::owns(s, &key) {
                return None;
            }
            s.loaded.remove(&bundle_name(&key));
            s.loaded.remove(&info_name(&key));
            Some(Batch::from([(bundle_name(&key), None), (info_name(&key), None)]))
        })
        .ok()
        .flatten()
    }

    /// The claimed entries read at launch, newest first. A `.pcraft` without a readable sidecar
    /// is not offered (and stays in storage). The bytes move into each entry's loader, so they
    /// are freed as soon as that document is decoded.
    pub fn recoverable(&self) -> Vec<Recoverable> {
        let mut found: Vec<(RecoveryInfo, Vec<u8>)> = self
            .with(|s| {
                let infos: Vec<RecoveryInfo> = s
                    .loaded
                    .iter()
                    .filter(|(_, b)| b.len() <= SIDECAR_MAX)
                    .filter_map(|(name, bytes)| {
                        let key = name.strip_suffix(".json")?;
                        let info = serde_json::from_slice::<RecoveryInfo>(bytes).ok().filter(|i| valid_key(key) && i.key == key)?;
                        if !Self::owns(s, key) {
                            return None;
                        }
                        // Browsers evict storage under pressure: a half-evicted entry is reported, not an error.
                        if !s.loaded.contains_key(&bundle_name(key)) {
                            log::warn!(
                                "Recovery data for \"{}\" is incomplete (the browser may have cleared part of it), so it is not offered",
                                info.document_name
                            );
                            return None;
                        }
                        Some(info)
                    })
                    .collect();
                infos
                    .into_iter()
                    .filter_map(|info| {
                        let bytes = s.loaded.remove(&bundle_name(&info.key))?;
                        Some((info, bytes))
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

    /// Stores a batch like the IndexedDB write does, and hands it to a fresh page load that
    /// owns the writing session (as when its lock was free).
    fn reload(batch: &Batch) -> Recovery {
        let next = Recovery::new("next".into());
        for (name, bytes) in batch {
            if let Some(b) = bytes {
                next.claim(session_of(name.rsplit_once('.').map_or(name, |(k, _)| k)).unwrap_or_default());
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
        assert_eq!(now.session(), "next");
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
        let r = Recovery::with_max_snapshot("s".into(), 16);
        let e = r.snapshot(&doc("Big"), 1, None, 1).unwrap_err();
        assert!(e.contains("too large"), "{e}");
    }

    #[test]
    fn entries_of_another_live_page_are_not_offered_or_deleted() {
        let theirs = Recovery::new("tabA".into());
        let d = doc("Live");
        let batch = theirs.snapshot(&d, 1, None, 5).unwrap();
        let key = format!("web-tabA-{}", d.id.0);
        assert_eq!(session_of(&key), Some("tabA"));
        // This page read the records but could not take tabA's lock: nothing is claimed.
        let me = Recovery::new("tabB".into());
        for (name, bytes) in &batch {
            me.load(name, bytes.as_ref().map(|b| b.to_vec()).unwrap_or_default());
        }
        assert!(me.recoverable().is_empty(), "a live page's entries are not offered");
        // Adopting a foreign key is ignored, so closing the document deletes nothing.
        let open = doc("Adopted");
        me.adopt(open.id.0, &key);
        assert!(me.discard(open.id.0).is_none());
        // Once the other page is gone and its lock is taken, the entry is offered and adoptable.
        me.claim("tabA");
        me.load(&info_name(&key), batch[&info_name(&key)].as_ref().map(|b| b.to_vec()).unwrap_or_default());
        me.load(&bundle_name(&key), batch[&bundle_name(&key)].as_ref().map(|b| b.to_vec()).unwrap_or_default());
        assert_eq!(me.recoverable().len(), 1);
        me.adopt(open.id.0, &key);
        assert!(me.discard(open.id.0).is_some());
    }

    #[test]
    fn recovered_bytes_are_handed_over_once() {
        let batch = Recovery::new("old".into()).snapshot(&doc("Lost"), 1, None, 5).unwrap();
        let now = reload(&batch);
        assert_eq!(now.recoverable().len(), 1);
        assert!(now.recoverable().is_empty(), "the loader owns the only copy of the bytes");
    }

    #[test]
    fn recover_on_launch_reads_the_stored_preference() {
        assert!(recover_on_launch(None));
        assert!(recover_on_launch(Some("not json")));
        assert!(recover_on_launch(Some(r#"{"fileHandling":{"autosave":true}}"#)));
        assert!(!recover_on_launch(Some(r#"{"fileHandling":{"recoverOnLaunch":false}}"#)));
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
        r.claim("x");
        r.claim("y");
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

    /// A quota error aborts the write transaction, so the previous snapshot stays whole; the failure
    /// is reported and a snapshot that is refused up front produces no batch to write at all.
    #[test]
    fn a_failed_write_leaves_the_previous_recovery_intact() {
        let r = Recovery::new("q".into());
        let d = doc("Keep");
        let stored = r.snapshot(&d, 1, None, 1).unwrap();
        let _next = r.snapshot(&d, 2, None, 2).unwrap(); // its write fails with a quota error below
        r.finish(d.id.0, 2, Err("QuotaExceededError".into()));
        assert_eq!(r.take_completed(), vec![(d.id.0, 2, Err("QuotaExceededError".into()))]);
        let mut found = reload(&stored).recoverable();
        assert_eq!(found.len(), 1, "revision 1 is still recoverable");
        assert_eq!((found.remove(0).load)().unwrap(), *d);
        let tight = Recovery::with_max_snapshot("q".into(), 16);
        assert!(tight.snapshot(&d, 3, None, 3).is_err(), "refused before anything is written");
    }

    /// Two tabs with the same document id (ids restart every page load) write to different entries,
    /// and closing the document in one tab never deletes the other tab's.
    #[test]
    fn two_tabs_on_the_same_document_keep_separate_entries() {
        let (a, b) = (Recovery::new("tabA".into()), Recovery::new("tabB".into()));
        let d = doc("Shared");
        let (ba, bb) = (a.snapshot(&d, 1, None, 1).unwrap(), b.snapshot(&d, 1, None, 1).unwrap());
        assert!(ba.keys().all(|k| !bb.contains_key(k)), "no key is shared");
        let gone = a.discard(d.id.0).unwrap();
        assert!(gone.keys().all(|k| ba.contains_key(k) && !bb.contains_key(k)), "only tabA's own entry is deleted");
        assert!(b.discard(d.id.0).unwrap().keys().all(|k| bb.contains_key(k)));
    }

    /// A snapshot this version cannot open (cut short, or from a build that wrote something else)
    /// fails to load with an error, not a panic, and stays offered so a later version can read it.
    #[test]
    fn a_snapshot_that_cannot_be_opened_reports_an_error() {
        let batch = Recovery::new("old".into()).snapshot(&doc("Odd"), 1, None, 5).unwrap();
        let key = batch.keys().find(|k| k.ends_with(".pcraft")).unwrap().clone();
        let full = batch[&key].as_ref().unwrap().to_vec();
        let now = reload(&batch);
        now.load(&key, full[..full.len() / 2].to_vec()); // cut short
        let mut found = now.recoverable();
        assert_eq!(found.len(), 1);
        assert!((found.remove(0).load)().is_err());
        let again = reload(&batch);
        again.load(&key, b"PK not a PhotoCraft bundle".to_vec()); // some other zip
        assert!((again.recoverable().remove(0).load)().is_err());
    }

    /// The browser may have evicted half of an entry: it is not offered, and nothing fails.
    #[test]
    fn a_half_evicted_entry_is_skipped() {
        let batch = Recovery::new("old".into()).snapshot(&doc("Evicted"), 1, None, 5).unwrap();
        for keep in [".json", ".pcraft"] {
            let now = Recovery::new("next".into());
            for (name, bytes) in batch.iter().filter(|(n, _)| n.ends_with(keep)) {
                now.claim(session_of(name.rsplit_once('.').map_or(name, |(k, _)| k)).unwrap_or_default());
                now.load(name, bytes.as_ref().unwrap().to_vec());
            }
            assert!(now.recoverable().is_empty(), "only the {keep} record is left");
        }
    }
}
