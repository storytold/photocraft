use super::{Arc, Document, History, LayerId, LayerTarget, channel_cmds, float_cmds, history_cmds, symmetry_cmds};

/// Per-document editing state. Plain data: the session holds each in a `Tracked`, whose revision moves with every
/// write, view state included.
#[derive(Clone, Debug, craft_ui_macros::PlainData)]
pub struct DocState {
    inputs: DocStateInputs,
    selected_cache: SelectedOrder,
    pub history: History,
    /// Anchor of range selection (the last plainly or command-clicked layer).
    pub layer_anchor: Option<LayerId>,
    pub path: Option<String>,
    /// An Affinity document (or its preview) must not acquire the native source as its Save path.
    pub source_read_only: bool,
    /// Counts the edit steps: save state ([`Self::is_dirty`]) and damage measure from it. Views key on the `Tracked`
    /// revision the session holds the state in, which also moves with view state.
    pub revision: u64,
    pub saved_revision: u64,
    /// How much of `revision` came from view-only changes (channel eyes, targeting): pixel caches can ignore those.
    pub view_revision: u64,
    /// Area changed since revision [`DocState::damage_from`] (None = assume everything changed):
    /// the latest revision's, or a whole command's when it made several.
    pub last_damage: Option<photocraft_geom::Rect>,
    /// `(from, to)`: the command that moved `revision` from `from` to `to` in more than one step,
    /// whose whole span `last_damage` covers. Written only by `Session::dispatch`; any later
    /// revision makes it stale, so [`DocState::damage_from`] falls back to one revision.
    pub(crate) damage_span: Option<(u64, u64)>,
    /// Bounded revision spans; unknown damage remains unknown when spans are combined.
    damage_log: std::collections::VecDeque<(u64, u64, Option<photocraft_geom::Rect>)>,
    /// Coalescing key of the latest history step (see [`Session::execute`]'s `coalesce` param).
    pub coalesce: Option<String>,
    /// Channels panel: targeted channel and eye toggles (view state, not history).
    pub channel_view: channel_cmds::ChannelView,
    /// Select › Isolate Layers: the Layers panel lists only these layers (empty = off; view state).
    pub isolated_layers: Vec<LayerId>,
    /// Painting symmetry preset or sampled path (tool state, not document pixels).
    pub symmetry: Option<symmetry_cmds::PaintingSymmetry>,
    /// Layers panel: layers whose effects list is collapsed under their row (the fx triangle;
    /// view state, not history). Effects lists start open.
    pub fx_collapsed: Vec<LayerId>,
    /// ⌥-click on a layer's eye (`layer.showOnly`): the layer shown alone and every layer's
    /// visibility before, so the next ⌥-click restores it (view state, not history).
    pub show_only: Option<(LayerId, Vec<(LayerId, bool)>)>,
    /// A floating selection (`select.float`): the cut piece and where it floats, until dropped
    /// (view state: the document is unchanged until `select.drop`).
    pub floating: Option<float_cmds::Floating>,
    /// Color Sampler points (`view.colorSamplers.*`, #1046): numbered sample positions in document
    /// pixel coordinates. Per-document view state: discarded when the document closes, never saved
    /// to `.pcraft` or PSD, and not a history step.
    pub color_samplers: Vec<[f64; 2]>,
    /// History panel snapshots, oldest first (`history.newSnapshot`, #2966): the first (id 0) is
    /// the document as opened (unless History Options › Automatically Create First Snapshot is
    /// off). Kept for the session only, as in Photoshop: not history states,
    /// and not saved with the document. Changed only through [`DocState::snapshots_mut`].
    snapshots: Vec<history_cmds::Snapshot>,
    /// See [`DocState::snapshots_generation`].
    snapshots_generation: u64,
    /// The id the next snapshot gets; ids are never reused, so deleting one keeps the others.
    pub(crate) next_snapshot: u64,
    /// The snapshot the latest save added (History Options › Automatically Create New Snapshot
    /// When Saving), so saving an unchanged document again doesn't add a duplicate row.
    pub(crate) save_snapshot: Option<u64>,
    /// The snapshot set as the History Brush's source (`history.setSource {snapshot}`); `None`:
    /// the source is a state, kept by the history ([`photocraft_ops::History::brush_source`]).
    pub(crate) source_snapshot: Option<u64>,
    /// Names the user gave layers (Rename Layer, Duplicate Layer's Name, `type.edit`'s name): a
    /// type layer keeps such a name although it looks like a copy's ("Title copy 2"), while a
    /// name Duplicate Layer made still follows the text (#9023). Kept for the session only.
    pub(crate) typed_names: std::collections::HashMap<LayerId, String>,
    /// The count of steps whose damage was not empty: pixel thumbnails (the Channels panel's composite and colour
    /// rows) change only when it moves, so a selection-only or view-only step keeps them. Written only by
    /// `Session::dispatch` and the job apply path.
    pub pixel_revision: u64,
}

/// Read-only document and layer selection inputs. Mutations go through `DocState` methods.
#[derive(Clone, Debug, craft_ui_macros::PlainData)]
pub struct DocStateInputs {
    /// The current document.
    pub doc: Arc<Document>,
    /// The primary layer for single-layer commands.
    pub active_layer: Option<LayerId>,
    /// The raw selection in selection order. Undo and redo restore it with the target state.
    pub selected_layers: Vec<LayerId>,
}

/// The selected layers in document order ([`DocState::selected_layers`]), built on the first read after a write.
#[derive(Clone, Debug, Default)]
struct SelectedOrder(std::sync::OnceLock<Vec<LayerId>>);

// The order is a pure function of the document and the raw selection: every write to either goes through a
// `DocState` method that drops it (`doc_mut`, `set_active_layer`, `selection_mut`), and readers get the same answer
// whether it is built or not. So a shared reference can't change what readers see.
impl craft_tracked::PlainData for SelectedOrder {}

impl std::ops::Deref for DocState {
    type Target = DocStateInputs;
    fn deref(&self) -> &Self::Target {
        &self.inputs
    }
}

impl DocState {
    /// Trim the history to its budget, counting the tiles only History panel snapshots hold
    /// against it too (#4530). Returns how many states were dropped.
    pub fn trim_history(&mut self) -> usize {
        let snaps: Vec<&Document> = self.snapshots.iter().map(|s| &*s.doc).collect();
        self.history.trim_with(&self.inputs.doc, &snaps)
    }

    /// Pixel bytes of the document, the tiles only its history holds and the tiles only its
    /// History panel snapshots hold; embedded smart object contents count too.
    pub fn memory_bytes(&self) -> usize {
        let snaps: Vec<&Document> = self.snapshots.iter().map(|s| &*s.doc).collect();
        self.history.pixel_bytes(&self.inputs.doc).saturating_add(self.history.extra_bytes(&self.inputs.doc, &snaps))
    }

    pub fn new(doc: Document, path: Option<String>) -> Self {
        let active_layer = doc.top_layer();
        let mut history = History::default();
        let target = LayerTarget { active: active_layer, selected: active_layer.into_iter().collect() };
        history.set_current_layers(target.clone());
        // Spot channels open visible, printing over the composite; alpha channels open hidden.
        let channel_view = channel_cmds::ChannelView {
            alpha_visible: (0..doc.channels.len()).map(|i| photocraft_compose::multichannel::is_overprint_spot(&doc, i)).collect(),
            ..Default::default()
        };
        let doc = Arc::new(doc);
        // Photoshop's first snapshot: the document as opened, named after it, so the History
        // panel's top row can always bring it back (#2966).
        let first = history_cmds::Snapshot { id: 0, name: doc.name.clone(), doc: doc.clone(), layers: target };
        Self {
            inputs: DocStateInputs { doc, active_layer, selected_layers: active_layer.into_iter().collect() },
            selected_cache: SelectedOrder::default(),
            history,
            layer_anchor: active_layer,
            path,
            source_read_only: false,
            revision: 1,
            saved_revision: 1,
            view_revision: 0,
            last_damage: None,
            damage_span: None,
            damage_log: Default::default(),
            coalesce: None,
            channel_view,
            isolated_layers: Vec::new(),
            symmetry: None,
            fx_collapsed: Vec::new(),
            show_only: None,
            floating: None,
            color_samplers: Vec::new(),
            snapshots: vec![first],
            snapshots_generation: next_snapshots_generation(),
            next_snapshot: 1,
            save_snapshot: None,
            source_snapshot: None,
            typed_names: Default::default(),
            pixel_revision: 0,
        }
    }
    /// The History panel snapshots, oldest first.
    pub fn snapshots(&self) -> &[history_cmds::Snapshot] {
        &self.snapshots
    }
    /// The snapshots to change: [`Self::snapshots_generation`] moves.
    pub fn snapshots_mut(&mut self) -> &mut Vec<history_cmds::Snapshot> {
        self.snapshots_generation = next_snapshots_generation();
        &mut self.snapshots
    }
    /// A number that identifies the snapshot list: every [`Self::snapshots_mut`] takes a fresh
    /// one, clones keep it, and no two lists that differ share one. With
    /// [`photocraft_ops::History::generation`] it says when the History panel's rows change.
    pub fn snapshots_generation(&self) -> u64 {
        self.snapshots_generation
    }
    /// The current document.
    pub fn doc(&self) -> &Arc<Document> {
        &self.inputs.doc
    }
    /// Consume the state and return its document without cloning its Arc.
    pub fn into_document(self) -> Arc<Document> {
        self.inputs.doc
    }
    /// Mutable access invalidates the ordered selection, including on document replacement.
    pub fn doc_mut(&mut self) -> &mut Arc<Document> {
        self.selected_cache.0.take();
        &mut self.inputs.doc
    }
    /// The active layer, if any.
    pub fn active_layer(&self) -> Option<LayerId> {
        self.inputs.active_layer
    }
    /// Set the active layer and invalidate the held ordered selection.
    pub fn set_active_layer(&mut self, active: Option<LayerId>) {
        if self.inputs.active_layer != active {
            self.selected_cache.0.take();
            self.inputs.active_layer = active;
        }
    }
    /// The raw selection in selection order.
    pub fn selection(&self) -> &[LayerId] {
        &self.inputs.selected_layers
    }
    /// Mutate the raw selection, invalidating the held ordered selection before access.
    pub fn selection_mut(&mut self) -> &mut Vec<LayerId> {
        self.selected_cache.0.take();
        &mut self.inputs.selected_layers
    }
    /// Selected layers in bottom-to-top document order, including the active layer.
    /// Missing IDs are skipped; if the active layer is not in the raw selection, only it is used.
    /// The first read after a mutation builds the held slice; later reads reuse it.
    pub fn selected_layers(&self) -> &[LayerId] {
        self.selected_cache.0.get_or_init(|| selected_in(&self.inputs.doc, self.inputs.active_layer, &self.inputs.selected_layers))
    }
    pub fn is_layer_selected(&self, id: LayerId) -> bool {
        self.active_layer == Some(id) || (self.active_layer.is_some_and(|a| self.selected_layers.contains(&a)) && self.selected_layers.contains(&id))
    }
    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }
    /// Whether the step from `before` to the current document leaves a clean document clean:
    /// it changed nothing but the selection (#6865). Making, changing or dropping a selection
    /// (marquee, lasso, Magic Wand, Quick / Object Selection, Select › …) is a history step but
    /// not an edit, so it neither marks the file modified nor makes closing ask to save.
    pub(crate) fn keeps_clean(&self, before: &Document) -> bool {
        self.saved_revision == self.revision && crate::damage::selection_only_change(before, &self.doc)
    }
    /// Damage since `revision`, or `None` when a revision is missing or has unknown damage.
    pub fn damage_since(&self, revision: u64) -> Option<photocraft_geom::Rect> {
        if revision == self.revision {
            return Some(photocraft_geom::Rect::EMPTY);
        }
        let mut at = revision;
        let mut total = photocraft_geom::Rect::EMPTY;
        for &(from, to, damage) in &self.damage_log {
            if to <= at {
                continue;
            }
            if from > at || to > self.revision {
                return None;
            }
            let damage = damage?;
            total = if total.is_empty() {
                damage
            } else if damage.is_empty() {
                total
            } else {
                total.union(&damage)
            };
            at = to;
            if at == self.revision {
                return Some(total);
            }
        }
        None
    }

    fn record_damage_span(&mut self, from: u64, to: u64, damage: Option<photocraft_geom::Rect>) {
        while self.damage_log.back().is_some_and(|(_, end, _)| *end > from) {
            self.damage_log.pop_back();
        }
        const MAX_SPANS: usize = 64;
        if self.damage_log.len() == MAX_SPANS {
            self.damage_log.pop_front();
        }
        self.damage_log.push_back((from, to, damage));
    }

    /// Record the current revision conservatively until its damage has been determined.
    pub(crate) fn record_unknown_damage(&mut self) {
        self.last_damage = None;
        self.record_damage_span(self.revision.saturating_sub(1), self.revision, None);
    }

    /// Record the latest revision or command span after its damage has been determined.
    pub(crate) fn record_damage(&mut self) {
        // Keep the effect reach at the time of the edit: later pixel-neutral revisions can
        // change which effects are visible without changing this already-painted damage.
        // The existing growth walk is recursive, so refuse a bounded answer on hostile depth.
        fn bounded(layers: &[photocraft_doc::Layer], depth: usize) -> bool {
            depth < 256 && layers.iter().all(|layer| layer.children().is_none_or(|children| bounded(children, depth + 1)))
        }
        let damage = self.last_damage.and_then(|r| {
            if r.is_empty() {
                Some(r)
            } else if bounded(&self.doc.layers, 0) {
                Some(crate::damage::grow_hint(&self.doc, r))
            } else {
                None
            }
        });
        self.record_damage_span(self.damage_from(), self.revision, damage);
    }

    /// The revision [`DocState::last_damage`] is measured from: the revision before the command
    /// that made it, or the previous revision. A canvas that showed any revision from this one
    /// up to (not including) `revision` needs only `last_damage` redrawn.
    pub fn damage_from(&self) -> u64 {
        match self.damage_span {
            Some((from, to)) if to == self.revision => from,
            _ => self.revision.saturating_sub(1),
        }
    }
    /// Move to the next revision; `keep_clean` (from [`Self::keeps_clean`]) moves the saved
    /// revision with it.
    pub(crate) fn bump_revision(&mut self, keep_clean: bool) {
        self.revision += 1;
        self.record_unknown_damage();
        if keep_clean {
            self.saved_revision = self.revision;
        }
    }
    /// Move to the next revision for a change of the layer selection, which is not an edit:
    /// readers keyed on the revision follow it, a clean document stays clean, and no pixels need
    /// drawing again.
    pub(crate) fn bump_selection_revision(&mut self) {
        self.bump_revision(!self.is_dirty());
        self.last_damage = Some(photocraft_geom::Rect::EMPTY);
        self.record_damage();
    }
    /// Move to the next revision for a view-only change (channel eyes, a history state deleted,
    /// Layers panel isolation): readers keyed on the revision follow it, pixel caches ignore it
    /// through `view_revision`, and no pixels need drawing again. `keep_clean` as for
    /// [`Self::bump_revision`].
    pub(crate) fn bump_view_revision(&mut self, keep_clean: bool) {
        self.view_revision += 1;
        self.bump_revision(keep_clean);
        self.last_damage = Some(photocraft_geom::Rect::EMPTY);
        self.record_damage();
    }
    /// The targeted layers, as history stores them with each state.
    pub(crate) fn layer_target(&self) -> LayerTarget {
        LayerTarget { active: self.active_layer, selected: self.selected_layers.clone() }
    }
}

fn next_snapshots_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// App-wide tool state that commands read (foreground colour, brush…).
#[derive(Clone, Debug)]
pub struct ToolState {
    pub foreground: [f32; 4],
    pub background: [f32; 4],
    /// Tracked, so a view that shows the brush keys on its revision: every write through `&mut` moves it.
    pub brush: craft_tracked::Tracked<photocraft_paint::BrushSettings>,
    /// Brush presets (built-ins plus user presets; see `brush.presets.*`), tracked as the brush is.
    pub presets: craft_tracked::Tracked<Arc<Vec<photocraft_paint::BrushPreset>>>,
    /// Bumped whenever `presets` changes ([`Session::brush_presets_changed`]); the preset store
    /// syncs when it moves.
    pub presets_rev: u64,
    /// Mixer Brush paint carried between strokes.
    pub mixer: photocraft_paint::mixer::MixerState,
    /// The coalescing key of the running `tools.setBrush` gesture and the brush before it, so the
    /// gesture journals as one call ([`brush_cmds::coalesce_journal`]).
    pub brush_gesture: Option<(String, photocraft_paint::BrushSettings)>,
    /// Name of the preset the current brush was last picked from. Kept across later edits (the
    /// preset stays "the current brush" so `brush.presets.update` can overwrite it); cleared when
    /// the brush is reset or the preset is gone.
    pub current_preset: Option<String>,
    /// Bumped whenever `brush` or `current_preset` changes ([`ToolState::brush_changed`]). The
    /// last-brush persistence watches it instead of comparing the brush itself, so an unchanged
    /// brush costs a counter compare per frame, not a clone and serialise.
    pub brush_rev: u64,
    /// The revision whose value is in `Preferences::last_brush` (a restored one counts as saved):
    /// [`brush_rev`](Self::brush_rev) == `brush_saved_rev` means there is nothing new to write.
    pub brush_saved_rev: u64,
    /// A layer mask is the edit target ([`ToolState::target_mask`]).
    pub mask_targeted: bool,
    /// The foreground/background pair of the edit target that isn't current: the layer pixels'
    /// colours while a mask is targeted, the mask's while the pixels are (#2166).
    pub other_colors: [[f32; 4]; 2],
    /// Colours picked as CMYK, newest last: the sRGB colour that stands for them and the ink
    /// values chosen. The tool colours are stored as sRGB, and an sRGB round trip does not give
    /// back what was typed (100/0/0/0 cyan is outside sRGB), so a CMYK document is filled with
    /// the inks the person picked (#4196). Matched by colour, so it follows a swap, a reset or a
    /// mask pair without bookkeeping.
    pub cmyk_picks: Vec<([f32; 3], [f32; 4])>,
}

impl ToolState {
    /// How many CMYK picks are remembered.
    pub const CMYK_PICKS: usize = 8;

    /// Remember that sRGB colour `rgb` was picked as the inks `cmyk` (each 0..1).
    pub fn remember_cmyk(&mut self, rgb: [f32; 3], cmyk: [f32; 4]) {
        self.cmyk_picks.retain(|(c, _)| *c != rgb);
        self.cmyk_picks.push((rgb, cmyk));
        let extra = self.cmyk_picks.len().saturating_sub(Self::CMYK_PICKS);
        self.cmyk_picks.drain(..extra);
    }

    /// The inks `rgb` was picked as, if it was picked as CMYK.
    pub fn picked_cmyk(&self, rgb: [f32; 3]) -> Option<[f32; 4]> {
        self.cmyk_picks.iter().rev().find(|(c, _)| *c == rgb).map(|(_, k)| *k)
    }

    /// Photoshop's colour pair for a layer mask that has none yet: white foreground, black
    /// background (reveal-paint first).
    pub const MASK_COLORS: [[f32; 4]; 2] = [[1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 1.0]];

    /// The pixel colour pair a fresh session starts with: black foreground, white background.
    pub const PIXEL_COLORS: [[f32; 4]; 2] = [[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]];

    /// The swatches as `Preferences::tool_colors` keeps them: the pixel pair, the mask pair and the CMYK picks.
    pub fn saved_colors(pairs: [[[f32; 4]; 2]; 2], cmyk_picks: &[([f32; 3], [f32; 4])]) -> serde_json::Value {
        let [[foreground, background], [mask_foreground, mask_background]] = pairs;
        serde_json::json!({
            "foreground": foreground, "background": background,
            "maskForeground": mask_foreground, "maskBackground": mask_background,
            "cmykPicks": cmyk_picks,
        })
    }

    /// Note that `brush` or `current_preset` changed. Every setter of either (commands and the
    /// shell's tool-switch paths) calls this; the last-brush persistence only serialises when the
    /// revision has moved since the stored value ([`Self::brush_saved_rev`]).
    pub fn brush_changed(&mut self) {
        self.brush_rev = self.brush_rev.wrapping_add(1);
    }

    /// Switch the edit target between layer pixels and a layer mask. Like Photoshop, the mask and
    /// the pixels keep separate app-wide foreground/background pairs: changing the target puts
    /// the current pair aside and brings back the other one, so colours picked while a mask is
    /// targeted are remembered for masks (#2166). The mask pair starts at [`Self::MASK_COLORS`].
    /// Returns whether the target changed.
    pub fn target_mask(&mut self, mask: bool) -> bool {
        if self.mask_targeted == mask {
            return false;
        }
        self.mask_targeted = mask;
        let [fg, bg] = std::mem::replace(&mut self.other_colors, [self.foreground, self.background]);
        self.foreground = fg;
        self.background = bg;
        true
    }
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            foreground: Self::PIXEL_COLORS[0],
            background: Self::PIXEL_COLORS[1],
            // Photoshop's Brush tool starts at 10 % Smoothing.
            brush: craft_tracked::Tracked::new(photocraft_paint::BrushSettings {
                smoothing: photocraft_paint::brush::Smoothing { amount: 0.1, ..Default::default() },
                ..Default::default()
            }),
            presets: craft_tracked::Tracked::new(photocraft_paint::presets::builtin_shared()),
            presets_rev: 0,
            mixer: Default::default(),
            brush_gesture: None,
            current_preset: None,
            brush_rev: 0,
            brush_saved_rev: 0,
            mask_targeted: false,
            other_colors: Self::MASK_COLORS,
            cmyk_picks: Vec::new(),
        }
    }
}

/// Layer › Layer Style › Copy Layer Style: the effects and the Blending Options, as Photoshop
/// copies them (Paste Layer Style puts them all on each target).
#[derive(Clone, Debug, craft_ui_macros::PlainData)]
pub struct CopiedStyle {
    pub effects: photocraft_doc::Effects,
    pub blend: photocraft_color::BlendMode,
    pub fill: f32,
    pub advanced: photocraft_doc::AdvancedBlending,
    pub blend_if: photocraft_doc::BlendIf,
    pub excluded_channels: u32,
    /// The colour mode it was copied in: Blend If's channel ranges and the channel restrictions
    /// only carry over to a document in the same mode.
    pub mode: photocraft_color::ColorMode,
}

/// The commands whose [`Session::precheck`] is complete: it accepts what `execute` does (Free
/// Transform applies its box before these, and only when they will run), but for the group
/// nesting limit ([`photocraft_doc::MAX_GROUP_DEPTH`]): grouping layers already that deep is
/// refused only as the edit is made. Layer via Copy and
/// Cut aren't: whether they have anything to copy (a selection over transparent pixels, a group
/// or fill layer) is known only when they run.
pub const PRECHECKED: &[&str] = &["layer.new.layer", "layer.new.group", "layer.new.groupFromLayers", "layer.new.layerFromBackground"];

/// The selected layers of `doc` whose active layer is `active` and whose selection set is `selected`, in bottom-to-top
/// order ([`DocState::selected_layers`]), taken from the document's layer index rather than a walk, for a reader that holds the document without its editing state.
pub fn selected_in(doc: &Document, active: Option<LayerId>, selected: &[LayerId]) -> Vec<LayerId> {
    let Some(active) = active else { return Vec::new() };
    let set: &[LayerId] = if selected.contains(&active) { selected } else { std::slice::from_ref(&active) };
    // Paths from the document's layer index order layers as the walk does (depth-first, bottom-to-top).
    let index = doc.layers.index();
    let mut ids: Vec<LayerId> = set.iter().copied().filter(|id| index.path(*id).is_some()).collect();
    ids.sort_unstable_by(|a, b| index.path(*a).cmp(&index.path(*b)));
    ids.dedup();
    ids
}

#[cfg(test)]
mod selected_tests {
    use super::selected_in;
    use photocraft_doc::{ColorMode, Document, Layer, LayerId, PixelFormat, SampleType, Size};

    fn raster(name: &str) -> Layer {
        Layer::raster(name, PixelFormat::RGBA8)
    }

    /// The answer before the layer index: filter the depth-first walk by the selection set.
    fn by_walk(doc: &Document, active: Option<LayerId>, selected: &[LayerId]) -> Vec<LayerId> {
        let Some(active) = active else { return Vec::new() };
        let set: &[LayerId] = if selected.contains(&active) { selected } else { std::slice::from_ref(&active) };
        doc.walk().into_iter().map(|(_, _, l)| l.id).filter(|id| set.contains(id)).collect()
    }

    /// `a`, group `g` (`b`, group `h` (`c`), `d`), `e`: nested groups mixed with root layers.
    fn mixed() -> (Document, Vec<LayerId>) {
        let mut doc = Document::new("t", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
        let (a, b, c, d, e) = (raster("a"), raster("b"), raster("c"), raster("d"), raster("e"));
        let h = Layer::group("h", vec![c.clone()]);
        let g = Layer::group("g", vec![b.clone(), h.clone(), d.clone()]);
        let ids = [&a, &g, &b, &h, &c, &d, &e].map(|l| l.id).to_vec();
        doc.layers = vec![a, g, e].into();
        (doc, ids)
    }

    #[test]
    fn held_selection_tracks_mutations_and_reuses_storage() {
        use super::DocState;
        use std::sync::Arc;
        let (doc, ids) = mixed();
        let mut st = DocState::new(doc, None);
        st.set_active_layer(Some(ids[0]));
        *st.selection_mut() = vec![ids[6], ids[0], ids[6], LayerId::fresh()];
        assert_eq!(st.selected_layers(), &[ids[0], ids[6]]);
        let ptr = st.selected_layers().as_ptr();
        let serial = st.doc.layers.index().serial();
        st.set_active_layer(Some(ids[0]));
        assert_eq!(st.selected_layers().as_ptr(), ptr, "setting the same active layer keeps the cache");
        for _ in 0..100 {
            assert_eq!(st.selected_layers().as_ptr(), ptr);
            assert_eq!(st.doc.layers.index().serial(), serial);
        }
        Arc::make_mut(st.doc_mut()).shift(ids[6], -2);
        assert_eq!(st.selected_layers(), &[ids[6], ids[0]]);
        Arc::make_mut(st.doc_mut()).remove(ids[6]);
        assert_eq!(st.selected_layers(), &[ids[0]]);
        st.set_active_layer(Some(ids[4]));
        assert_eq!(st.selected_layers(), &[ids[4]]);
        st.set_active_layer(None);
        assert!(st.selected_layers().is_empty());
        let (replacement, other) = mixed();
        *st.doc_mut() = Arc::new(replacement);
        st.set_active_layer(Some(other[6]));
        *st.selection_mut() = vec![other[6], other[0]];
        assert_eq!(st.selected_layers(), &[other[0], other[6]]);
    }

    #[test]
    fn selection_order_matches_the_walk_through_edits() {
        let (mut doc, ids) = mixed();
        let stale = LayerId::fresh();
        let sets: Vec<Vec<LayerId>> =
            vec![vec![ids[6], ids[4], ids[0]], vec![ids[5], ids[2], ids[1], ids[3]], vec![ids[4], ids[4], ids[0], stale], vec![ids[0]], vec![]];
        let check = |doc: &Document| {
            let first = doc.layers.index().serial();
            for set in &sets {
                for active in [None, Some(ids[0]), Some(ids[4]), Some(ids[6]), Some(stale)] {
                    assert_eq!(selected_in(doc, active, set), by_walk(doc, active, set));
                }
            }
            assert_eq!(doc.layers.index().serial(), first, "no rebuild while the document is unchanged");
        };
        check(&doc);
        // Reorder: move `e` to the bottom, then a layer inside a group.
        doc.shift(ids[6], -2);
        check(&doc);
        doc.shift(ids[5], -1);
        check(&doc);
        // Add and delete.
        doc.insert_above(Some(ids[4]), raster("new"));
        check(&doc);
        doc.remove(ids[3]);
        check(&doc);
        assert_eq!(selected_in(&doc, Some(ids[0]), &[ids[0], ids[3], ids[4]]), vec![ids[0]], "deleted layers are skipped");
    }
}

#[cfg(test)]
mod damage_log_tests {
    use super::DocState;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::Document;
    use photocraft_geom::{Rect, Size};

    #[test]
    fn revision_helpers_keep_empty_damage_and_signal_view_changes() {
        let doc = Document::new("test", Size::new(40, 40), ColorMode::Rgb, SampleType::U8);
        let mut st = DocState::new(doc, None);
        let before = st.revision;
        let view = st.view_revision;
        st.bump_selection_revision();
        st.bump_view_revision(true);
        assert_eq!(st.damage_since(before), Some(Rect::EMPTY));
        assert_eq!(st.view_revision, view + 1);
        assert!(!st.is_dirty());
        // A channel view changes the displayed pixels, even though it is view-only.
        st.last_damage = None;
        st.record_damage();
        assert_eq!(st.damage_since(before), None);
    }

    #[test]
    fn damage_keeps_the_effect_reach_that_existed_at_the_edit() {
        let mut doc = Document::new("test", Size::new(40, 40), ColorMode::Rgb, SampleType::U8);
        let mut layer = photocraft_doc::Layer::raster("empty", photocraft_color::PixelFormat::RGBA8);
        layer.effects.items.push(photocraft_doc::Effect::default_drop_shadow());
        doc.layers = vec![layer].into();
        let mut st = DocState::new(doc, None);
        let before = st.revision;
        let ink = Rect::new(10, 10, 12, 12);
        st.bump_revision(false);
        st.last_damage = Some(ink);
        st.record_damage();
        let with_effect = st.damage_since(before).unwrap();
        assert!(with_effect.width() > ink.width());
        // An empty layer shows no pixels, so removing its effect is pixel-neutral.
        std::sync::Arc::make_mut(st.doc_mut()).layers[0].effects.items.clear();
        st.bump_revision(false);
        st.last_damage = Some(Rect::EMPTY);
        st.record_damage();
        assert_eq!(st.damage_since(before), Some(with_effect));
    }

    #[test]
    fn deeply_nested_effect_growth_falls_back_to_full_damage() {
        let mut doc = Document::new("test", Size::new(20, 20), ColorMode::Rgb, SampleType::U8);
        let mut group = photocraft_doc::Layer::group("leaf", Vec::new());
        for _ in 0..256 {
            group = photocraft_doc::Layer::group("group", vec![group]);
        }
        doc.layers = vec![group].into();
        let mut st = DocState::new(doc, None);
        let before = st.revision;
        st.bump_revision(false);
        st.last_damage = Some(Rect::new(1, 1, 2, 2));
        st.record_damage();
        assert_eq!(st.damage_since(before), None);
    }

    #[test]
    fn damage_spans_union_and_unknown_or_evicted_spans_force_a_full_redraw() {
        let mut st = DocState::new(Document::new("test", Size::new(20, 20), ColorMode::Rgb, SampleType::U8), None);
        let start = st.revision;
        let first = Rect::new(1, 2, 4, 5);
        let second = Rect::new(8, 9, 11, 12);
        st.bump_revision(false);
        assert_eq!(st.damage_since(start), None);
        st.last_damage = Some(first);
        st.record_damage();
        st.bump_revision(false);
        st.last_damage = Some(Rect::EMPTY);
        st.record_damage();
        st.bump_revision(false);
        st.last_damage = Some(second);
        st.record_damage();
        let combined = st.damage_since(start).unwrap();
        assert_eq!(combined.intersect(&first), first);
        assert_eq!(combined.intersect(&second), second);
        st.bump_revision(false);
        assert_eq!(st.damage_since(start), None);
        for _ in 0..65 {
            st.bump_revision(false);
            st.last_damage = Some(Rect::EMPTY);
            st.record_damage();
        }
        assert_eq!(st.damage_since(start), None);
        let current = st.revision;
        st.revision += 1; // A caller bypassed the recording API.
        st.bump_revision(false);
        st.last_damage = Some(Rect::EMPTY);
        st.record_damage();
        assert_eq!(st.damage_since(current), None);
    }
}
