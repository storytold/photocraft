//! Placement markers for the active layer's type path.
use crate::{PhotocraftApp, canvas::ViewXform, state::Tool};
use std::{
    collections::VecDeque,
    sync::{Arc, Weak},
};

use egui::{Pos2, Stroke, Vec2};
use photocraft_doc::{Document, LayerContent, TextLayer, text::TextShape};
use photocraft_geom::{Point, Rect};
use photocraft_text::warp::PathGeometry;

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

fn alignment_factor(align: photocraft_doc::text::TextAlign) -> f64 {
    use photocraft_doc::text::TextAlign;
    match align {
        TextAlign::Center | TextAlign::JustifyCenter => 0.5,
        TextAlign::Right | TextAlign::JustifyRight => 1.0,
        _ => 0.0,
    }
}

fn effective_range(text: &TextLayer, geometry: &PathGeometry) -> Option<[f64; 2]> {
    let TextShape::Path { path, start, end, .. } = &text.shape else { return None };
    let length = geometry.length();
    let start = f64::from(*start);
    if !length.is_finite() || length <= 0.0 || !start.is_finite() {
        return None;
    }
    let range = if let Some(end) = end {
        [start, f64::from(*end)]
    } else {
        // Different paragraph alignments cannot share one equivalent interval.
        let runs = text.paragraph_runs();
        let f = alignment_factor(runs.first()?.style.align);
        if runs.iter().any(|run| alignment_factor(run.style.align) != f) {
            return None;
        }
        if path.closed {
            [start - f * length, start + (1.0 - f) * length]
        } else if !(0.0..=length).contains(&start) {
            return None;
        } else if f == 0.0 {
            [start, length]
        } else if f == 1.0 {
            [0.0, start]
        } else if start <= length / 2.0 {
            [0.0, 2.0 * start]
        } else {
            [2.0 * start - length, length]
        }
    };
    if range.iter().any(|n| !n.is_finite()) || range[1] < range[0] || (!path.closed && (range[0] < 0.0 || range[1] > length)) {
        return None;
    }
    Some(range)
}

pub(crate) fn markers(text: &TextLayer, xf: &ViewXform) -> Option<[(Pos2, Vec2); 2]> {
    let TextShape::Path { path, flip, .. } = &text.shape else { return None };
    let geometry = PathGeometry::new(path, *flip)?;
    let [start, finish] = effective_range(text, &geometry)?;
    let sample = |arc| {
        let ((x, y), (tx, ty)) = geometry.sample(arc)?;
        let a = text.transform.apply(Point::new(x, y));
        let b = text.transform.apply(Point::new(x + tx, y + ty));
        let p = xf.to_screen(a.x as f32, a.y as f32);
        let q = xf.to_screen(b.x as f32, b.y as f32);
        let direction = q - p;
        (p.is_finite() && direction.is_finite() && direction.length_sq() > 1e-10).then_some((p, direction.normalized()))
    };
    Some([sample(start)?, sample(finish)?])
}

fn marker_tip(point: Pos2, tangent: Vec2, index: usize) -> Pos2 {
    let side = if index == 0 { -1.0 } else { 1.0 };
    point + Vec2::new(-tangent.y, tangent.x) * (10.0 * side)
}

fn marker_hit(ends: [(Pos2, Vec2); 2], p: Pos2) -> Option<(usize, f32)> {
    ends.into_iter()
        .enumerate()
        .map(|(index, (point, tangent))| (index, marker_tip(point, tangent, index).distance(p)))
        .filter(|(_, distance)| *distance <= 6.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

pub(crate) fn cursor(app: &mut PhotocraftApp, xf: &ViewXform, p: Pos2) -> Option<bool> {
    if let Some(drag) = app.ui.type_path_drag.as_ref().filter(|drag| drag.valid(app)) {
        return Some(drag.endpoint != Some(1));
    }
    if !app.ui.view.shows(app.ui.view.show.target_path) {
        return None;
    }
    let st = app.session.active()?;
    let layer = st.active_layer?;
    if (st.doc.effective_locks(layer).all || st.doc.effective_locks(layer).position) || !st.doc.layer(layer)?.visible {
        return None;
    }
    let text = active_text(app)?;
    let ends = markers(text, xf)?;
    if let Some((index, _)) = marker_hit(ends, p) {
        return Some(index == 0);
    }
    let d = xf.to_doc(p);
    (crate::type_tool::hit_layer(app, d[0], d[1]) == Some(layer)).then_some(true)
}

pub(crate) fn draw_cursor(ctx: &egui::Context, p: Pos2, right: bool) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("type-path-cursor")));
    let sign = if right { 1.0 } else { -1.0 };
    let segments = [
        (p + Vec2::new(0.0, -7.0), p + Vec2::new(0.0, 7.0)),
        (p + Vec2::new(-3.0, -7.0), p + Vec2::new(3.0, -7.0)),
        (p + Vec2::new(-3.0, 7.0), p + Vec2::new(3.0, 7.0)),
        (p + Vec2::new(2.0 * sign, 0.0), p + Vec2::new(9.0 * sign, 0.0)),
        (p + Vec2::new(9.0 * sign, 0.0), p + Vec2::new(6.0 * sign, -3.0)),
        (p + Vec2::new(9.0 * sign, 0.0), p + Vec2::new(6.0 * sign, 3.0)),
    ];
    for (a, b) in segments {
        painter.line_segment([a, b], Stroke::new(3.0, egui::Color32::BLACK));
        painter.line_segment([a, b], Stroke::new(1.0, egui::Color32::WHITE));
    }
}

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

pub(crate) fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(text) = active_text(app) else { return };
    let mut preview = text.clone();
    if let Some(drag) = app.ui.type_path_drag.as_ref().filter(|d| d.valid(app) && (d.proposed != d.original || d.flipped)) {
        let (range, side) = drag.placement();
        if let TextShape::Path { start, end, flip, .. } = &mut preview.shape {
            *start = range[0] as f32;
            *end = Some(range[1] as f32);
            *flip = side;
        }
    }
    let Some(markers) = markers(&preview, xf) else { return };
    let stroke = Stroke::new(1.5, crate::theme::Tokens::get(painter.ctx()).accent);
    let overflow = overflows(app, painter.ctx(), &preview);
    for (index, (point, tangent)) in markers.into_iter().enumerate() {
        let normal = Vec2::new(-tangent.y, tangent.x);
        let side = if index == 0 { -1.0 } else { 1.0 };
        let tip = marker_tip(point, tangent, index);
        painter.line_segment([point, tip], stroke);
        painter.line_segment([tip, tip + tangent * (5.0 * -side)], stroke);
        if index == 1 && overflow {
            let center = tip + normal * 5.0;
            painter.line_segment([center - tangent * 3.0, center + tangent * 3.0], stroke);
            painter.line_segment([center - normal * 3.0, center + normal * 3.0], stroke);
        }
    }
}

#[derive(Clone, Debug)]
struct Preview {
    range: [f64; 2],
    flip: bool,
    shown: Arc<Document>,
    key: u64,
}

pub(crate) struct Damage {
    document: photocraft_doc::DocId,
    revision: u64,
    original_area: Rect,
    areas: VecDeque<(u64, Rect)>,
}

pub(crate) fn damage(app: &PhotocraftApp, doc: photocraft_doc::DocId, revision: u64, before: u64, after: u64) -> Option<Rect> {
    let p = app.type_path_damage.as_ref().filter(|p| p.document == doc && p.revision == revision)?;
    let area = |key| if key == 0 { Some(p.original_area) } else { p.areas.iter().find(|(k, _)| *k == key).map(|(_, r)| *r) };
    Some(area(before)?.union(&area(after)?))
}

pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let drag = app.ui.type_path_drag.as_ref()?;
    if !drag.valid(app) {
        return None;
    }
    let st = app.session.documents().get(idx)?;
    if st.doc.id != drag.document || st.revision != drag.revision {
        return None;
    }
    if drag.proposed == drag.original && !drag.flipped {
        return None;
    }
    let (range, flip) = drag.placement();
    if let Some(preview) = &drag.preview
        && preview.range == range
        && preview.flip == flip
    {
        return Some((preview.shown.clone(), preview.key));
    }
    let source = st.doc.clone();
    let layer = drag.layer;
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
    let mut damage = app.type_path_damage.take().filter(|p| p.document == source.id && p.revision == st.revision).unwrap_or_else(|| Damage {
        document: source.id,
        revision: st.revision,
        original_area,
        areas: VecDeque::new(),
    });
    damage.original_area = damage.original_area.union(&original_area);
    if damage.areas.len() >= 128 {
        damage.areas.pop_front();
    }
    damage.areas.push_back((key, area));
    app.type_path_damage = Some(damage);
    app.ui.type_path_drag.as_mut()?.preview = Some(Preview { range, flip, shown: shown.clone(), key });
    Some((shown, key))
}

/// One marker gesture. The document is changed only on release, so Escape is lossless.
#[derive(Clone, Debug)]
pub struct Drag {
    document: photocraft_doc::DocId,
    source: Weak<Document>,
    layer: photocraft_doc::LayerId,
    revision: u64,
    tool: Tool,
    inverse: photocraft_doc::Affine,
    geometry: PathGeometry,
    closed: bool,
    endpoint: Option<usize>,
    alignment: f64,
    original: [f64; 2],
    proposed: [f64; 2],
    previous: f64,
    delta: f64,
    original_flip: bool,
    flipped: bool,
    initial_side: f64,
    transform: photocraft_doc::Affine,
    xf: ViewXform,
    preview: Option<Preview>,
}

// Geometry is fixed by the source document revision; cached preview pixels are derived state.
impl PartialEq for Drag {
    fn eq(&self, other: &Self) -> bool {
        self.source.ptr_eq(&other.source)
            && (self.document, self.layer, self.revision, self.tool) == (other.document, other.layer, other.revision, other.tool)
            && self.inverse == other.inverse
            && self.closed == other.closed
            && self.endpoint == other.endpoint
            && self.alignment == other.alignment
            && self.original == other.original
            && self.proposed == other.proposed
            && self.previous == other.previous
            && self.delta == other.delta
            && self.original_flip == other.original_flip
            && self.flipped == other.flipped
            && self.initial_side == other.initial_side
            && self.transform == other.transform
            && self.xf.rect == other.xf.rect
            && self.xf.zoom == other.xf.zoom
            && self.xf.center == other.xf.center
            && self.xf.flip == other.xf.flip
            && self.xf.rotation == other.xf.rotation
            && self.xf.aspect == other.xf.aspect
    }
}

impl Drag {
    fn valid(&self, app: &PhotocraftApp) -> bool {
        active_text(app).is_some()
            && app.ui.tool == self.tool
            && app.session.active().is_some_and(|st| {
                st.doc.id == self.document
                    && self.source.ptr_eq(&Arc::downgrade(&st.doc))
                    && st.active_layer == Some(self.layer)
                    && st.revision == self.revision
            })
    }

    fn placement(&self) -> ([f64; 2], bool) {
        let range = if self.flipped {
            let length = self.geometry.length();
            let start = length - self.proposed[1];
            let start = if self.closed { start.rem_euclid(length) } else { start };
            [start, start + self.proposed[1] - self.proposed[0]]
        } else {
            self.proposed
        };
        (range, self.original_flip ^ self.flipped)
    }

    fn update(&mut self, x: f64, y: f64) {
        let point = self.inverse.apply(Point::new(x, y));
        let Some((arc, distance)) = self.geometry.project_near(point.x, point.y, self.previous) else { return };
        let length = self.geometry.length();
        if let Some(((px, py), (tx, ty))) = self.geometry.sample(arc) {
            let a = self.transform.apply(Point::new(px, py));
            let b = self.transform.apply(Point::new(px + tx, py + ty));
            let c = self.transform.apply(Point::new(px - ty, py + tx));
            let a = self.xf.to_screen(a.x as f32, a.y as f32);
            let b = self.xf.to_screen(b.x as f32, b.y as f32) - a;
            let c = self.xf.to_screen(c.x as f32, c.y as f32) - a;
            let screen_distance = if b.length() > 1e-6 { distance * f64::from((b.x * c.y - b.y * c.x).abs() / b.length()) } else { 0.0 };
            if screen_distance.abs() > 4.0 {
                self.flipped = screen_distance.signum() != self.initial_side;
            }
        }
        let mut delta = arc - self.previous;
        if self.closed {
            delta = (delta + length / 2.0).rem_euclid(length) - length / 2.0;
        }
        self.delta += delta;
        self.previous = arc;
        if self.delta == 0.0 && self.proposed == self.original {
            return;
        }
        let [start, end] = self.original;
        self.proposed = if self.endpoint == Some(0) {
            [(start + self.delta).clamp(if self.closed { end - length } else { 0.0 }, end), end]
        } else if self.endpoint == Some(1) {
            [start, (end + self.delta).clamp(start, if self.closed { start + length } else { length })]
        } else if self.closed {
            [start + self.delta, end + self.delta]
        } else {
            moved_open_range(self.original, self.delta, self.alignment, length)
        };
    }
}

// Preserve the paragraph anchor when an open range reaches an endpoint. Keeping
// the whole span fixed would prevent a range ending at the path endpoint from moving.
fn moved_open_range([start, end]: [f64; 2], delta: f64, alignment: f64, length: f64) -> [f64; 2] {
    let span = end - start;
    let anchor = (start + span * alignment + delta).clamp(0.0, length);
    if alignment == 0.0 {
        [anchor, (anchor + span).min(length)]
    } else if alignment == 1.0 {
        [(anchor - span).max(0.0), anchor]
    } else {
        let half = (span / 2.0).min(anchor).min(length - anchor);
        [anchor - half, anchor + half]
    }
}

/// Discard a capture when its original document or tool is no longer active.
pub(crate) fn cancel_stale(app: &mut PhotocraftApp) {
    if app.ui.type_path_drag.as_ref().is_some_and(|drag| !drag.valid(app)) {
        cancel(app);
    }
}

pub(crate) fn cancel_missing_release(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.input(|input| !input.focused || !input.pointer.primary_down()) {
        cancel(app);
    }
}

pub(crate) fn cancel(app: &mut PhotocraftApp) -> bool {
    app.ui.type_path_drag.take().is_some()
}

#[cfg(test)]
pub(crate) fn pointer(app: &mut PhotocraftApp, event: crate::canvas::ToolEvent) -> bool {
    pointer_with_view(app, event, None)
}

pub(crate) fn pointer_with_view(app: &mut PhotocraftApp, event: crate::canvas::ToolEvent, view: Option<&ViewXform>) -> bool {
    use crate::canvas::ToolEvent;
    if matches!(event, ToolEvent::Down { .. }) {
        cancel(app);
    }
    if let Some(mut drag) = app.ui.type_path_drag.take() {
        if !drag.valid(app) {
            return true;
        }
        if let Some(xf) = view {
            drag.xf = *xf;
        }
        match event {
            ToolEvent::Move { x, y, .. } => {
                drag.update(x, y);
                app.ui.type_path_drag = Some(drag);
            }
            ToolEvent::Up { x, y } => {
                drag.update(x, y);
                if drag.proposed != drag.original || drag.flipped {
                    let (range, flip) = drag.placement();
                    if let Err(error) = app.run(
                        "type.setPathPlacement",
                        serde_json::json!({
                            "layer": drag.layer.0, "start": range[0], "end": range[1], "flip": flip
                        }),
                    ) {
                        app.ui.status = error;
                        app.ui.status_error = true;
                    }
                }
            }
            ToolEvent::Down { .. } => {}
        }
        return true;
    }
    let ToolEvent::Down { x, y, .. } = event else { return false };
    if !app.ui.view.shows(app.ui.view.show.target_path) {
        return false;
    }
    let Some(text) = active_text(app).cloned() else { return false };
    let TextShape::Path { path, flip, .. } = &text.shape else { return false };
    let Some(geometry) = PathGeometry::new(path, *flip) else { return false };
    let Some(inverse) = text.transform.inverse() else { return false };
    let Some(original) = effective_range(&text, &geometry) else { return false };
    let Some(xf) = view.copied().or_else(|| ViewXform::active(app)) else { return false };
    let p = xf.to_screen(x as f32, y as f32);
    let best = markers(&text, &xf).and_then(|ends| marker_hit(ends, p));
    let active_layer = app.session.active().and_then(|st| st.active_layer);
    let endpoint = if let Some((index, _)) = best {
        Some(index)
    } else if crate::type_tool::hit_layer(app, x, y) == active_layer && active_layer.is_some() {
        None
    } else {
        return false;
    };
    let Some(st) = app.session.active() else { return false };
    let Some(layer) = st.active_layer else { return false };
    if st.doc.effective_locks(layer).all || st.doc.effective_locks(layer).position {
        return false;
    }
    let point = inverse.apply(Point::new(x, y));
    let Some((previous, distance)) = geometry.project(point.x, point.y) else { return false };
    app.ui.type_path_drag = Some(Drag {
        document: st.doc.id,
        source: Arc::downgrade(&st.doc),
        layer,
        revision: st.revision,
        tool: app.ui.tool,
        inverse,
        geometry,
        closed: path.closed,
        endpoint,
        alignment: text.paragraph_runs().first().map_or(0.0, |run| alignment_factor(run.style.align)),
        original,
        proposed: original,
        previous,
        delta: 0.0,
        original_flip: *flip,
        flipped: false,
        initial_side: if distance.abs() > 1e-6 { distance.signum() } else { -1.0 },
        transform: text.transform,
        xf,
        preview: None,
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::ToolEvent;
    use serde_json::json;

    fn app(tool: Tool) -> PhotocraftApp {
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

    #[test]
    fn missing_release_cleanup_preserves_held_capture_and_cancels_lost_release() {
        let mut app = app(Tool::PathSelection);
        let ctx = egui::Context::default();
        let pos = egui::pos2(1000.0, 1000.0);
        ctx.begin_pass(egui::RawInput {
            focused: true,
            events: vec![
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerGone,
            ],
            ..Default::default()
        });
        down(&mut app, 40.0, 100.0);
        cancel_missing_release(&mut app, &ctx);
        assert!(app.ui.type_path_drag.is_some());
        let _ = ctx.end_pass();
        ctx.begin_pass(egui::RawInput {
            focused: true,
            events: vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }],
            ..Default::default()
        });
        let revision = app.session.active().unwrap().revision;
        cancel_missing_release(&mut app, &ctx);
        assert!(app.ui.type_path_drag.is_none());
        assert_eq!(app.session.active().unwrap().revision, revision);
        down(&mut app, 40.0, 100.0);
        assert!(app.ui.type_path_drag.is_some());
        let _ = ctx.end_pass();
        ctx.begin_pass(egui::RawInput { focused: false, ..Default::default() });
        cancel_missing_release(&mut app, &ctx);
        assert!(app.ui.type_path_drag.is_none());
        let _ = ctx.end_pass();
    }

    #[test]
    fn selecting_another_tool_cancels_capture_before_switching_back() {
        let mut app = app(Tool::PathSelection);
        down(&mut app, 40.0, 100.0);
        assert!(app.ui.type_path_drag.is_some());
        crate::tool_memory::select(&mut app, Tool::Move);
        crate::tool_memory::select(&mut app, Tool::PathSelection);
        assert!(app.ui.type_path_drag.is_none());
    }

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
        assert_eq!(app.session.active().unwrap().doc.work_path, source);
    }

    fn shape(app: &PhotocraftApp) -> TextShape {
        active_text(app).unwrap().shape.clone()
    }

    fn placement(app: &PhotocraftApp) -> (f32, Option<f32>, bool) {
        let TextShape::Path { start, end, flip, .. } = shape(app) else { panic!("expected path") };
        (start, end, flip)
    }

    fn steps(app: &PhotocraftApp) -> usize {
        app.session.active().unwrap().history.entries().len()
    }

    fn down(app: &mut PhotocraftApp, x: f64, y: f64) {
        assert!(pointer(app, ToolEvent::Down { x, y, pressure: 1.0 }));
    }
    fn move_to(app: &mut PhotocraftApp, x: f64, y: f64) {
        assert!(pointer(app, ToolEvent::Move { x, y, pressure: 1.0 }));
    }
    fn up(app: &mut PhotocraftApp, x: f64, y: f64) {
        assert!(pointer(app, ToolEvent::Up { x, y }));
    }

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

    #[test]
    fn hidden_active_layer_rejects_placement() {
        let mut app = app(Tool::PathSelection);
        let id = app.session.active().unwrap().active_layer.unwrap();
        app.session
            .edit("hide", |doc, _| {
                doc.layer_mut(id).unwrap().visible = false;
                Ok(())
            })
            .unwrap();
        assert!(active_text(&app).is_none());
        assert!(!pointer(&mut app, ToolEvent::Down { x: 40.0, y: 90.0, pressure: 1.0 }));
    }

    #[test]
    fn legacy_click_and_release_preserves_model_and_history() {
        let mut app = app(Tool::PathSelection);
        app.run("type.setPathPlacement", json!({"end":null})).unwrap();
        let before = shape(&app);
        let count = steps(&app);
        down(&mut app, 40.0, 90.0);
        up(&mut app, 40.0, 90.0);
        assert_eq!(shape(&app), before);
        assert_eq!(steps(&app), count);
    }

    #[test]
    fn marker_cursor_and_press_agree_in_transformed_views() {
        for rotation in [0.0, 37.0, 90.0] {
            for mirror in [false, true] {
                let mut app = app(Tool::DirectSelection);
                let id = app.session.active().unwrap().active_layer.unwrap();
                app.session
                    .edit("skew text", |doc, _| {
                        let LayerContent::Text(text) = &mut doc.layer_mut(id).unwrap().content else { panic!("text") };
                        text.transform = photocraft_doc::Affine { m: [1.8, 0.3, 0.6, 0.9, 12.0, -7.0] };
                        Ok(())
                    })
                    .unwrap();
                app.ui.views[0].rotation = rotation;
                app.ui.view.flip_horizontal = mirror;
                let xf = ViewXform::active(&app).unwrap();
                let ends = markers(active_text(&app).unwrap(), &xf).unwrap();
                for (index, (point, tangent)) in ends.into_iter().enumerate() {
                    let screen = marker_tip(point, tangent, index);
                    assert_eq!(cursor(&mut app, &xf, screen), Some(index == 0));
                    let doc = xf.to_doc(screen);
                    down(&mut app, doc[0], doc[1]);
                    assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, Some(index));
                    assert!(cancel(&mut app));
                }
            }
        }
    }

    #[test]
    fn explicit_view_routes_marker_gesture() {
        let mut app = app(Tool::DirectSelection);
        let mut xf = ViewXform::active(&app).unwrap();
        xf.zoom = 2.0;
        xf.rotation = 37.0;
        xf.aspect = 1.5;
        let (point, tangent) = markers(active_text(&app).unwrap(), &xf).unwrap()[0];
        let from = xf.to_doc(marker_tip(point, tangent, 0));
        assert!(pointer_with_view(&mut app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Some(&xf)));
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, Some(0));
        let to = [from[0] + 10.0, from[1]];
        assert!(pointer_with_view(&mut app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, Some(&xf)));
        assert!(pointer_with_view(&mut app, ToolEvent::Up { x: to[0], y: to[1] }, Some(&xf)));
        let (start, end, flip) = placement(&app);
        assert!((start - 30.0).abs() < 0.01);
        assert_eq!((end, flip), (Some(100.0), false));
    }

    #[test]
    fn shear_flip_uses_perpendicular_screen_distance() {
        let mut app = app(Tool::PathSelection);
        let id = app.session.active().unwrap().active_layer.unwrap();
        app.session
            .edit("shear text", |doc, _| {
                let LayerContent::Text(text) = &mut doc.layer_mut(id).unwrap().content else { panic!("text") };
                text.transform = photocraft_doc::Affine { m: [1.0, 0.0, 10.0, 1.0, 0.0, 0.0] };
                Ok(())
            })
            .unwrap();
        let xf = ViewXform::active(&app).unwrap();
        let (point, tangent) = markers(active_text(&app).unwrap(), &xf).unwrap()[0];
        let from = xf.to_doc(marker_tip(point, tangent, 0));
        assert!(pointer_with_view(&mut app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Some(&xf)));
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, Some(0));
        // Local (40, 103) has a transformed normal displacement over 30 pixels,
        // but is only 3 pixels from the transformed baseline.
        let near = Point::new(40.0 + 10.0 * 103.0, 103.0);
        assert!(pointer_with_view(&mut app, ToolEvent::Move { x: near.x, y: near.y, pressure: 1.0 }, Some(&xf)));
        assert!(!app.ui.type_path_drag.as_ref().unwrap().flipped);
        let far = Point::new(40.0 + 10.0 * 105.0, 105.0);
        assert!(pointer_with_view(&mut app, ToolEvent::Move { x: far.x, y: far.y, pressure: 1.0 }, Some(&xf)));
        assert!(app.ui.type_path_drag.as_ref().unwrap().flipped);
        assert!(pointer_with_view(&mut app, ToolEvent::Up { x: far.x, y: far.y }, Some(&xf)));
        assert!(placement(&app).2);
    }

    #[test]
    fn preview_is_cached_and_does_not_change_history() {
        let mut app = app(Tool::PathSelection);
        let before = shape(&app);
        let n = steps(&app);
        down(&mut app, 40.0, 90.0);
        assert!(display_doc(&mut app, 0).is_none());
        move_to(&mut app, 50.0, 90.0);
        let (shown, key) = display_doc(&mut app, 0).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let LayerContent::Text(text) = &shown.layer(layer).unwrap().content else { panic!("expected text") };
        let TextShape::Path { start, .. } = &text.shape else { panic!("expected path") };
        assert_eq!(*start, 30.0);
        let (again, same_key) = display_doc(&mut app, 0).unwrap();
        assert_eq!(key, same_key);
        assert!(Arc::ptr_eq(&shown, &again));
        assert_eq!(shape(&app), before);
        assert_eq!(steps(&app), n);
        assert!(cancel(&mut app));
        assert!(display_doc(&mut app, 0).is_none());
        down(&mut app, 40.0, 90.0);
        move_to(&mut app, 50.0, 90.0);
        up(&mut app, 50.0, 90.0);
        assert!(display_doc(&mut app, 0).is_none());
    }

    #[test]
    fn damage_tracks_preview_and_return() {
        let mut app = app(Tool::PathSelection);
        let st = app.session.active().unwrap();
        let (doc, revision) = (st.doc.id, st.revision);
        down(&mut app, 40.0, 90.0);
        move_to(&mut app, 50.0, 90.0);
        let (_, first) = display_doc(&mut app, 0).unwrap();
        move_to(&mut app, 60.0, 90.0);
        let (_, second) = display_doc(&mut app, 0).unwrap();
        let history = app.type_path_damage.as_ref().unwrap();
        let original = history.original_area;
        let first_area = history.areas[0].1;
        let second_area = history.areas[1].1;
        assert_eq!(damage(&app, doc, revision, 0, first), Some(original.union(&first_area)));
        assert_eq!(damage(&app, doc, revision, first, second), Some(first_area.union(&second_area)));
        assert!(cancel(&mut app));
        assert_eq!(damage(&app, doc, revision, second, 0), Some(second_area.union(&original)));
        assert_eq!(damage(&app, doc, revision + 1, second, 0), None);
    }

    #[test]
    fn marker_drags_commit_once_and_undo() {
        for tool in [Tool::PathSelection, Tool::DirectSelection] {
            for (from, to, expected) in [([40.0, 90.0], [50.0, 90.0], (30.0, Some(100.0), false)), ([120.0, 110.0], [130.0, 110.0], (20.0, Some(110.0), false))]
            {
                let mut app = app(tool);
                let before = shape(&app);
                let n = steps(&app);
                down(&mut app, from[0], from[1]);
                move_to(&mut app, to[0], to[1]);
                assert_eq!(shape(&app), before);
                assert_eq!(steps(&app), n);
                up(&mut app, to[0], to[1]);
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

    fn body_point(app: &mut PhotocraftApp) -> Point {
        let id = app.session.active().unwrap().active_layer.unwrap();
        let (layout, _, _) = crate::type_tool::layout(app, id).unwrap();
        let cluster = layout.clusters.last().unwrap();
        let line = &layout.lines[cluster.line];
        let text = active_text(app).unwrap();
        let warp = photocraft_text::warp::Warp::on_path(&text.shape).unwrap();
        let (x, y) = warp.apply(f64::from(cluster.x + cluster.advance / 2.0), f64::from(line.baseline - line.ascent * 0.4));
        text.transform.apply(Point::new(x, y))
    }

    #[test]
    fn body_drag_can_move_text_when_range_reaches_the_open_endpoint() {
        let mut app = app(Tool::PathSelection);
        app.run("type.setPathPlacement", json!({"end": null})).unwrap();
        for expected in [30.0, 40.0] {
            let point = body_point(&mut app);
            down(&mut app, point.x, point.y);
            assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, None);
            move_to(&mut app, point.x + 10.0, point.y);
            up(&mut app, point.x + 10.0, point.y);
            assert_eq!(placement(&app), (expected, Some(160.0), false));
        }
    }

    #[test]
    fn body_drag_moves_range_once_and_undoes() {
        for tool in [Tool::PathSelection, Tool::DirectSelection] {
            let mut app = app(tool);
            let before = shape(&app);
            let n = steps(&app);
            let point = body_point(&mut app);
            let text = active_text(&app).unwrap();
            let xf = ViewXform::active(&app).unwrap();
            assert!(marker_hit(markers(text, &xf).unwrap(), xf.to_screen(point.x as f32, point.y as f32)).is_none());
            down(&mut app, point.x, point.y);
            assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, None);
            move_to(&mut app, point.x + 10.0, point.y);
            up(&mut app, point.x + 10.0, point.y);
            assert_eq!(placement(&app), (30.0, Some(110.0), false));
            let TextShape::Path { path: old, .. } = &before else { panic!("expected path") };
            let TextShape::Path { path: new, .. } = shape(&app) else { panic!("expected path") };
            assert_eq!(*old, new);
            assert_eq!(steps(&app), n + 1);
            assert!(app.session.undo());
            assert_eq!(shape(&app), before);
        }
    }

    #[test]
    fn position_lock_rejects_marker_and_command() {
        for tool in [Tool::PathSelection, Tool::DirectSelection] {
            let mut app = app(tool);
            let id = app.session.active().unwrap().active_layer.unwrap();
            app.session
                .edit("lock position", |doc, _| {
                    doc.layer_mut(id).unwrap().locks.position = true;
                    Ok(())
                })
                .unwrap();
            let before = shape(&app);
            let n = steps(&app);
            let xf = ViewXform::active(&app).unwrap();
            let (point, tangent) = markers(active_text(&app).unwrap(), &xf).unwrap()[0];
            let screen = marker_tip(point, tangent, 0);
            let doc = xf.to_doc(screen);
            assert!(!pointer(&mut app, ToolEvent::Down { x: doc[0], y: doc[1], pressure: 1.0 }));
            assert!(app.session.execute("type.setPathPlacement", json!({"start":30})).is_err());
            assert_eq!(shape(&app), before);
            assert_eq!(steps(&app), n);
        }
    }

    #[test]
    fn crossing_the_path_flips_and_remaps_the_interval() {
        for tool in [Tool::PathSelection, Tool::DirectSelection] {
            let mut app = app(tool);
            down(&mut app, 40.0, 90.0);
            move_to(&mut app, 40.0, 110.0);
            up(&mut app, 40.0, 110.0);
            assert_eq!(placement(&app), (60.0, Some(140.0), true));
        }
    }

    #[test]
    fn cancel_and_document_switch_discard_the_gesture() {
        for switch in [false, true] {
            let mut app = app(Tool::PathSelection);
            let before = shape(&app);
            let n = steps(&app);
            down(&mut app, 40.0, 90.0);
            move_to(&mut app, 50.0, 90.0);
            if switch {
                let source = app.session.active().unwrap().doc.clone();
                app.session.add_document((*source).clone(), None);
                let new_before = shape(&app);
                let new_steps = steps(&app);
                up(&mut app, 50.0, 90.0);
                assert_eq!(shape(&app), new_before);
                assert_eq!(steps(&app), new_steps);
                assert_eq!(app.session.documents()[0].history.entries().len(), n);
            } else {
                assert!(cancel(&mut app));
                assert_eq!(shape(&app), before);
                assert_eq!(steps(&app), n);
            }
        }
    }
    #[test]
    fn replacing_snapshot_with_same_id_and_revision_cancels_capture() {
        let mut app = app(Tool::PathSelection);
        let before = shape(&app);
        let count = steps(&app);
        down(&mut app, 40.0, 90.0);
        move_to(&mut app, 50.0, 90.0);
        let st = app.session.active_mut().unwrap();
        let id = st.doc.id;
        let revision = st.revision;
        let original = st.doc.clone();
        st.doc = Arc::new((*original).clone());
        assert_eq!(st.doc.id, id);
        assert_eq!(st.revision, revision);
        assert!(!Arc::ptr_eq(&st.doc, &original));
        cancel_stale(&mut app);
        assert!(app.ui.type_path_drag.is_none());
        assert!(!pointer(&mut app, ToolEvent::Up { x: 50.0, y: 90.0 }));
        assert_eq!(shape(&app), before);
        assert_eq!(steps(&app), count);
    }

    #[test]
    fn closed_range_over_one_lap_click_preserves_range_and_history() {
        let mut app = app(Tool::PathSelection);
        let st = app.session.active_mut().unwrap();
        let id = st.active_layer.unwrap();
        let layer = Arc::make_mut(&mut st.doc).layer_mut(id).unwrap();
        let LayerContent::Text(text) = &mut layer.content else { panic!("expected text") };
        text.shape = TextShape::Path {
            path: photocraft_doc::vector::Subpath::polygon(&[(20.0, 20.0), (180.0, 20.0), (180.0, 180.0), (20.0, 180.0)]),
            start: 20.0,
            end: Some(700.0),
            flip: false,
        };
        let before = shape(&app);
        let count = steps(&app);
        let xf = ViewXform::active(&app).unwrap();
        let ends = markers(active_text(&app).unwrap(), &xf).unwrap();
        let tip = marker_tip(ends[1].0, ends[1].1, 1);
        let point = xf.to_doc(tip);
        down(&mut app, point[0], point[1]);
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, Some(1));
        up(&mut app, point[0], point[1]);
        assert_eq!(shape(&app), before);
        assert_eq!(steps(&app), count);
    }

    #[test]
    fn pointer_crossing_gesture_stays_on_branch() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.sync_views();
        app.ui.tool = Tool::PathSelection;
        app.run("path.set", json!({"name":"work", "path":{"subpaths":[{"closed":true,"knots":[[20,20],[180,180],[20,180],[180,20]]}]}})).unwrap();
        app.ui.selected_path = Some("work".into());
        app.run("type.create", json!({"text":"ABC", "x":20, "y":20})).unwrap();
        app.run("type.attachToPath", json!({"start":20})).unwrap();
        app.run("type.setPathPlacement", json!({"end":80})).unwrap();
        app.ui.views[0].zoom = 1.0;

        let xf = ViewXform::active(&app).unwrap();
        let ends = markers(active_text(&app).unwrap(), &xf).unwrap();
        let tip = marker_tip(ends[1].0, ends[1].1, 1);
        let from = xf.to_doc(tip);
        assert!(pointer_with_view(&mut app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Some(&xf)));
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, Some(1));
        assert!(pointer_with_view(&mut app, ToolEvent::Move { x: 100.0, y: 100.0, pressure: 1.0 }, Some(&xf)));
        assert!(pointer_with_view(&mut app, ToolEvent::Move { x: 140.0, y: 140.0, pressure: 1.0 }, Some(&xf)));
        assert!(pointer_with_view(&mut app, ToolEvent::Up { x: 140.0, y: 140.0 }, Some(&xf)));

        let (start, end, flip) = placement(&app);
        assert_eq!(start, 20.0);
        let end = end.expect("explicit end");
        assert!((end - 169.71).abs() < 1.0, "end should stay on branch: {end}");
        assert!(!flip);
    }
}
