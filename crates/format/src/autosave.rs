//! Durable document/history checkpoints and bounded background autosave.
//! A sidecar is the single atomic publication point. Immutable objects are
//! shared across checkpoints; a failed save never replaces that descriptor.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError, Weak, mpsc};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use photocraft_doc::Document;
use photocraft_ops::{ArchivedDocument, History, HistoryCheckpoint, HistoryState};
use photocraft_raster::Tile;
use serde::{Deserialize, Serialize};

use crate::store::{self, Source, write_atomic};
use crate::{FormatError, LoadOptions, PcraftWriter, Result, SaveOptions, SaveStats};

const MAX_DESCRIPTOR: u64 = 256 << 20;
/// Hard admission cap; full recovery storage fails visibly rather than growing indefinitely.
const MAX_RECOVERY_BYTES: u64 = 8 << 30;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoveryInfo {
    pub key: String,
    pub document_name: String,
    pub original_path: Option<String>,
    pub saved_at: u64,
    pub revision: u64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryEntry {
    pub info: RecoveryInfo,
    pub bundle: PathBuf,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoredLayerTarget {
    active: Option<photocraft_doc::LayerId>,
    selected: Vec<photocraft_doc::LayerId>,
}
impl From<photocraft_ops::LayerTarget> for StoredLayerTarget {
    fn from(value: photocraft_ops::LayerTarget) -> Self {
        Self { active: value.active, selected: value.selected }
    }
}
impl From<StoredLayerTarget> for photocraft_ops::LayerTarget {
    fn from(value: StoredLayerTarget) -> Self {
        Self { active: value.active, selected: value.selected }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredState {
    label: String,
    has_selection: bool,
    manifest: String,
    #[serde(default)]
    layers: StoredLayerTarget,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Checkpoint {
    version: u32,
    current: String,
    undo: Vec<StoredState>,
    redo: Vec<StoredState>,
    current_label: String,
    #[serde(default)]
    current_layers: StoredLayerTarget,
    max_states: usize,
    max_bytes: usize,
}
#[derive(Serialize, Deserialize)]
struct Descriptor {
    #[serde(flatten)]
    info: RecoveryInfo,
    #[serde(default)]
    checkpoint: Option<Checkpoint>,
    #[serde(default)]
    context: serde_json::Value,
}
struct Job {
    snapshot: Arc<Document>,
    history: HistoryCheckpoint,
    info: RecoveryInfo,
    opts: SaveOptions,
    context: serde_json::Value,
}
#[derive(Debug, Clone)]
pub struct AutosaveCompletion {
    pub revision: u64,
    pub result: std::result::Result<SaveStats, String>,
}
#[derive(Default)]
struct Mailbox {
    pending: Option<Job>,
    completion: Option<AutosaveCompletion>,
    last: Option<AutosaveCompletion>,
    stopped: bool,
    discard: bool,
    failure: Option<String>,
}
/// Exactly one in-flight snapshot and one newest pending snapshot are retained.
pub struct Autosaver {
    dir: PathBuf,
    key: String,
    shared: Arc<(Mutex<Mailbox>, Condvar)>,
    handle: Option<JoinHandle<()>>,
}
fn sanitize(key: &str) -> String {
    let s: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if s.is_empty() { "untitled".into() } else { s }
}
fn safe_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 200 && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
impl Autosaver {
    pub fn new(recovery_dir: impl Into<PathBuf>, key: &str) -> Self {
        let dir = recovery_dir.into();
        let key = sanitize(key);
        let shared = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let state = shared.clone();
        let bundle = dir.join(format!("{key}.pcraft"));
        let sidecar = dir.join(format!("{key}.json"));
        let spawned = std::thread::Builder::new().name(format!("autosave-{key}")).spawn(move || worker(state, bundle, sidecar));
        let handle = match spawned {
            Ok(handle) => Some(handle),
            Err(error) => {
                shared.0.lock().unwrap_or_else(PoisonError::into_inner).failure = Some(format!("Could not start autosave worker: {error}"));
                None
            }
        };
        Self { dir, key, shared, handle }
    }
    pub fn bundle_path(&self) -> PathBuf {
        self.dir.join(format!("{}.pcraft", self.key))
    }
    /// Queue acceptance is not durable completion; poll [`Self::take_completion`].
    pub fn request(&self, snapshot: Arc<Document>, revision: u64, original_path: Option<String>, opts: SaveOptions) -> std::result::Result<(), String> {
        self.request_checkpoint(snapshot, History::default().checkpoint(), revision, original_path, opts)
    }
    pub fn request_checkpoint(
        &self,
        snapshot: Arc<Document>,
        history: HistoryCheckpoint,
        revision: u64,
        original_path: Option<String>,
        opts: SaveOptions,
    ) -> std::result::Result<(), String> {
        self.request_checkpoint_with_context(snapshot, history, revision, original_path, opts, serde_json::Value::Null)
    }
    pub fn request_checkpoint_with_context(
        &self,
        snapshot: Arc<Document>,
        history: HistoryCheckpoint,
        revision: u64,
        original_path: Option<String>,
        opts: SaveOptions,
        context: serde_json::Value,
    ) -> std::result::Result<(), String> {
        let context_bytes = serde_json::to_vec(&context).map_err(|e| e.to_string())?;
        if context_bytes.len() > 64 << 10 {
            return Err("Recovery view context exceeds 64 KiB".into());
        }
        let mut state = self.shared.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(error) = &state.failure {
            return Err(error.clone());
        }
        if state.stopped || self.handle.as_ref().is_none_or(JoinHandle::is_finished) {
            return Err("Autosave worker has stopped".into());
        }
        if !safe_key(&self.key) {
            return Err("Recovery key exceeds its safe filename limit".into());
        }
        let info = RecoveryInfo {
            key: self.key.clone(),
            document_name: snapshot.name.clone(),
            original_path,
            saved_at: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
            revision,
        };
        state.pending = Some(Job { snapshot, history, info, opts, context });
        self.shared.1.notify_one();
        Ok(())
    }
    /// Retire on the worker without blocking the caller. Poll after `is_finished`.
    pub fn begin_discard(&self) -> std::result::Result<(), String> {
        let mut state = self.shared.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(error) = &state.failure {
            return Err(error.clone());
        }
        state.pending = None;
        state.discard = true;
        state.stopped = true;
        self.shared.1.notify_one();
        Ok(())
    }
    pub fn is_finished(&self) -> bool {
        self.handle.as_ref().is_none_or(JoinHandle::is_finished)
    }
    pub fn take_completion(&self) -> Option<AutosaveCompletion> {
        self.shared.0.lock().unwrap_or_else(PoisonError::into_inner).completion.take()
    }
    pub fn last_result(&self) -> Option<std::result::Result<SaveStats, String>> {
        self.shared.0.lock().unwrap_or_else(PoisonError::into_inner).last.as_ref().map(|c| c.result.clone())
    }
    pub fn flush(mut self) -> Option<std::result::Result<SaveStats, String>> {
        self.shutdown();
        self.last_result()
    }
    pub fn discard(mut self) -> Result<()> {
        self.shutdown();
        remove_entry(&self.dir, &self.key)
    }
    fn shutdown(&mut self) {
        self.shared.0.lock().unwrap_or_else(PoisonError::into_inner).stopped = true;
        self.shared.1.notify_one();
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            let mut state = self.shared.0.lock().unwrap_or_else(PoisonError::into_inner);
            let revision = state.last.as_ref().map_or(0, |c| c.revision);
            let c = AutosaveCompletion { revision, result: Err("Autosave worker terminated unexpectedly".into()) };
            state.last = Some(c.clone());
            state.completion = Some(c);
        }
    }
}
impl Drop for Autosaver {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn worker(shared: Arc<(Mutex<Mailbox>, Condvar)>, bundle: PathBuf, sidecar: PathBuf) {
    let mut writer = PcraftWriter::new();
    let mut verified = HashMap::new();
    loop {
        let job = {
            let mut state = shared.0.lock().unwrap_or_else(PoisonError::into_inner);
            while state.pending.is_none() && !state.stopped {
                state = shared.1.wait(state).unwrap_or_else(PoisonError::into_inner);
            }
            let Some(job) = state.pending.take() else {
                let discard = state.discard;
                let revision = state.last.as_ref().map_or(0, |c| c.revision);
                drop(state);
                if discard {
                    let result = match (bundle.parent(), bundle.file_stem().and_then(|s| s.to_str())) {
                        (Some(dir), Some(key)) => remove_entry(dir, key).map(|_| SaveStats::default()).map_err(|e| e.to_string()),
                        _ => Err("Invalid recovery location".into()),
                    };
                    let c = AutosaveCompletion { revision, result };
                    let mut state = shared.0.lock().unwrap_or_else(PoisonError::into_inner);
                    state.last = Some(c.clone());
                    state.completion = Some(c);
                }
                return;
            };
            job
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| save_checkpoint(&mut writer, &mut verified, &job, &bundle, &sidecar)))
            .map_err(|_| "Autosave could not complete because its worker panicked; your document remains open".to_string())
            .and_then(|r| r.map_err(|e| e.to_string()));
        let completion = AutosaveCompletion { revision: job.info.revision, result };
        let mut state = shared.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.last = Some(completion.clone());
        state.completion = Some(completion);
    }
}
fn bundle_bytes(root: &Path) -> Result<u64> {
    let mut total = 0u64;
    for sub in ["tiles", "blobs"] {
        let path = root.join(sub);
        if !path.exists() {
            continue;
        }
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(FormatError::corrupt("unexpected recovery object directory"));
            }
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}
fn prepare_state(
    writer: &mut PcraftWriter,
    verified: &mut HashMap<PathBuf, FileFingerprint>,
    doc: &Document,
    opts: &SaveOptions,
    bundle: &Path,
    bytes: &mut u64,
    stats: &mut SaveStats,
) -> Result<String> {
    let prepared = writer.prepare(doc, opts)?;
    if prepared.manifest.len() as u64 > MAX_DESCRIPTOR
        || prepared.decoded_bytes > LoadOptions::default().max_total_bytes
        || prepared.objects.values().any(|object| matches!(object, store::Object::Blob(blob) if blob.len() > LoadOptions::default().max_blob_bytes))
    {
        return Err(FormatError::LimitExceeded("recovery state exceeds restore limits".into()));
    }
    for (name, object) in prepared.objects {
        let path = bundle.join(&name);
        if std::fs::symlink_metadata(&path).is_ok() {
            ensure_file(&path)?;
            let fingerprint = file_fingerprint(&path)?;
            if verified.get(&path) != Some(&fingerprint) {
                // First reuse verifies canonical lossless bytes. Later saves
                // reuse unchanged file identity without rehashing large tiles.
                let expected = object.compressed();
                let matches = fingerprint.len == expected.len() as u64 && read_bounded(&path, expected.len() as u64)? == expected;
                if !matches {
                    let next = bytes.saturating_sub(fingerprint.len).saturating_add(expected.len() as u64);
                    if next > MAX_RECOVERY_BYTES {
                        return Err(FormatError::LimitExceeded("recovery storage is full".into()));
                    }
                    write_atomic(&path, &expected)?;
                    *bytes = next;
                }
                verified.insert(path.clone(), file_fingerprint(&path)?);
            }
            if matches!(object, store::Object::Tile(..)) {
                stats.tiles_reused += 1;
            }
            continue;
        }
        let compressed = object.compressed();
        let next = bytes.saturating_add(compressed.len() as u64);
        if next > MAX_RECOVERY_BYTES {
            return Err(FormatError::LimitExceeded("recovery storage is full (8 GiB); save your document and free recovery storage".into()));
        }
        write_atomic(&path, &compressed)?;
        verified.insert(path.clone(), file_fingerprint(&path)?);
        *bytes = next;
        match object {
            store::Object::Tile(..) => stats.tiles_written += 1,
            store::Object::Blob(..) => stats.blobs_written += 1,
        }
    }
    String::from_utf8(prepared.manifest).map_err(|_| FormatError::corrupt("invalid UTF-8 manifest"))
}
fn save_checkpoint(writer: &mut PcraftWriter, verified: &mut HashMap<PathBuf, FileFingerprint>, job: &Job, bundle: &Path, sidecar: &Path) -> Result<SaveStats> {
    // Serialize publication and deferred cleanup so a retired root cannot be
    // deleted while a replacement checkpoint is being written.
    let root_registry = roots().lock().unwrap_or_else(PoisonError::into_inner);
    History::from_checkpoint(job.history.clone()).map_err(FormatError::corrupt)?;
    if let Some(parent) = bundle.parent() {
        create_directory(parent)?;
    }
    create_directory(bundle)?;
    for sub in ["tiles", "blobs"] {
        create_directory(&bundle.join(sub))?;
    }
    let mut bytes = bundle_bytes(bundle)?;
    let mut stats = SaveStats::default();
    let current = prepare_state(writer, verified, &job.snapshot, &job.opts, bundle, &mut bytes, &mut stats)?;
    let mut save_states = |states: &[HistoryState]| -> Result<Vec<StoredState>> {
        let mut stored = Vec::with_capacity(states.len());
        for state in states {
            let doc = state.load_document().map_err(FormatError::corrupt)?;
            let manifest = prepare_state(writer, verified, &doc, &SaveOptions::default(), bundle, &mut bytes, &mut stats)?;
            stored.push(StoredState { label: state.label.clone(), has_selection: state.has_selection(), manifest, layers: state.layers.clone().into() });
        }
        Ok(stored)
    };
    let undo = save_states(&job.history.undo)?;
    let redo = save_states(&job.history.redo)?;
    let checkpoint = Checkpoint {
        version: 1,
        current,
        undo,
        redo,
        current_label: job.history.current_label.clone(),
        current_layers: job.history.current_layers.clone().into(),
        max_states: job.history.max_states,
        max_bytes: job.history.max_bytes,
    };
    let descriptor = serde_json::to_vec(&Descriptor { info: job.info.clone(), checkpoint: Some(checkpoint), context: job.context.clone() })?;
    if descriptor.len() as u64 > MAX_DESCRIPTOR || bytes.saturating_add(descriptor.len() as u64) > MAX_RECOVERY_BYTES {
        return Err(FormatError::LimitExceeded("recovery checkpoint is too large".into()));
    }
    stats.manifest_bytes = descriptor.len();
    if std::fs::symlink_metadata(sidecar).is_ok() {
        ensure_file(sidecar)?;
    }
    write_atomic(sidecar, &descriptor)?;
    // A recovered lazy history pins the entire root conservatively. This keeps
    // cold states valid after another checkpoint replaces the descriptor.
    let pinned = root_registry.get(bundle).and_then(Weak::upgrade).is_some();
    if !pinned {
        let mut hashes = std::collections::HashSet::new();
        let published: Descriptor = match serde_json::from_slice(&descriptor) {
            Ok(published) => published,
            Err(_) => return Ok(stats),
        };
        if let Some(checkpoint) = published.checkpoint {
            for manifest in std::iter::once(&checkpoint.current).chain(checkpoint.undo.iter().chain(checkpoint.redo.iter()).map(|s| &s.manifest)) {
                let value: serde_json::Value = match serde_json::from_str(manifest) {
                    Ok(value) => value,
                    Err(_) => return Ok(stats),
                };
                collect_hashes(&value, &mut hashes);
            }
        }
        for sub in ["tiles", "blobs"] {
            if let Ok(entries) = std::fs::read_dir(bundle.join(sub)) {
                for entry in entries.flatten() {
                    if entry.path().file_stem().and_then(|s| s.to_str()).is_some_and(|h| !hashes.contains(h)) && std::fs::remove_file(entry.path()).is_ok() {
                        stats.objects_removed += 1;
                    }
                }
            }
        }
        verified.retain(|path, _| path.file_stem().and_then(|stem| stem.to_str()).is_some_and(|hash| hashes.contains(hash)));
    }
    Ok(stats)
}

#[derive(Debug)]
struct RootLease {
    root: PathBuf,
    retired: AtomicBool,
    tiles: Mutex<HashMap<String, Weak<Tile>>>,
    blobs: Mutex<HashMap<String, Weak<Vec<u8>>>>,
}
impl Drop for RootLease {
    fn drop(&mut self) {
        // Lazy history handles can drop on the UI thread: cleanup is queued.
        if self.retired.load(Ordering::Acquire)
            && std::fs::symlink_metadata(self.root.with_extension("json")).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            && let Some(tx) = cleanup_sender()
        {
            let _ = tx.send(self.root.clone());
        }
    }
}
type Roots = Mutex<HashMap<PathBuf, Weak<RootLease>>>;
fn roots() -> &'static Roots {
    static ROOTS: OnceLock<Roots> = OnceLock::new();
    ROOTS.get_or_init(Mutex::default)
}
fn root_lease(root: &Path) -> Arc<RootLease> {
    let mut roots = roots().lock().unwrap_or_else(PoisonError::into_inner);
    roots.retain(|_, lease| lease.strong_count() > 0);
    if let Some(lease) = roots.get(root).and_then(Weak::upgrade) {
        return lease;
    }
    let lease = Arc::new(RootLease { root: root.to_path_buf(), retired: AtomicBool::new(false), tiles: Mutex::default(), blobs: Mutex::default() });
    roots.insert(root.to_path_buf(), Arc::downgrade(&lease));
    lease
}
fn cleanup_sender() -> Option<&'static mpsc::Sender<PathBuf>> {
    static CLEANUP: OnceLock<Option<mpsc::Sender<PathBuf>>> = OnceLock::new();
    CLEANUP
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel::<PathBuf>();
            std::thread::Builder::new()
                .name("recovery-cleanup".into())
                .spawn(move || {
                    while let Ok(root) = rx.recv() {
                        let registry = roots().lock().unwrap_or_else(PoisonError::into_inner);
                        let pinned = registry.get(&root).and_then(Weak::upgrade).is_some();
                        if !pinned
                            && std::fs::symlink_metadata(root.with_extension("json")).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                            && ensure_directory(&root).is_ok()
                        {
                            let _ = std::fs::remove_dir_all(root);
                        }
                    }
                })
                .ok()
                .map(|_| tx)
        })
        .as_ref()
}
#[derive(Debug)]
struct RecoverySource {
    lease: Arc<RootLease>,
    manifest: String,
}
impl Source for RecoverySource {
    fn get(&self, path: &str, max: usize) -> Result<Vec<u8>> {
        if path == store::MANIFEST {
            if self.manifest.len() > max {
                return Err(FormatError::LimitExceeded("recovery manifest too large".into()));
            }
            return Ok(self.manifest.as_bytes().to_vec());
        }
        let relative = Path::new(path);
        if relative.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
            return Err(FormatError::corrupt("unsafe recovery object path"));
        }
        ensure_directory(&self.lease.root)?;
        if let Some(parent) = relative.parent() {
            ensure_directory(&self.lease.root.join(parent))?;
        }
        read_bounded(&self.lease.root.join(relative), max as u64)
    }
}
impl RecoverySource {
    fn load_native(&self) -> Result<Document> {
        // Reuse the undo archive's native decoder cache. Weak entries share
        // unchanged live pixels/blobs without retaining a second RAM copy.
        let mut tiles = self.lease.tiles.lock().unwrap_or_else(PoisonError::into_inner);
        let mut blobs = self.lease.blobs.lock().unwrap_or_else(PoisonError::into_inner);
        tiles.retain(|_, value| value.strong_count() > 0);
        blobs.retain(|_, value| value.strong_count() > 0);
        store::load_cached(self, &LoadOptions::default(), Some(&mut tiles), Some(&mut blobs))
    }
}
impl ArchivedDocument for RecoverySource {
    fn load(&self) -> std::result::Result<Arc<Document>, String> {
        self.load_native().map(Arc::new).map_err(|e| e.to_string())
    }
}

fn descriptor(entry: &RecoveryEntry) -> Result<Descriptor> {
    if !safe_key(&entry.info.key) || entry.bundle.file_name().and_then(|n| n.to_str()) != Some(format!("{}.pcraft", entry.info.key).as_str()) {
        return Err(FormatError::corrupt("unsafe recovery key"));
    }
    if let Some(parent) = entry.bundle.parent() {
        ensure_directory(parent)?;
    }
    let path = entry.bundle.with_extension("json");
    ensure_directory(&entry.bundle)?;
    let result: Descriptor = serde_json::from_slice(&read_bounded(&path, MAX_DESCRIPTOR)?)?;
    if result.info.key != entry.info.key {
        return Err(FormatError::corrupt("recovery descriptor key mismatch"));
    }
    Ok(result)
}
/// Discovery errors are reported without deleting suspect files.
pub fn list_recovery_checked(recovery_dir: &Path) -> (Vec<RecoveryEntry>, Vec<String>) {
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    if std::fs::symlink_metadata(recovery_dir).is_ok()
        && let Err(e) = ensure_directory(recovery_dir)
    {
        return (entries, vec![e.to_string()]);
    }
    let rd = match std::fs::read_dir(recovery_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (entries, errors),
        Err(e) => return (entries, vec![format!("Could not inspect recovery storage: {e}")]),
    };
    for item in rd {
        let item = match item {
            Ok(item) => item,
            Err(e) => {
                errors.push(e.to_string());
                continue;
            }
        };
        let path = item.path();
        if !path.extension().is_some_and(|ext| ext == "json") {
            continue;
        }
        let read = (|| -> Result<Descriptor> { Ok(serde_json::from_slice(&read_bounded(&path, MAX_DESCRIPTOR)?)?) })();
        match read {
            Ok(d) if safe_key(&d.info.key) && path.file_stem().and_then(|n| n.to_str()) == Some(d.info.key.as_str()) => {
                let bundle = recovery_dir.join(format!("{}.pcraft", d.info.key));
                if ensure_directory(&bundle).is_ok() && (d.checkpoint.is_some() || bundle.join(store::MANIFEST).is_file()) {
                    entries.push(RecoveryEntry { info: d.info, bundle });
                } else {
                    errors.push(format!("Recovery bundle is missing: {}", path.display()));
                }
            }
            Ok(_) => errors.push(format!("Unsafe recovery key in {}", path.display())),
            Err(e) => errors.push(format!("Could not read recovery {}: {e}", path.display())),
        }
    }
    entries.sort_by(|a, b| b.info.saved_at.cmp(&a.info.saved_at).then(a.info.key.cmp(&b.info.key)));
    (entries, errors)
}
pub fn list_recovery(recovery_dir: &Path) -> Vec<RecoveryEntry> {
    list_recovery_checked(recovery_dir).0
}
pub fn recover_checkpoint(entry: &RecoveryEntry) -> Result<(Document, HistoryCheckpoint)> {
    recover_checkpoint_with_context(entry).map(|(doc, history, _)| (doc, history))
}
pub fn recover_checkpoint_with_context(entry: &RecoveryEntry) -> Result<(Document, HistoryCheckpoint, serde_json::Value)> {
    if !safe_key(&entry.info.key) || entry.bundle.file_name().and_then(|n| n.to_str()) != Some(format!("{}.pcraft", entry.info.key).as_str()) {
        return Err(FormatError::corrupt("unsafe recovery key"));
    }
    // Pin before reading publication: a concurrent commit must not collect
    // objects named by the descriptor that this recovery reader observes.
    let lease = root_lease(&entry.bundle);
    let descriptor = descriptor(entry)?;
    if serde_json::to_vec(&descriptor.context)?.len() > 64 << 10 {
        return Err(FormatError::LimitExceeded("recovery context exceeds 64 KiB".into()));
    }
    let Some(checkpoint) = descriptor.checkpoint else {
        let bytes = read_bounded(&entry.bundle.join(store::MANIFEST), LoadOptions::default().max_manifest_bytes as u64)?;
        let manifest = String::from_utf8(bytes).map_err(|_| FormatError::corrupt("invalid legacy manifest UTF-8"))?;
        let doc = store::load(&RecoverySource { lease: lease.clone(), manifest }, &LoadOptions::default())?;
        return Ok((doc, History::default().checkpoint(), descriptor.context));
    };
    if checkpoint.version != 1 {
        return Err(FormatError::Unsupported("newer recovery checkpoint version".into()));
    }
    if checkpoint.undo.len().saturating_add(checkpoint.redo.len()) > 10_000 {
        return Err(FormatError::LimitExceeded("too many recovery history states".into()));
    }
    let current = RecoverySource { lease: lease.clone(), manifest: checkpoint.current }.load_native()?;
    let restore = |states: Vec<StoredState>| -> Result<Vec<HistoryState>> {
        states
            .into_iter()
            .map(|state| {
                let source = RecoverySource { lease: lease.clone(), manifest: state.manifest };
                // Validate manifest syntax now; object reads remain lazy and fallible.
                store::read_manifest(&source, &LoadOptions::default())?;
                let mut restored = HistoryState::from_archive(state.label, state.has_selection, Arc::new(source));
                restored.layers = state.layers.into();
                Ok(restored)
            })
            .collect()
    };
    let history = HistoryCheckpoint {
        undo: restore(checkpoint.undo)?,
        redo: restore(checkpoint.redo)?,
        current_label: checkpoint.current_label,
        current_layers: checkpoint.current_layers.into(),
        max_states: checkpoint.max_states,
        max_bytes: checkpoint.max_bytes,
    };
    History::from_checkpoint(history.clone()).map_err(FormatError::corrupt)?;
    Ok((current, history, descriptor.context))
}
pub fn recover(entry: &RecoveryEntry) -> Result<Document> {
    recover_checkpoint(entry).map(|(doc, _)| doc)
}
pub fn discard_recovery(recovery_dir: &Path, entry: &RecoveryEntry) -> Result<()> {
    if !safe_key(&entry.info.key) || entry.bundle != recovery_dir.join(format!("{}.pcraft", entry.info.key)) {
        return Err(FormatError::corrupt("unsafe recovery key"));
    }
    remove_entry(recovery_dir, &entry.info.key)
}
fn remove_entry(dir: &Path, key: &str) -> Result<()> {
    if !safe_key(key) {
        return Err(FormatError::corrupt("unsafe recovery key"));
    }
    let registry = roots().lock().unwrap_or_else(PoisonError::into_inner);
    ensure_directory(dir)?;
    let root = dir.join(format!("{key}.pcraft"));
    if std::fs::symlink_metadata(&root).is_ok() {
        ensure_directory(&root)?;
    }
    let sidecar = dir.join(format!("{key}.json"));
    if std::fs::symlink_metadata(&sidecar).is_ok() {
        ensure_file(&sidecar)?;
        std::fs::remove_file(sidecar)?;
    }
    let active = registry.get(&root).and_then(Weak::upgrade);
    if let Some(active) = active {
        active.retired.store(true, Ordering::Release);
    } else if root.exists() {
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}

fn collect_hashes(value: &serde_json::Value, out: &mut std::collections::HashSet<String>) {
    match value {
        serde_json::Value::String(s) if s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) => {
            out.insert(s.clone());
        }
        serde_json::Value::Array(a) => {
            for v in a {
                collect_hashes(v, out);
            }
        }
        serde_json::Value::Object(o) => {
            for v in o.values() {
                collect_hashes(v, out);
            }
        }
        _ => {}
    }
}

fn ensure_directory(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(FormatError::corrupt("recovery directory is a symlink or not a directory"));
    }
    Ok(())
}
fn create_directory(path: &Path) -> Result<()> {
    if std::fs::symlink_metadata(path).is_err() {
        std::fs::create_dir_all(path)?;
    }
    ensure_directory(path)
}
fn ensure_file(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(FormatError::corrupt("recovery file is a symlink or not a regular file"));
    }
    Ok(())
}
fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    ensure_file(path)?;
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > max {
        return Err(FormatError::LimitExceeded("recovery file exceeds read limit".into()));
    }
    ensure_file(path)?;
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(FormatError::LimitExceeded("recovery file exceeds read limit".into()));
    }
    Ok(bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileFingerprint {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}
fn file_fingerprint(path: &Path) -> Result<FileFingerprint> {
    let metadata = std::fs::symlink_metadata(path)?;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    Ok(FileFingerprint {
        len: metadata.len(),
        modified: metadata.modified().ok(),
        #[cfg(unix)]
        device: metadata.dev(),
        #[cfg(unix)]
        inode: metadata.ino(),
    })
}
