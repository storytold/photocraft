//! Autosave and crash recovery (architecture §9): the same incremental writer
//! saves a document *snapshot* into a recovery directory on a background
//! thread, so the UI never blocks.
//!
//! Layout: `<recovery_dir>/<key>.pcraft/` (directory bundles, so repeated
//! autosaves write only changed tiles) plus `<key>.json` with
//! [`RecoveryInfo`]. Native targets only (threads and a filesystem).
//!
//! [`RecoveryStore`] is the app-facing lifecycle: an entry is only deleted once
//! the document is saved or closed, never just because it was recovered.

#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use photocraft_doc::Document;
use serde::{Deserialize, Serialize};

use crate::store::write_atomic;
use crate::{FormatError, PcraftWriter, Result, SaveOptions, SaveStats};

/// Sidecar describing an autosave.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoveryInfo {
    pub key: String,
    pub document_name: String,
    /// Where the user last saved the document, if anywhere.
    pub original_path: Option<String>,
    /// Seconds since the Unix epoch.
    pub saved_at: u64,
    pub revision: u64,
}

/// One recoverable document found by [`list_recovery`].
#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryEntry {
    pub info: RecoveryInfo,
    pub bundle: PathBuf,
}

struct Job {
    snapshot: Arc<Document>,
    info: RecoveryInfo,
    opts: SaveOptions,
}

/// One completed revision's disk-write outcome (distinct from enqueue success).
type SaveOutcome = (u64, std::result::Result<(), String>);
type CompletedWrites = Arc<Mutex<Vec<SaveOutcome>>>;

/// Background autosaver for one document. Requests are coalesced: if saves
/// arrive faster than they complete, only the newest snapshot is written.
pub struct Autosaver {
    dir: PathBuf,
    key: String,
    tx: Option<Sender<Job>>,
    handle: Option<JoinHandle<()>>,
    last: Arc<Mutex<Option<Result<SaveStats>>>>,
    /// Per-request write outcomes, delivered to the desktop without blocking its frame.
    completed: CompletedWrites,
}

fn sanitize(key: &str) -> String {
    let s: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if s.is_empty() { "untitled".into() } else { s }
}

impl Autosaver {
    /// `key` identifies the document across autosaves (e.g. its DocId).
    pub fn new(recovery_dir: impl Into<PathBuf>, key: &str) -> Self {
        let dir = recovery_dir.into();
        let key = sanitize(key);
        let (tx, rx) = mpsc::channel::<Job>();
        let last = Arc::new(Mutex::new(None));
        let completed = Arc::new(Mutex::new(Vec::new()));
        let bundle = dir.join(format!("{key}.pcraft"));
        let sidecar = dir.join(format!("{key}.json"));
        let last2 = last.clone();
        let completed2 = completed.clone();
        // A thread that never started must not look like a working autosave: record the
        // failure where `last_result`/`flush` report it, so recovery health checks see it.
        let spawned = std::thread::Builder::new().name(format!("autosave-{key}")).spawn(move || worker(rx, bundle, sidecar, last2, completed2));
        let handle = match spawned {
            Ok(h) => Some(h),
            Err(e) => {
                note(&last, Err(FormatError::Io(std::io::Error::other(format!("the autosave thread could not start: {e}")))));
                None
            }
        };
        Autosaver { dir, key, tx: Some(tx), handle, last, completed }
    }

    pub fn bundle_path(&self) -> PathBuf {
        self.dir.join(format!("{}.pcraft", self.key))
    }

    /// Queue a snapshot for saving (returns immediately). Existing callers that only
    /// need best-effort autosaving can use this; the app uses `request_checked`.
    pub fn request(&self, snapshot: Arc<Document>, revision: u64, original_path: Option<String>, opts: SaveOptions) {
        let _ = self.request_checked(snapshot, revision, original_path, opts);
    }

    /// Queue a snapshot, reporting a worker that could not start or has stopped.
    /// Success here means queued, not written: use `take_completed` for write outcomes.
    pub fn request_checked(&self, snapshot: Arc<Document>, revision: u64, original_path: Option<String>, opts: SaveOptions) -> std::result::Result<(), String> {
        let info = RecoveryInfo {
            key: self.key.clone(),
            document_name: snapshot.name.clone(),
            original_path,
            saved_at: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            revision,
        };
        self.tx
            .as_ref()
            .ok_or_else(|| "autosave worker is stopped".to_string())?
            .send(Job { snapshot, info, opts })
            .map_err(|_| self.last_result().and_then(std::result::Result::err).unwrap_or_else(|| "autosave worker is unavailable".to_string()))
    }

    /// Drain completed write results without waiting for the worker.
    pub fn take_completed(&self) -> Vec<(u64, std::result::Result<(), String>)> {
        self.completed.lock().map(|mut results| std::mem::take(&mut *results)).unwrap_or_default()
    }

    /// Result of the most recent completed save.
    pub fn last_result(&self) -> Option<std::result::Result<SaveStats, String>> {
        let g = self.last.lock().ok()?;
        g.as_ref().map(|r| r.as_ref().map(|s| *s).map_err(|e| e.to_string()))
    }

    /// Finish pending saves and stop the thread.
    pub fn flush(mut self) -> Option<std::result::Result<SaveStats, String>> {
        self.shutdown();
        self.last_result()
    }

    /// The document was saved normally or closed: drop its recovery data.
    pub fn discard(mut self) -> Result<()> {
        self.shutdown();
        remove_entry(&self.dir, &self.key)
    }

    fn shutdown(&mut self) {
        self.tx.take();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Autosaver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn worker(rx: Receiver<Job>, bundle: PathBuf, sidecar: PathBuf, last: Arc<Mutex<Option<Result<SaveStats>>>>, completed: CompletedWrites) {
    let mut writer = PcraftWriter::new();
    while let Ok(mut job) = rx.recv() {
        // Coalesce: skip to the newest queued snapshot.
        while let Ok(newer) = rx.try_recv() {
            job = newer;
        }
        let r = (|| {
            std::fs::create_dir_all(&bundle)?;
            let stats = writer.save_dir(&job.snapshot, &bundle, &job.opts)?;
            write_atomic(&sidecar, &serde_json::to_vec_pretty(&job.info)?)?;
            Ok(stats)
        })();
        if let Ok(mut results) = completed.lock() {
            results.push((job.info.revision, r.as_ref().map(|_| ()).map_err(ToString::to_string)));
        }
        if let Ok(mut g) = last.lock() {
            *g = Some(r);
        }
    }
}

/// Recoverable documents in `recovery_dir`, newest first. A sidecar must be named after its own
/// (sanitized) key, so a stray or hand-edited one can never point outside the directory.
pub fn list_recovery(recovery_dir: &Path) -> Vec<RecoveryEntry> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(recovery_dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "json")
            && let Ok(bytes) = std::fs::read(&p)
            && let Ok(info) = serde_json::from_slice::<RecoveryInfo>(&bytes)
            && sanitize(&info.key) == info.key
            && p.file_stem().is_some_and(|stem| *stem == *info.key)
        {
            let bundle = recovery_dir.join(format!("{}.pcraft", info.key));
            if bundle.join(crate::store::MANIFEST).is_file() {
                out.push(RecoveryEntry { info, bundle });
            }
        }
    }
    out.sort_by(|a, b| b.info.saved_at.cmp(&a.info.saved_at).then(a.info.key.cmp(&b.info.key)));
    out
}

/// Load a recovered document.
pub fn recover(entry: &RecoveryEntry) -> Result<Document> {
    crate::load_path(&entry.bundle)
}

/// Delete a recovery entry.
pub fn discard_recovery(recovery_dir: &Path, entry: &RecoveryEntry) -> Result<()> {
    remove_entry(recovery_dir, &entry.info.key)
}

/// The crash-recovery autosaves of one app session, keyed by document id (the `DocId` value).
///
/// New documents save under `doc-<session>-<id>`, with a `<session>` unique to this store: ids
/// restart every launch, so a new document must never overwrite an entry an earlier launch left.
/// A recovered document [adopts](Self::adopt) the entry it was loaded from. Its autosaves replace
/// that entry in place (each save is atomic, so the old copy stays loadable until the new one is
/// complete), and saving or closing it removes the entry. Nothing is deleted just because it was
/// recovered, so a second crash before the next autosave loses nothing.
#[cfg(not(target_arch = "wasm32"))]
pub struct RecoveryStore {
    dir: PathBuf,
    session: String,
    savers: HashMap<u64, Autosaver>,
    adopted: HashMap<u64, String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl RecoveryStore {
    pub fn new(recovery_dir: impl Into<PathBuf>) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static STORES: AtomicU64 = AtomicU64::new(0);
        let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let session = format!("{t:x}_{:x}_{:x}", std::process::id(), STORES.fetch_add(1, Ordering::Relaxed));
        RecoveryStore { dir: recovery_dir.into(), session, savers: HashMap::new(), adopted: HashMap::new() }
    }

    /// Load every recoverable document, newest first, with the entry it came from. Nothing is
    /// deleted: [`adopt`](Self::adopt) each document the app opens. Entries that fail to load stay
    /// on disk untouched.
    pub fn recover(&self) -> Vec<(RecoveryEntry, Document)> {
        list_recovery(&self.dir).into_iter().filter_map(|e| recover(&e).ok().map(|doc| (e, doc))).collect()
    }

    /// Document `doc_id` was opened from the recovery entry `key`: its autosaves now replace that
    /// entry, and [`discard`](Self::discard) removes it.
    pub fn adopt(&mut self, doc_id: u64, key: &str) {
        self.adopted.insert(doc_id, sanitize(key));
    }

    /// Queue an autosave of `doc` (returns immediately; see [`Autosaver::request`]).
    pub fn autosave(&mut self, doc: &Arc<Document>, revision: u64, original_path: Option<String>) {
        let _ = self.autosave_checked(doc, revision, original_path);
    }

    /// Queue a snapshot, reporting enqueue failures to the application.
    pub fn autosave_checked(&mut self, doc: &Arc<Document>, revision: u64, original_path: Option<String>) -> std::result::Result<(), String> {
        let id = doc.id.0;
        let saver = self.savers.entry(id).or_insert_with(|| {
            let key = self.adopted.get(&id).cloned().unwrap_or_else(|| format!("doc-{}-{id}", self.session));
            Autosaver::new(&self.dir, &key)
        });
        saver.request_checked(doc.clone(), revision, original_path, SaveOptions::default())
    }

    /// Return completed write outcomes as (document id, revision, outcome), without blocking.
    pub fn take_completed(&mut self) -> Vec<(u64, u64, std::result::Result<(), String>)> {
        self.savers.iter().flat_map(|(&id, saver)| saver.take_completed().into_iter().map(move |(revision, result)| (id, revision, result))).collect()
    }

    /// Document `doc_id` was saved or closed: remove its recovery data, both its own autosaves
    /// and the entry it adopted.
    pub fn discard(&mut self, doc_id: u64) -> Result<()> {
        let own = self.savers.remove(&doc_id).map_or(Ok(()), Autosaver::discard);
        let adopted = self.adopted.remove(&doc_id).map_or(Ok(()), |key| remove_entry(&self.dir, &key));
        own.and(adopted)
    }
}

fn remove_entry(dir: &Path, key: &str) -> Result<()> {
    let bundle = dir.join(format!("{key}.pcraft"));
    if bundle.exists() {
        std::fs::remove_dir_all(bundle)?;
    }
    let sidecar = dir.join(format!("{key}.json"));
    if sidecar.exists() {
        std::fs::remove_file(sidecar)?;
    }
    Ok(())
}

/// Record the outcome of the last save attempt where `last_result`/`flush` report it. A
/// poisoned lock keeps the previous result rather than panicking in a crash-recovery path.
fn note(last: &Arc<Mutex<Option<Result<SaveStats>>>>, r: Result<SaveStats>) {
    if let Ok(mut g) = last.lock() {
        *g = Some(r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spawn_failure_is_reported_not_silent() {
        // What the spawn-failure arm of `Autosaver::new` records: recovery health checks
        // read this through `last_result`/`flush` instead of seeing a working autosave.
        let last = Arc::new(Mutex::new(None));
        note(&last, Err(FormatError::Io(std::io::Error::other("the autosave thread could not start: resources"))));
        let g = last.lock().expect("lock");
        let err = g.as_ref().and_then(|r| r.as_ref().err()).expect("the failure is recorded");
        let msg = err.to_string();
        assert!(msg.contains("could not start"), "{msg}");
    }
}
