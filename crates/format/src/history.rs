//! Session-private, content-addressed scratch storage for cold undo states.
//! Files are allocated only on the first spill; a snapshot lease owns its
//! manifest and references shared compressed objects until its last clone drops.

use photocraft_raster::Tile;
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak, mpsc};

use photocraft_doc::Document;

use crate::store::{self, PcraftWriter, Source};
use crate::{FormatError, LoadOptions, Result, SaveOptions};

type SharedTiles = Arc<Mutex<HashMap<String, Weak<Tile>>>>;
type SharedBlobs = Arc<Mutex<HashMap<String, Weak<Vec<u8>>>>>;

#[derive(Clone, Debug)]
pub struct HistoryArchive {
    inner: Arc<Mutex<Archive>>,
    used: Arc<AtomicU64>,
    releases: mpsc::Sender<Release>,
    pending_releases: Arc<AtomicUsize>,
}

#[derive(Clone, Debug)]
pub struct HistorySnapshot {
    lease: Arc<Lease>,
}

#[derive(Debug)]
struct Lease {
    _archive: Arc<Mutex<Archive>>,
    root: PathBuf,
    tiles: SharedTiles,
    blobs: SharedBlobs,
    manifest: String,
    objects: Vec<String>,
    releases: mpsc::Sender<Release>,
    pending_releases: Arc<AtomicUsize>,
}

#[derive(Debug)]
struct Release {
    manifest: String,
    objects: Vec<String>,
}

struct Archive {
    requested_root: Option<PathBuf>,
    dir: Option<tempfile::TempDir>,
    budget: u64,
    used: Arc<AtomicU64>,
    next_id: u64,
    files: BTreeMap<String, FileEntry>,
    writer: PcraftWriter,
    tiles: SharedTiles,
    blobs: SharedBlobs,
    releases: mpsc::Receiver<Release>,
    pending_releases: Arc<AtomicUsize>,
}

impl std::fmt::Debug for Archive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HistoryArchive")
            .field("budget", &self.budget)
            .field("used", &self.used.load(Ordering::Relaxed))
            .field("files", &self.files.len())
            .finish()
    }
}

struct FileEntry {
    bytes: u64,
    refs: usize,
}

impl HistoryArchive {
    /// `root` is an existing scratch parent, not a directory owned by this
    /// archive. `None` uses the platform temporary directory. Zero disables spill.
    pub fn new(root: Option<PathBuf>, max_disk_bytes: u64) -> Self {
        let used = Arc::new(AtomicU64::new(0));
        let (tx, rx) = mpsc::channel();
        let pending_releases = Arc::new(AtomicUsize::new(0));
        Self {
            used: used.clone(),
            releases: tx,
            pending_releases: pending_releases.clone(),
            inner: Arc::new(Mutex::new(Archive {
                requested_root: root,
                dir: None,
                budget: max_disk_bytes,
                used,
                next_id: 0,
                files: BTreeMap::new(),
                writer: PcraftWriter::new(),
                tiles: Arc::new(Mutex::new(HashMap::new())),
                blobs: Arc::new(Mutex::new(HashMap::new())),
                releases: rx,
                pending_releases,
            })),
        }
    }

    /// Reclaim dropped leases on a storage worker. Snapshot Drop only queues a
    /// release; it never waits for a concurrent compression or touches disk.
    pub fn collect_garbage(&self) -> Result<()> {
        let mut a = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        a.reclaim();
        if a.files.values().any(|file| file.refs == 0) {
            return Err(FormatError::Io(std::io::Error::other("could not remove unused scratch history files")));
        }
        Ok(())
    }

    pub fn has_pending_releases(&self) -> bool {
        self.pending_releases.load(Ordering::Acquire) > 0
    }

    pub fn bytes_used(&self) -> u64 {
        self.used.load(Ordering::Relaxed)
    }

    /// Existing leases are never invalidated. Release old snapshots before
    /// lowering the budget below their currently allocated bytes.
    pub fn set_budget(&self, bytes: u64) -> Result<()> {
        let mut a = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let used = a.used.load(Ordering::Relaxed);
        if used > bytes {
            return Err(FormatError::LimitExceeded(format!("scratch history uses {} bytes; release old states before lowering to {bytes}", used)));
        }
        a.budget = bytes;
        Ok(())
    }

    pub fn store(&self, doc: &Document) -> Result<HistorySnapshot> {
        self.store_with_limits(doc, &LoadOptions::default())
    }

    /// Apply tighter admission limits. A published snapshot always fits the
    /// default decoder limits, so releasing its resident state remains safe.
    pub fn store_with_limits(&self, doc: &Document, limits: &LoadOptions) -> Result<HistorySnapshot> {
        let defaults = LoadOptions::default();
        let manifest_limit = limits.max_manifest_bytes.min(defaults.max_manifest_bytes);
        let blob_limit = limits.max_blob_bytes.min(defaults.max_blob_bytes);
        let total_limit = limits.max_total_bytes.min(defaults.max_total_bytes);
        let mut a = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        a.reclaim();
        if a.budget == 0 {
            return Err(FormatError::LimitExceeded("scratch history is disabled".into()));
        }
        // prepare retains only Weak tile identities: no compressed data or
        // strong pixel references remain in the writer after this call.
        let prepared = a.writer.prepare(doc, &SaveOptions::default())?;
        if prepared.manifest.len() > manifest_limit
            || prepared.decoded_bytes > total_limit
            || prepared.objects.values().any(|object| matches!(object, store::Object::Blob(blob) if blob.len() > blob_limit))
        {
            return Err(FormatError::LimitExceeded("scratch snapshot exceeds restore limits".into()));
        }
        {
            let mut tiles = a.tiles.lock().unwrap_or_else(PoisonError::into_inner);
            tiles.retain(|_, tile| tile.strong_count() > 0);
            tiles.extend(a.writer.hash_cache.values().map(|(tile, hash)| (hash.clone(), tile.clone())));
        }
        {
            let mut blobs = a.blobs.lock().unwrap_or_else(PoisonError::into_inner);
            blobs.retain(|_, blob| blob.strong_count() > 0);
            for (path, object) in &prepared.objects {
                if let store::Object::Blob(blob) = object
                    && let Some(hash) = path.strip_prefix("blobs/").and_then(|p| p.strip_suffix(".zst"))
                {
                    blobs.insert(hash.to_owned(), Arc::downgrade(blob));
                }
            }
        }
        let id = a.next_id;
        a.next_id = id.checked_add(1).ok_or_else(|| FormatError::LimitExceeded("scratch snapshot id exhausted".into()))?;
        let manifest = format!("state-{id}.json");
        let objects: Vec<String> = prepared.objects.keys().cloned().collect();
        let result = (|| {
            a.write_new(&manifest, &prepared.manifest)?;
            for (path, object) in &prepared.objects {
                if a.files.get(path).is_some_and(|f| f.refs == 0) {
                    return Err(FormatError::Io(std::io::Error::other("could not reclaim an incomplete scratch object")));
                }
                if !a.files.contains_key(path) {
                    // Only one compressed tile/blob is buffered at a time.
                    let compressed = object.compressed();
                    a.write_new(path, &compressed)?;
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            a.reclaim();
            return Err(e);
        }
        if let Some(f) = a.files.get_mut(&manifest) {
            f.refs = 1;
        }
        for path in &objects {
            if let Some(f) = a.files.get_mut(path) {
                f.refs = f.refs.saturating_add(1);
            }
        }
        let root = a.dir.as_ref().ok_or_else(|| FormatError::corrupt("scratch directory is missing"))?.path().to_owned();
        Ok(HistorySnapshot {
            lease: Arc::new(Lease {
                _archive: self.inner.clone(),
                root,
                tiles: a.tiles.clone(),
                blobs: a.blobs.clone(),
                manifest,
                objects,
                releases: self.releases.clone(),
                pending_releases: self.pending_releases.clone(),
            }),
        })
    }
}

impl Archive {
    fn ensure_dir(&mut self) -> Result<PathBuf> {
        if let Some(dir) = &self.dir {
            return Ok(dir.path().to_owned());
        }
        let mut builder = tempfile::Builder::new();
        builder.prefix("photocraft-history-");
        let dir = match &self.requested_root {
            Some(root) => builder.tempdir_in(root)?,
            None => builder.tempdir()?,
        };
        let root = dir.path().to_owned();
        std::fs::create_dir(root.join("tiles"))?;
        std::fs::create_dir(root.join("blobs"))?;
        self.dir = Some(dir);
        Ok(root)
    }

    fn write_new(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        let n = u64::try_from(bytes.len()).map_err(|_| FormatError::LimitExceeded("scratch object too large".into()))?;
        let total = self.used.load(Ordering::Relaxed).checked_add(n).ok_or_else(|| FormatError::LimitExceeded("scratch size overflow".into()))?;
        if total > self.budget {
            return Err(FormatError::LimitExceeded(format!("scratch history exceeds {} bytes", self.budget)));
        }
        let root = self.ensure_dir()?;
        // Files live in a private directory and have unique names. Publish leases
        // only after every write succeeds; scratch needs no per-tile fsync.
        self.used.store(total, Ordering::Relaxed);
        self.files.insert(name.to_owned(), FileEntry { bytes: n, refs: 0 });
        let mut file = std::fs::OpenOptions::new().create_new(true).write(true).open(root.join(name))?;
        file.write_all(bytes)?;
        Ok(())
    }

    fn reclaim(&mut self) {
        while let Ok(release) = self.releases.try_recv() {
            self.pending_releases.fetch_sub(1, Ordering::AcqRel);
            for path in std::iter::once(&release.manifest).chain(release.objects.iter()) {
                if let Some(file) = self.files.get_mut(path) {
                    file.refs = file.refs.saturating_sub(1);
                }
            }
        }
        let Some(root) = self.dir.as_ref().map(|d| d.path().to_owned()) else { return };
        let garbage: Vec<String> = self.files.iter().filter(|(_, f)| f.refs == 0).map(|(p, _)| p.clone()).collect();
        for path in garbage {
            let result = std::fs::remove_file(root.join(&path));
            if (result.is_ok() || result.as_ref().is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound))
                && let Some(f) = self.files.remove(&path)
            {
                self.used.fetch_sub(f.bytes, Ordering::Relaxed);
            }
            // Failed deletion stays accounted and is retried on later operations.
        }
    }
}

impl HistorySnapshot {
    pub fn load(&self) -> Result<Document> {
        self.load_with_limits(&LoadOptions::default())
    }

    pub fn load_with_limits(&self, limits: &LoadOptions) -> Result<Document> {
        // Lease owns immutable published files and keeps the directory alive.
        // Never wait for the archive writer mutex on an undo read.
        let source = SnapshotSource { root: self.lease.root.clone(), manifest: &self.lease.manifest };
        let opts = LoadOptions { preserve_ids: true, ..*limits };
        let mut cache = self.lease.tiles.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let mut blobs = self.lease.blobs.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let doc = store::load_cached(&source, &opts, Some(&mut cache), Some(&mut blobs))?;
        self.lease.tiles.lock().unwrap_or_else(PoisonError::into_inner).extend(cache);
        self.lease.blobs.lock().unwrap_or_else(PoisonError::into_inner).extend(blobs);
        Ok(doc)
    }
}

struct SnapshotSource<'a> {
    root: PathBuf,
    manifest: &'a str,
}

impl Source for SnapshotSource<'_> {
    fn get(&self, path: &str, max: usize) -> Result<Vec<u8>> {
        let path = if path == store::MANIFEST { self.manifest } else { path };
        store::DirSource { root: self.root.clone() }.get(path, max)
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.pending_releases.fetch_add(1, Ordering::AcqRel);
        if self.releases.send(Release { manifest: std::mem::take(&mut self.manifest), objects: std::mem::take(&mut self.objects) }).is_err() {
            self.pending_releases.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::{ColorMode, Layer, LayerContent, SampleType, Size, Surface};
    use photocraft_geom::TileCoord;

    fn document() -> Document {
        let mut doc = Document::new("Scratch test", Size::new(256, 256), ColorMode::Rgb, SampleType::U8);
        let mut pixels = Surface::new(PixelFormat::RGBA8);
        pixels.tile_mut(TileCoord::new(0, 0)).bytes_mut().fill(123);
        doc.layers.clear();
        doc.layers.push(Layer::new("Pixels", LayerContent::Raster(pixels)));
        let mut selection = Surface::new(PixelFormat::GRAY8);
        selection.tile_mut(TileCoord::new(0, 0)).bytes_mut().fill(45);
        doc.selection = Some(selection);
        doc
    }

    #[test]
    fn lazy_roundtrip_shared_tiles_and_lease_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let archive = HistoryArchive::new(Some(root.path().to_owned()), 1 << 20);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        let doc = document();
        let a = archive.store(&doc).unwrap();
        let first = archive.bytes_used();
        let b = archive.store(&doc).unwrap();
        assert!(archive.bytes_used() > first);
        assert!(archive.bytes_used() < first * 2);
        let restored = a.load().unwrap();
        assert_eq!(doc, restored);
        let old = doc.layers.first().unwrap().surface().unwrap().tile(TileCoord::new(0, 0)).unwrap();
        let new = restored.layers.first().unwrap().surface().unwrap().tile(TileCoord::new(0, 0)).unwrap();
        assert!(Arc::ptr_eq(old, new));
        let clone = a.clone();
        drop(a);
        drop(b);
        archive.collect_garbage().unwrap();
        assert_eq!(archive.bytes_used(), first);
        drop(clone);
        archive.collect_garbage().unwrap();
        assert_eq!(archive.bytes_used(), 0);
    }

    #[test]
    fn quota_failure_rolls_back_and_preserves_existing_snapshot() {
        let archive = HistoryArchive::new(None, 1 << 20);
        let doc = document();
        let a = archive.store(&doc).unwrap();
        let used = archive.bytes_used();
        archive.set_budget(used).unwrap();
        assert!(archive.store(&doc).is_err());
        assert_eq!(archive.bytes_used(), used);
        assert_eq!(a.load().unwrap(), doc);
        assert!(archive.set_budget(used - 1).is_err());
        drop(a);
        archive.collect_garbage().unwrap();
        archive.set_budget(0).unwrap();
        assert!(archive.store(&doc).is_err());
        assert_eq!(archive.bytes_used(), 0);
    }

    #[test]
    fn cold_corruption_and_limits_return_errors() {
        let archive = HistoryArchive::new(None, 1 << 20);
        let doc = document();
        let snapshot = archive.store(&doc).unwrap();
        drop(doc);
        let limits = LoadOptions { max_total_bytes: 1, ..Default::default() };
        assert!(snapshot.load_with_limits(&limits).is_err());
        let a = archive.inner.lock().unwrap();
        let root = a.dir.as_ref().unwrap().path();
        let tile = snapshot.lease.objects.iter().find(|p| p.starts_with("tiles/")).unwrap();
        std::fs::write(root.join(tile), b"broken zstd").unwrap();
        drop(a);
        assert!(snapshot.load().is_err());
    }

    #[test]
    fn missing_scratch_parent_is_fallible_and_does_not_reserve_bytes() {
        let root = tempfile::tempdir().unwrap();
        let archive = HistoryArchive::new(Some(root.path().join("missing")), 1 << 20);
        assert!(archive.store(&document()).is_err());
        assert_eq!(archive.bytes_used(), 0);
    }

    #[test]
    fn dropping_snapshot_does_not_acquire_storage_lock() {
        let archive = HistoryArchive::new(None, 1 << 20);
        let snapshot = archive.store(&document()).unwrap();
        let held = archive.inner.lock().unwrap();
        drop(snapshot);
        // Drop merely queues: bytes remain charged until the worker reclaims.
        assert!(archive.bytes_used() > 0);
        drop(held);
        archive.collect_garbage().unwrap();
        assert_eq!(archive.bytes_used(), 0);
    }

    #[test]
    fn loading_snapshot_does_not_acquire_writer_lock() {
        let archive = HistoryArchive::new(None, 1 << 20);
        let doc = document();
        let snapshot = archive.store(&doc).unwrap();
        let held = archive.inner.lock().unwrap();
        held.tiles.lock().unwrap().clear();
        assert_eq!(snapshot.load().unwrap(), doc);
        drop(held);
    }

    #[test]
    fn live_binary_blobs_remain_shared_when_restoring() {
        let archive = HistoryArchive::new(None, 1 << 20);
        let mut doc = document();
        let blob = Arc::new(vec![31; 128 << 10]);
        doc.icc_profile = Some(blob.clone());
        doc.metadata.exif = Some(blob.clone());
        let snapshot = archive.store(&doc).unwrap();
        let restored = snapshot.load().unwrap();
        assert!(Arc::ptr_eq(&blob, restored.icc_profile.as_ref().unwrap()));
        assert!(Arc::ptr_eq(&blob, restored.metadata.exif.as_ref().unwrap()));
        let limits = LoadOptions { max_blob_bytes: 1, ..Default::default() };
        assert!(snapshot.load_with_limits(&limits).is_err());
    }

    #[test]
    fn admission_limits_reject_before_publishing_any_files() {
        let root = tempfile::tempdir().unwrap();
        let archive = HistoryArchive::new(Some(root.path().to_owned()), 1 << 20);
        let mut doc = document();
        doc.icc_profile = Some(Arc::new(vec![31; 1024]));
        for limits in [
            LoadOptions { max_manifest_bytes: 1, ..Default::default() },
            LoadOptions { max_blob_bytes: 1, ..Default::default() },
            LoadOptions { max_total_bytes: 1, ..Default::default() },
        ] {
            assert!(archive.store_with_limits(&doc, &limits).is_err());
            assert_eq!(archive.bytes_used(), 0);
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        }
        let snapshot = archive.store(&doc).unwrap();
        assert_eq!(snapshot.load().unwrap(), doc);
    }

    #[test]
    fn cold_pixels_are_lossless_at_every_depth() {
        for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            let archive = HistoryArchive::new(None, 8 << 20);
            let mut doc = document();
            doc.depth = format.sample;
            let mut pixels = Surface::new(format);
            let data = pixels.tile_mut(TileCoord::new(-1, 2)).bytes_mut();
            for (i, byte) in data.iter_mut().enumerate() {
                *byte = (i % 251) as u8;
            }
            doc.layers.clear();
            doc.layers.push(Layer::new("Any depth", LayerContent::Raster(pixels)));
            let snapshot = archive.store(&doc).unwrap();
            let expected = doc.clone();
            drop(doc);
            // Force the cold path even though the comparison document shares tiles.
            archive.inner.lock().unwrap().tiles.lock().unwrap().clear();
            assert_eq!(snapshot.load().unwrap(), expected);
        }
    }
}
