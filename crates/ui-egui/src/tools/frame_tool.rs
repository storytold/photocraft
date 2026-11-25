//! Frame Tool: persistent placeholders and independently selectable frame/image geometry.
//!
//! The canvas runs `tool`/`drain` for `Family::Frame`. The Frame-target box is a framework `TransformBox`
//! shown by `show`; a Content or Both press opens Free Transform and drives its box.
use crate::{
    PhotocraftApp,
    canvas::{
        document_display::CanvasHost,
        tools::{FamilyTool, ToolScope},
    },
    state::{FreeTransform, Tool},
    theme::Tokens,
};
use craft::behavior::{
    CancelReason, Cursor, CursorIcon, EditEvent, GestureId, IconId, Mixed, PointerButton,
    geometry::{BoxAllow, BoxPart, BoxRules, BoxState, ConstrainStyle, Directions, MoveGesture, ScaleRule, drag as box_drag},
    kurbo::{Point as KPoint, Rect as KRect},
};
use craft::draw::ui::{
    ShapeDragEvent, ShapeDragOptions, ShapeDragTool, ShapeKind,
    transform::{TransformBox, TransformEdit},
};
use craft::keymap::Mods;
use craft::ui::canvas::{CanvasTool, ToolAction, ToolClaims, ToolContext, ToolHostResponse, ToolKey, ToolPath, ToolPointer};
use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, Entry, Show};
use craft::ui::display::Divider;
use craft::ui::primitives::Draw;
use craft::ui::text::NumberField;
use egui::{Pos2, Stroke, StrokeKind};
use photocraft_doc::{Frame, Layer, LayerContent, LayerId};
use photocraft_geom::{Affine, Point, Rect};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

mod properties;
mod stroke;
pub(crate) mod task_bar;
pub(crate) use properties::properties;

/// Which part of a frame the tool edits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Target {
    /// The frame's clip.
    #[default]
    Frame,
    /// The frame's image.
    Content,
    /// The frame and its image together.
    Both,
}

/// The Frame tool's options and selection, saved with the UI state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FrameToolUi {
    /// Kept for old saved UI state; the current shape selector also supports polygons/custom shapes.
    pub ellipse: bool,
    /// The shape new frames take.
    pub shape: String,
    /// The polygon's sides.
    pub sides: u32,
    /// The stroke new frames take.
    pub stroke: Option<photocraft_doc::ShapeStroke>,
    /// The part of the selected frame the tool edits.
    pub target: Target,
    /// The selected frame's layer.
    pub selected: Option<u64>,
    /// Set while a frame press runs Free Transform and commits it on release.
    #[serde(skip)]
    pub auto_transform: bool,
}
impl Default for FrameToolUi {
    fn default() -> Self {
        Self { ellipse: false, shape: "rectangle".into(), sides: 5, stroke: None, target: Target::Frame, selected: None, auto_transform: false }
    }
}

pub(crate) struct Preview {
    doc: Arc<photocraft_doc::Document>,
    revision: u64,
    key: u64,
}

/// A body drag of a Frame-target frame.
struct FrameMove {
    layer: LayerId,
    original: Affine,
    gesture: MoveGesture,
}
impl FrameMove {
    /// The frame transform after moving by `delta`, rounded to whole document pixels.
    fn proposed(&self, delta: craft::behavior::kurbo::Vec2) -> Affine {
        // The original transform carries the frame's own shape; the move only adds a whole-pixel translation.
        Affine::translate(delta.x.round(), delta.y.round()).mul(&self.original)
    }
}

/// A press that opened Free Transform for a Content or Both target; its drag edits the open session's box.
struct ContentPress {
    id: GestureId,
    start: KPoint,
    origin: BoxState,
    last: BoxState,
    part: BoxPart,
    begun: bool,
}

/// The Frame tool's state across passes: the shape drag and the gesture that owns the press.
pub(crate) struct FrameTool {
    draw: ShapeDragTool,
    moving: Option<FrameMove>,
    content: Option<ContentPress>,
    next_gesture: GestureId,
}
impl Default for FrameTool {
    fn default() -> Self {
        Self { draw: ShapeDragTool::new(), moving: None, content: None, next_gesture: GestureId::new(1) }
    }
}
impl FrameTool {
    #[cfg(test)]
    /// Whether a gesture of the tool runs.
    pub(crate) fn busy(&self) -> bool {
        self.moving.is_some() || self.content.is_some() || self.draw.shape().is_some()
    }
    fn next_id(&mut self) -> GestureId {
        let id = self.next_gesture;
        self.next_gesture = id.next();
        id
    }
}

pub fn selected(app: &PhotocraftApp) -> Option<&Layer> {
    let st = app.session.active()?;
    let id = photocraft_engine::frame_cmds::frame_of(&st.doc, st.active_layer?)?;
    st.doc.layer(id)
}
fn path_visible(doc: &photocraft_doc::Document, path: &[usize]) -> bool {
    (1..=path.len()).all(|n| path.get(..n).and_then(|p| doc.layer_at(p)).is_some_and(|l| l.visible))
}
fn visible(app: &PhotocraftApp, id: LayerId) -> bool {
    app.session.active().is_some_and(|st| st.doc.path_of(id).is_some_and(|p| path_visible(&st.doc, &p)))
}
fn content_layer(app: &PhotocraftApp, r: [i64; 4]) -> Option<u64> {
    let st = app.session.active()?;
    st.doc.walk().into_iter().rev().find_map(|(path, _, l)| {
        if !matches!(l.content, LayerContent::Raster(_) | LayerContent::Smart(_)) || st.doc.is_background(l.id) || !path_visible(&st.doc, &path) {
            return None;
        }
        let locks = st.doc.effective_locks(l.id);
        if locks.position || locks.all || photocraft_engine::frame_cmds::frame_of(&st.doc, l.id).is_some() {
            return None;
        }
        let b = photocraft_engine::transform_cmds::transform_bounds(&st.doc, l);
        (!b.is_empty() && i64::from(b.x0) < r[2] && r[0] < i64::from(b.x1) && i64::from(b.y0) < r[3] && r[1] < i64::from(b.y1)).then_some(l.id.0)
    })
}
#[cfg(test)]
mod content_pick_tests;

fn report(app: &mut PhotocraftApp, r: Result<Value, String>) {
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}
pub(crate) fn select(app: &mut PhotocraftApp, id: LayerId, target: Target) {
    let r = select_result(app, id, target);
    if let Err(e) = r {
        report(app, Err(e));
    }
}
pub(crate) fn select_result(app: &mut PhotocraftApp, id: LayerId, target: Target) -> Result<(), String> {
    let child = app.session.active().and_then(|st| st.doc.layer(id)).and_then(|l| l.children()).and_then(|c| c.first()).map(|l| l.id);
    let layer = if target == Target::Content { child.unwrap_or(id) } else { id };
    app.run("layer.select", json!({"layer":layer.0}))?;
    app.ui.frame_tool.target = if child.is_none() { Target::Frame } else { target };
    app.ui.frame_tool.selected = Some(id.0);
    app.ui.mask_target = false;
    app.ui.vector_mask_target = false;
    app.ui.selected_path = None;
    Ok(())
}
pub(crate) fn target(app: &PhotocraftApp, l: &Layer) -> Target {
    if app.ui.frame_tool.selected != Some(l.id.0) {
        if app.session.active().is_some_and(|st| st.active_layer != Some(l.id)) { Target::Content } else { Target::Frame }
    } else {
        app.ui.frame_tool.target
    }
}
/// The shape the options bar has chosen.
fn current_shape(f: &FrameToolUi) -> &str {
    if f.ellipse {
        "ellipse"
    } else if f.shape.is_empty() {
        "rectangle"
    } else {
        &f.shape
    }
}
fn kind_of(shape: &str) -> ShapeKind {
    match shape {
        "ellipse" => ShapeKind::Ellipse,
        "triangle" => ShapeKind::Triangle,
        "polygon" => ShapeKind::Polygon,
        // A custom shape is sized by a box drag like a rectangle.
        _ => ShapeKind::Rectangle,
    }
}
fn shape_params(app: &PhotocraftApp, r: [i64; 4]) -> Value {
    let [x, y, x1, y1] = r;
    let shape = current_shape(&app.ui.frame_tool);
    let mut p = json!({"x":x,"y":y,"width":x1.saturating_sub(x),"height":y1.saturating_sub(y),"shape":shape,"sides":app.ui.frame_tool.sides.clamp(3,100)});
    if shape == "custom" {
        let name = app.ui.presets_ui.shape();
        if let Some(sh) = photocraft_engine::presets::shapes::all_groups(&app.session).iter().flat_map(|g| &g.items).find(|s| s.name == name) {
            p["path"] = photocraft_engine::vector_cmds::path_json(&sh.path);
        }
    }
    if let Some(s) = &app.ui.frame_tool.stroke {
        p["stroke"] = stroke::json(s);
    }
    p
}
fn on_border(f: &Frame, p: [f64; 2], zoom: f64) -> bool {
    let path = f.outline();
    photocraft_vector::flatten::flatten_path(&path, 0.5 / zoom.max(0.01)).iter().any(|line| {
        let segments = line.pts.windows(2).filter_map(|s| Some((*s.first()?, *s.get(1)?)));
        let closing = line.closed.then(|| Some((*line.pts.last()?, *line.pts.first()?))).flatten();
        segments.chain(closing).any(|((ax, ay), (bx, by))| {
            let (dx, dy) = (bx - ax, by - ay);
            let n = dx * dx + dy * dy;
            let t = if n > 0.0 { ((p[0] - ax) * dx + (p[1] - ay) * dy) / n } else { 0.0 }.clamp(0.0, 1.0);
            (p[0] - ax - t * dx).hypot(p[1] - ay - t * dy) * zoom <= 5.0
        })
    })
}
fn inside(f: &Frame, p: [f64; 2]) -> bool {
    let (x, y) = (p[0].floor() as i32, p[1].floor() as i32);
    let (Some(x1), Some(y1)) = (x.checked_add(1), y.checked_add(1)) else { return false };
    photocraft_vector::path_coverage(&f.outline(), Rect::new(x, y, x1, y1)).first().is_some_and(|v| *v > 0.0)
}
pub(crate) fn frame_at(app: &PhotocraftApp, p: [f64; 2]) -> Option<LayerId> {
    let st = app.session.active()?;
    let zoom = f64::from(app.point_zoom());
    st.doc
        .walk()
        .into_iter()
        .rev()
        .find(|(path, _, l)| {
            path_visible(&st.doc, path)
                && l.frame.as_ref().is_some_and(|f| {
                    let b = f.bounds().inflate((20.0 / zoom.max(0.01)).ceil() as i32);
                    p[0] >= f64::from(b.x0)
                        && p[0] <= f64::from(b.x1)
                        && p[1] >= f64::from(b.y0)
                        && p[1] <= f64::from(b.y1)
                        && (inside(f, p) || on_border(f, p, zoom))
                })
        })
        .map(|(_, _, l)| l.id)
}

/// The unit square through the frame transform, as a box with its reference point at the centre.
fn frame_box(m: &Affine) -> BoxState {
    let q = [(0., 0.), (1., 0.), (1., 1.), (0., 1.)].map(|(x, y)| {
        let p = m.apply(Point::new(x, y));
        KPoint::new(p.x, p.y)
    });
    BoxState::new(q, KPoint::new((q[0].x + q[2].x) / 2., (q[0].y + q[2].y) / 2.))
}
/// The frame transform that maps the unit square onto the box (corners 0, 1 and 3 fix an affine map).
fn frame_affine(b: &BoxState) -> Affine {
    let [q0, q1, _, q3] = [b.quad[0], b.quad[1], b.quad[2], b.quad[3]];
    Affine { m: [q1.x - q0.x, q1.y - q0.y, q3.x - q0.x, q3.y - q0.y, q0.x, q0.y] }
}
/// The frame's first child's bounds as a box.
fn child_box(app: &PhotocraftApp, id: LayerId) -> Option<BoxState> {
    let st = app.session.active()?;
    let child = st.doc.layer(id)?.children()?.first()?;
    let b = photocraft_engine::transform_cmds::transform_frame(&st.doc, child);
    Some(BoxState::from_rect(KRect::new(f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1))))
}

/// The preview's transform of frame `id`, while a gesture shows one for the current revision.
fn shown_transform(app: &PhotocraftApp, id: LayerId) -> Option<Affine> {
    let st = app.session.active()?;
    let p = app.frame_preview.as_ref().filter(|p| p.doc.id == st.doc.id && p.revision == st.revision)?;
    p.doc.layer(id)?.frame.as_ref().map(|f| f.transform)
}

fn update_preview(app: &mut PhotocraftApp, layer: LayerId, proposed: Affine) {
    let Some(st) = app.session.active() else {
        app.frame_preview = None;
        return;
    };
    let revision = st.revision;
    let mut shown = (*st.doc).clone();
    if let Some(l) = shown.layer_mut(layer) {
        if let Some(f) = &mut l.frame {
            f.transform = proposed;
            if !f.valid() {
                app.frame_preview = None;
                return;
            }
        }
        photocraft_engine::frame_cmds::refresh(l);
    }
    let key = app.frame_preview.as_ref().map_or(1, |p| p.key.wrapping_add(1));
    app.frame_preview = Some(Preview { doc: Arc::new(shown), revision, key });
}
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<photocraft_doc::Document>, u64)> {
    let live = app.ui.transform.as_ref().filter(|t| frame_transform(t)).map(|t| (t.id, t.layer, transformed_frame(app, t)));
    if let Some((id, layer, transform)) = live {
        let transform = transform.ok()?;
        let st = app.session.documents().get(idx)?;
        if app.transform_preview.as_ref().is_none_or(|pv| pv.doc.id != st.doc.id) {
            return None;
        }
        use std::hash::Hasher;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        hash.write_u64(id);
        for value in transform.m {
            hash.write_u64(value.to_bits());
        }
        let key = hash.finish();
        if app.frame_preview.as_ref().is_none_or(|p| p.doc.id != st.doc.id || p.revision != st.revision || p.key != key) {
            let mut doc = (*st.doc).clone();
            let l = doc.layer_mut(LayerId(layer))?;
            l.frame.as_mut()?.transform = transform;
            photocraft_engine::frame_cmds::refresh(l);
            app.frame_preview = Some(Preview { doc: Arc::new(doc), revision: st.revision, key });
        }
    }
    let st = app.session.documents().get(idx)?;
    let p = app.frame_preview.as_ref()?;
    (app.ui.tool == Tool::Frame && st.doc.id == p.doc.id && st.revision == p.revision).then(|| (p.doc.clone(), (1 << 57) | p.key))
}
/// Drops the live preview (a tool change, a reset).
pub fn clear(app: &mut PhotocraftApp) {
    app.frame_preview = None;
}

// ---- the canvas tool ----

/// What a press of the Frame tool turned into.
enum Pressed {
    /// Nothing of a frame: the shape drag takes the press.
    Draw,
    /// Handled whole.
    Done,
    Move(FrameMove),
    Content(ContentPress),
}

/// The Frame tool's press rules: the double-click toggle, target selection, and what each target does.
fn press_rule(app: &mut PhotocraftApp, at: KPoint, clicks: u32, reach: f64, style: &ConstrainStyle, mods: Mods) -> Pressed {
    app.frame_preview = None;
    if !at.x.is_finite() || !at.y.is_finite() || at.x.abs() > 600_000.0 || at.y.abs() > 600_000.0 {
        app.ui.status = tl!("Frame coordinates must be finite and within 600000 pixels").into();
        app.ui.status_error = true;
        return Pressed::Done;
    }
    let p = [at.x, at.y];
    // A repeated press with no frame under it is an ordinary press: a drag from where the last click was still draws.
    if clicks >= 2
        && let Some(id) = frame_at(app, p)
    {
        let next = if selected(app).is_some_and(|l| l.id == id && target(app, l) == Target::Content) { Target::Both } else { Target::Content };
        select(app, id, next);
        return Pressed::Done;
    }
    let zoom = f64::from(app.point_zoom());
    let active = selected(app).map(|l| (l.id, target(app, l)));
    // A handle of the selected image's bounds reaches outside the frame.
    let content_control = active.is_some_and(|(id, t)| {
        let reference_point = app.ui.tool_options.transform.show_reference_point;
        t == Target::Content
            && child_box(app, id).is_some_and(|b| matches!(content_part(&b, at, reach, reference_point), BoxPart::Corner(_) | BoxPart::Edge(_)))
    });
    let Some(id) = content_control.then(|| active.map(|(id, _)| id)).flatten().or_else(|| frame_at(app, p)) else { return Pressed::Draw };
    let (frame, t) = {
        let Some(st) = app.session.active() else { return Pressed::Done };
        let Some(l) = st.doc.layer(id) else { return Pressed::Done };
        let Some(f) = l.frame.clone() else { return Pressed::Done };
        let locks = st.doc.effective_locks(id);
        if locks.all || locks.position {
            app.ui.status = tl!("The frame is locked").into();
            app.ui.status_error = true;
            return Pressed::Done;
        }
        let t = if content_control {
            Target::Content
        } else if on_border(&f, p, zoom) || l.children().is_none_or(|c| c.is_empty()) {
            Target::Frame
        } else if active.is_some_and(|(prev, t)| prev == id && t == Target::Content) {
            Target::Content
        } else {
            Target::Both
        };
        (f, t)
    };
    select(app, id, t);
    if t != Target::Frame {
        let ctx = app.ctx().clone();
        return match crate::transform_tool::begin_from_frame(app, &ctx) {
            Ok(()) => {
                let Some(t) = app.ui.transform.as_ref() else { return Pressed::Done };
                let origin = *t.session.state();
                let part = content_part(&origin, at, reach, t.session.reference_point());
                Pressed::Content(ContentPress { id: GestureId::new(0), start: at, origin, last: origin, part, begun: false })
            }
            Err(e) => {
                app.ui.status = e;
                app.ui.status_error = true;
                Pressed::Done
            }
        };
    }
    Pressed::Move(FrameMove { layer: id, original: frame.transform, gesture: MoveGesture::press(style, at, mods, Directions::Eight) })
}

/// The part of the image's box under `at`, as the Free Transform box takes presses: the reference point is a part only
/// while Show Reference Point is on; hidden, the middle of the box is inside, so a press there moves the image
/// (production `box_hit`).
fn content_part(state: &BoxState, at: KPoint, reach: f64, reference_point: bool) -> BoxPart {
    if reference_point {
        return state.part_at(at, reach);
    }
    let q = state.quad;
    let corners = (0..4u8).map(|i| (q[usize::from(i)], BoxPart::Corner(i)));
    let edges = (0..4u8).map(|i| (q[usize::from(i)].midpoint(q[(usize::from(i) + 1) % 4]), BoxPart::Edge(i)));
    let nearest = corners.chain(edges).map(|(p, part)| (p.distance(at), part)).filter(|(d, _)| *d < reach).min_by(|a, b| a.0.total_cmp(&b.0));
    match nearest {
        Some((_, part)) => part,
        None if state.contains(at) => BoxPart::Inside,
        None => BoxPart::Outside,
    }
}

/// The cursor icon over document point `p`, with `reach` document units of handle slop (#13883):
/// a selected Frame-target frame's handles resize, the frame itself is the arrow, and empty canvas
/// has no icon (the tool's crosshair).
fn cursor_icon(app: &PhotocraftApp, p: [f64; 2], reach: f64) -> Option<CursorIcon> {
    if let Some(l) = selected(app)
        && visible(app, l.id)
        && let Some(f) = &l.frame
        && target(app, l) == Target::Frame
        && !app.session.active().is_some_and(|st| {
            let locks = st.doc.effective_locks(l.id);
            locks.all || locks.position
        })
    {
        let state = frame_box(&f.transform);
        match content_part(&state, KPoint::new(p[0], p[1]), reach, false) {
            BoxPart::Corner(i) => {
                // The corner's diagonal on screen, not its index: a turned frame swaps them.
                let (a, b) = (state.quad[usize::from(i)], state.quad[(usize::from(i) + 2) % 4]);
                let same = (b.x - a.x) * (b.y - a.y) >= 0.0;
                return Some(if same { CursorIcon::ResizeNwSe } else { CursorIcon::ResizeNeSw });
            }
            BoxPart::Edge(i) => {
                // The edge's on-screen direction, not its index: a turned frame swaps them.
                let (a, b) = (state.quad[usize::from(i)], state.quad[(usize::from(i) + 1) % 4]);
                let horizontal = (b.x - a.x).abs() >= (b.y - a.y).abs();
                return Some(if horizontal { CursorIcon::ResizeVertical } else { CursorIcon::ResizeHorizontal });
            }
            _ => {}
        }
    }
    frame_at(app, p).map(|_| CursorIcon::Default)
}

/// A drag of the open Free Transform's box, as the box's own gesture: Begin at the first move, then Updates.
fn drag_content(app: &mut PhotocraftApp, c: &mut ContentPress, to: KPoint, mods: Mods, time: f64) {
    let Some(t) = app.ui.transform.as_mut() else { return };
    let next = box_drag(&c.origin, &c.last, c.part, c.start, to, mods, &BoxRules::default());
    if !c.begun {
        c.begun = true;
        let begin = EditEvent::begin(c.id, Mixed::Same(c.origin), Arc::from(tl!("Free Transform")), None);
        let _ = t.session.apply(TransformEdit::Box(begin), time);
    }
    c.last = next;
    let _ = t.session.apply(TransformEdit::Box(EditEvent::update(c.id, next)), time);
}

/// The release of a content press: the box's Commit (when it moved) and the automatic commit of the session.
fn release_content(app: &mut PhotocraftApp, c: &ContentPress, to: KPoint, mods: Mods) {
    if !c.begun {
        // A click moved nothing: the session its press opened ends with the press, as the release of any press that
        // opened one. The untouched box closes without a step.
        if app.ui.frame_tool.auto_transform {
            app.ui.frame_tool.auto_transform = false;
            crate::transform_tool::commit(app);
        }
        return;
    }
    let after = box_drag(&c.origin, &c.last, c.part, c.start, to, mods, &BoxRules::default());
    let edits = [TransformEdit::Box(EditEvent::commit(c.id, Mixed::Same(c.origin), after))];
    crate::transform_tool::auto_commit(app, &edits);
}

fn cancel_content(app: &mut PhotocraftApp, c: &ContentPress, reason: CancelReason, time: f64) {
    if c.begun
        && let Some(t) = app.ui.transform.as_mut()
    {
        let _ = t.session.apply(TransformEdit::Box(EditEvent::cancel(c.id, reason, Some(Mixed::Same(c.origin)))), time);
    }
    // Ends the session and clears its automatic commit.
    crate::transform_tool::cancel(app);
}

/// Arrow keys respect which thumbnail is selected; Shift moves ten document pixels. A run is one step.
fn nudge(app: &mut PhotocraftApp, dx: f64, dy: f64, mods: Mods, now: f64) -> bool {
    if app.ui.tool != Tool::Frame || app.ui.transform.is_some() || mods.cmd || mods.ctrl || mods.alt {
        return false;
    }
    let Some(l) = selected(app) else { return false };
    let (id, t, frame, child) = (l.id, target(app, l), l.frame.clone(), l.children().and_then(|c| c.first()).map(|l| l.id));
    let k = if mods.shift { 10. } else { 1. };
    let (dx, dy) = (dx.round() * k, dy.round() * k);
    app.nudge_bundle.now = now;
    let (command, params) = if t == Target::Frame {
        let Some(mut f) = frame else { return true };
        f.transform = Affine::translate(dx, dy).mul(&f.transform);
        ("layer.frame.set", json!({"layer":id.0,"transform":f.transform.m}))
    } else {
        ("layer.translate", json!({"layer":if t==Target::Content{child.unwrap_or(id).0}else{id.0},"dx":dx,"dy":dy}))
    };
    let r = crate::nudge_bundle::run(app, "frame", command, params);
    report(app, r);
    true
}

/// One pass of the Frame tool over the app.
struct FramePass<'a> {
    scope: &'a ToolScope<'a>,
    state: &'a mut FrameTool,
}

/// The Frame family's tool for one pass.
pub(crate) fn tool<'a>(scope: &'a ToolScope<'a>, state: &'a mut FrameTool, _host: CanvasHost) -> FamilyTool<'a> {
    let (shape, sides) = scope.with_app(|app| (current_shape(&app.ui.frame_tool).to_string(), app.ui.frame_tool.sides.clamp(3, 100)));
    state.draw.options(ShapeDragOptions::new(kind_of(&shape)).sides(sides));
    Box::new(FramePass { scope, state })
}

/// Makes the frames the shape drag committed this pass.
pub(crate) fn drain(app: &mut PhotocraftApp, state: &mut FrameTool, _response: &ToolHostResponse) {
    for event in state.draw.take_events() {
        let ShapeDragEvent::Draw(EditEvent::Commit { after, .. }) = event else { continue };
        make_frame(app, after.bounds());
    }
}

/// Makes a frame in the drawn box, around the active layer's image when the box overlaps it, and selects it.
fn make_frame(app: &mut PhotocraftApp, b: KRect) {
    let r = [b.x0, b.y0, b.x1, b.y1].map(|v| v.round() as i64);
    if r[2] <= r[0] || r[3] <= r[1] {
        return;
    }
    let mut p = shape_params(app, r);
    if let Some(id) = content_layer(app, r) {
        p["layer"] = json!(id);
    }
    match app.run("layer.new.frame", p) {
        Ok(v) => {
            if let Some(id) = v.get("layer").and_then(Value::as_u64) {
                select(app, LayerId(id), Target::Frame);
            }
        }
        Err(e) => report(app, Err(e)),
    }
}

impl CanvasTool for FramePass<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(view) = cx.space().map(|space| space.view) else { return ToolAction::Ignored };
        let at = view.to_doc(pointer.pos);
        let (reach, scope, mods, clicks) = (view.doc_len(cx.reach()), self.scope, pointer.mods, pointer.clicks);
        let style = ConstrainStyle::new(f64::from(cx.slop), f64::from(cx.slop));
        match scope.with_app(|app| press_rule(app, at, clicks, reach, &style, mods)) {
            Pressed::Draw => self.state.draw.press(pointer, cx),
            Pressed::Done => ToolAction::Done,
            Pressed::Move(m) => {
                self.state.moving = Some(m);
                ToolAction::Claimed
            }
            Pressed::Content(mut c) => {
                c.id = self.state.next_id();
                self.state.content = Some(c);
                ToolAction::Claimed
            }
        }
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let (scope, time) = (self.scope, cx.time);
        let Some(view) = cx.space().map(|space| space.view) else { return };
        let at = view.to_doc(pointer.pos);
        if let Some(c) = self.state.content.as_mut() {
            scope.with_app(|app| drag_content(app, c, at, pointer.mods, time));
        } else if let Some(m) = self.state.moving.as_mut() {
            if let Some(value) = m.gesture.follow(at, pointer.mods, view.zoom) {
                let (layer, proposed) = (m.layer, m.proposed(value.delta));
                scope.with_app(|app| update_preview(app, layer, proposed));
            }
        } else {
            self.state.draw.drag(pointer, cx);
        }
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let scope = self.scope;
        let Some(view) = cx.space().map(|space| space.view) else { return };
        let at = view.to_doc(pointer.pos);
        if let Some(c) = self.state.content.take() {
            scope.with_app(|app| release_content(app, &c, at, pointer.mods));
        } else if let Some(m) = self.state.moving.take() {
            let FrameMove { layer, original, gesture } = m;
            let value = gesture.release(at, pointer.mods, view.zoom);
            let transform = Affine::translate(value.delta.x.round(), value.delta.y.round()).mul(&original);
            scope.with_app(|app| {
                app.frame_preview = None;
                // A click with no drag leaves the transform as it was.
                if transform != original {
                    let mut p = json!({"layer":layer.0,"transform":transform.m});
                    // Moving an empty frame is a "Move" step (#13880); a frame with an image keeps "Frame Properties".
                    let empty = app.session.active().and_then(|st| st.doc.layer(layer)).is_some_and(|l| l.children().is_none_or(<[Layer]>::is_empty));
                    if empty {
                        p["historyName"] = json!("Move");
                    }
                    let r = app.run("layer.frame.set", p);
                    report(app, r);
                }
            });
        } else {
            self.state.draw.release(pointer, cx);
        }
    }

    fn cancel(&mut self, reason: CancelReason, cx: &mut ToolContext<'_>) {
        let (scope, time) = (self.scope, cx.time);
        let content = self.state.content.take();
        self.state.moving = None;
        // Escape while a frame is drawn makes it as drawn, as the reference application does (#13866); a drag still
        // inside the click distance has no shape and makes nothing.
        let drawn = (reason == CancelReason::Escape).then(|| self.state.draw.shape().map(|shape| shape.bounds())).flatten();
        scope.with_app(|app| {
            app.frame_preview = None;
            if let Some(c) = &content {
                cancel_content(app, c, reason, time);
            }
            if let Some(b) = drawn {
                make_frame(app, b);
            }
        });
        self.state.draw.cancel(reason, cx);
    }

    /// ⌘ is the Move tool's stand-in: with it held the press belongs to the stand-in, not to a frame.
    fn takes(&self, button: PointerButton, mods: Mods) -> bool {
        button == PointerButton::Primary && !mods.cmd
    }

    fn claims(&self) -> ToolClaims {
        let nudges = self.scope.with_app(|app| app.ui.tool == Tool::Frame && app.ui.transform.is_none() && selected(app).is_some());
        if nudges { ToolClaims::NONE.nudges() } else { ToolClaims::NONE }
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        let ToolKey::Nudge { direction, mods, .. } = key else { return false };
        let (dx, dy, mods, now) = (direction.x as f64, direction.y as f64, *mods, cx.time);
        self.scope.with_app(|app| nudge(app, dx, dy, mods, now))
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        self.state.draw.paint(draw, cx);
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        let Some((p, space)) = cx.pointer.as_ref().zip(cx.space()) else { return self.state.draw.cursor(cx) };
        let at = space.view.to_doc(p.pos);
        let reach = space.view.doc_len(cx.reach());
        match self.scope.with_app(|app| cursor_icon(app, [at.x, at.y], reach)) {
            Some(icon) => Cursor::System(icon),
            None => self.state.draw.cursor(cx),
        }
    }

    fn command_form(&self) -> ToolPath {
        self.state.draw.command_form()
    }
}

// ---- the canvas body ----

fn paint_path(painter: &egui::Painter, view: &craft::behavior::CanvasView, path: &photocraft_doc::Path, stroke: Stroke) {
    for line in photocraft_vector::flatten::flatten_path(path, 0.5 / view.zoom.max(0.01)) {
        let mut points: Vec<Pos2> = line.pts.iter().map(|&(x, y)| view.to_screen(KPoint::new(x, y))).collect();
        if line.closed
            && let Some(first) = points.first().copied()
        {
            points.push(first);
        }
        painter.add(egui::Shape::line(points, stroke));
    }
}
fn paint_handle(p: &egui::Painter, at: Pos2, t: &Tokens) {
    let r = egui::Rect::from_center_size(at, egui::vec2(6., 6.));
    p.rect_filled(r, 0., t.card);
    p.rect_stroke(r, 0., Stroke::new(1., t.accent), StrokeKind::Inside);
}

/// Shows the Frame tool's marks in the canvas body: placeholders of empty frames for every tool, and for the Frame
/// tool the selected frame's outline with its box (Frame target), its image's bounds (Content) or its outline (Both).
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: egui::Rect) {
    let Some(view) = crate::canvas::document_display::active_view(app) else { return };
    let t = Tokens::get(ui.ctx());
    let painter = ui.painter().clone();
    if let Some(st) = app.session.active() {
        for id in app.doc_queries.empty_frames(&app.session, st).iter() {
            let Some(f) = st.doc.layer(*id).and_then(|l| l.frame.as_ref()) else { continue };
            let q = f.corners().map(|p| view.to_screen(KPoint::new(p.x, p.y)));
            paint_path(&painter, &view, &f.outline(), Stroke::new(1., t.text_faint));
            painter.line_segment([q[0], q[2]], Stroke::new(1., t.text_faint));
            painter.line_segment([q[1], q[3]], Stroke::new(1., t.text_faint));
        }
    }
    if app.ui.tool != Tool::Frame || app.ui.transform.is_some() {
        return;
    }
    let Some(l) = selected(app) else { return };
    if !visible(app, l.id) {
        return;
    }
    let (id, tgt) = (l.id, target(app, l));
    let Some(mut f) = l.frame.clone() else { return };
    let original = f.transform;
    if let Some(shown) = shown_transform(app, id) {
        f.transform = shown;
    }
    if !f.valid() {
        return;
    }
    let stroke = Stroke::new(1.5, t.accent);
    paint_path(&painter, &view, &f.outline(), stroke);
    match tgt {
        Target::Both => {}
        Target::Content => {
            if let Some(b) = child_box(app, id) {
                let r = egui::Rect::from_two_pos(view.to_screen(b.quad[0]), view.to_screen(b.quad[2]));
                painter.rect_stroke(r, 0., stroke, StrokeKind::Inside);
                for p in [r.left_top(), r.center_top(), r.right_top(), r.right_center(), r.right_bottom(), r.center_bottom(), r.left_bottom(), r.left_center()]
                {
                    paint_handle(&painter, p, &t);
                }
            }
        }
        Target::Frame => {
            let locked = app.session.active().is_some_and(|st| {
                let locks = st.doc.effective_locks(id);
                locks.all || locks.position
            });
            if locked {
                return;
            }
            // The box is rebuilt from the frame each pass, so no box state outlives it.
            let state = frame_box(&f.transform);
            let start = KRect::from_points(state.quad[0], state.quad[2]);
            let rules = BoxRules::default().with_scale(ScaleRule::Free).with_allow(BoxAllow::new(true, false, false));
            let shown = TransformBox::new("Frame", Bind::given(state), start, 0.0).reference_point(false).inside_moves(false).rules(rules).show_at(ui, rect);
            let events: Vec<_> = shown.edit.events.iter().cloned().collect();
            for event in events {
                match event {
                    EditEvent::Update { value, .. } => update_preview(app, id, frame_affine(&value)),
                    EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => {
                        app.frame_preview = None;
                        let transform = frame_affine(&after);
                        if transform != original {
                            let r = app.run("layer.frame.set", json!({"layer":id.0,"transform":transform.m}));
                            report(app, r);
                        }
                    }
                    EditEvent::Cancel { .. } => app.frame_preview = None,
                    _ => {}
                }
            }
        }
    }
}

// ---- the options bar ----

pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if tool != Tool::Frame {
        return false;
    }
    // An icon entry's label is its tooltip and accessible name.
    let entries: Vec<Entry<String>> = [
        ("square", "rectangle", tl!("Create a new rectangular frame")),
        ("circle", "ellipse", tl!("Create a new elliptical frame")),
        ("triangle", "triangle", tl!("Create a new triangular frame")),
        ("pentagon", "polygon", tl!("Create a new polygonal frame")),
        ("shapes", "custom", tl!("Create a new custom frame")),
    ]
    .into_iter()
    .map(|(icon, shape, label)| Entry::new(shape.to_string(), label).icon(IconId::new(icon)))
    .collect();
    let current = current_shape(&app.ui.frame_tool).to_string();
    let mut shape = current.clone();
    Segmented::new("Frame shape", Bind::bound(&mut shape), &entries).show(ui);
    if shape != current {
        app.ui.frame_tool.ellipse = shape == "ellipse";
        app.ui.frame_tool.shape = shape;
    }
    if app.ui.frame_tool.shape == "polygon" {
        let mut sides = Some(f64::from(app.ui.frame_tool.sides.clamp(3, 100)));
        NumberField::new("Sides", Bind::bound(&mut sides)).range(3.0..=100.0).show(ui);
        if let Some(n) = sides {
            app.ui.frame_tool.sides = (n.round() as u32).clamp(3, 100);
        }
    }
    if app.ui.frame_tool.shape == "custom" {
        crate::preset_panels::shape_picker(app, ui);
    }
    Divider::vertical().show(ui);
    let mut value = selected(app).and_then(|l| l.frame.as_ref()).map_or_else(|| app.ui.frame_tool.stroke.clone(), |f| f.stroke.clone());
    if let Some(change) = stroke::cells(ui, &mut value, "frame-bar") {
        app.ui.frame_tool.stroke = value.clone();
        if let Some(id) = selected(app).map(|l| l.id) {
            let r = app.run("layer.frame.set", json!({"layer":id.0,"stroke":stroke::param(&value),"coalesce":change.coalesce}));
            report(app, r);
        }
    }
    true
}

#[cfg(test)]
mod cursor_tests;
#[cfg(test)]
mod escape_commit_tests;
#[cfg(test)]
mod move_history_tests;
#[cfg(test)]
mod shift_square_tests;
#[cfg(test)]
mod size_hud_tests;
#[cfg(test)]
mod space_move_tests;
#[cfg(test)]
mod tests;

/// Free Transform on the frame thumbnail edits the clipping geometry, leaving its image intact.
pub(crate) fn frame_transform(t: &FreeTransform) -> bool {
    t.target.as_ref().and_then(|v| v.get("frame")).and_then(Value::as_u64) == Some(t.layer)
}
pub(crate) fn transformed_frame(app: &PhotocraftApp, t: &FreeTransform) -> Result<Affine, String> {
    if t.session.warp().is_some() {
        return Err(tl!("A frame supports affine transforms; select its image to warp it").into());
    }
    let r = t.session.rect();
    let (x0, y0, w, h) = (r.x0, r.y0, r.width(), r.height());
    let quad = t.session.quad();
    let [q0, q1, q2, q3] = quad;
    if w <= 0.
        || h <= 0.
        || !quad.iter().all(|p| p.x.is_finite() && p.y.is_finite())
        || ((q1.x + q3.x - q0.x) - q2.x).abs() > 0.001
        || ((q1.y + q3.y - q0.y) - q2.y).abs() > 0.001
    {
        return Err(tl!("A frame supports affine transforms; select its image for perspective transforms").into());
    }
    let (a, b, c, d) = ((q1.x - q0.x) / w, (q1.y - q0.y) / w, (q3.x - q0.x) / h, (q3.y - q0.y) / h);
    let map = Affine { m: [a, b, c, d, q0.x - a * x0 - c * y0, q0.y - b * x0 - d * y0] };
    let original = app
        .session
        .active()
        .and_then(|st| st.doc.layer(LayerId(t.layer)))
        .and_then(|l| l.frame.as_ref())
        .ok_or_else(|| tl!("The frame is unavailable").to_string())?;
    let mut f = original.clone();
    f.transform = map.mul(&f.transform);
    if !f.valid() {
        return Err(tl!("Invalid frame transform").into());
    }
    Ok(f.transform)
}
