//! Placement markers for the active layer's type path: the app side of the framework's
//! `PathTextMarkers` (the track the markers sit on, the glyph body, the proposed range shown live, and
//! the one command that commits a placement).
use crate::{PhotocraftApp, state::Tool};
use std::{
    collections::VecDeque,
    sync::{Arc, Weak},
};

use craft::behavior::EditEvent;
use craft::behavior::kurbo::{Point as Pt, Vec2 as Dir};
use craft::text::ui::editor::TextEditorHost as _;
use craft::text::ui::path_text::{PathRange, PathTextMarkers, PathTrack};
use craft::ui::contract::{Bind, Show as _};
use photocraft_doc::{Affine, DocId, Document, LayerContent, LayerId, TextLayer, text::TextShape};
use photocraft_geom::{Point, Rect};
use photocraft_text::warp::PathGeometry;

/// Returns the active text layer when a path selection tool is active and targeted at the type path.
pub(crate) fn active_text(app: &PhotocraftApp) -> Option<&TextLayer> {
    if !matches!(app.ui.tool, Tool::PathSelection | Tool::DirectSelection)
        || app.ui.transform.is_some()
        || app.ui.vector_mask_target
        || app.ui.selected_path.as_deref().is_some_and(|name| name != "layer")
    {
        return None;
    }
    let st = app.session.active()?;
    if st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::VectorMask {
        return None;
    }
    let id = st.active_layer?;
    if !st.doc.shown_at(&st.doc.path_of(id)?) {
        return None;
    }
    let layer = st.doc.layer(id)?;
    match &layer.content {
        LayerContent::Text(text) if matches!(text.shape, TextShape::Path { .. }) => Some(text),
        _ => None,
    }
}

/// Returns the normalized paragraph alignment weight (0.0 for left, 0.5 for center, 1.0 for right).
fn alignment_factor(align: photocraft_doc::text::TextAlign) -> f64 {
    use photocraft_doc::text::TextAlign;
    match align {
        TextAlign::Center | TextAlign::JustifyCenter => 0.5,
        TextAlign::Right | TextAlign::JustifyRight => 1.0,
        _ => 0.0,
    }
}

/// Computes the start and end arc-length interval of path text, converting legacy alignment anchors to explicit spans.
fn effective_range(text: &TextLayer, geometry: &PathGeometry) -> Option<[f64; 2]> {
    let TextShape::Path { path, start, end, .. } = &text.shape else { return None };
    let length = geometry.length();
    let mut start = f64::from(*start);
    if !length.is_finite() || length <= 0.0 || !start.is_finite() {
        return None;
    }
    let range = if let Some(end) = end {
        let mut end = f64::from(*end);
        if !end.is_finite() || end < start {
            return None;
        }
        if !path.closed {
            if start < 0.0 {
                return None;
            }
            start = start.min(length);
            end = end.min(length);
        }
        [start, end]
    } else {
        // Different paragraph alignments cannot share one equivalent interval.
        let runs = text.paragraph_runs();
        let f = alignment_factor(runs.first()?.style.align);
        if runs.iter().any(|run| alignment_factor(run.style.align) != f) {
            return None;
        }
        if path.closed {
            [start - f * length, start + (1.0 - f) * length]
        } else if start < 0.0 {
            return None;
        } else {
            let start = start.min(length);
            if f == 0.0 {
                [start, length]
            } else if f == 1.0 {
                [0.0, start]
            } else if start <= length / 2.0 {
                [0.0, 2.0 * start]
            } else {
                [2.0 * start - length, length]
            }
        }
    };
    if range.iter().any(|n| !n.is_finite()) || range[1] < range[0] || (!path.closed && (range[0] < 0.0 || range[1] > length)) {
        return None;
    }
    Some(range)
}

/// Single-entry cache for flattened type path geometry keyed by document snapshot and shape identity.
#[derive(Clone, Debug)]
pub struct CachedGeometry {
    pub(crate) doc: Weak<Document>,
    pub(crate) layer: photocraft_doc::LayerId,
    pub(crate) path: photocraft_doc::vector::Subpath,
    pub(crate) flip: bool,
    pub(crate) geometry: Arc<PathGeometry>,
    pub(crate) reversed: Arc<PathGeometry>,
}

impl PartialEq for CachedGeometry {
    fn eq(&self, other: &Self) -> bool {
        self.doc.ptr_eq(&other.doc) && self.layer == other.layer && self.flip == other.flip && self.path == other.path
    }
}

/// Bounded single-entry geometry cache wrapper implementing equality and interior mutability.
#[derive(Clone, Debug, Default)]
pub struct GeometryCache(pub(crate) Arc<std::sync::Mutex<Option<CachedGeometry>>>);

impl PartialEq for GeometryCache {
    fn eq(&self, _other: &Self) -> bool {
        // Cached geometry is transient and does not change logical UI state.
        true
    }
}

impl Eq for GeometryCache {}

/// The text's path geometry as its `flip` reads it, and the same path read the other way, from the
/// single-entry cache.
fn cached_geometries(app: &PhotocraftApp, text: &TextLayer) -> Option<(Arc<PathGeometry>, Arc<PathGeometry>)> {
    let TextShape::Path { path, flip, .. } = &text.shape else { return None };
    let st = app.session.active()?;
    let layer = st.active_layer?;
    let mut guard = app.ui.type_path_geometry_cache.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(cached) = guard.as_ref()
        && cached.doc.ptr_eq(&Arc::downgrade(&st.doc))
        && cached.layer == layer
        && cached.path == *path
    {
        return Some(if cached.flip == *flip {
            (Arc::clone(&cached.geometry), Arc::clone(&cached.reversed))
        } else {
            (Arc::clone(&cached.reversed), Arc::clone(&cached.geometry))
        });
    }
    let geometry = Arc::new(PathGeometry::new(path, *flip)?);
    let reversed = Arc::new(geometry.flipped());
    *guard = Some(CachedGeometry {
        doc: Arc::downgrade(&st.doc),
        layer,
        path: path.clone(),
        flip: *flip,
        geometry: Arc::clone(&geometry),
        reversed: Arc::clone(&reversed),
    });
    Some((geometry, reversed))
}

/// Retrieves or creates cached path geometry for the text's own direction.
pub(crate) fn active_path_geometry(app: &PhotocraftApp, text: &TextLayer) -> Option<Arc<PathGeometry>> {
    cached_geometries(app, text).map(|(geometry, _)| geometry)
}

/// The path the markers sit on, in document units. It runs in the path's own direction whatever the
/// text's `flip` is: the markers map a flipped range onto the reversed path themselves.
struct Track {
    geometry: Arc<PathGeometry>,
    transform: Affine,
    inverse: Affine,
    closed: bool,
}

impl Track {
    /// The track of `text`'s path; `None` for a text that is not on a path, a degenerate path or a
    /// singular text transform.
    fn new(app: &PhotocraftApp, text: &TextLayer) -> Option<Self> {
        let TextShape::Path { path, flip, .. } = &text.shape else { return None };
        let (own, other) = cached_geometries(app, text)?;
        Some(Track { geometry: if *flip { other } else { own }, transform: text.transform, inverse: text.transform.inverse()?, closed: path.closed })
    }
}

impl PathTrack for Track {
    fn length(&self) -> f64 {
        self.geometry.length()
    }

    fn closed(&self) -> bool {
        self.closed
    }

    fn at(&self, s: f64) -> Option<(Pt, Dir)> {
        let ((x, y), (tx, ty)) = self.geometry.sample(s)?;
        let a = self.transform.apply(Point::new(x, y));
        let b = self.transform.apply(Point::new(x + tx, y + ty));
        let direction = Dir::new(b.x - a.x, b.y - a.y);
        let length = direction.hypot();
        (length.is_finite() && length > 1e-9).then(|| (Pt::new(a.x, a.y), direction / length))
    }

    fn project(&self, p: Pt, near: Option<f64>) -> Option<f64> {
        let q = self.inverse.apply(Point::new(p.x, p.y));
        let hit = match near {
            Some(near) => self.geometry.project_near(q.x, q.y, near),
            None => self.geometry.project(q.x, q.y),
        };
        hit.map(|(arc, _)| arc)
    }
}

/// Checks whether text glyphs exceed the current path range, caching by document snapshot, revision, and placement.
fn overflows(app: &PhotocraftApp, ctx: &egui::Context, text: &TextLayer) -> bool {
    let Some(st) = app.session.active() else { return false };
    let TextShape::Path { start, end, flip, .. } = &text.shape else { return false };
    let key = (st.revision, start.to_bits(), end.map(f32::to_bits), *flip);
    let source = Arc::downgrade(&st.doc);
    let id = egui::Id::unique(("path-text-overflow", st.doc.id.0, st.active_layer));
    if let Some((_, _, value)) = ctx
        .data(|data| data.get_temp::<((u64, u32, Option<u32>, bool), Weak<Document>, bool)>(id))
        .filter(|(cached, doc, _)| *cached == key && doc.ptr_eq(&source))
    {
        return value;
    }
    let mut engine = photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let layout = engine.layout(text, st.doc.resolution_dpi);
    let value = photocraft_text::render::path_overflows(&layout, &text.shape);
    ctx.data_mut(|data| data.insert_temp(id, (key, source, value)));
    value
}

/// Whether the layer takes no placement: a lock on its position or on everything.
fn locked(app: &PhotocraftApp, layer: LayerId) -> bool {
    app.session.active().is_some_and(|st| {
        let locks = st.doc.effective_locks(layer);
        locks.all || locks.position
    })
}

/// The layer's glyph body: one quad per glyph shown on the path, in document units. These are the
/// quads the type tool highlights for a selection of the whole text, so glyphs past the path's end
/// are not part of the body.
fn body_quads(app: &mut PhotocraftApp, layer: LayerId) -> Vec<[Pt; 4]> {
    let mut host = crate::type_tool::LayerHost::new(app, layer);
    let chars = host.text().chars().count();
    host.selection_quads(0..chars)
}

/// What `show` hands the markers.
struct Markers {
    layer: LayerId,
    track: Track,
    shown: PathRange,
    anchor: f64,
    overflow: bool,
    body: Vec<[Pt; 4]>,
    locked: bool,
}

impl Markers {
    /// The markers' inputs for the active layer, or `None` when no markers show: no path text under a
    /// path tool, Target Path hidden, or a path with no usable range.
    fn of(app: &mut PhotocraftApp, ctx: &egui::Context) -> Option<Self> {
        if !app.ui.view.shows(app.ui.view.show.target_path) {
            return None;
        }
        let text = active_text(app)?;
        let layer = app.session.active()?.active_layer?;
        let track = Track::new(app, text)?;
        let TextShape::Path { flip, .. } = &text.shape else { return None };
        let original = effective_range(text, &track.geometry)?;
        let shown = valid_proposal(app).map_or(PathRange::new(original[0], original[1], *flip), |p| PathRange::new(p.range[0], p.range[1], p.flip));
        // The text is measured with the range it is shown with, so the plus follows the drag.
        let mut preview = text.clone();
        if let TextShape::Path { start, end, flip, .. } = &mut preview.shape {
            *start = shown.start as f32;
            *end = Some(shown.end as f32);
            *flip = shown.flip;
        }
        let anchor = text.paragraph_runs().first().map_or(0.0, |run| alignment_factor(run.style.align));
        let overflow = overflows(app, ctx, &preview);
        let locked = locked(app, layer);
        Some(Markers { layer, shown, anchor, overflow, body: body_quads(app, layer), locked, track })
    }
}

/// Shows the placement markers over the canvas body `rect`, after the document display and after the
/// other path controls, so a marker on an anchor takes the press. Nothing shows (and the proposal is
/// dropped) unless a path selection tool targets a visible path text with Target Path shown.
///
/// The control's updates become this pass's proposal and its commit places the text; a second pass of
/// a frame applies only its own output, and the control proposes the same range again.
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: egui::Rect) {
    let Some(markers) = Markers::of(app, ui.ctx()) else {
        propose(app, None);
        return;
    };
    let out = PathTextMarkers::new(tl!("Text on path").to_owned(), Bind::given(markers.shown), &markers.track)
        .anchor(markers.anchor)
        .overflow(markers.overflow)
        .body(&markers.body)
        .locked(markers.locked)
        .show_at(ui, rect);
    for event in out.range.events.iter() {
        match event {
            EditEvent::Update { value, .. } => propose(app, Some(*value)),
            EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => {
                place(app, markers.layer, *after);
                propose(app, None);
            }
            EditEvent::Cancel { .. } => propose(app, None),
            _ => {}
        }
    }
}

/// The range a drag of the markers proposes this pass, shown on the canvas without touching the
/// document. A tool switch drops it (`tools::tool_changed`).
#[derive(Clone, Debug)]
pub(crate) struct Preview {
    document: DocId,
    source: Weak<Document>,
    revision: u64,
    layer: LayerId,
    range: [f64; 2],
    flip: bool,
    /// The document with the range applied and its display key, built on first use.
    built: Option<(Arc<Document>, u64)>,
}

/// Writes this pass's proposed placement of the active layer (`None`: no proposal).
pub(crate) fn propose(app: &mut PhotocraftApp, range: Option<PathRange>) {
    let Some(range) = range else {
        app.type_path_preview = None;
        return;
    };
    let Some((document, source, revision, layer)) =
        app.session.active().and_then(|st| Some((st.doc.id, Arc::downgrade(&st.doc), st.revision, st.active_layer?)))
    else {
        app.type_path_preview = None;
        return;
    };
    // The same proposal keeps its built document, so a repeated pass shows the same picture.
    let same = app.type_path_preview.as_ref().is_some_and(|p| {
        p.document == document
            && p.source.ptr_eq(&source)
            && p.revision == revision
            && p.layer == layer
            && p.range == [range.start, range.end]
            && p.flip == range.flip
    });
    if !same {
        app.type_path_preview = Some(Preview { document, source, revision, layer, range: [range.start, range.end], flip: range.flip, built: None });
    }
}

/// The proposal, while it still belongs to the active document snapshot, layer and revision.
fn valid_proposal(app: &PhotocraftApp) -> Option<&Preview> {
    let p = app.type_path_preview.as_ref()?;
    let st = app.session.active()?;
    let valid = active_text(app).is_some()
        && st.doc.id == p.document
        && p.source.ptr_eq(&Arc::downgrade(&st.doc))
        && st.active_layer == Some(p.layer)
        && st.revision == p.revision;
    valid.then_some(p)
}

/// Commits the placement of `layer`'s text as one History step. Nothing happens for a range the layer
/// already has (a click that moved nothing), for a locked or hidden layer, or when the layer is not the
/// path text the markers show.
pub(crate) fn place(app: &mut PhotocraftApp, layer: LayerId, range: PathRange) {
    if app.session.active().and_then(|st| st.active_layer) != Some(layer) || locked(app, layer) {
        return;
    }
    let Some(text) = active_text(app) else { return };
    let TextShape::Path { path, flip, .. } = &text.shape else { return };
    let Some(geometry) = active_path_geometry(app, text) else { return };
    if effective_range(text, &geometry) == Some([range.start, range.end]) && *flip == range.flip {
        return;
    }
    if !(range.start.is_finite() && range.end.is_finite()) {
        return;
    }
    let (mut start, mut end) = (range.start, range.end);
    if !path.closed {
        end = end.min(f64::from(geometry.canonical_length_f32())).max(0.0);
        start = start.clamp(0.0, end);
    }
    if let Err(error) = app.run("type.setPathPlacement", serde_json::json!({ "layer": layer.0, "start": start, "end": end, "flip": range.flip })) {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

pub(crate) struct Damage {
    document: photocraft_doc::DocId,
    revision: u64,
    original_area: Rect,
    areas: VecDeque<(u64, Rect)>,
}

/// Calculates canvas invalidation damage bounding the union of preview text regions across display revisions.
pub(crate) fn damage(app: &PhotocraftApp, doc: photocraft_doc::DocId, revision: u64, before: u64, after: u64) -> Option<Rect> {
    let p = app.type_path_damage.as_ref().filter(|p| p.document == doc && p.revision == revision)?;
    let area = |key| if key == 0 { Some(p.original_area) } else { p.areas.iter().find(|(k, _)| *k == key).map(|(_, r)| *r) };
    Some(area(before)?.union(&area(after)?))
}

/// Returns a transient document snapshot with live placement preview modifications and tracks damage bounding boxes.
/// `None` without a proposal of this pass for document `idx`, or for one that equals the layer's placement.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let p = valid_proposal(app)?;
    let st = app.session.documents().get(idx)?;
    if st.doc.id != p.document {
        return None;
    }
    let text = active_text(app)?;
    let geometry = active_path_geometry(app, text)?;
    let TextShape::Path { flip: own_flip, .. } = &text.shape else { return None };
    if effective_range(text, &geometry) == Some(p.range) && *own_flip == p.flip {
        return None;
    }
    if let Some(built) = &p.built {
        return Some(built.clone());
    }
    let (layer, range, flip, revision, source) = (p.layer, p.range, p.flip, st.revision, st.doc.clone());
    let original_area = source.layer(layer)?.surface().map_or(Rect::EMPTY, |s| s.tile_bounds());
    let mut shown = (*source).clone();
    let LayerContent::Text(text) = &mut shown.layer_mut(layer)?.content else { return None };
    let TextShape::Path { start, end, flip: side, .. } = &mut text.shape else { return None };
    *start = range[0] as f32;
    *end = Some(range[1] as f32);
    *side = flip;
    photocraft_engine::type_cmds::refresh(&source, text);
    let area = text.cache.as_ref().map_or(Rect::EMPTY, |s| s.tile_bounds());
    let shown = Arc::new(photocraft_engine::mode_cmds::display_document(&shown).unwrap_or(shown));
    const BASE: u64 = 1 << 50;
    let key = BASE | (app.ui.alloc_id() & (BASE - 1));
    let mut damage = app.type_path_damage.take().filter(|p| p.document == source.id && p.revision == revision).unwrap_or_else(|| Damage {
        document: source.id,
        revision,
        original_area,
        areas: VecDeque::new(),
    });
    damage.original_area = damage.original_area.union(&original_area);
    if damage.areas.len() >= 128 {
        damage.areas.pop_front();
    }
    damage.areas.push_back((key, area));
    app.type_path_damage = Some(damage);
    app.type_path_preview.as_mut()?.built = Some((shown.clone(), key));
    Some((shown, key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Sets up a test harness with a document, path, and attached type layer for placement tests.
    fn app(tool: Tool) -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.sync_views();
        app.ui.tool = tool;
        app.run("path.set", json!({"name":"work", "path":{"subpaths":[{"closed":false,"knots":[[20,100],[180,100]]}]}})).unwrap();
        app.ui.selected_path = Some("work".into());
        app.run("type.create", json!({"text":"ABC", "x":20, "y":100})).unwrap();
        app.run("type.attachToPath", json!({"start":20})).unwrap();
        app.run("type.setPathPlacement", json!({"end":100})).unwrap();
        app.ui.views[0].zoom = 1.0;
        app
    }

    /// The active layer.
    fn layer(app: &PhotocraftApp) -> LayerId {
        app.session.active().unwrap().active_layer.unwrap()
    }

    /// A range that sits on the layer's own side of the path.
    fn range(start: f64, end: f64) -> PathRange {
        PathRange::new(start, end, false)
    }

    /// Returns the text shape of the active text layer in test assertions.
    fn shape(app: &PhotocraftApp) -> TextShape {
        active_text(app).unwrap().shape.clone()
    }

    /// Returns the (start, end, flip) placement parameters of the active path text layer.
    fn placement(app: &PhotocraftApp) -> (f32, Option<f32>, bool) {
        let TextShape::Path { start, end, flip, .. } = shape(app) else { panic!("expected path") };
        (start, end, flip)
    }

    /// Returns the count of committed history entries in the active session.
    fn steps(app: &PhotocraftApp) -> usize {
        app.session.active().unwrap().history.entries().len()
    }

    /// Ensures attaching type to a work path targets the layer's path for editing without mutating the source work path.
    #[test]
    fn attaching_type_selects_its_path_and_preserves_the_source() {
        let mut app = app(Tool::PathSelection);
        assert_eq!(app.ui.selected_path.as_deref(), Some("layer"));
        assert!(active_text(&app).is_some());
        let source = app.session.active().unwrap().doc.work_path.clone();
        app.ui.selected_path = Some("work".into());
        app.session.active_mut().unwrap().channel_view.target = photocraft_engine::channel_cmds::ChannelTarget::VectorMask;
        assert!(active_text(&app).is_none());
        app.run("type.attachToPath", json!({"start":20})).unwrap();
        assert_eq!(app.session.active().unwrap().channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::Composite);
        assert_eq!(app.ui.selected_path.as_deref(), Some("layer"));
        assert!(active_text(&app).is_some());
        let id = layer(&app);
        place(&mut app, id, range(30.0, 110.0));
        assert_eq!(placement(&app), (30.0, Some(110.0), false));
        assert_eq!(app.session.active().unwrap().doc.work_path, source);
    }

    /// Verifies converting legacy single-point anchor alignment to explicit ranges preserves exact glyph rasterization coordinates.
    #[test]
    fn legacy_alignment_conversion_preserves_glyph_positions() {
        use photocraft_doc::text::{ParagraphStyle, TextAlign};
        use photocraft_doc::vector::Subpath;
        use photocraft_text::warp::Warp;
        let mut engine = photocraft_text::TextEngine::new();
        for closed in [false, true] {
            for flip in [false, true] {
                for align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
                    let path = if closed {
                        Subpath::polygon(&[(0.0, 0.0), (200.0, 0.0), (200.0, 200.0), (0.0, 200.0)])
                    } else {
                        Subpath::polyline(&[(0.0, 0.0), (500.0, 0.0)])
                    };
                    let mut text = TextLayer {
                        text: "ABC".into(),
                        paragraphs: vec![photocraft_doc::text::ParagraphRun { len: 3, style: ParagraphStyle { align, ..Default::default() } }],
                        ..Default::default()
                    };
                    text.shape = TextShape::Path { path: path.clone(), start: 200.0, end: None, flip };
                    let old = engine.layout(&text, 72.0);
                    let old_warp = Warp::on_path_with_text_start(&text.shape, f64::from(old.lines[0].x0)).unwrap();
                    let geometry = PathGeometry::new(&path, flip).unwrap();
                    let range = effective_range(&text, &geometry).unwrap();
                    text.shape = TextShape::Path { path, start: range[0] as f32, end: Some(range[1] as f32), flip };
                    let new = engine.layout(&text, 72.0);
                    let new_warp = Warp::on_path_with_text_start(&text.shape, f64::from(new.lines[0].x0)).unwrap();
                    assert_eq!(old.glyphs.len(), new.glyphs.len());
                    for (old, new) in old.glyphs.iter().zip(&new.glyphs) {
                        let a = old_warp.apply(f64::from(old.x), f64::from(old.y));
                        let b = new_warp.apply(f64::from(new.x), f64::from(new.y));
                        assert!((a.0 - b.0).hypot(a.1 - b.1) < 0.02, "{closed} {flip} {align:?}: {a:?} != {b:?}");
                    }
                }
            }
        }
    }

    /// Ensures hidden text layers show no markers and refuse a placement.
    #[test]
    fn hidden_active_layer_rejects_placement() {
        let mut app = app(Tool::PathSelection);
        let id = layer(&app);
        app.session
            .edit("hide", |doc, _| {
                doc.layer_mut(id).unwrap().visible = false;
                Ok(())
            })
            .unwrap();
        assert!(active_text(&app).is_none());
        let (before, count) = (doc_shape(&app, id), steps(&app));
        place(&mut app, id, range(30.0, 110.0));
        assert_eq!(steps(&app), count);
        assert_eq!(doc_shape(&app, id), before);
    }

    /// The shape of the layer `id` read from the document, for a layer `active_text` does not show.
    fn doc_shape(app: &PhotocraftApp, id: LayerId) -> TextShape {
        let LayerContent::Text(text) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { panic!("expected text") };
        text.shape.clone()
    }

    /// Proves a placement equal to the layer's own range commits nothing: a click on a marker that
    /// moved nothing leaves the model and the history as they were.
    #[test]
    fn legacy_click_and_release_preserves_model_and_history() {
        let mut app = app(Tool::PathSelection);
        app.run("type.setPathPlacement", json!({"end":null})).unwrap();
        let before = shape(&app);
        let count = steps(&app);
        let text = active_text(&app).unwrap();
        let own = effective_range(text, &active_path_geometry(&app, text).unwrap()).unwrap();
        let id = layer(&app);
        place(&mut app, id, range(own[0], own[1]));
        assert_eq!(shape(&app), before);
        assert_eq!(steps(&app), count);
    }

    /// Ensures a proposed range is shown without a history step, keeps one built picture while it
    /// stays the same (a second pass of a frame), and drops with `propose(None)`.
    #[test]
    fn the_preview_follows_the_proposed_range_without_a_history_step() {
        let mut app = app(Tool::PathSelection);
        let before = shape(&app);
        let n = steps(&app);
        assert!(display_doc(&mut app, 0).is_none());
        propose(&mut app, Some(range(30.0, 100.0)));
        let (shown, key) = display_doc(&mut app, 0).unwrap();
        let LayerContent::Text(text) = &shown.layer(layer(&app)).unwrap().content else { panic!("expected text") };
        let TextShape::Path { start, .. } = &text.shape else { panic!("expected path") };
        assert_eq!(*start, 30.0);
        // The next pass proposes the same range again.
        propose(&mut app, Some(range(30.0, 100.0)));
        let (again, same_key) = display_doc(&mut app, 0).unwrap();
        assert_eq!(key, same_key);
        assert!(Arc::ptr_eq(&shown, &again));
        assert_eq!(shape(&app), before);
        assert_eq!(steps(&app), n);
        // A proposal equal to the layer's own placement shows the document as it is.
        propose(&mut app, Some(range(20.0, 100.0)));
        assert!(display_doc(&mut app, 0).is_none());
        propose(&mut app, Some(range(30.0, 100.0)));
        assert!(display_doc(&mut app, 0).is_some());
        propose(&mut app, None);
        assert!(display_doc(&mut app, 0).is_none());
    }

    /// Proves preview damage calculations bound union regions across successive preview revisions and restore correctly when the proposal ends.
    #[test]
    fn damage_tracks_preview_and_return() {
        let mut app = app(Tool::PathSelection);
        let st = app.session.active().unwrap();
        let (doc, revision) = (st.doc.id, st.revision);
        propose(&mut app, Some(range(30.0, 100.0)));
        let (first_doc, first) = display_doc(&mut app, 0).unwrap();
        propose(&mut app, Some(range(40.0, 100.0)));
        let (second_doc, second) = display_doc(&mut app, 0).unwrap();
        let layer_id = layer(&app);
        let original_bounds = app.session.active().unwrap().doc.layer(layer_id).unwrap().surface().map_or(Rect::EMPTY, |s| s.tile_bounds());
        assert!(!original_bounds.is_empty(), "original rendered text surface bounds must be non-empty");

        let first_bounds = first_doc.layer(layer_id).unwrap().surface().map_or(Rect::EMPTY, |s| s.tile_bounds());
        assert!(!first_bounds.is_empty(), "first preview rendered text surface bounds must be non-empty");

        let second_bounds = second_doc.layer(layer_id).unwrap().surface().map_or(Rect::EMPTY, |s| s.tile_bounds());
        assert!(!second_bounds.is_empty(), "second preview rendered text surface bounds must be non-empty");

        assert_eq!(damage(&app, doc, revision, 0, first), Some(original_bounds.union(&first_bounds)));
        assert_eq!(damage(&app, doc, revision, first, second), Some(first_bounds.union(&second_bounds)));
        propose(&mut app, None);
        assert_eq!(damage(&app, doc, revision, second, 0), Some(second_bounds.union(&original_bounds)));
        assert_eq!(damage(&app, doc, revision + 1, second, 0), None);
    }

    /// Verifies a placement commits exactly one history step and cleanly rolls back on undo.
    #[test]
    fn marker_drags_commit_once_and_undo() {
        for tool in [Tool::PathSelection, Tool::DirectSelection] {
            for (to, expected) in [(range(30.0, 100.0), (30.0, Some(100.0), false)), (range(20.0, 110.0), (20.0, Some(110.0), false))] {
                let mut app = app(tool);
                let before = shape(&app);
                let n = steps(&app);
                let id = layer(&app);
                propose(&mut app, Some(to));
                assert_eq!(shape(&app), before);
                assert_eq!(steps(&app), n);
                place(&mut app, id, to);
                propose(&mut app, None);
                assert_eq!(placement(&app), expected);
                let TextShape::Path { path: old, .. } = &before else { panic!("expected path") };
                let TextShape::Path { path: new, .. } = shape(&app) else { panic!("expected path") };
                assert_eq!(*old, new);
                assert_eq!(steps(&app), n + 1);
                assert!(app.session.undo());
                assert_eq!(shape(&app), before);
            }
        }
    }

    /// Verifies a body move (both ends shifted) is one step and Undo restores it.
    #[test]
    fn body_drag_moves_range_once_and_undoes() {
        for tool in [Tool::PathSelection, Tool::DirectSelection] {
            let mut app = app(tool);
            let before = shape(&app);
            let n = steps(&app);
            let id = layer(&app);
            place(&mut app, id, range(30.0, 110.0));
            assert_eq!(placement(&app), (30.0, Some(110.0), false));
            assert_eq!(steps(&app), n + 1);
            assert!(app.session.undo());
            assert_eq!(shape(&app), before);
        }
    }

    /// Verifies a range on the other side of the path commits with its flip.
    #[test]
    fn a_flipped_range_commits_with_its_side() {
        let mut app = app(Tool::PathSelection);
        let id = layer(&app);
        place(&mut app, id, PathRange::new(60.0, 140.0, true));
        assert_eq!(placement(&app), (60.0, Some(140.0), true));
    }

    /// Ensures position locks on text layers give the markers `locked` and refuse a placement and the command.
    #[test]
    fn position_lock_rejects_marker_and_command() {
        for tool in [Tool::PathSelection, Tool::DirectSelection] {
            let mut app = app(tool);
            let id = layer(&app);
            assert!(!locked(&app, id));
            app.session
                .edit("lock position", |doc, _| {
                    doc.layer_mut(id).unwrap().locks.position = true;
                    Ok(())
                })
                .unwrap();
            assert!(locked(&app, id));
            let before = shape(&app);
            let n = steps(&app);
            place(&mut app, id, range(30.0, 110.0));
            assert!(app.session.execute("type.setPathPlacement", json!({"start":30})).is_err());
            assert_eq!(shape(&app), before);
            assert_eq!(steps(&app), n);
        }
    }

    /// Ensures a proposal made before the document snapshot was replaced is not shown.
    #[test]
    fn a_proposal_of_a_replaced_snapshot_is_not_shown() {
        let mut app = app(Tool::PathSelection);
        propose(&mut app, Some(range(30.0, 100.0)));
        assert!(display_doc(&mut app, 0).is_some());
        let st = app.session.active_mut().unwrap();
        *st.doc_mut() = Arc::new((*st.doc).clone());
        assert!(display_doc(&mut app, 0).is_none());
    }

    /// Ensures placing the `effective_range` of a closed path over one lap leaves range and history unchanged.
    #[test]
    fn closed_range_over_one_lap_click_preserves_range_and_history() {
        let mut app = app(Tool::PathSelection);
        let st = app.session.active_mut().unwrap();
        let id = st.active_layer.unwrap();
        let layer = Arc::make_mut(st.doc_mut()).layer_mut(id).unwrap();
        let LayerContent::Text(text) = &mut layer.content else { panic!("expected text") };
        text.shape = TextShape::Path {
            path: photocraft_doc::vector::Subpath::polygon(&[(20.0, 20.0), (180.0, 20.0), (180.0, 180.0), (20.0, 180.0)]),
            start: 20.0,
            end: Some(700.0),
            flip: false,
        };
        let before = shape(&app);
        let count = steps(&app);
        let text = active_text(&app).unwrap();
        let own = effective_range(text, &active_path_geometry(&app, text).unwrap()).unwrap();
        assert_eq!(own, [20.0, 700.0]);
        place(&mut app, id, range(own[0], own[1]));
        assert_eq!(shape(&app), before);
        assert_eq!(steps(&app), count);
    }

    /// Verifies that hiding an ancestor layer group excludes child path text from the markers and refuses a placement.
    #[test]
    fn hidden_ancestor_excludes_markers_and_capture() {
        let mut app = app(Tool::PathSelection);
        let text_id = layer(&app);
        let group_id = LayerId(app.run("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
        app.run("layer.select", json!({"layer": text_id.0, "mode": "replace"})).unwrap();
        assert_eq!(app.session.active().unwrap().active_layer, Some(text_id));
        assert!(active_text(&app).is_some());
        let before = doc_shape(&app, text_id);

        app.session
            .edit("hide group", |doc, _| {
                doc.layer_mut(group_id).unwrap().visible = false;
                Ok(())
            })
            .unwrap();

        assert!(active_text(&app).is_none());
        let count = steps(&app);
        propose(&mut app, Some(range(30.0, 100.0)));
        assert!(display_doc(&mut app, 0).is_none());
        place(&mut app, text_id, range(30.0, 100.0));
        assert_eq!(steps(&app), count);
        assert_eq!(doc_shape(&app, text_id), before);
    }

    /// Proves overflow marker caches invalidate when a document Arc is replaced with modified content at the same revision.
    #[test]
    fn overflow_cache_invalidates_on_replacement_arc_with_same_id_and_revision() {
        let mut app = app(Tool::PathSelection);
        let ctx = app.ctx().clone();
        let text = active_text(&app).unwrap().clone();
        let initial = overflows(&app, &ctx, &text);
        assert!(!initial, "short text should not overflow range [20, 100]");

        // 1. Replacement Arc with same docid/revision but different text content:
        let st = app.session.active_mut().unwrap();
        let doc_id = st.doc.id;
        let revision = st.revision;
        let original_arc = st.doc.clone();
        let mut modified = (*original_arc).clone();
        let layer_id = st.active_layer.unwrap();
        let LayerContent::Text(t) = &mut modified.layer_mut(layer_id).unwrap().content else { panic!("expected text") };
        t.text = "A very long sentence on a path that is guaranteed to overflow the assigned span".into();
        *st.doc_mut() = Arc::new(modified);
        assert_eq!(st.doc.id, doc_id);
        assert_eq!(st.revision, revision);
        assert!(!Arc::ptr_eq(&st.doc, &original_arc));

        let new_text = active_text(&app).unwrap().clone();
        let updated = overflows(&app, &ctx, &new_text);
        assert!(updated, "overflow cache must invalidate on replacement Arc with different text content");

        // 2. Replacement Arc with same docid/revision but different range content:
        let st = app.session.active_mut().unwrap();
        let prev_arc = st.doc.clone();
        let mut modified = (*prev_arc).clone();
        let LayerContent::Text(t) = &mut modified.layer_mut(layer_id).unwrap().content else { panic!("expected text") };
        t.text = "ABC".into();
        let TextShape::Path { end, .. } = &mut t.shape else { panic!("expected path") };
        *end = Some(21.0);
        *st.doc_mut() = Arc::new(modified);
        assert_eq!(st.doc.id, doc_id);
        assert_eq!(st.revision, revision);
        assert!(!Arc::ptr_eq(&st.doc, &prev_arc));

        let tiny_range_text = active_text(&app).unwrap().clone();
        let tiny_overflow = overflows(&app, &ctx, &tiny_range_text);
        assert!(tiny_overflow, "overflow cache must invalidate on replacement Arc with tiny range");

        // 3. Expanding range on replacement Arc so it no longer overflows:
        let st = app.session.active_mut().unwrap();
        let prev_arc = st.doc.clone();
        let mut modified = (*prev_arc).clone();
        let LayerContent::Text(t) = &mut modified.layer_mut(layer_id).unwrap().content else { panic!("expected text") };
        let TextShape::Path { end, .. } = &mut t.shape else { panic!("expected path") };
        *end = Some(180.0);
        *st.doc_mut() = Arc::new(modified);
        assert_eq!(st.doc.id, doc_id);
        assert_eq!(st.revision, revision);

        let wide_range_text = active_text(&app).unwrap().clone();
        let wide_overflow = overflows(&app, &ctx, &wide_range_text);
        assert!(!wide_overflow, "overflow cache must invalidate on replacement Arc with expanded range");
    }

    /// Proves non-integer path lengths whose `f32` conversion rounds upward keep both markers on the track, flipped too.
    #[test]
    fn type_path_placement_noninteger_line_rounding_upward_keeps_markers_visible() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.sync_views();
        app.ui.tool = Tool::PathSelection;
        // Line from (20, 20) to (27, 27) with length 7 * sqrt2 ≈ 9.899495.
        app.run("path.set", json!({"name":"work", "path":{"subpaths":[{"closed":false,"knots":[[20,20],[27,27]]}]}})).unwrap();
        app.ui.selected_path = Some("work".into());
        app.run("type.create", json!({"text":"AB", "x":20, "y":20})).unwrap();
        app.run("type.attachToPath", json!({"start":0})).unwrap();
        app.ui.views[0].zoom = 1.0;

        let geom = PathGeometry::new(&photocraft_doc::vector::Subpath::polyline(&[(20.0, 20.0), (27.0, 27.0)]), false).unwrap();
        let length_f32 = geom.length() as f32;
        assert!(f64::from(length_f32) > geom.length(), "scale 7 * sqrt2 must round up in f32");

        // Both marks of the layer's range sit on the track, as the control reads them (a flipped range on the reversed path).
        let on_track = |app: &PhotocraftApp| {
            let text = active_text(app).unwrap();
            let track = Track::new(app, text).unwrap();
            let [start, end] = effective_range(text, &track.geometry).unwrap();
            let TextShape::Path { flip, .. } = &text.shape else { panic!("expected path") };
            let at = |s: f64| track.at(if *flip { track.length() - s } else { s });
            at(start).is_some() && at(end).is_some()
        };
        app.run("type.setPathPlacement", json!({"end": length_f32})).unwrap();
        assert!(on_track(&app), "markers must remain on the track after the endpoint command");
        app.run("type.setPathPlacement", json!({"start": length_f32, "end": length_f32})).unwrap();
        assert!(on_track(&app), "zero-capacity endpoint markers must remain on the track");
        app.run("type.setPathPlacement", json!({"start": 0.0, "end": length_f32, "flip": true})).unwrap();
        assert!(on_track(&app), "flipped endpoint markers must remain on the track");

        // A placement past the end is committed within the path.
        app.run("type.setPathPlacement", json!({"start": 0.0, "end": 5.0, "flip": false})).unwrap();
        let id = layer(&app);
        place(&mut app, id, range(0.0, 50.0));
        let (_, end, _) = placement(&app);
        assert!(end.is_some_and(|end| end <= geom.canonical_length_f32()), "{end:?}");
        assert!(on_track(&app), "markers must remain on the track after a placement past the end");
    }

    #[test]
    fn marker_geometry_cache_reuse_and_invalidation() {
        let mut app = app(Tool::PathSelection);
        let text = active_text(&app).unwrap().clone();

        let geom1 = active_path_geometry(&app, &text).expect("initial geometry");
        let initial_length = geom1.length();
        assert!(app.ui.type_path_geometry_cache.0.lock().unwrap().is_some());

        let geom2 = active_path_geometry(&app, &text).expect("cached geometry");
        assert!(Arc::ptr_eq(&geom1, &geom2));

        let st = app.session.active_mut().unwrap();
        let cloned_doc = (*st.doc).clone();
        *st.doc_mut() = Arc::new(cloned_doc);
        let geom_doc_replaced = active_path_geometry(&app, &text).expect("recomputed geometry");
        assert!(!Arc::ptr_eq(&geom1, &geom_doc_replaced));

        app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[0, 0], [200, 0]]}]}})).unwrap();
        app.run("type.attachToPath", json!({"start": 0})).unwrap();
        let text_after_path = active_text(&app).unwrap();
        let geom_new_path = active_path_geometry(&app, text_after_path).expect("new path geometry");
        assert_ne!(geom_new_path.length(), initial_length);
        assert!(!Arc::ptr_eq(&geom_doc_replaced, &geom_new_path));

        app.run("type.setPathPlacement", json!({"flip": true})).unwrap();
        let text_after_flip = active_text(&app).unwrap();
        let geom_flipped = active_path_geometry(&app, text_after_flip).expect("flipped geometry");
        assert!(!Arc::ptr_eq(&geom_new_path, &geom_flipped));
        let geom_flipped2 = active_path_geometry(&app, text_after_flip).expect("cached flipped geometry");
        assert!(Arc::ptr_eq(&geom_flipped, &geom_flipped2));
        assert_eq!(geom_flipped.sample(10.0).unwrap().1.0, -geom_new_path.sample(10.0).unwrap().1.0);
    }

    /// Proves a stale explicit end on a shortened open path is clamped for the markers, a placement equal to that
    /// clamped range commits nothing, and an edit commits a valid bounded range that Undo takes back.
    #[test]
    fn shortened_path_stale_explicit_end_preserves_markers_cancellation_and_commits_valid_range() {
        let mut app = app(Tool::PathSelection);
        let id = layer(&app);
        // Shorten path from length 160 (20 to 180) to length 60 (20 to 80).
        let shortened = photocraft_doc::vector::Subpath::polyline(&[(20.0, 100.0), (80.0, 100.0)]);
        app.session
            .edit("shorten path", |doc, _| {
                let LayerContent::Text(text) = &mut doc.layer_mut(id).unwrap().content else { panic!("expected text") };
                let TextShape::Path { path, .. } = &mut text.shape else { panic!("expected path") };
                *path = shortened;
                Ok(())
            })
            .unwrap();

        // Stored document retains stale explicit end (100.0) past shortened path length (60.0).
        assert_eq!(placement(&app), (20.0, Some(100.0), false));

        // The effective range is bounded, and both ends sit on the track.
        let text = active_text(&app).unwrap();
        let track = Track::new(&app, text).unwrap();
        let own = effective_range(text, &track.geometry).expect("range must remain usable after shortening");
        assert_eq!(own, [20.0, 60.0]);
        assert!(track.at(own[0]).is_some() && track.at(own[1]).is_some());

        // Placing the range the markers show changes nothing.
        let before = shape(&app);
        let initial_steps = steps(&app);
        place(&mut app, id, range(own[0], own[1]));
        assert_eq!(shape(&app), before);
        assert_eq!(steps(&app), initial_steps);

        // Moving the end 10 units back commits a valid bounded endpoint.
        place(&mut app, id, range(20.0, 50.0));
        assert_eq!(steps(&app), initial_steps + 1);
        assert_eq!(placement(&app), (20.0, Some(50.0), false));

        // Undo cleanly rolls back to the preserved original stale document.
        assert!(app.session.undo());
        assert_eq!(shape(&app), before);
    }

    /// Proves a start beyond the shortened path is clamped for the markers and an edit commits a valid range.
    #[test]
    fn shortened_path_start_beyond_new_length_preserves_markers_cancellation_and_commits_valid_range() {
        let mut app = app(Tool::PathSelection);
        let id = layer(&app);
        // Shorten path to length 40 (20 to 60) and place stale start at 50.0 and end at 80.0.
        let shortened = photocraft_doc::vector::Subpath::polyline(&[(20.0, 100.0), (60.0, 100.0)]);
        app.session
            .edit("shorten path beyond start", |doc, _| {
                let LayerContent::Text(text) = &mut doc.layer_mut(id).unwrap().content else { panic!("expected text") };
                text.shape = TextShape::Path { path: shortened, start: 50.0, end: Some(80.0), flip: false };
                Ok(())
            })
            .unwrap();
        assert_eq!(placement(&app), (50.0, Some(80.0), false));

        // Both ends are clamped to the path's end, where the markers coincide.
        let text = active_text(&app).unwrap();
        let track = Track::new(&app, text).unwrap();
        let own = effective_range(text, &track.geometry).expect("range must remain usable when start is beyond length");
        assert_eq!(own, [40.0, 40.0]);
        assert!(track.at(own[0]).is_some() && track.at(own[1]).is_some());

        // Placing that range changes nothing.
        let before = shape(&app);
        let initial_steps = steps(&app);
        place(&mut app, id, range(own[0], own[1]));
        assert_eq!(shape(&app), before);
        assert_eq!(steps(&app), initial_steps);

        // Pulling the start 25 units back commits valid endpoints.
        place(&mut app, id, range(15.0, 40.0));
        assert_eq!(steps(&app), initial_steps + 1);
        assert_eq!(placement(&app), (15.0, Some(40.0), false));

        // Undo restores original shape.
        assert!(app.session.undo());
        assert_eq!(shape(&app), before);
    }

    /// Verifies malformed, non-finite, and inverted ranges continue to be rejected by effective_range.
    #[test]
    fn effective_range_safeguards_reject_malformed_and_inverted_ranges() {
        use photocraft_doc::vector::Subpath;
        let path = Subpath::polyline(&[(0.0, 0.0), (100.0, 0.0)]);
        let geometry = PathGeometry::new(&path, false).unwrap();
        let make_text = |start: f32, end: Option<f32>| TextLayer {
            text: "ABC".into(),
            shape: TextShape::Path { path: path.clone(), start, end, flip: false },
            ..Default::default()
        };

        // Inverted explicit range (start > end) must return None.
        assert!(effective_range(&make_text(50.0, Some(20.0)), &geometry).is_none());

        // Negative start on open path must return None.
        assert!(effective_range(&make_text(-5.0, Some(50.0)), &geometry).is_none());
        assert!(effective_range(&make_text(-5.0, None), &geometry).is_none());

        // Non-finite values must return None.
        assert!(effective_range(&make_text(f32::NAN, Some(50.0)), &geometry).is_none());
        assert!(effective_range(&make_text(10.0, Some(f32::NAN)), &geometry).is_none());
        assert!(effective_range(&make_text(10.0, Some(f32::INFINITY)), &geometry).is_none());
    }
}
