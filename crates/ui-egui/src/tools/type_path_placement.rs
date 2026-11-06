//! Placement markers for the active layer's type path.
use crate::{PhotocraftApp, canvas::ViewXform, state::Tool};
use std::{collections::VecDeque, sync::Arc};

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
    let layer = st.doc.layer(st.active_layer?)?;
    match &layer.content {
        LayerContent::Text(text) if matches!(text.shape, TextShape::Path { .. }) => Some(text),
        _ => None,
    }
}

fn effective_range(text: &TextLayer, geometry: &PathGeometry) -> Option<[f64; 2]> {
    use photocraft_doc::text::TextAlign;
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
        let factor = |align| match align {
            TextAlign::Center | TextAlign::JustifyCenter => 0.5,
            TextAlign::Right | TextAlign::JustifyRight => 1.0,
            _ => 0.0,
        };
        let f = factor(runs.first()?.style.align);
        if runs.iter().any(|run| factor(run.style.align) != f) {
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

fn overflows(app: &PhotocraftApp, ctx: &egui::Context, text: &TextLayer) -> bool {
    let Some(st) = app.session.active() else { return false };
    let TextShape::Path { start, end, flip, .. } = &text.shape else { return false };
    let key = (st.revision, start.to_bits(), end.map(f32::to_bits), *flip);
    let id = egui::Id::new(("path-text-overflow", st.doc.id.0, st.active_layer));
    if let Some((_, value)) = ctx.data(|data| data.get_temp::<((u64, u32, Option<u32>, bool), bool)>(id)).filter(|(cached, _)| *cached == key) {
        return value;
    }
    let mut engine = photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let layout = engine.layout(text, st.doc.resolution_dpi);
    let value = photocraft_text::render::path_overflows(&layout, &text.shape);
    ctx.data_mut(|data| data.insert_temp(id, (key, value)));
    value
}

pub(crate) fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(text) = active_text(app) else { return };
    let mut preview = text.clone();
    if let Some(drag) = app.ui.type_path_drag.as_ref().filter(|d| d.valid(app)) {
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
        let tip = point + normal * (10.0 * side);
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
    layer: photocraft_doc::LayerId,
    revision: u64,
    tool: Tool,
    inverse: photocraft_doc::Affine,
    geometry: PathGeometry,
    closed: bool,
    endpoint: Option<usize>,
    original: [f64; 2],
    proposed: [f64; 2],
    previous: f64,
    delta: f64,
    original_flip: bool,
    flipped: bool,
    initial_side: f64,
    transform: photocraft_doc::Affine,
    zoom: f64,
    preview: Option<Preview>,
}

// Geometry is fixed by the source document revision; cached preview pixels are derived state.
impl PartialEq for Drag {
    fn eq(&self, other: &Self) -> bool {
        (self.document, self.layer, self.revision, self.tool) == (other.document, other.layer, other.revision, other.tool)
            && self.inverse == other.inverse
            && self.closed == other.closed
            && self.endpoint == other.endpoint
            && self.original == other.original
            && self.proposed == other.proposed
            && self.previous == other.previous
            && self.delta == other.delta
            && self.original_flip == other.original_flip
            && self.flipped == other.flipped
            && self.initial_side == other.initial_side
            && self.transform == other.transform
            && self.zoom == other.zoom
    }
}

impl Drag {
    fn valid(&self, app: &PhotocraftApp) -> bool {
        active_text(app).is_some()
            && app.ui.tool == self.tool
            && app.session.active().is_some_and(|st| st.doc.id == self.document && st.active_layer == Some(self.layer) && st.revision == self.revision)
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
        let Some((arc, distance)) = self.geometry.project(point.x, point.y) else { return };
        let length = self.geometry.length();
        if let Some(((px, py), (tx, ty))) = self.geometry.sample(arc) {
            let a = self.transform.apply(Point::new(px, py));
            let b = self.transform.apply(Point::new(px - ty, py + tx));
            let screen_distance = distance * (b.x - a.x).hypot(b.y - a.y) * self.zoom;
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
        let [start, end] = self.original;
        self.proposed = if self.endpoint == Some(0) {
            [(start + self.delta).clamp(if self.closed { end - length } else { 0.0 }, end), end]
        } else if self.endpoint == Some(1) {
            [start, (end + self.delta).clamp(start, if self.closed { start + length } else { length })]
        } else {
            let delta = if self.closed { self.delta } else { self.delta.clamp(-start, length - end) };
            [start + delta, end + delta]
        };
    }
}

pub(crate) fn cancel(app: &mut PhotocraftApp) -> bool {
    app.ui.type_path_drag.take().is_some()
}

pub(crate) fn pointer(app: &mut PhotocraftApp, event: crate::canvas::ToolEvent) -> bool {
    use crate::canvas::ToolEvent;
    if let Some(mut drag) = app.ui.type_path_drag.take() {
        if !drag.valid(app) {
            return true;
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
    let zoom = f64::from(app.point_zoom().max(0.01));
    let mut best = None;
    for (index, arc) in original.into_iter().enumerate() {
        let Some(((px, py), (tx, ty))) = geometry.sample(arc) else { continue };
        let a = text.transform.apply(Point::new(px, py));
        let b = text.transform.apply(Point::new(px - ty, py + tx));
        let normal_length = (b.x - a.x).hypot(b.y - a.y);
        if normal_length <= 1e-9 {
            continue;
        }
        let side = if index == 0 { -1.0 } else { 1.0 };
        let tip = Point::new(a.x + (b.x - a.x) / normal_length * side * 10.0 / zoom, a.y + (b.y - a.y) / normal_length * side * 10.0 / zoom);
        let distance = (tip.x - x).hypot(tip.y - y) * zoom;
        if distance <= 6.0 && best.is_none_or(|(_, d)| distance < d) {
            best = Some((index, distance));
        }
    }
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
    if st.doc.effective_locks(layer).all {
        return false;
    }
    let point = inverse.apply(Point::new(x, y));
    let Some((previous, distance)) = geometry.project(point.x, point.y) else { return false };
    app.ui.type_path_drag = Some(Drag {
        document: st.doc.id,
        layer,
        revision: st.revision,
        tool: app.ui.tool,
        inverse,
        geometry,
        closed: path.closed,
        endpoint,
        original,
        proposed: original,
        previous,
        delta: 0.0,
        original_flip: *flip,
        flipped: false,
        initial_side: if distance.abs() > 1e-6 { distance.signum() } else { -1.0 },
        transform: text.transform,
        zoom,
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
        app.run("type.create", json!({"text":"ABC", "x":20, "y":100})).unwrap();
        app.run("type.attachToPath", json!({"start":20})).unwrap();
        app.run("type.setPathPlacement", json!({"end":100})).unwrap();
        app.ui.selected_path = None;
        app.ui.views[0].zoom = 1.0;
        app
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
}
