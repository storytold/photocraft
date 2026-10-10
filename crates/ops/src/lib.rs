//! Undo/redo history.
//!
//! Because pixel tiles are `Arc`-shared copy-on-write (see `photocraft-raster`), a full
//! [`Document`] clone costs O(layers + tiles) pointer copies, not pixel copies. History therefore
//! stores whole-document snapshots per transaction, which is simple, obviously correct, and the
//! same approach Photoshop's History panel exposes to users (one state per step).
//!
//! Memory is bounded by `max_states` plus an approximate byte budget. Accounting deduplicates shared tiles; archived snapshots do not retain resident pixels.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use photocraft_doc::{Document, LayerId};

/// The layers a state targeted when it was created: the active ("key") layer and every selected
/// layer. Undo and redo bring them back with the state's document, as the reference app does;
/// selecting layers is not a step of its own, so it doesn't change a state's target.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerTarget {
    pub active: Option<LayerId>,
    pub selected: Vec<LayerId>,
}

#[derive(Clone, Debug)]
pub struct HistoryState {
    pub label: String,
    document: StoredDocument,
    has_selection: bool,
    /// The layers targeted when this state was created.
    pub layers: LayerTarget,
}

/// Engine-owned immutable scratch archive. Loading must preserve the complete document.
pub trait ArchivedDocument: std::fmt::Debug + Send + Sync {
    fn load(&self) -> Result<Arc<Document>, String>;
    fn metadata_bytes(&self) -> usize {
        0
    }
    /// Resident backing for compact replay states; counted once, never mistaken for disk.
    fn resident(&self) -> Option<&Arc<Document>> {
        None
    }
}

#[derive(Clone, Debug)]
enum StoredDocument {
    Resident(Arc<Document>),
    Archived(Arc<dyn ArchivedDocument>),
}

impl StoredDocument {
    fn load(&self) -> Result<Arc<Document>, String> {
        match self {
            Self::Resident(doc) => Ok(doc.clone()),
            Self::Archived(archive) => archive.load(),
        }
    }
    fn resident(&self) -> Option<&Arc<Document>> {
        match self {
            Self::Resident(doc) => Some(doc),
            Self::Archived(archive) => archive.resident(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct History {
    /// Past states; the last one is the state *before* the current document.
    undo: VecDeque<HistoryState>,
    redo: Vec<HistoryState>,
    pub max_states: usize,
    /// Pixel memory budget in bytes for the current document plus the tiles only history holds
    /// (0 = unlimited). [`History::trim`] drops the oldest states beyond it.
    pub max_bytes: usize,
    /// Label of the step that produced the current document.
    current_label: String,
    /// The layers targeted when the current document was created (opened or edited).
    current_layers: LayerTarget,
    current_archive: Option<Arc<dyn ArchivedDocument>>,
    current_archive_document: Option<std::sync::Weak<Document>>,
}

impl Default for History {
    fn default() -> Self {
        Self::new(50)
    }
}

impl History {
    pub fn new(max_states: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            max_states: max_states.max(1),
            max_bytes: 0,
            current_label: "Open".into(),
            current_layers: LayerTarget::default(),
            current_archive: None,
            current_archive_document: None,
        }
    }

    /// Record that `before` was replaced by a new current document via step `label`, which left
    /// `layers` targeted.
    pub fn record(&mut self, label: impl Into<String>, before: Arc<Document>, layers: LayerTarget) {
        self.clear_current_archive();
        let prev_label = std::mem::replace(&mut self.current_label, label.into());
        let prev_layers = std::mem::replace(&mut self.current_layers, layers);
        self.undo.push_back(HistoryState {
            label: prev_label,
            has_selection: before.selection.is_some(),
            document: StoredDocument::Resident(before),
            layers: prev_layers,
        });
        self.redo.clear();
        while self.undo.len() > self.max_states {
            self.undo.pop_front();
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.can_undo().then_some(self.current_label.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|s| s.label.as_str())
    }
    /// Labels of the redo states, the next redo first (the History panel's greyed rows).
    pub fn redo_labels(&self) -> impl Iterator<Item = &str> {
        self.redo.iter().rev().map(|s| s.label.as_str())
    }

    pub fn set_current_layers(&mut self, layers: LayerTarget) {
        self.current_layers = layers;
    }

    /// Restore first: a failed archive read never moves either history stack.
    pub fn try_undo(&mut self, current: Arc<Document>) -> Result<Option<(Arc<Document>, LayerTarget)>, String> {
        let Some(prev) = self.undo.back() else { return Ok(None) };
        let restored = prev.document.load()?;
        let Some(prev) = self.undo.pop_back() else { return Ok(None) };
        let label = std::mem::replace(&mut self.current_label, prev.label);
        let layers = std::mem::replace(&mut self.current_layers, prev.layers.clone());
        let has_selection = current.selection.is_some();
        let document = self.take_current_document(current);
        self.redo.push(HistoryState { label, has_selection, document, layers });
        self.current_archive = match prev.document {
            StoredDocument::Archived(archive) => Some(archive),
            _ => None,
        };
        self.current_archive_document = self.current_archive.as_ref().map(|_| Arc::downgrade(&restored));
        Ok(Some((restored, prev.layers)))
    }

    pub fn try_redo(&mut self, current: Arc<Document>) -> Result<Option<(Arc<Document>, LayerTarget)>, String> {
        let Some(next) = self.redo.last() else { return Ok(None) };
        let restored = next.document.load()?;
        let Some(next) = self.redo.pop() else { return Ok(None) };
        let label = std::mem::replace(&mut self.current_label, next.label);
        let layers = std::mem::replace(&mut self.current_layers, next.layers.clone());
        let has_selection = current.selection.is_some();
        let document = self.take_current_document(current);
        self.undo.push_back(HistoryState { label, has_selection, document, layers });
        self.current_archive = match next.document {
            StoredDocument::Archived(archive) => Some(archive),
            _ => None,
        };
        self.current_archive_document = self.current_archive.as_ref().map(|_| Arc::downgrade(&restored));
        Ok(Some((restored, next.layers)))
    }

    pub fn undo(&mut self, current: Arc<Document>) -> Option<(Arc<Document>, LayerTarget)> {
        self.try_undo(current).ok().flatten()
    }
    pub fn redo(&mut self, current: Arc<Document>) -> Option<(Arc<Document>, LayerTarget)> {
        self.try_redo(current).ok().flatten()
    }

    /// Entries for a History panel: past labels oldest→newest, then the current label.
    pub fn entries(&self) -> Vec<String> {
        self.undo.iter().map(|s| s.label.clone()).chain(std::iter::once(self.current_label.clone())).collect()
    }

    /// Document of past entry `i`, indexed like [`History::entries`] (0 = oldest). The last entry is
    /// the current document, which the history does not hold, so it (and any index past it) is `None`.
    pub fn state(&self, i: usize) -> Option<Arc<Document>> {
        self.try_state(i).ok().flatten()
    }

    /// Metadata-only query for menus; never hydrates a cold snapshot.
    pub fn has_past_selection(&self) -> bool {
        self.undo.iter().any(|state| state.has_selection)
    }

    /// Most recent selection-bearing state, found without reading scratch storage.
    pub fn latest_selection_state(&self) -> Option<usize> {
        self.undo.iter().enumerate().rev().find_map(|(index, state)| state.has_selection.then_some(index))
    }

    pub fn resident_state(&self, i: usize) -> Option<Arc<Document>> {
        self.undo.get(i).and_then(|state| match &state.document {
            StoredDocument::Resident(doc) => Some(doc.clone()),
            _ => None,
        })
    }

    pub fn try_state(&self, i: usize) -> Result<Option<Arc<Document>>, String> {
        self.undo.get(i).map(|state| state.document.load()).transpose()
    }

    /// Resident snapshots only; this never reads scratch storage.
    pub fn resident_states(&self) -> Vec<Arc<Document>> {
        let mut seen = HashSet::new();
        self.undo
            .iter()
            .chain(self.redo.iter())
            .filter_map(|state| state.document.resident().cloned())
            .chain(self.current_archive.iter().filter_map(|archive| archive.resident().cloned()))
            .filter(|doc| seen.insert(Arc::as_ptr(doc) as usize))
            .collect()
    }

    /// Concrete snapshots eligible for scratch serialization. Compact replay backing stays resident.
    pub fn spillable_states(&self) -> Vec<Arc<Document>> {
        self.undo
            .iter()
            .chain(self.redo.iter())
            .filter_map(|state| match &state.document {
                StoredDocument::Resident(doc) => Some(doc.clone()),
                _ => None,
            })
            .collect()
    }

    /// State descriptors and compact replay records, separate from pixel payload accounting.
    pub fn metadata_bytes(&self) -> usize {
        let mut total = self.current_label.capacity().saturating_add(self.current_layers.selected.capacity().saturating_mul(std::mem::size_of::<LayerId>()));
        for state in self.undo.iter().chain(self.redo.iter()) {
            total = total
                .saturating_add(std::mem::size_of::<HistoryState>())
                .saturating_add(state.label.capacity())
                .saturating_add(state.layers.selected.capacity().saturating_mul(std::mem::size_of::<LayerId>()));
            if let StoredDocument::Archived(archive) = &state.document {
                total = total.saturating_add(archive.metadata_bytes());
            }
        }
        if let Some(archive) = &self.current_archive {
            total = total.saturating_add(archive.metadata_bytes());
        }
        total
    }

    pub fn archived_states(&self) -> usize {
        self.undo
            .iter()
            .chain(self.redo.iter())
            .filter(|state| matches!(&state.document, StoredDocument::Archived(archive) if archive.resident().is_none()))
            .count()
    }

    /// Adjacent resident snapshots pinned for immediate undo/redo.
    pub fn hot_states(&self) -> Vec<Arc<Document>> {
        self.undo
            .back()
            .into_iter()
            .chain(self.redo.last())
            .filter_map(|state| state.document.resident().cloned())
            .chain(self.current_archive.iter().filter_map(|archive| archive.resident().cloned()))
            .collect()
    }

    /// Whether this allocation is the immediate undo or redo state.
    /// Used to revalidate asynchronous spill results after intervening navigation.
    pub fn is_hot(&self, doc: &Arc<Document>) -> bool {
        self.undo.back().and_then(|state| state.document.resident()).is_some_and(|hot| Arc::ptr_eq(doc, hot))
            || self.redo.last().and_then(|state| state.document.resident()).is_some_and(|hot| Arc::ptr_eq(doc, hot))
    }

    /// Oldest eligible snapshot, keeping the immediate undo and redo states hot.
    pub fn spill_candidate(&self) -> Option<Arc<Document>> {
        self.undo
            .iter()
            .take(self.undo.len().saturating_sub(1))
            .chain(self.redo.iter().take(self.redo.len().saturating_sub(1)))
            .filter_map(|state| state.document.resident())
            .find(|doc| {
                !self.undo.back().and_then(|state| state.document.resident()).is_some_and(|hot| Arc::ptr_eq(doc, hot))
                    && !self.redo.last().and_then(|state| state.document.resident()).is_some_and(|hot| Arc::ptr_eq(doc, hot))
            })
            .cloned()
    }

    /// Replace only the allocation actually archived; stale worker results are harmless.
    pub fn replace_resident(&mut self, doc: &Arc<Document>, archive: Arc<dyn ArchivedDocument>) -> bool {
        let mut replaced = false;
        for state in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            if matches!(&state.document, StoredDocument::Resident(resident) if Arc::ptr_eq(resident, doc)) {
                state.document = StoredDocument::Archived(archive.clone());
                replaced = true;
            }
        }
        replaced
    }

    /// Budget fallback, preserving the immediate undo and redo steps.
    pub fn drop_oldest(&mut self) -> bool {
        if self.undo.len() > 1 {
            self.undo.pop_front();
            true
        } else if self.redo.len() > 1 {
            self.redo.remove(0);
            true
        } else {
            false
        }
    }

    /// Explicit preference reduction may retire even the final undo/redo state.
    /// The current document is never owned by these stacks.
    pub fn discard_oldest(&mut self) -> bool {
        if self.undo.pop_front().is_some() {
            true
        } else if !self.redo.is_empty() {
            self.redo.remove(0);
            true
        } else {
            false
        }
    }

    /// Enforce a lowered history-count preference without reading scratch storage.
    pub fn enforce_state_limit(&mut self) -> usize {
        let limit = self.max_states.max(1);
        let mut removed = 0;
        while self.undo.len().saturating_add(self.redo.len()) > limit {
            if !self.discard_oldest() {
                break;
            }
            removed += 1;
        }
        removed
    }

    /// Number of past states (entries before the current one).
    pub fn past_len(&self) -> usize {
        self.undo.len()
    }

    /// Rename the current step (e.g. after folding several steps into one with [`Self::purge_last`]).
    pub fn set_current_label(&mut self, label: impl Into<String>) {
        self.current_label = label.into();
    }

    /// Forget the most recent undo state and every redo state (Edit › Purge › Undo): the
    /// last step can no longer be undone and its pixels are released.
    pub fn purge_last(&mut self) -> bool {
        self.redo.clear();
        self.undo.pop_back().is_some()
    }

    /// Forget the redo states, e.g. after undoing half of a compound step that failed.
    pub fn clear_redo(&mut self) {
        self.redo.clear();
    }

    pub fn clear_current_archive(&mut self) {
        self.current_archive = None;
        self.current_archive_document = None;
    }

    pub fn set_current_archive(&mut self, current: &Arc<Document>, archive: Arc<dyn ArchivedDocument>) {
        self.current_archive_document = Some(Arc::downgrade(current));
        self.current_archive = Some(archive);
    }

    fn take_current_document(&mut self, current: Arc<Document>) -> StoredDocument {
        let matches = self.current_archive_document.take().and_then(|bound| bound.upgrade()).is_some_and(|bound| Arc::ptr_eq(&bound, &current));
        match self.current_archive.take() {
            Some(archive) if matches => StoredDocument::Archived(archive),
            _ => StoredDocument::Resident(current),
        }
    }

    pub fn clear(&mut self) {
        self.clear_current_archive();
        self.undo.clear();
        self.redo.clear();
    }

    /// Approximate unique tile and shared binary-payload bytes held by history.
    pub fn unique_bytes(&self, current: &Document) -> usize {
        let mut seen = HashSet::new();
        document_bytes(current, &mut seen);
        self.resident_states().iter().fold(0usize, |n, doc| n.saturating_add(document_bytes(doc, &mut seen)))
    }

    /// Managed bytes of `current` plus the allocations only history holds (what [`History::trim`]
    /// bounds by [`History::max_bytes`]).
    pub fn pixel_bytes(&self, current: &Document) -> usize {
        let mut seen = HashSet::new();
        let own = document_bytes(current, &mut seen);
        self.resident_states().iter().fold(own, |n, doc| n.saturating_add(document_bytes(doc, &mut seen)))
    }

    /// Keep pixel memory within [`History::max_bytes`]: the current document's tiles plus the
    /// tiles only history holds (newest states first). The oldest undo states that don't fit are
    /// dropped; the most recent one is always kept so the last step can be undone. Returns how
    /// many states were dropped.
    pub fn trim(&mut self, current: &Document) -> usize {
        if self.max_bytes == 0 || self.undo.len() <= 1 {
            return 0;
        }
        let mut seen = HashSet::new();
        let mut total = document_bytes(current, &mut seen);
        for s in self.redo.iter().rev() {
            if let Some(doc) = s.document.resident() {
                total = total.saturating_add(document_bytes(doc, &mut seen));
            }
        }
        let mut keep = 0;
        for s in self.undo.iter().rev() {
            if let Some(doc) = s.document.resident() {
                total = total.saturating_add(document_bytes(doc, &mut seen));
            }
            if total > self.max_bytes && keep >= 1 {
                break;
            }
            keep += 1;
        }
        let drop = self.undo.len() - keep;
        self.undo.drain(..drop);
        drop
    }
}

/// Managed tile and shared binary-payload bytes not already in `seen`.
/// Metadata containers, strings, renderer caches and allocator overhead are additional memory.
pub fn document_bytes(doc: &Document, seen: &mut HashSet<usize>) -> usize {
    let mut add = |surface: &photocraft_doc::Surface| {
        surface.tiles().filter(|(_, tile)| seen.insert(Arc::as_ptr(tile) as usize)).fold(0usize, |n, (_, tile)| n.saturating_add(tile.bytes().len()))
    };
    let mut total = 0usize;
    let layers = doc.walk();
    for (_, _, layer) in &layers {
        if let Some(surface) = layer.surface() {
            total = total.saturating_add(add(surface));
        }
        if let Some(mask) = &layer.mask {
            total = total.saturating_add(add(&mask.surface));
        }
        if let Some(cache) = &layer.fill_cache {
            total = total.saturating_add(add(&cache.surface));
        }
        if let photocraft_doc::LayerContent::Smart(smart) = &layer.content
            && let Some(mask) = &smart.filter_mask
        {
            total = total.saturating_add(add(&mask.surface));
        }
        if let Some(video) = &layer.video {
            for frame in &video.frames {
                total = total.saturating_add(add(frame));
            }
        }
    }
    for channel in doc.channels.iter().chain(&doc.quick_mask) {
        total = total.saturating_add(add(&channel.surface));
    }
    if let Some(selection) = &doc.selection {
        total = total.saturating_add(add(selection));
    }
    for pattern in &doc.patterns {
        total = total.saturating_add(add(&pattern.surface));
    }
    // Distinct live Arc allocations cannot have the same address, including across
    // payload types. A caller keeps all traversed documents alive while deduplicating.
    let mut blob = |bytes: &Arc<Vec<u8>>| {
        if seen.insert(Arc::as_ptr(bytes) as usize) { bytes.capacity() } else { 0 }
    };
    for bytes in doc.icc_profile.iter().chain(doc.metadata.exif.iter()) {
        total = total.saturating_add(blob(bytes));
    }
    for (_, _, bytes) in &doc.metadata.psd_resources {
        total = total.saturating_add(blob(bytes));
    }
    for (_, _, bytes) in &doc.metadata.psd_global_blocks {
        total = total.saturating_add(blob(bytes));
    }
    for path in &doc.paths {
        if let Some(bytes) = &path.psd_raw {
            total = total.saturating_add(blob(bytes));
        }
    }
    for (_, _, layer) in &layers {
        for (_, bytes) in &layer.psd_blocks {
            total = total.saturating_add(blob(bytes));
        }
        if let Some(bytes) = &layer.effects.psd_raw {
            total = total.saturating_add(blob(bytes));
        }
        match &layer.content {
            photocraft_doc::LayerContent::Text(text) => {
                if let Some(bytes) = &text.psd_raw {
                    total = total.saturating_add(blob(bytes));
                }
            }
            photocraft_doc::LayerContent::Shape(shape) => {
                if let Some(bytes) = &shape.psd_raw {
                    total = total.saturating_add(blob(bytes));
                }
            }
            photocraft_doc::LayerContent::Smart(smart) => {
                if let Some(bytes) = &smart.psd_raw {
                    total = total.saturating_add(blob(bytes));
                }
                if let photocraft_doc::SmartSource::Embedded { bytes, .. } = &smart.source {
                    total = total.saturating_add(blob(bytes));
                }
            }
            _ => {}
        }
    }
    for (_, _, layer) in &layers {
        if let photocraft_doc::LayerContent::Adjustment(photocraft_doc::Adjustment::ColorLookup { lut: Some(lut), .. }) = &layer.content
            && seen.insert(Arc::as_ptr(lut) as usize)
        {
            total = total.saturating_add(lut.capacity().saturating_mul(std::mem::size_of::<f32>()));
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Color, ColorMode, Layer, SampleType, Size};

    fn base() -> Document {
        Document::with_background("h", Size::new(64, 64), ColorMode::Rgb, SampleType::U8, Color::WHITE)
    }

    /// Apply an edit the way the engine does: snapshot, mutate a clone, record.
    fn edit(h: &mut History, cur: &mut Arc<Document>, label: &str, f: impl FnOnce(&mut Document)) {
        let before = cur.clone();
        let mut d = (**cur).clone();
        f(&mut d);
        *cur = Arc::new(d);
        h.record(label, before, LayerTarget::default());
    }

    #[test]
    fn undo_redo_cycle() {
        let mut h = History::default();
        let mut cur = Arc::new(base());
        edit(&mut h, &mut cur, "New Layer", |d| {
            d.insert_above(None, Layer::raster("L", d.pixel_format()));
        });
        assert_eq!(cur.layers.len(), 2);
        assert_eq!(h.undo_label(), Some("New Layer"));
        cur = h.undo(cur).unwrap().0;
        assert_eq!(cur.layers.len(), 1);
        assert_eq!(h.redo_label(), Some("New Layer"));
        cur = h.redo(cur).unwrap().0;
        assert_eq!(cur.layers.len(), 2);
        assert!(h.redo(cur.clone()).is_none());
    }

    #[test]
    fn undo_and_redo_return_the_layers_each_state_targeted_when_created() {
        let mut h = History::default();
        let open = Arc::new(base());
        let bg = open.layers[0].id;
        let target = |id: LayerId| LayerTarget { active: Some(id), selected: vec![id] };
        h.set_current_layers(target(bg));
        // New Layer targets the new layer; the background is then selected (not a step) and filled.
        let mut d = (*open).clone();
        let new = d.insert_above(None, Layer::raster("L", d.pixel_format()));
        let layered = Arc::new(d);
        h.record("New Layer", open, target(new));
        let mut d = (*layered).clone();
        d.name = "filled".into();
        h.record("Fill", layered, target(bg));
        let (d, layers) = h.undo(Arc::new(d)).unwrap();
        assert_eq!(layers, target(new), "New Layer's state targeted the new layer when created");
        let (d, layers) = h.undo(d).unwrap();
        assert_eq!(layers, target(bg), "the opened document's target");
        let (d, layers) = h.redo(d).unwrap();
        assert_eq!(layers, target(new));
        let (_, layers) = h.redo(d).unwrap();
        assert_eq!(layers, target(bg), "Fill's state targeted the background");
    }

    #[test]
    fn state_accessor_matches_entries() {
        let mut h = History::default();
        let mut cur = Arc::new(base());
        let open = cur.clone();
        edit(&mut h, &mut cur, "A", |d| d.name = "a".into());
        edit(&mut h, &mut cur, "B", |d| d.name = "b".into());
        assert_eq!(h.entries().len(), h.past_len() + 1);
        assert!(Arc::ptr_eq(&h.state(0).unwrap(), &open));
        assert_eq!(h.state(1).unwrap().name, "a");
        assert!(h.state(2).is_none(), "the current document is not held");
        cur = h.undo(cur).unwrap().0;
        h.undo(cur).unwrap();
        assert_eq!(h.redo_labels().collect::<Vec<_>>(), ["A", "B"], "next redo first");
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut h = History::default();
        let mut cur = Arc::new(base());
        edit(&mut h, &mut cur, "A", |d| d.name = "a".into());
        cur = h.undo(cur).unwrap().0;
        edit(&mut h, &mut cur, "B", |d| d.name = "b".into());
        assert!(!h.can_redo());
        assert_eq!(h.entries(), vec!["Open".to_string(), "B".to_string()]);
    }

    #[test]
    fn max_states_bound() {
        let mut h = History::new(3);
        let mut cur = Arc::new(base());
        for i in 0..10 {
            edit(&mut h, &mut cur, &format!("step {i}"), |d| d.name = format!("{i}"));
        }
        let mut n = 0;
        while let Some((d, _)) = h.undo(cur.clone()) {
            cur = d;
            n += 1;
        }
        assert_eq!(n, 3);
        assert_eq!(cur.name, "6");
    }

    #[test]
    fn byte_budget_drops_oldest_states_but_keeps_one() {
        let mut h = History::new(50);
        let mut cur = Arc::new(base());
        let tile = 256 * 256 * 4;
        // Each step repaints the background's tile: one unique tile per state.
        for i in 0..6 {
            edit(&mut h, &mut cur, &format!("paint {i}"), |d| {
                let id = d.layers[0].id;
                d.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 8.0, 0.0, 0.0, 1.0]);
            });
        }
        assert_eq!(h.trim(&cur), 0, "unlimited by default");
        assert_eq!(h.past_len(), 6);
        // The current tile plus three history tiles fit.
        h.max_bytes = 4 * tile;
        assert_eq!(h.trim(&cur), 3);
        assert_eq!(h.past_len(), 3);
        assert_eq!(h.unique_bytes(&cur), 3 * tile);
        assert_eq!(h.pixel_bytes(&cur), 4 * tile, "the current tile plus history's");
        assert_eq!(h.entries()[0], "paint 2");
        // A budget smaller than one state still keeps the last step undoable.
        h.max_bytes = 1;
        h.trim(&cur);
        assert_eq!(h.past_len(), 1);
        assert!(h.undo(cur.clone()).is_some());
    }

    #[test]
    fn history_shares_untouched_tiles() {
        let mut h = History::default();
        let mut cur = Arc::new(base());
        assert_eq!(h.unique_bytes(&cur), 0);
        // rename only: no pixel changes, history holds no unique tiles
        edit(&mut h, &mut cur, "Rename", |d| d.name = "x".into());
        assert_eq!(h.unique_bytes(&cur), 0);
        // paint one pixel: exactly one tile becomes unique to history
        edit(&mut h, &mut cur, "Paint", |d| {
            let id = d.layers[0].id;
            d.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(1, 1, &[0.0, 0.0, 0.0, 1.0]);
        });
        assert_eq!(h.unique_bytes(&cur), 256 * 256 * 4);
    }
    #[derive(Debug)]
    struct Archive(Result<Arc<Document>, String>);
    impl ArchivedDocument for Archive {
        fn load(&self) -> Result<Arc<Document>, String> {
            self.0.clone()
        }
    }

    #[test]
    fn cold_restore_and_failed_read_preserve_stacks() {
        let mut history = History::new(50);
        let mut current = Arc::new(base());
        let original = current.clone();
        edit(&mut history, &mut current, "A", |doc| doc.name = "a".into());
        let labels = history.entries();
        assert!(history.replace_resident(&original, Arc::new(Archive(Err("corrupt scratch blob".into())))));
        assert_eq!(history.archived_states(), 1);
        assert!(history.try_state(0).is_err());
        assert!(history.try_undo(current.clone()).is_err());
        assert_eq!(history.entries(), labels);
        assert!(!history.can_redo());
        history.clear();
        history.record("A", original.clone(), LayerTarget::default());
        assert!(history.replace_resident(&original, Arc::new(Archive(Ok(original.clone())))));
        let restored = history.try_undo(current.clone()).unwrap().unwrap().0;
        assert!(Arc::ptr_eq(&restored, &original));
        assert!(Arc::ptr_eq(&history.try_redo(restored).unwrap().unwrap().0, &current));
    }

    #[test]
    fn replacement_deduplicates_and_stale_results_are_ignored() {
        let mut history = History::new(50);
        let original = Arc::new(base());
        history.record("A", original.clone(), LayerTarget::default());
        history.record("B", original.clone(), LayerTarget::default());
        assert!(history.spill_candidate().is_none(), "shared immediate undo allocation stays hot");
        assert!(history.replace_resident(&original, Arc::new(Archive(Ok(original.clone())))));
        assert_eq!(history.archived_states(), 2);
        assert!(history.resident_states().is_empty());
        assert!(!history.replace_resident(&original, Arc::new(Archive(Ok(original.clone())))));
        assert!(history.drop_oldest());
        assert!(!history.drop_oldest());
        history.clear();
        assert_eq!(history.archived_states(), 0);
    }

    #[test]
    fn selection_patterns_and_video_pixels_are_accounted_once() {
        let mut doc = base();
        let format = doc.pixel_format();
        let mut surface = photocraft_doc::Surface::new(format);
        surface.write_pixel(0, 0, &[0.25, 0.5, 0.75, 1.0]);
        doc.selection = Some(surface.clone());
        doc.patterns.push(photocraft_doc::Pattern::new("P", surface.clone(), 1, 1));
        doc.layers[0].video = Some(photocraft_doc::VideoData::new(vec![surface], 24.0));
        let mut seen = HashSet::new();
        assert_eq!(document_bytes(&doc, &mut seen), 2 * 256 * 256 * 4);
        assert_eq!(document_bytes(&doc, &mut seen), 0);
    }

    #[test]
    fn cold_redo_failure_keeps_the_next_step() {
        let mut history = History::new(50);
        let mut current = Arc::new(base());
        edit(&mut history, &mut current, "A", |doc| doc.name = "a".into());
        let future = current.clone();
        current = history.try_undo(current).unwrap().unwrap().0;
        assert!(history.replace_resident(&future, Arc::new(Archive(Err("read failed".into())))));
        let label = history.redo_label().map(str::to_owned);
        assert!(history.try_redo(current).is_err());
        assert_eq!(history.redo_label(), label.as_deref());
        assert!(!history.can_undo());
    }
    #[test]
    fn menu_queries_never_load_a_cold_selection() {
        let mut history = History::new(50);
        let mut doc = base();
        doc.selection = Some(photocraft_doc::Surface::new(doc.pixel_format()));
        let doc = Arc::new(doc);
        history.record("Select", doc.clone(), LayerTarget::default());
        assert!(history.replace_resident(&doc, Arc::new(Archive(Err("must not read".into())))));
        assert!(history.has_past_selection());
        assert_eq!(history.latest_selection_state(), Some(0));
        history.record("Clear selection", Arc::new(base()), LayerTarget::default());
        assert_eq!(history.latest_selection_state(), Some(0), "skip newer no-selection states without loading archives");
        assert!(history.resident_state(0).is_none());
        assert!(history.try_state(0).is_err());
    }

    #[test]
    fn spill_and_drop_keep_adjacent_steps_hot_and_contiguous() {
        let mut history = History::new(50);
        let mut current = Arc::new(base());
        let oldest = current.clone();
        for label in ["A", "B", "C"] {
            edit(&mut history, &mut current, label, |doc| doc.name = label.into());
        }
        assert!(Arc::ptr_eq(&history.spill_candidate().unwrap(), &oldest));
        assert!(history.drop_oldest());
        assert_eq!(history.entries(), ["A", "B", "C"]);
        current = history.try_undo(current).unwrap().unwrap().0;
        current = history.try_undo(current).unwrap().unwrap().0;
        assert_eq!(current.name, "A");
        assert!(history.drop_oldest(), "discard farthest redo");
        assert_eq!(history.redo_label(), Some("B"));
        assert!(!history.drop_oldest());
        assert_eq!(history.try_redo(current).unwrap().unwrap().0.name, "B");
    }
    #[test]
    fn explicit_budget_reduction_can_discard_final_cold_state() {
        let mut history = History::new(50);
        let doc = Arc::new(base());
        history.record("A", doc.clone(), LayerTarget::default());
        history.replace_resident(&doc, Arc::new(Archive(Err("must not load".into()))));
        assert!(!history.drop_oldest());
        assert!(history.discard_oldest());
        assert!(!history.can_undo());
        assert!(!history.discard_oldest());
    }

    #[test]
    fn lowered_count_bounds_both_stacks_without_loading() {
        let mut history = History::new(50);
        let mut current = Arc::new(base());
        for label in ["A", "B", "C"] {
            edit(&mut history, &mut current, label, |doc| doc.name = label.into());
        }
        current = history.undo(current).unwrap().0;
        history.max_states = 1;
        assert_eq!(history.enforce_state_limit(), 2);
        assert_eq!(history.past_len(), 0);
        assert_eq!(history.redo_label(), Some("C"));
        assert_eq!(history.redo(current).unwrap().0.name, "C");
    }
    #[test]
    fn asynchronous_candidate_can_become_hot_after_navigation() {
        let mut history = History::new(50);
        let mut current = Arc::new(base());
        let oldest = current.clone();
        for label in ["A", "B", "C"] {
            edit(&mut history, &mut current, label, |doc| doc.name = label.into());
        }
        assert!(!history.is_hot(&oldest));
        current = history.undo(current).unwrap().0;
        let _current = history.undo(current).unwrap().0;
        assert!(history.is_hot(&oldest));
        assert!(history.spill_candidate().is_none_or(|candidate| !Arc::ptr_eq(&candidate, &oldest)));
    }
    #[test]
    fn large_shared_binary_payloads_are_counted_once() {
        let mut doc = base();
        let bytes = Arc::new(vec![17; 4096]);
        doc.icc_profile = Some(bytes.clone());
        doc.metadata.exif = Some(bytes.clone());
        doc.metadata.psd_resources.push((1, "resource".into(), bytes.clone()));
        doc.metadata.psd_global_blocks.push((*b"8BIM", *b"lnk2", bytes.clone()));
        doc.layers[0].psd_blocks.push((*b"test", bytes));
        let baseline = 256 * 256 * 4;
        let mut seen = HashSet::new();
        assert_eq!(document_bytes(&doc, &mut seen), baseline + 4096);
        let mut clone = doc.clone();
        clone.metadata.exif = Some(Arc::new(vec![4; 1024]));
        assert_eq!(document_bytes(&clone, &mut seen), 1024);
    }
}
