//! Linked smart objects follow their files while the document is open, as in Photoshop
//! ("Linked Smart Objects update automatically when the Photoshop document is open, provided the
//! source file has changed", <https://helpx.adobe.com/photoshop/desktop/create-manage-layers/smart-objects/update-linked-smart-objects.html>):
//! a file another app saves (VectorCraft, LightCraft, Illustrator) updates its layers without
//! Layer › Smart Objects › Update Modified Content.
//!
//! The active document's linked files are compared by size and modification time with what they
//! were when last seen; the layers of a file whose stamp moved are re-rendered from it, one
//! Update Modified Content undo step per file. The desktop app takes the stamps every two seconds
//! and when its window comes back to the front, on a worker thread so a file on a slow network
//! share never holds up the interface ([`LinkWatchExt::start_link_scan`](crate::link_watch::LinkWatchExt::start_link_scan), [`LinkWatchExt::poll_link_scan`](crate::link_watch::LinkWatchExt::poll_link_scan));
//! `layer.smartObjects.updateChanged` takes them on the spot (CLI, agents).
//!
//! Stamps are kept per document: one in the background is looked at when it comes to the front.
//! A file seen for the first time is only remembered (one changed while its document was closed
//! keeps waiting for Update Modified Content, as in Photoshop). For a historical XCF, the first
//! look also checks its saved fingerprint and warns if relinking is needed. This fingerprint
//! check is retried when the file couldn't be read. Regular refreshes of files that can't be read
//! are tried again once the stamp changes. Only smart objects
//! of the document itself follow their files, not ones nested in another smart object's contents.
//! While a background job runs on the document, its files aren't looked at: an update would replace
//! the snapshot the job works on and its result would be thrown away. The first look after the job
//! ends catches up.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError};

use photocraft_doc::{Document, LayerContent, LayerId, SmartSource};
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

use crate::state::{LinkObservation, LinkStamp};
pub use crate::state::{LinkScan, Stamp};

/// Stamps `file`, a linked path resolved by [`crate::doc_path`]; `None` (not looked at, like a
/// missing file) when it is not followed.
#[cfg(not(target_arch = "wasm32"))]
fn stamp(file: Option<&std::path::Path>) -> Option<Stamp> {
    let meta = std::fs::metadata(file?).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some((meta.len(), modified.as_nanos()))
}

#[cfg(target_arch = "wasm32")]
fn stamp(_: Option<&std::path::Path>) -> Option<Stamp> {
    None
}

#[derive(Default)]
struct LinkedLayers {
    ids: Vec<u64>,
    expected: BTreeSet<[u8; 32]>,
}

fn linked_files_by_path(d: &Document) -> BTreeMap<String, LinkedLayers> {
    let mut by_file: BTreeMap<String, LinkedLayers> = BTreeMap::new();
    for (_, _, l) in d.walk() {
        if let LayerContent::Smart(sm) = &l.content
            && let SmartSource::Linked { path } = &sm.source
        {
            let layers = by_file.entry(path.clone()).or_default();
            layers.ids.push(l.id.0);
            layers.expected.extend(sm.historical_xcf_link_digest);
        }
    }
    by_file
}

/// The files of `d`'s linked smart objects, with the layers showing each.
pub fn linked_files(d: &Document) -> Vec<(String, Vec<u64>)> {
    linked_files_by_path(d).into_iter().map(|(path, layers)| (path, layers.ids)).collect()
}

/// `layer.smartObjects.updateChanged`: stamp the active document's linked files now and update
/// the layers of the ones that changed.
pub(crate) fn update_changed(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let (doc, files) = (st.doc.id.0, resolved_files(s, st));
    // Drop a look still running on a worker thread. It stamped the files before this one, and
    // applying it later would replace this look's result.
    s.invalidate_link_scan();
    let cancelled = AtomicBool::new(false);
    let now = files.into_iter().map(|file| file.observe(&cancelled)).collect();
    apply_link_stamps(s, doc, now)
}

struct ResolvedLink {
    path: String,
    file: Option<std::path::PathBuf>,
    layers: Vec<u64>,
    check_first_digest: bool,
}

impl ResolvedLink {
    fn observe(self, cancelled: &AtomicBool) -> LinkObservation {
        let stamp = stamp(self.file.as_deref());
        let first_digest =
            if self.check_first_digest { stamp.zip(self.file.as_deref()).and_then(|(observed, file)| first_digest(file, observed, cancelled)) } else { None };
        LinkObservation { path: self.path, file: self.file, layers: self.layers, stamp, first_digest }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn first_digest(path: &std::path::Path, observed: Stamp, cancelled: &AtomicBool) -> Option<[u8; 32]> {
    use std::io::Read as _;

    if cancelled.load(Ordering::Relaxed) {
        return None;
    }
    let (file, metadata) = photocraft_format::read::open_regular_file(path).ok()?;
    if metadata.len() != observed.0 || observed.0 > photocraft_format::read::MAX_OPEN_BYTES {
        return None;
    }
    // Stream at most the observed file size plus one byte to detect growth, with no
    // source-sized allocation or decoding. Leave the saved Smart Object pixels alone.
    let mut source = file.take(observed.0.saturating_add(1));
    let mut hash = blake3::Hasher::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let read = source.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hash.update(buffer.get(..read)?);
    }
    if cancelled.load(Ordering::Relaxed)
        || hash.count() != observed.0
        || stamp(Some(path)) != Some(observed)
        || !path_still_names_file(path, source.into_inner()).unwrap_or(false)
    {
        return None;
    }
    Some(*hash.finalize().as_bytes())
}

#[cfg(unix)]
pub(crate) fn path_still_names_file(path: &std::path::Path, opened: std::fs::File) -> std::io::Result<bool> {
    use std::os::unix::fs::MetadataExt as _;
    let opened = opened.metadata()?;
    let current = std::fs::metadata(path)?;
    Ok((opened.dev(), opened.ino()) == (current.dev(), current.ino()))
}

#[cfg(windows)]
pub(crate) fn path_still_names_file(path: &std::path::Path, opened: std::fs::File) -> std::io::Result<bool> {
    Ok(same_file::Handle::from_file(opened)? == same_file::Handle::from_path(path)?)
}

#[cfg(target_arch = "wasm32")]
fn first_digest(_: &std::path::Path, _: Stamp, _: &AtomicBool) -> Option<[u8; 32]> {
    None
}

fn saved_digest(doc: &Document, layer: u64) -> Option<[u8; 32]> {
    let LayerContent::Smart(smart) = &doc.layer(LayerId(layer))?.content else { return None };
    smart.historical_xcf_link_digest
}

/// Resolve the disk paths and identify first observations that need a fingerprint.
fn resolved_files(s: &Session, st: &crate::DocState) -> Vec<ResolvedLink> {
    let document = st.path.as_deref().map(std::path::Path::new);
    let can_check_digest = s.active_job().is_none();
    linked_files_by_path(&st.doc)
        .into_iter()
        .map(|(path, layers)| {
            let file = crate::doc_path::resolve_in(&path, document).ok();
            let check_first_digest = can_check_digest
                && file.is_some()
                && !layers.expected.is_empty()
                && s.link_stamps.get(&(st.doc.id.0, path.clone())).and_then(|stamp| stamp.checked_digests.as_ref()) != Some(&layers.expected)
                && s.authorize.is_none_or(|gate| gate("layer.smartObjects.updateModifiedContent", &json!({"path":path})).is_ok());
            ResolvedLink { path, file, layers: layers.ids, check_first_digest }
        })
        .collect()
}

fn historical_link_digests(document: &Document) -> BTreeMap<&str, BTreeSet<[u8; 32]>> {
    let mut paths = BTreeMap::<_, BTreeSet<_>>::new();
    for (_, _, layer) in document.walk() {
        if let LayerContent::Smart(smart) = &layer.content
            && let SmartSource::Linked { path } = &smart.source
            && let Some(digest) = smart.historical_xcf_link_digest
        {
            paths.entry(path.as_str()).or_default().insert(digest);
        }
    }
    paths
}

pub(crate) struct ScanWorker {
    handle: std::thread::JoinHandle<()>,
    cancelled: Arc<AtomicBool>,
}

impl Session {
    /// Discard the result immediately and keep the worker accounted for until it exits.
    pub(crate) fn invalidate_link_scan(&mut self) {
        if let Some(worker) = &self.link_scan_worker {
            worker.cancelled.store(true, Ordering::Relaxed);
        }
        self.link_scan = None;
    }

    /// A history step can restore a different expected fingerprint at the same stored path.
    pub(crate) fn invalidate_link_digests_for_history(&mut self, before: Option<Arc<Document>>) {
        let (Some(before), Some(after)) = (before, self.active().map(|state| Arc::clone(state.doc()))) else { return };
        if Arc::ptr_eq(&before, &after) || (self.link_scan.is_none() && !self.link_stamps.keys().any(|(id, _)| *id == after.id.0)) {
            return;
        }
        let old = historical_link_digests(&before);
        let new = historical_link_digests(&after);
        if old != new {
            self.invalidate_link_scan();
            for ((doc, path), stamp) in &mut self.link_stamps {
                if *doc == after.id.0 && old.get(path.as_str()) != new.get(path.as_str()) {
                    stamp.checked_digests = None;
                }
            }
        }
    }

    /// Save As can change what a stored path names. Discard the pending look and
    /// forget any prior stamp whose resolved source changed, including its digest.
    pub(crate) fn invalidate_link_scan_for_save(&mut self, doc: u64, before: Option<&str>, after: &str) {
        use std::path::Path;
        self.invalidate_link_scan();
        self.link_stamps.retain(|(id, path), _| {
            *id != doc || crate::doc_path::resolve_in(path, before.map(Path::new)) == crate::doc_path::resolve_in(path, Some(Path::new(after)))
        });
    }
}

/// Watching the active document's linked smart object files (the link fields are core session data).
pub trait LinkWatchExt {
    /// Start stamping the active document's linked files on a worker thread, unless a look is
    /// still running (or the build has no files to look at: the web app).
    fn start_link_scan(&mut self);
    /// The active document's linked files that were not there at the last look (the Layers
    /// panel's Missing filter). A path that is not followed ([`crate::doc_path`]) is never looked
    /// at, so it is not listed.
    fn missing_links(&self) -> HashSet<String>;
    /// Moves whenever the missing linked files ([`Self::missing_links`]) of any open document change.
    fn missing_links_revision(&self) -> craft_tracked::Revision;
    /// Apply a finished look (`None` while there is none, or it is still running).
    fn poll_link_scan(&mut self) -> Option<Result<Value>>;
}

impl LinkWatchExt for Session {
    /// Start stamping the active document's linked files on a worker thread, unless a look is
    /// still running (or the build has no files to look at: the web app).
    fn start_link_scan(&mut self) {
        if self.link_scan.is_some() || cfg!(target_arch = "wasm32") {
            return;
        }
        if self.link_scan_worker.as_ref().is_some_and(|worker| !worker.handle.is_finished()) {
            return;
        }
        if let Some(worker) = self.link_scan_worker.take() {
            let _ = worker.handle.join();
        }
        let Some(st) = self.active() else { return };
        let (doc, files) = (st.doc.id.0, resolved_files(self, st));
        // Nothing to look at: no links, or none that is followed. None of them is listed as
        // missing either, so a link added back isn't shown missing before its first look.
        if files.iter().all(|link| link.file.is_none()) {
            if self.link_missing.iter().any(|(d, _)| *d == doc) {
                self.link_missing.retain(|(d, _)| *d != doc);
            }
            return;
        }
        let slot: LinkScan = Arc::default();
        let out = slot.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let spawned = std::thread::Builder::new().name("link-watch".into()).spawn(move || {
            let stamps = files.into_iter().take_while(|_| !stop.load(Ordering::Relaxed)).map(|file| file.observe(&stop)).collect();
            if stop.load(Ordering::Relaxed) {
                return;
            }
            *out.lock().unwrap_or_else(PoisonError::into_inner) = Some((doc, stamps));
        });
        if let Ok(handle) = spawned {
            self.link_scan_worker = Some(ScanWorker { handle, cancelled });
            self.link_scan = Some(slot);
        }
    }

    /// The active document's linked files that were not there at the last look (the Layers
    /// panel's Missing filter). A path that is not followed ([`crate::doc_path`]) is never looked
    /// at, so it is not listed.
    fn missing_links(&self) -> HashSet<String> {
        let Some(doc) = self.active().map(|d| d.doc.id.0) else { return HashSet::new() };
        self.link_missing.iter().filter(|(d, _)| *d == doc).map(|(_, p)| p.clone()).collect()
    }

    /// Moves whenever the missing linked files ([`Self::missing_links`]) of any open document change.
    fn missing_links_revision(&self) -> craft_tracked::Revision {
        self.link_missing.revision()
    }

    /// Apply a finished look (`None` while there is none, or it is still running).
    fn poll_link_scan(&mut self) -> Option<Result<Value>> {
        let (doc, stamps) = self.link_scan.as_ref()?.lock().unwrap_or_else(PoisonError::into_inner).take()?;
        self.link_scan = None;
        // A relink or embed while the look ran: it stamped files the document no longer links in
        // that way, so drop it; the next look takes the current links.
        if let Some(st) = self.active().filter(|st| st.doc.id.0 == doc)
            && !resolved_files(self, st)
                .iter()
                .map(|link| (&link.path, &link.file, &link.layers))
                .eq(stamps.iter().map(|link| (&link.path, &link.file, &link.layers)))
        {
            return Some(Ok(json!({"updated": []})));
        }
        Some(apply_link_stamps(self, doc, stamps))
    }
}

/// Remember which of document `doc`'s files in `now` were not there, and forget the files of
/// closed documents. The web app has no files to look at, so it lists none.
fn note_missing_links(s: &mut Session, doc: u64, now: &[LinkObservation]) {
    let open: HashSet<u64> = s.documents().iter().map(|d| d.doc.id.0).collect();
    let mut missing: HashSet<(u64, String)> = s.link_missing.iter().filter(|(d, _)| *d != doc && open.contains(d)).cloned().collect();
    if !cfg!(target_arch = "wasm32") {
        let document = s.active().and_then(|st| st.path.clone());
        for LinkObservation { path, stamp, .. } in now {
            if stamp.is_none() && crate::doc_path::resolve_in(path, document.as_deref().map(std::path::Path::new)).is_ok() {
                missing.insert((doc, path.clone()));
            }
        }
    }
    if *s.link_missing != missing {
        *s.link_missing = missing;
    }
}

/// Compare the stamps of document `doc`'s linked files with the ones last seen and update the
/// layers of the changed ones. Returns `{updated: [layer ids]}`.
fn apply_link_stamps(s: &mut Session, doc: u64, now: Vec<LinkObservation>) -> Result<Value> {
    // The document went to the back meanwhile: it's looked at when it's in front again.
    if s.active().map(|d| d.doc.id.0) != Some(doc) {
        return Ok(json!({"updated": []}));
    }
    note_missing_links(s, doc, &now);
    // A background job holds the document: refreshing now would replace the snapshot it works
    // on and its result would be discarded. The stamps stay as they were, so the change is
    // seen again by the first look after the job ends.
    if s.active_job().is_some() {
        return Ok(json!({"updated": []}));
    }
    // This document's last stamps; those of closed documents are dropped.
    let open: HashSet<u64> = s.documents().iter().map(|d| d.doc.id.0).collect();
    let mut before = HashMap::new();
    s.link_stamps.retain(|(d, p), s| {
        if *d == doc {
            before.insert(p.clone(), s.clone());
            false
        } else {
            open.contains(d)
        }
    });
    let mut changed = Vec::new();
    let mut warnings = BTreeSet::new();
    let mut needs_relink = Vec::new();
    for LinkObservation { path, layers, stamp, first_digest, .. } in now {
        match (before.get(&path), stamp) {
            // Not there right now (an app saving by delete and rename): keep the last stamp.
            (Some(old), None) => {
                s.link_stamps.insert((doc, path), old.clone());
            }
            (old, Some(new)) => {
                let checked_digests = if let Some(digest) = first_digest
                    && let Some(state) = s.active()
                {
                    let expected: BTreeSet<_> = layers.iter().filter_map(|&layer| saved_digest(&state.doc, layer)).collect();
                    if expected.iter().any(|&saved| saved != digest) {
                        let message = photocraft_io::linked::HISTORICAL_XCF_LINK_CHANGED;
                        warnings.insert(format!("{path}: {message}"));
                        needs_relink.push(json!({"path":path,"layers":layers,"message":message}));
                    }
                    Some(expected)
                } else {
                    old.and_then(|old| old.checked_digests.clone())
                };
                if old.is_some_and(|old| old.file != new) {
                    changed.push((path.clone(), layers));
                }
                s.link_stamps.insert((doc, path), LinkStamp { file: new, checked_digests });
            }
            (None, None) => {}
        }
    }
    let mut updated = Vec::new();
    for (path, ids) in changed {
        let mut stale = None;
        let r = s.edit("Update Modified Content", |d, _| {
            let mut refreshed = Vec::new();
            let mut failure = None;
            for &id in &ids {
                if failure.is_some() && saved_digest(d, id).is_none() {
                    continue;
                }
                match crate::smart_cmds::refresh(d, LayerId(id)) {
                    Ok(true) => refreshed.push(id),
                    Err(EngineError::HistoricalXcfLinkChanged(message)) => stale = Some(message),
                    Ok(false) => {
                        failure.get_or_insert_with(|| EngineError::Other(format!("{path} can't be read")));
                    }
                    Err(error) => {
                        failure.get_or_insert(error);
                    }
                }
            }
            if let Some(error) = failure {
                return Err(error);
            }
            if refreshed.is_empty()
                && let Some(message) = stale.as_ref()
            {
                return Err(EngineError::HistoricalXcfLinkChanged(message.clone()));
            }
            Ok(refreshed)
        });
        // Not readable (still being written, or not a format smart objects read): the edit left
        // nothing behind, and the file is tried again once it changes again, not on every look.
        if let Ok(refreshed) = r {
            updated.extend(refreshed);
        }
        if let Some(message) = stale {
            warnings.insert(format!("{path}: {message}"));
            if !needs_relink.iter().any(|entry| entry["path"] == path) {
                needs_relink.push(json!({"path":path,"layers":ids,"message":message}));
            }
            // The refresh itself verified the digest even if the first read failed.
            let expected = s.active().map(|state| ids.iter().filter_map(|&id| saved_digest(&state.doc, id)).collect());
            if let Some(stamp) = s.link_stamps.get_mut(&(doc, path)) {
                stamp.checked_digests = expected;
            }
        }
    }
    let mut result = json!({"updated": updated});
    if !warnings.is_empty() {
        result["warnings"] = json!(warnings);
    }
    if !needs_relink.is_empty() {
        result["needsRelink"] = json!(needs_relink);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use photocraft_doc::LayerContent;
    use photocraft_geom::Rect;
    use serde_json::json;

    use super::LinkWatchExt;
    use crate::{EngineError, Session};

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("photocraft-link-watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The bytes of a solid-colour `w × h` PNG (made in a folder of its own: tests run in
    /// parallel).
    fn png(w: u32, h: u32, color: &str) -> Vec<u8> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = dir(&format!("png-{}-{n}", color.trim_start_matches('#')));
        let path = d.join("x.png").to_string_lossy().into_owned();
        let mut s = crate::session();
        s.execute("file.new", json!({"width": w, "height": h, "background": color})).unwrap();
        s.execute("file.saveACopy", json!({"path": path, "layers": false})).unwrap();
        std::fs::read(&path).unwrap()
    }

    /// Write `bytes` to `path` so its modification time moves even on coarse file systems.
    fn save(path: &std::path::Path, bytes: &[u8]) {
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(path, bytes).unwrap();
    }

    /// A 40×40 white document with the file at `path` placed linked → (session, layer id).
    fn placed(path: &std::path::Path) -> (Session, u64) {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 40, "height": 40})).unwrap();
        s.execute("file.placeLinked", json!({"path": path.to_string_lossy()})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap().0;
        (s, id)
    }

    // The public v11 RGBA16 fixture, with a second valid encoded revision of equal length.
    fn historical_pixel(original: bool) -> Vec<u8> {
        let mut bytes = crate::jobs::historical_fixture::historical_rgba16();
        if !original {
            *bytes.last_mut().unwrap() ^= 1;
        }
        bytes
    }

    #[test]
    fn a_changed_historical_xcf_link_reports_relink_once_without_editing() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("source.xcf");
        let bytes = historical_pixel(true);
        std::fs::write(&path, bytes).unwrap();
        let mut s = crate::session();
        s.execute("file.new", json!({"width":1,"height":1})).unwrap();
        s.execute("file.placeLinked", json!({"path":path,"historicalXcfSampleOrder":"little-endian"})).unwrap();
        let before = s.active().unwrap().doc.clone();
        assert_eq!(s.execute("layer.smartObjects.updateChanged", json!({})).unwrap(), json!({"updated":[]}));
        save(&path, &historical_pixel(false));
        let result = s.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
        assert_eq!(result["updated"], json!([]));
        assert_eq!(result["warnings"].as_array().unwrap().len(), 1);
        assert_eq!(result["needsRelink"].as_array().unwrap().len(), 1);
        assert_eq!(result["needsRelink"][0]["path"], json!(path));
        assert!(result["needsRelink"][0]["message"].as_str().unwrap().contains("relink and choose"));
        assert!(result["warnings"][0].as_str().unwrap().contains("relink and choose"));
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
        assert_eq!(s.execute("layer.smartObjects.updateChanged", json!({})).unwrap(), json!({"updated":[]}));
    }

    #[test]
    fn the_first_background_scan_warns_about_a_replaced_xcf_from_a_reopened_document() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("source.xcf");
        let saved = folder.path().join("doc.pcraft");
        std::fs::write(&path, historical_pixel(false)).unwrap();
        let mut s = crate::session();
        s.execute("file.new", json!({"width":1,"height":1})).unwrap();
        s.execute("file.placeLinked", json!({"path":path,"historicalXcfSampleOrder":"big-endian"})).unwrap();
        s.execute("file.saveACopy", json!({"path":saved})).unwrap();
        assert_eq!(s.execute("layer.smartObjects.updateChanged", json!({})).unwrap(), json!({"updated":[]}));
        s.close(0);
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::write(&path, historical_pixel(true)).unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_times(std::fs::FileTimes::new().set_modified(modified)).unwrap();
        s.execute("file.openAs", json!({"path":saved})).unwrap();
        let before = s.active().unwrap().doc.clone();
        s.start_link_scan();
        let started = std::time::Instant::now();
        let result = loop {
            if let Some(result) = s.poll_link_scan() {
                break result.unwrap();
            }
            assert!(started.elapsed() < std::time::Duration::from_secs(10));
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(result["updated"], json!([]));
        assert_eq!(result["warnings"].as_array().unwrap().len(), 1);
        assert!(result["warnings"][0].as_str().unwrap().contains("relink and choose"));
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
        assert_eq!(s.execute("layer.smartObjects.updateChanged", json!({})).unwrap(), json!({"updated":[]}));
    }

    #[test]
    fn undo_redo_and_rollback_recheck_the_restored_historical_digest_at_the_same_stamp() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("historical.xcf");
        let original = historical_pixel(false);
        let replacement = historical_pixel(true);
        std::fs::write(&path, &original).unwrap();
        let mut s = crate::session();
        s.execute("file.new", json!({"width":1,"height":1})).unwrap();
        let id = s.execute("file.placeLinked", json!({"path":path,"historicalXcfSampleOrder":"big-endian"})).unwrap()["layer"].as_u64().unwrap();
        let relink = |s: &mut Session, expected: &[u8]| {
            s.edit("Restore source assignment", |doc, _| {
                let LayerContent::Smart(object) = &mut doc.layer_mut(photocraft_doc::LayerId(id)).unwrap().content else { panic!("smart object") };
                object.historical_xcf_link_digest = Some(photocraft_io::linked::historical_xcf_digest(expected));
                Ok(())
            })
            .unwrap();
        };
        let look = |s: &mut Session| s.execute("layer.smartObjects.updateChanged", json!({})).unwrap()["warnings"].is_array();
        relink(&mut s, &replacement);
        assert!(look(&mut s));
        assert!(s.undo());
        assert!(!look(&mut s));
        assert!(s.redo());
        assert!(look(&mut s));
        relink(&mut s, &original);
        assert!(s.rollback());
        assert!(look(&mut s));
    }

    #[test]
    fn an_unreadable_first_fingerprint_is_retried_at_the_same_stamp() {
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("source.xcf");
        std::fs::write(&source, historical_pixel(false)).unwrap();
        let mut s = crate::session();
        s.execute("file.new", json!({"width":1,"height":1})).unwrap();
        s.execute("file.placeLinked", json!({"path":source,"historicalXcfSampleOrder":"big-endian"})).unwrap();
        std::fs::write(&source, historical_pixel(true)).unwrap();
        let state = s.active().unwrap();
        let doc = state.doc.id.0;
        let before = state.doc.clone();
        let mut files = super::resolved_files(&s, state);
        let resolved = files.pop().unwrap();
        assert!(resolved.check_first_digest);
        let stamp = super::stamp(resolved.file.as_deref()).unwrap();
        let key = (doc, resolved.path.clone());
        // Stat succeeded, but the worker could not read the file for its fingerprint.
        let unreadable = super::LinkObservation { path: resolved.path, file: resolved.file, layers: resolved.layers, stamp: Some(stamp), first_digest: None };
        assert_eq!(super::apply_link_stamps(&mut s, doc, vec![unreadable]).unwrap(), json!({"updated":[]}));
        assert_eq!(s.link_stamps[&key].file, stamp);

        let retried = s.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
        assert_eq!(retried["updated"], json!([]));
        assert_eq!(retried["warnings"].as_array().unwrap().len(), 1);
        assert!(retried["warnings"][0].as_str().unwrap().contains("relink and choose"));
        assert_eq!(s.link_stamps[&key].file, stamp);
        assert!(s.link_stamps[&key].checked_digests.is_some());
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
        assert_eq!(s.execute("layer.smartObjects.updateChanged", json!({})).unwrap(), json!({"updated":[]}));
    }

    #[test]
    fn a_forward_copy_with_a_new_expected_digest_triggers_a_check() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("source.xcf");
        std::fs::write(&path, historical_pixel(true)).unwrap();
        let mut s = crate::session();
        s.execute("file.new", json!({"width":1,"height":1})).unwrap();
        s.execute("file.placeLinked", json!({"path":path,"historicalXcfSampleOrder":"little-endian"})).unwrap();
        let command = "layer.smartObjects.updateChanged";
        assert_eq!(s.execute(command, json!({})).unwrap(), json!({"updated":[]}));
        s.edit("Copy linked source", |doc, active| {
            let LayerContent::Smart(mut copy) = doc.layer(active.unwrap()).unwrap().content.clone() else { panic!("linked layer") };
            copy.historical_xcf_link_digest = Some([42; 32]);
            doc.insert_above(*active, photocraft_doc::Layer::new("Linked copy", LayerContent::Smart(copy)));
            Ok(())
        })
        .unwrap();
        let result = s.execute(command, json!({})).unwrap();
        assert!(result["warnings"][0].as_str().unwrap().contains("relink and choose"));
        assert_eq!(s.execute(command, json!({})).unwrap(), json!({"updated":[]}));
    }

    #[test]
    fn an_invalidated_worker_blocks_a_replacement_until_it_exits() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("source.png");
        std::fs::write(&path, png(1, 1, "#0000ff")).unwrap();
        let (mut s, _) = placed(&path);
        let slot: super::LinkScan = Default::default();
        let (finish, blocked) = std::sync::mpsc::channel();
        let cancelled = std::sync::Arc::new(super::AtomicBool::new(false));
        let handle = std::thread::spawn(move || {
            let _ = blocked.recv();
        });
        s.link_scan = Some(slot);
        s.link_scan_worker = Some(super::ScanWorker { handle, cancelled: cancelled.clone() });
        s.invalidate_link_scan();
        s.start_link_scan();
        assert!(cancelled.load(super::Ordering::Relaxed));
        assert!(s.link_scan.is_none());
        finish.send(()).unwrap();
        s.link_scan_worker.take().unwrap().handle.join().unwrap();
        s.start_link_scan();
        assert!(s.link_scan.is_some());
    }

    #[test]
    fn save_as_drops_the_old_scan_and_checks_the_new_relative_source_at_the_same_stamp() {
        let folder = tempfile::tempdir().unwrap();
        let other = folder.path().join("other");
        std::fs::create_dir(&other).unwrap();
        let source = folder.path().join("source.xcf");
        let replacement = other.join("source.xcf");
        std::fs::write(&source, historical_pixel(false)).unwrap();
        std::fs::write(&replacement, historical_pixel(true)).unwrap();
        let timestamp = std::fs::metadata(&source).unwrap().modified().unwrap();
        std::fs::File::options().write(true).open(&replacement).unwrap().set_times(std::fs::FileTimes::new().set_modified(timestamp)).unwrap();
        assert_eq!(super::stamp(Some(&source)), super::stamp(Some(&replacement)));
        let mut session = crate::session();
        session.execute("file.new", json!({"width":1,"height":1})).unwrap();
        session.saved_to(folder.path().join("original.pcraft").to_string_lossy().into_owned());
        let layer = session.execute("file.placeLinked", json!({"path":source,"historicalXcfSampleOrder":"big-endian"})).unwrap()["layer"].as_u64().unwrap();
        session
            .edit("Store a relative link", |doc, _| {
                let Some(photocraft_doc::LayerContent::Smart(smart)) = doc.layer_mut(photocraft_doc::LayerId(layer)).map(|layer| &mut layer.content) else {
                    panic!("smart object")
                };
                smart.source = photocraft_doc::SmartSource::Linked { path: "source.xcf".into() };
                Ok(())
            })
            .unwrap();
        assert_eq!(session.execute("layer.smartObjects.updateChanged", json!({})).unwrap(), json!({"updated":[]}));
        session.start_link_scan();
        assert!(session.link_scan.is_some());
        session.saved_to(other.join("copy.pcraft").to_string_lossy().into_owned());
        assert!(session.link_scan.is_none());
        let update = session.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
        assert_eq!(update["updated"], json!([]));
        assert!(update["warnings"][0].as_str().unwrap().contains("relink and choose"));
    }

    #[test]
    fn the_busy_document_defers_its_first_digest_without_deferring_metadata() {
        use crate::jobs::Started;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("source.xcf");
        std::fs::write(&path, historical_pixel(false)).unwrap();
        let mut session = crate::session();
        session.execute("file.new", json!({"width":1,"height":1})).unwrap();
        session.execute("file.placeLinked", json!({"path":path,"historicalXcfSampleOrder":"big-endian"})).unwrap();
        let Started::Job(id) = session
            .start_job(
                "filter.blur.gaussianBlur",
                json!({}),
                "Held job",
                true,
                |ctx| {
                    loop {
                        ctx.check()?;
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                },
                |_, ()| Ok(json!({})),
            )
            .unwrap()
        else {
            panic!("background job")
        };
        let mut links = super::resolved_files(&session, session.active().unwrap());
        let link = links.pop().unwrap();
        assert!(!link.check_first_digest);
        assert!(link.observe(&super::AtomicBool::new(false)).stamp.is_some());
        assert!(session.cancel_job(id));
        assert!(super::resolved_files(&session, session.active().unwrap())[0].check_first_digest);
    }

    fn centre(s: &Session) -> [f32; 3] {
        let p = photocraft_compose::render(&s.active().unwrap().doc, Rect::new(20, 20, 21, 21)).px[0];
        [p[0], p[1], p[2]]
    }

    fn is(c: [f32; 3], rgb: [f32; 3]) -> bool {
        c.iter().zip(rgb).all(|(a, b)| (a - b).abs() < 0.02)
    }

    const RED: [f32; 3] = [1.0, 0.0, 0.0];
    const BLUE: [f32; 3] = [0.0, 0.0, 1.0];

    #[test]
    fn a_linked_smart_object_follows_its_file() {
        let d = dir("follow");
        let path = d.join("art.png");
        std::fs::write(&path, png(10, 10, "#ff0000")).unwrap();
        let (mut s, id) = placed(&path);
        assert!(is(centre(&s), RED));
        // First look: remembered, nothing to update.
        assert_eq!(s.execute("layer.smartObjects.updateChanged", json!({})).unwrap(), json!({"updated": []}));
        save(&path, &png(10, 10, "#0000ff"));
        assert_eq!(s.execute("layer.smartObjects.updateChanged", json!({})).unwrap(), json!({"updated": [id]}));
        assert!(is(centre(&s), BLUE), "{:?}", centre(&s));
        // An Update Modified Content step: undoing it is respected until the file changes again.
        assert!(s.undo());
        assert!(is(centre(&s), RED));
        assert_eq!(s.execute("layer.smartObjects.updateChanged", json!({})).unwrap(), json!({"updated": []}));
        assert!(is(centre(&s), RED));
    }

    #[test]
    fn a_file_still_being_written_is_tried_again() {
        let d = dir("partial");
        let path = d.join("art.png");
        std::fs::write(&path, png(10, 10, "#ff0000")).unwrap();
        let (mut s, id) = placed(&path);
        s.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
        let undo = s.active().unwrap().history.past_len();
        // Another app has only written the start of the file so far.
        let full = png(10, 10, "#0000ff");
        save(&path, &full[..16]);
        assert_eq!(s.execute("layer.smartObjects.updateChanged", json!({})).unwrap(), json!({"updated": []}));
        assert!(is(centre(&s), RED), "the last good pixels stay");
        assert_eq!(s.active().unwrap().history.past_len(), undo, "no step for a failed update");
        save(&path, &full);
        assert_eq!(s.execute("layer.smartObjects.updateChanged", json!({})).unwrap(), json!({"updated": [id]}));
        assert!(is(centre(&s), BLUE));
    }

    #[test]
    fn a_document_in_the_back_catches_up_in_front() {
        let d = dir("back");
        let path = d.join("art.png");
        std::fs::write(&path, png(10, 10, "#ff0000")).unwrap();
        let (mut s, id) = placed(&path);
        s.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
        s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
        save(&path, &png(10, 10, "#0000ff"));
        assert_eq!(s.execute("layer.smartObjects.updateChanged", json!({})).unwrap(), json!({"updated": []}));
        s.set_active(0);
        assert_eq!(s.execute("layer.smartObjects.updateChanged", json!({})).unwrap(), json!({"updated": [id]}));
        assert!(is(centre(&s), BLUE));
    }

    #[test]
    fn a_look_lists_the_linked_files_that_are_not_there() {
        let d = dir("missing");
        let path = d.join("art.png");
        std::fs::write(&path, png(10, 10, "#ff0000")).unwrap();
        let (mut s, _) = placed(&path);
        let stored = super::linked_files(&s.active().unwrap().doc)[0].0.clone();
        assert!(s.missing_links().is_empty(), "nothing looked at yet");
        s.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
        assert!(s.missing_links().is_empty());
        std::fs::remove_file(&path).unwrap();
        s.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
        assert_eq!(s.missing_links(), [stored.clone()].into());
        // Only the document's own files: another one in front lists none.
        s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
        assert!(s.missing_links().is_empty());
        s.set_active(0);
        assert_eq!(s.missing_links(), [stored].into());
        std::fs::write(&path, png(10, 10, "#0000ff")).unwrap();
        s.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
        assert!(s.missing_links().is_empty(), "the file is back");
    }

    #[test]
    fn a_reopened_document_lists_no_file_missing_before_it_closed() {
        let d = dir("reopen");
        let (path, saved) = (d.join("art.png"), d.join("doc.pcraft"));
        std::fs::write(&path, png(10, 10, "#ff0000")).unwrap();
        let (mut s, _) = placed(&path);
        s.execute("file.saveACopy", json!({"path": saved.to_string_lossy()})).unwrap();
        let id = s.active().unwrap().doc.id;
        std::fs::remove_file(&path).unwrap();
        s.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
        assert_eq!(s.missing_links().len(), 1);
        s.close(0);
        std::fs::write(&path, png(10, 10, "#0000ff")).unwrap();
        let bytes = std::fs::read(&saved).unwrap();
        crate::file_cmds::open_bytes_as(&mut s, "doc.pcraft", &bytes, None, Some(saved.to_string_lossy().into_owned())).unwrap();
        assert_eq!(s.active().unwrap().doc.id, id, "the saved document keeps its id");
        assert!(s.missing_links().is_empty(), "nothing looked at since it opened");
    }

    #[test]
    fn a_look_started_before_update_changed_does_not_replace_its_result() {
        let d = dir("stale");
        let path = d.join("art.png");
        std::fs::write(&path, png(10, 10, "#ff0000")).unwrap();
        let (mut s, _) = placed(&path);
        s.start_link_scan();
        let t0 = std::time::Instant::now();
        while s.link_scan.as_ref().is_some_and(|l| l.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_none()) {
            assert!(t0.elapsed().as_secs() < 10, "the look never finished");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::fs::remove_file(&path).unwrap();
        s.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
        assert_eq!(s.missing_links().len(), 1);
        let _ = s.poll_link_scan();
        assert_eq!(s.missing_links().len(), 1, "the earlier look found the file, but it is gone");
    }

    #[test]
    fn a_scan_finished_after_close_cannot_change_a_reopened_document() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("art.png");
        let saved = directory.path().join("doc.pcraft");
        std::fs::write(&path, png(10, 10, "#ff0000")).unwrap();
        let (mut s, layer) = placed(&path);
        s.execute("file.saveACopy", json!({"path": saved.to_string_lossy()})).unwrap();
        let id = s.active().unwrap().doc.id;
        let stored = super::linked_files(&s.active().unwrap().doc)[0].0.clone();
        let scan = std::sync::Arc::new(std::sync::Mutex::new(None));
        s.link_scan = Some(scan.clone());
        s.close(0);
        let bytes = std::fs::read(&saved).unwrap();
        crate::file_cmds::open_bytes_as(&mut s, "doc.pcraft", &bytes, None, Some(saved.to_string_lossy().into_owned())).unwrap();
        assert_eq!(s.active().unwrap().doc.id, id);
        *scan.lock().unwrap() = Some((
            id.0,
            vec![super::LinkObservation {
                path: stored.clone(),
                file: crate::doc_path::resolve_in(&stored, Some(&saved)).ok(),
                layers: vec![layer],
                stamp: None,
                first_digest: None,
            }],
        ));
        assert!(s.poll_link_scan().is_none());
        assert!(s.missing_links().is_empty());
    }

    #[test]
    fn a_scan_taken_in_the_old_folder_is_dropped_after_save_as() {
        let old = dir("saveas-old");
        let new = dir("saveas-new");
        std::fs::write(new.join("art.png"), png(10, 10, "#ff0000")).unwrap();
        let (mut s, layer) = placed(&new.join("art.png"));
        s.saved_to(new.join("doc.pcraft").to_string_lossy().into_owned());
        s.execute("layer.smartObjects.relinkToFile", json!({"path": "art.png"})).unwrap();
        s.saved_to(old.join("doc.pcraft").to_string_lossy().into_owned());
        let doc = s.active().unwrap().doc.id.0;
        // The look ran in the old folder, where art.png is not.
        let file = Some(old.join("art.png"));
        s.link_scan = Some(std::sync::Arc::new(std::sync::Mutex::new(Some((
            doc,
            vec![super::LinkObservation { path: "art.png".to_string(), file, layers: vec![layer], stamp: None, first_digest: None }],
        )))));
        // Exercise the resolved-path publication guard independently of Session::saved_to's cancellation.
        s.active_mut().unwrap().saved_to(new.join("doc.pcraft").to_string_lossy().into_owned());
        assert_eq!(s.poll_link_scan().unwrap().unwrap(), json!({"updated": []}));
        assert!(s.missing_links().is_empty());
    }

    #[test]
    fn a_document_without_links_lists_none_missing() {
        let d = dir("unlinked");
        let path = d.join("art.png");
        std::fs::write(&path, png(10, 10, "#ff0000")).unwrap();
        let (mut s, id) = placed(&path);
        std::fs::remove_file(&path).unwrap();
        s.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
        assert_eq!(s.missing_links().len(), 1);
        s.execute("layer.delete", json!({"layer": id})).unwrap();
        s.start_link_scan();
        assert!(s.missing_links().is_empty(), "the last linked layer is gone");
    }

    #[test]
    fn a_look_at_links_that_changed_meanwhile_is_dropped() {
        let d = dir("relinked");
        let path = d.join("art.png");
        std::fs::write(&path, png(10, 10, "#ff0000")).unwrap();
        let (mut s, layer) = placed(&path);
        let doc = s.active().unwrap().doc.id.0;
        let stored = super::linked_files(&s.active().unwrap().doc)[0].0.clone();
        let inject = |s: &mut Session, file: &str| {
            let resolved = crate::doc_path::resolve_in(file, None).ok();
            let scan = std::sync::Arc::new(std::sync::Mutex::new(Some((
                doc,
                vec![super::LinkObservation { path: file.to_string(), file: resolved, layers: vec![layer], stamp: None, first_digest: None }],
            ))));
            s.link_scan = Some(scan);
        };
        // Taken before a relink: its file is no longer linked.
        inject(&mut s, &d.join("before-relink.png").to_string_lossy());
        assert_eq!(s.poll_link_scan().unwrap().unwrap(), json!({"updated": []}));
        assert!(s.missing_links().is_empty(), "a file no longer linked is not listed");
        // Taken of the current links: applied.
        inject(&mut s, &stored);
        s.poll_link_scan().unwrap().unwrap();
        assert_eq!(s.missing_links(), [stored].into());
    }

    #[test]
    fn a_background_look_updates_changed_files() {
        let d = dir("scan");
        let path = d.join("art.png");
        std::fs::write(&path, png(10, 10, "#ff0000")).unwrap();
        let (mut s, id) = placed(&path);
        let look = |s: &mut Session| {
            s.start_link_scan();
            let t0 = std::time::Instant::now();
            loop {
                if let Some(r) = s.poll_link_scan() {
                    return r.unwrap();
                }
                assert!(t0.elapsed().as_secs() < 10, "the look never finished");
                std::thread::yield_now();
            }
        };
        assert_eq!(look(&mut s), json!({"updated": []}));
        save(&path, &png(10, 10, "#0000ff"));
        assert_eq!(look(&mut s), json!({"updated": [id]}));
        assert!(is(centre(&s), BLUE));
        assert!(s.link_scan.is_none());
    }

    #[test]
    fn a_refresh_waits_for_a_running_job_on_the_document() {
        let d = dir("job");
        let path = d.join("art.png");
        std::fs::write(&path, png(10, 10, "#ff0000")).unwrap();
        let (mut s, id) = placed(&path);
        s.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
        let (release, job) = gated_job(&mut s);
        save(&path, &png(10, 10, "#0000ff"));
        let snapshot = s.active().unwrap().doc.clone();
        // The path the desktop app's timer takes: it doesn't go through the command conflict check.
        assert_eq!(super::update_changed(&mut s, &json!({})).unwrap(), json!({"updated": []}));
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &snapshot), "the job's snapshot must stay in place");
        release.send(()).unwrap();
        s.wait_job(job).unwrap();
        // The job finished; the change that came meanwhile is picked up exactly once.
        assert_eq!(s.execute("layer.smartObjects.updateChanged", json!({})).unwrap(), json!({"updated": [id]}));
        assert!(is(centre(&s), BLUE));
        assert_eq!(s.execute("layer.smartObjects.updateChanged", json!({})).unwrap(), json!({"updated": []}));
    }

    #[test]
    fn a_job_on_another_document_does_not_hold_up_a_refresh() {
        let d = dir("job-other");
        let path = d.join("art.png");
        std::fs::write(&path, png(10, 10, "#ff0000")).unwrap();
        let (mut s, id) = placed(&path);
        s.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
        s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
        let (release, job) = gated_job(&mut s);
        s.set_active(0);
        save(&path, &png(10, 10, "#0000ff"));
        assert_eq!(super::update_changed(&mut s, &json!({})).unwrap(), json!({"updated": [id]}));
        assert!(is(centre(&s), BLUE));
        release.send(()).unwrap();
        s.wait_job(job).unwrap();
    }

    /// A background job that locks the active document until something is sent on the returned
    /// sender.
    fn gated_job(s: &mut Session) -> (std::sync::mpsc::Sender<()>, crate::jobs::JobId) {
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let r = s.start_job("test.gated", json!({}), "Gated job", true, move |_| gate.recv().map_err(|_| EngineError::Cancelled), |_, ()| Ok(json!(null)));
        match r.unwrap() {
            crate::jobs::Started::Job(j) => (release, j),
            other => panic!("expected a background job, got {other:?}"),
        }
    }
    #[test]
    fn a_cancelled_first_fingerprint_never_certifies_the_source() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("source.xcf");
        std::fs::write(&path, historical_pixel(true)).unwrap();
        let observed = super::stamp(Some(&path)).unwrap();
        assert_eq!(super::first_digest(&path, observed, &super::AtomicBool::new(true)), None);
        assert!(super::first_digest(&path, observed, &super::AtomicBool::new(false)).is_some());
    }

    #[test]
    fn a_fingerprint_handle_cannot_certify_another_file_with_the_same_bytes() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("first.xcf");
        let other = folder.path().join("second.xcf");
        let bytes = historical_pixel(true);
        std::fs::write(&path, &bytes).unwrap();
        std::fs::write(&other, &bytes).unwrap();
        assert!(!super::path_still_names_file(&other, std::fs::File::open(&path).unwrap()).unwrap());
        assert!(super::path_still_names_file(&path, std::fs::File::open(&path).unwrap()).unwrap());
    }
    #[test]
    fn a_stale_historical_owner_does_not_block_a_healthy_owner_of_the_same_path() {
        for historical_first in [false, true] {
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("source.xcf");
            std::fs::write(&path, historical_pixel(true)).unwrap();
            let mut s = crate::session();
            s.execute("file.new", json!({"width":40,"height":40})).unwrap();
            let old = s.execute("file.placeLinked", json!({"path":path,"historicalXcfSampleOrder":"little-endian"})).unwrap()["layer"].as_u64().unwrap();
            std::fs::write(&path, png(10, 10, "#ff0000")).unwrap();
            let current = s.execute("file.placeLinked", json!({"path":path})).unwrap()["layer"].as_u64().unwrap();
            s.edit("Order linked owners", |doc, _| {
                let left = doc.layers.iter().position(|l| l.id.0 == old).unwrap();
                let right = doc.layers.iter().position(|l| l.id.0 == current).unwrap();
                if (left < right) != historical_first {
                    doc.layers.swap(left, right);
                }
                Ok(())
            })
            .unwrap();
            let before = s.active().unwrap().doc.clone();
            let first = s.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
            assert_eq!(first["updated"], json!([]));
            assert!(first["needsRelink"].is_array());
            save(&path, &png(10, 10, "#0000ff"));
            let refreshed = s.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
            assert_eq!(refreshed["updated"], json!([current]));
            assert_eq!(refreshed["needsRelink"][0]["path"], json!(path));
            let doc = &s.active().unwrap().doc;
            assert_eq!(doc.layer(photocraft_doc::LayerId(old)), before.layer(photocraft_doc::LayerId(old)));
            let pixel = doc.layer(photocraft_doc::LayerId(current)).unwrap().surface().unwrap().pixel(0, 0);
            assert!(is([pixel[0], pixel[1], pixel[2]], BLUE));
            assert_eq!(s.execute("layer.smartObjects.updateChanged", json!({})).unwrap(), json!({"updated":[]}));
        }
    }
}
