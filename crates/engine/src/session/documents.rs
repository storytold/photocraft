use crate::{EngineError, Result};

use super::history::fix_selection;
use super::{DocState, Document, LayerId, Session, channel_cmds, smart_cmds};

/// Move item `i` of `v` to position `to`, clamped to the end. Returns where it went; `None` when
/// `i` is out of range.
pub fn move_item<T>(v: &mut Vec<T>, i: usize, to: usize) -> Option<usize> {
    if i >= v.len() {
        return None;
    }
    let x = v.remove(i);
    let to = to.min(v.len());
    v.insert(to, x);
    Some(to)
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn documents(&self) -> &[DocState] {
        &self.docs
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    pub fn active(&self) -> Option<&DocState> {
        self.active.and_then(|i| self.docs.get(i))
    }

    pub fn active_mut(&mut self) -> Option<&mut DocState> {
        self.active.and_then(|i| self.docs.get_mut(i))
    }

    /// Monotonic active-document generation. It changes when a different document becomes
    /// active, including a switch away and back between two observations; equal values therefore
    /// mean the document was not switched in between.
    pub fn active_generation(&self) -> u64 {
        self.active_generation
    }

    /// Change marker: advances at least once whenever the active document or layer changes through
    /// the `Session` API (see the field). Compare two readings for equality to know whether anything
    /// changed in between; it is not an exact count of operations. A direct write to
    /// `DocState::active_layer` through `active_mut()` bypasses it.
    pub fn active_layer_changes(&self) -> u64 {
        self.active_layer_changes
    }

    /// Bump [`Self::active_generation`] when the active document id changed. Call after every
    /// assignment to `self.active`.
    pub(crate) fn note_active_change(&mut self) {
        let id = self.active().map(|st| st.doc.id);
        if id != self.active_doc_for_generation {
            self.active_layer_changes = self.active_layer_changes.wrapping_add(1);
            self.active_doc_for_generation = id;
            self.active_generation = self.active_generation.wrapping_add(1);
        }
    }

    /// The active document and its active layer, for [`Self::note_layer_change`].
    pub(crate) fn active_layer_stamp(&self) -> Option<(photocraft_doc::DocId, Option<LayerId>)> {
        self.active().map(|st| (st.doc.id, st.active_layer))
    }

    /// Count a change of the active document or layer since `before` (from
    /// [`Self::active_layer_stamp`]). Every path that can change either calls this after it.
    pub(crate) fn note_layer_change(&mut self, before: Option<(photocraft_doc::DocId, Option<LayerId>)>) {
        if self.active_layer_stamp() != before {
            self.active_layer_changes = self.active_layer_changes.wrapping_add(1);
        }
    }

    /// Make `layer` the active document's active layer and nothing else (the selection list and
    /// anchor are left as they are), counted in [`Self::active_layer_changes`].
    pub fn set_active_layer(&mut self, layer: Option<LayerId>) {
        let before = self.active_layer_stamp();
        if let Some(st) = self.active_mut() {
            st.active_layer = layer;
        }
        self.note_layer_change(before);
    }

    pub fn set_active(&mut self, index: usize) -> bool {
        if index < self.docs.len() {
            self.active = Some(index);
            self.note_active_change();
            self.sync_tools_mask_target();
            true
        } else {
            false
        }
    }

    /// Color Sampler points dropped by the last canvas-size change (crop, Canvas Size, Image Size,
    /// rotation) since this was last called, for the status message. See [`edit`](Self::edit).
    pub fn take_samplers_removed(&mut self) -> usize {
        std::mem::take(&mut self.samplers_removed)
    }

    /// Move the Color Sampler points through `a` in the next [`Session::edit`] (crop, Canvas Size,
    /// Image Size, rotation), dropping the ones that land outside the new canvas. View state: the
    /// points stay pinned to their pixels, no history step. Cleared by the edit it is set for.
    /// The painting symmetry moves through `a` too.
    pub(crate) fn set_sampler_map(&mut self, a: photocraft_geom::Affine) {
        self.set_canvas_map(a, a);
    }

    /// [`set_sampler_map`](Self::set_sampler_map) for an edit whose sampler map differs from its
    /// geometry map (a flip or rotation maps pixel centres): `geometry` moves the painting symmetry.
    pub(crate) fn set_canvas_map(&mut self, geometry: photocraft_geom::Affine, samplers: photocraft_geom::Affine) {
        self.sampler_map = Some(samplers);
        self.symmetry_map = Some(geometry);
    }

    pub fn sync_tools_mask_target(&mut self) {
        let mask = self.active().is_some_and(|st| st.channel_view.target == crate::channel_cmds::ChannelTarget::LayerMask);
        self.tools.target_mask(mask);
    }

    /// Move the document at `from` to tab position `to` (clamped to the last), keeping the active
    /// document active. Returns its new index; `None` when `from` is out of range.
    pub fn move_document(&mut self, from: usize, to: usize) -> Option<usize> {
        let active = self.active().map(|d| d.doc.id);
        let to = move_item(&mut self.docs, from, to)?;
        self.active = active.and_then(|id| self.docs.iter().position(|d| d.doc.id == id));
        self.note_active_change();
        Some(to)
    }

    /// Resolve a Photoshop action's offset in opening order, returning its current tab index.
    /// Tab dragging changes `docs`, but must not change which document an imported action edits.
    /// Offsets wrap at either end; native zero keeps the active document.
    pub(crate) fn document_at_offset(&self, offset: i64) -> Result<usize> {
        let id = self.active().ok_or(EngineError::NoDocument)?.doc.id;
        let active = self.open_document_order.iter().position(|&open| open == id).ok_or(EngineError::NoDocument)?;
        // Widen before addition so even extreme signed offsets wrap without overflow.
        let target = (active as i128 + i128::from(offset)).rem_euclid(self.open_document_order.len() as i128) as usize;
        let id = self.open_document_order.get(target).ok_or(EngineError::NoDocument)?;
        self.docs.iter().position(|st| st.doc.id == *id).ok_or(EngineError::NoDocument)
    }

    /// A recorded index is one-based opening order, even after tabs are dragged.
    pub(crate) fn document_at_index(&self, index: u64) -> Result<usize> {
        let id = index
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| self.open_document_order.get(index))
            .ok_or_else(|| EngineError::Other(format!("no document at opening index {index}")))?;
        self.docs.iter().position(|st| st.doc.id == *id).ok_or(EngineError::NoDocument)
    }

    /// Relative opening-order distance between two open documents, for portable action steps.
    pub(crate) fn document_offset_between(&self, from: photocraft_doc::DocId, to: photocraft_doc::DocId) -> Option<i64> {
        let from = self.open_document_order.iter().position(|&id| id == from)?;
        let to = self.open_document_order.iter().position(|&id| id == to)?;
        i64::try_from(to).ok()?.checked_sub(i64::try_from(from).ok()?)
    }

    /// Add a document (from File → New, an import, etc.) and make it active.
    /// Preserve its identity unless an open document or a picked clone source reserves it.
    /// Identity-dependent callers must read the admitted document at the returned index.
    pub fn add_document(&mut self, mut doc: Document, path: Option<String>) -> usize {
        // A saved copy can reuse a closed source's persisted ID. Give the new tab another ID so
        // a closed clone source stays invalid until the user explicitly picks a new source.
        // Persisted IDs can overlap the allocator, so check every replacement too.
        while self.docs.iter().any(|st| st.doc.id == doc.id) || self.presets.clone.slots.iter().any(|slot| slot.document == Some(doc.id)) {
            doc.id = photocraft_doc::DocId::fresh();
        }
        self.open_document_order.push(doc.id);
        let mut st = DocState::new(doc, path);
        st.history.max_states = self.prefs.get().performance.history_states.max(1) as usize;
        st.history.max_bytes = self.prefs.get().performance.history_cap_bytes();
        st.history.non_linear = self.prefs.get().history_options.allow_non_linear_history;
        // History Options › Automatically Create First Snapshot off: no snapshot of the document
        // as opened, so the History panel starts at the opening state (#2966).
        if !self.prefs.get().history_options.automatically_create_first_snapshot {
            st.snapshots.clear();
        }
        self.docs.push(st);
        let i = self.docs.len() - 1;
        self.active = Some(i);
        self.note_active_change();
        i
    }

    /// Holds a decoded document back from opening (see `held_opens`); returns its id.
    pub(crate) fn hold_open(&mut self, doc: Document, source_read_only: bool) -> u64 {
        self.next_held_open = self.next_held_open.wrapping_add(1);
        self.held_opens.push((self.next_held_open, doc, source_read_only));
        self.next_held_open
    }

    /// Whether a document held back for its prompt already has `name`.
    pub(crate) fn held_open_named(&self, name: &str) -> bool {
        self.held_opens.iter().any(|(_, d, _)| d.name == name)
    }

    /// Takes a document a background open held back for the profile prompt: the document and
    /// whether its source is read-only. `None` when there is no such document (already taken).
    pub fn take_held_open(&mut self, id: u64) -> Option<(Document, bool)> {
        let i = self.held_opens.iter().position(|(h, _, _)| *h == id)?;
        let (_, doc, read_only) = self.held_opens.remove(i);
        Some((doc, read_only))
    }

    pub fn close(&mut self, index: usize) -> Option<DocState> {
        if index >= self.docs.len() {
            return None;
        }
        smart_cmds::on_close(self, index);
        if let Some(id) = self.docs.get(index).map(|d| d.doc.id) {
            self.cancel_jobs_on(id);
            self.subject_cache.forget_doc(id);
            self.object_finder.forget_doc(id);
            self.presets.clone.forget_doc(id);
            self.link_missing.retain(|(d, _)| *d != id.0);
            self.link_scan = None;
        }
        let d = self.docs.remove(index);
        self.open_document_order.retain(|&id| id != d.doc.id);
        self.active = if self.docs.is_empty() {
            None
        } else {
            Some(match self.active {
                Some(active) if active > index => active - 1,
                Some(active) if active < index => active,
                _ => index.min(self.docs.len() - 1),
            })
        };
        self.note_active_change();
        Some(d)
    }
}

/// Closed documents still being freed on the release thread ([`Session::close_and_release`]).
static RELEASING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// One document counted in [`RELEASING`] until this is dropped.
#[cfg(not(target_arch = "wasm32"))]
struct Releasing;

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Releasing {
    fn drop(&mut self) {
        RELEASING.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Session {
    /// How many closed documents are still being freed off the calling thread
    /// ([`Session::close_and_release`]): tests wait for none before checking the scratch pool.
    pub fn documents_being_released() -> usize {
        RELEASING.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Close document `index` as [`Session::close`] does, and free it off the UI thread. A
    /// gigapixel document and its history are millions of tiles: dropping them where the command
    /// ran took over a second, long enough for a Wayland compositor to drop the window. Returns
    /// whether there was such a document.
    pub fn close_and_release(&mut self, index: usize) -> bool {
        let Some(closed) = self.close(index) else { return false };
        #[cfg(not(target_arch = "wasm32"))]
        release_elsewhere(closed);
        #[cfg(target_arch = "wasm32")]
        drop(closed);
        true
    }
}

/// A closed document on its way to the release thread, counted in [`RELEASING`] until freed.
#[cfg(not(target_arch = "wasm32"))]
type Closed = (DocState, Releasing);

/// How many release threads were started ([`release_elsewhere`]): one, unless one stopped.
#[cfg(not(target_arch = "wasm32"))]
static RELEASE_THREADS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Where closed documents go to be freed ([`release_elsewhere`]); `None` until the first close.
#[cfg(not(target_arch = "wasm32"))]
static RELEASER: std::sync::Mutex<Option<std::sync::mpsc::Sender<Closed>>> = std::sync::Mutex::new(None);

/// Free `closed` on the release thread: one thread frees closed documents one after another, so
/// closing many at once (Close All) starts no more threads than closing one. The thread starts
/// with the first close, and again if it stopped (a panic while freeing); if it can't start,
/// the document is freed here, as before.
#[cfg(not(target_arch = "wasm32"))]
fn release_elsewhere(closed: DocState) {
    RELEASING.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    // Counts the document back out however its release ends, a panic included.
    let item: Closed = (closed, Releasing);
    let mut releaser = RELEASER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let item = match releaser.as_ref() {
        Some(tx) => match tx.send(item) {
            Ok(()) => return,
            // The thread stopped: its queue went with it, counted out as it dropped.
            Err(std::sync::mpsc::SendError(item)) => item,
        },
        None => item,
    };
    let (tx, rx) = std::sync::mpsc::channel::<Closed>();
    let spawned = std::thread::Builder::new().name("photocraft-close".into()).spawn(move || {
        for (closed, _releasing) in rx {
            drop(closed);
            // Its tiles went back to the scratch pool as they dropped, after the command's own
            // release: give that memory back too.
            photocraft_raster::scratch::release();
        }
    });
    if spawned.is_ok() {
        RELEASE_THREADS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    let item = match spawned {
        Ok(_) => match tx.send(item) {
            Ok(()) => {
                *releaser = Some(tx);
                return;
            }
            Err(std::sync::mpsc::SendError(item)) => item,
        },
        Err(_) => item,
    };
    // No release thread: the document is freed here instead, as before, and counted out only
    // once its memory is back too.
    *releaser = None;
    let (closed, releasing) = item;
    drop(closed);
    photocraft_raster::scratch::release();
    drop(releasing);
}

pub(super) fn fix_active(st: &mut DocState) {
    if st.active_layer.is_none_or(|id| st.doc.layer(id).is_none()) {
        st.active_layer = st.doc.top_layer();
    }
    fix_selection(st);
    channel_cmds::fix_view(st);
}

#[cfg(test)]
mod close_tests {
    use serde_json::json;

    use crate::Session;

    /// Closing removes the document and says so; a document that isn't open is reported missing.
    #[test]
    fn closing_frees_the_document_and_reports_a_missing_one() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        let first = std::sync::Arc::downgrade(&s.documents()[0].doc);
        assert!(s.close_and_release(0));
        assert_eq!(s.documents().len(), 1, "closed like Session::close");
        assert_eq!(s.documents()[0].doc.size.width, 32, "the other document stays");
        assert!(!s.close_and_release(5), "no such document");
        // Freed on its own thread: gone shortly after.
        let t = std::time::Instant::now();
        while first.upgrade().is_some() {
            assert!(t.elapsed() < std::time::Duration::from_secs(10), "the closed document was never freed");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Closing many documents at once (Close All) starts at most one release thread, however
    /// many documents it closes: they queue for the same thread.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn closing_many_documents_starts_at_most_one_release_thread() {
        let mut s = Session::new();
        let docs: Vec<_> = (0..24)
            .map(|_| {
                s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
                std::sync::Arc::downgrade(&s.documents().last().unwrap().doc)
            })
            .collect();
        let before = super::RELEASE_THREADS.load(std::sync::atomic::Ordering::SeqCst);
        s.execute("file.closeAll", json!({})).unwrap();
        assert!(s.documents().is_empty());
        let t = std::time::Instant::now();
        while docs.iter().any(|d| d.upgrade().is_some()) {
            assert!(t.elapsed() < std::time::Duration::from_secs(10), "the closed documents were never freed");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        // Other tests close documents on the same thread meanwhile; none starts another.
        let started = super::RELEASE_THREADS.load(std::sync::atomic::Ordering::SeqCst) - before;
        assert!(started <= 1, "{started} release threads for one Close All");
    }
}
