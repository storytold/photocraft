//! Frame Tool: persistent placeholders and independently selectable frame/image geometry.
use crate::{
    PhotocraftApp,
    canvas::{ToolEvent, ViewXform},
    state::Tool,
    theme::Tokens,
};
use egui::{CursorIcon, Pos2, Stroke, StrokeKind};
use photocraft_doc::{DocId, Frame, Layer, LayerContent, LayerId};
use photocraft_geom::{Affine, Point, Rect};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const CLICK_PX: f64 = 3.0;
const HANDLE_PX: f64 = 7.0;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Target {
    #[default]
    Frame,
    Content,
    Both,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FrameToolUi {
    /// Kept for old saved UI state; the current shape selector also supports polygons/custom shapes.
    pub ellipse: bool,
    pub shape: String,
    pub sides: u32,
    /// The corner radius of Rectangle, Triangle and Polygon frames, in pixels (#13875).
    pub radius: f32,
    /// A Polygon frame's star ratio in whole percents, 100 for a plain polygon (#13876).
    pub star_ratio: f32,
    /// Smooth Star Indents (#13876). Shown and kept; it doesn't change the outline yet (not
    /// measured for frames).
    pub smooth_indents: bool,
    pub stroke: Option<photocraft_doc::ShapeStroke>,
    pub target: Target,
    pub selected: Option<u64>,
    #[serde(skip)]
    pub drag: Option<FrameDrag>,
    #[serde(skip)]
    pub auto_transform: bool,
    /// Space (the reposition key) is held: a frame being drawn moves instead of sizing (#13867).
    #[serde(skip)]
    pub space: bool,
    /// The options bar's stroke width, alignment and colour, kept while the stroke is off (No
    /// Color) for the next time a colour turns it on (#13873).
    #[serde(default = "default_stroke_style")]
    pub stroke_style: photocraft_doc::ShapeStroke,
}

/// A new frame's stroke as the reference application starts it (#13873): 1 px, Center, solid
/// black; off until a colour is picked.
pub fn default_stroke_style() -> photocraft_doc::ShapeStroke {
    photocraft_doc::ShapeStroke { width: 1.0, ..photocraft_doc::ShapeStroke::default() }
}

#[cfg(test)]
mod options_stroke_target_tests;
#[cfg(test)]
mod stroke_options_tests;
/// The options bar's stroke width range (#13873).
pub const STROKE_WIDTH: std::ops::RangeInclusive<f32> = 0.0..=288.0;

/// The alert for a width typed past [`STROKE_WIDTH`], as the reference application words it.
pub const STROKE_WIDTH_ALERT: &str = "A value between 0 pixel and 288 Pixels is required. Closest value inserted.";
impl Default for FrameToolUi {
    fn default() -> Self {
        Self {
            ellipse: false,
            shape: "rectangle".into(),
            sides: 5,
            radius: 0.0,
            star_ratio: 100.0,
            smooth_indents: false,
            stroke: None,
            target: Target::Frame,
            selected: None,
            drag: None,
            auto_transform: false,
            space: false,
            stroke_style: default_stroke_style(),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum FrameDrag {
    New {
        start: [f64; 2],
        cur: [f64; 2],
        square: bool,
        centred: bool,
        doc: DocId,
        revision: u64,
    },
    Edit {
        layer: LayerId,
        doc: DocId,
        revision: u64,
        original: Box<Frame>,
        proposed: Affine,
        start: [f64; 2],
        mode: EditMode,
    },
    /// A press inside a frame (`frame`) that holds an image: dragged, the image (`layer`) moves
    /// and the frame stays (#13878); a click selects both, as before. Without a frame, a shape
    /// lying above an empty frame under the press (#13882): dragged, the shape moves.
    Content {
        frame: Option<LayerId>,
        layer: LayerId,
        doc: DocId,
        revision: u64,
        start: [f64; 2],
        cur: [f64; 2],
    },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EditMode {
    Move,
    Resize(i8, i8),
    Rotate,
}
pub(crate) struct Preview {
    doc: std::sync::Arc<photocraft_doc::Document>,
    revision: u64,
    key: u64,
}

fn drawn(start: [f64; 2], cur: [f64; 2], square: bool, centred: bool) -> [i64; 4] {
    let [sx, sy] = start;
    let [x, y] = cur;
    let (mut dx, mut dy) = (x - sx, y - sy);
    if square {
        // Shift squares from the drag's shorter side, as the reference application does (#13865).
        let side = dx.abs().min(dy.abs());
        dx = side.copysign(dx);
        dy = side.copysign(dy);
    }
    let (a, b) = if centred { ([sx - dx, sy - dy], [sx + dx, sy + dy]) } else { (start, [sx + dx, sy + dy]) };
    [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])].map(|v| v.round() as i64)
}
#[cfg(test)]
mod shift_square_tests;
#[cfg(test)]
mod size_hud_tests;
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
    let l = st.doc.layer(st.active_layer?)?;
    if !matches!(l.content, LayerContent::Raster(_) | LayerContent::Text(_) | LayerContent::Shape(_) | LayerContent::Smart(_)) || st.doc.is_background(l.id) {
        return None;
    }
    let locks = st.doc.effective_locks(l.id);
    if locks.position || locks.all {
        return None;
    }
    // Another frame's image stays in its frame; the drag draws an empty frame instead.
    if photocraft_engine::frame_cmds::frame_of(&st.doc, l.id).is_some() {
        return None;
    }
    let b = photocraft_engine::transform_cmds::transform_frame(&st.doc, l);
    (!b.is_empty() && i64::from(b.x0) < r[2] && r[0] < i64::from(b.x1) && i64::from(b.y0) < r[3] && r[1] < i64::from(b.y1)).then_some(l.id.0)
}
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
fn shape_params(app: &PhotocraftApp, r: [i64; 4]) -> Value {
    let [x, y, x1, y1] = r;
    let shape = if app.ui.frame_tool.ellipse {
        "ellipse"
    } else if app.ui.frame_tool.shape.is_empty() {
        "rectangle"
    } else {
        &app.ui.frame_tool.shape
    };
    let mut p = json!({"x":x,"y":y,"width":x1.saturating_sub(x),"height":y1.saturating_sub(y),"shape":shape,"sides":app.ui.frame_tool.sides.clamp(3,100)});
    if rounds(shape) && app.ui.frame_tool.radius > 0.0 {
        p["radius"] = json!(f64::from(app.ui.frame_tool.radius));
    }
    if shape == "polygon" && app.ui.frame_tool.star_ratio < 100.0 {
        p["starRatio"] = json!(f64::from(app.ui.frame_tool.star_ratio.round().clamp(1.0, 100.0)));
    }
    if shape == "custom" {
        let name = app.ui.presets_ui.shape();
        if let Some(sh) = photocraft_engine::presets::shapes::all_groups(&app.session).iter().flat_map(|g| &g.items).find(|s| s.name == name) {
            p["path"] = photocraft_engine::vector_cmds::path_json(&sh.path);
        }
    }
    if let Some(s) = &app.ui.frame_tool.stroke {
        p["stroke"] = stroke_json(s);
    }
    p
}
fn outline(app: &PhotocraftApp, r: [i64; 4]) -> Option<photocraft_doc::Path> {
    let p = shape_params(app, r);
    let shape = p.get("shape").and_then(Value::as_str)?;
    // A star previews as the frame will be: the engine's own outline over the drag's box (#13876).
    if let Some(percent) = p.get("starRatio").and_then(Value::as_f64) {
        let [x, y, xe, ye] = r;
        let (w, h) = ((xe - x) as f64, (ye - y) as f64);
        let radius = p.get("radius").and_then(Value::as_f64).unwrap_or(0.0);
        let unit = photocraft_engine::frame_cmds::star_path(u64::from(app.ui.frame_tool.sides.clamp(3, 100)), percent, radius, w, h).ok()?;
        return Some(unit.transform(&Affine { m: [w, 0.0, 0.0, h, x as f64, y as f64] }));
    }
    if shape == "custom" {
        let sh = photocraft_engine::vector_cmds::parse_path(p.get("path")?).ok()?;
        let (x0, y0, x1, y1) = sh.control_bounds()?;
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let [x, y, xe, ye] = r;
        let (sx, sy) = ((xe - x) as f64 / (x1 - x0), (ye - y) as f64 / (y1 - y0));
        return Some(sh.transform(&Affine { m: [sx, 0.0, 0.0, sy, x as f64 - x0 * sx, y as f64 - y0 * sy] }));
    }
    let [x, y, xe, ye] = r;
    // The preview rounds as the frame will (#13875).
    let radius = p.get("radius").and_then(Value::as_f64).unwrap_or(0.0);
    photocraft_engine::vector_cmds::shape_path(&json!({"kind":match shape{"ellipse"=>"ellipse","triangle"|"polygon"=>"polygon",_=>"rect"},"rect":[x,y,xe-x,ye-y],"sides":if shape=="triangle"{3}else{app.ui.frame_tool.sides.clamp(3,100)},"radius":radius,"cornerRadius":radius})).ok()
}

/// The alert for a star ratio typed past 1–100 % (#13876), in the app's wording for whole percents.
pub const STAR_RATIO_ALERT: &str = "An integer between 1% and 100% is required. Closest value inserted.";

/// The frame shapes with corners to round (#13875): Rectangle, Triangle and Polygon.
fn rounds(shape: &str) -> bool {
    matches!(shape, "rectangle" | "triangle" | "polygon")
}

/// The alert for a corner radius typed past its range, as the reference application words it.
pub const RADIUS_ALERT: &str = "A value between 0.000 pixel and 150000.000 Pixels is required. Closest value inserted.";

/// The options bar's corner radius field (#13875): a rounded-corner icon and the value, 0–150000
/// px with decimals, one value for the three shapes that round. Whether a value past the range
/// was typed.
fn radius_field(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, icon) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
    let r = rect.shrink(2.0);
    let corner: Vec<Pos2> = (0..=8)
        .map(|i| {
            let a = std::f32::consts::FRAC_PI_2 * i as f32 / 8.0;
            egui::pos2(r.right() - r.width() * a.sin(), r.bottom() - r.height() * a.cos())
        })
        .collect();
    ui.painter().add(egui::Shape::line(corner, Stroke::new(1.5, t.text_dim)));
    icon.on_hover_text(tl!("Set radius of rounded corners"));
    let (resp, alert) =
        crate::widgets::value_field_alerting(ui, tl!("Set radius of rounded corners"), &mut app.ui.frame_tool.radius, 0.0..=150_000.0, "px", 70.0);
    resp.on_hover_text(tl!("Set radius of rounded corners"));
    alert
}
fn handles(f: &Frame) -> [(Point, i8, i8); 8] {
    [(0., 0., -1, -1), (0.5, 0., 0, -1), (1., 0., 1, -1), (1., 0.5, 1, 0), (1., 1., 1, 1), (0.5, 1., 0, 1), (0., 1., -1, 1), (0., 0.5, -1, 0)]
        .map(|(x, y, hx, hy)| (f.transform.apply(Point::new(x, y)), hx, hy))
}
fn control_hit(f: &Frame, p: [f64; 2], zoom: f64) -> Option<EditMode> {
    if let Some((_, hx, hy)) = handles(f).into_iter().find(|(q, _, _)| (q.x - p[0]).hypot(q.y - p[1]) * zoom <= HANDLE_PX) {
        return Some(EditMode::Resize(hx, hy));
    }
    let local = f.transform.inverse()?.apply(Point::new(p[0], p[1]));
    if (!(0.0..=1.0).contains(&local.x) || !(0.0..=1.0).contains(&local.y)) && f.corners().iter().any(|q| (q.x - p[0]).hypot(q.y - p[1]) * zoom <= 20.0) {
        return Some(EditMode::Rotate);
    }
    None
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
/// Whether a group above the layer at `path` clips it away at `p`: a frame group clips its children to its
/// outline, and a group's enabled vector mask to its path. A shape visible there only outside that outline
/// is not what the press is on.
fn clipped_by_ancestor(doc: &photocraft_doc::Document, path: &[usize], pixel: Rect, p: [f64; 2]) -> bool {
    (1..path.len()).filter_map(|n| doc.layer_at(path.get(..n)?)).any(|group| {
        group.frame.as_ref().is_some_and(|f| !inside(f, p))
            || group.vector_mask.as_ref().is_some_and(|m| m.enabled && !photocraft_vector::path_coverage(&m.path, pixel).first().is_some_and(|v| *v > 0.0))
    })
}
/// The shape layer drawn above the empty frame `frame` whose fill covers `p` (#13882): visible,
/// not position-locked, the topmost one. Other kinds of layers above a frame, and frames that hold
/// an image, weren't measured and keep today's press.
fn shape_above(app: &PhotocraftApp, frame: LayerId, p: [f64; 2]) -> Option<LayerId> {
    let st = app.session.active()?;
    if !st.doc.layer(frame)?.children().is_none_or(<[Layer]>::is_empty) {
        return None;
    }
    let (x, y) = (p[0].floor() as i32, p[1].floor() as i32);
    let pixel = Rect::new(x, y, x.checked_add(1)?, y.checked_add(1)?);
    for (path, _, l) in st.doc.walk().into_iter().rev() {
        if l.id == frame {
            return None;
        }
        let LayerContent::Shape(sh) = &l.content else { continue };
        let locks = st.doc.effective_locks(l.id);
        if path_visible(&st.doc, &path)
            && !(locks.all || locks.position)
            && sh.fill.is_some()
            && photocraft_vector::path_coverage(&sh.path, pixel).first().is_some_and(|v| *v > 0.0)
            && !clipped_by_ancestor(&st.doc, &path, pixel, p)
        {
            return Some(l.id);
        }
    }
    None
}
#[cfg(test)]
mod hit_order_tests;
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
fn edit_transform(f: &Frame, mode: EditMode, start: [f64; 2], p: [f64; 2], mods: egui::Modifiers) -> Affine {
    match mode {
        EditMode::Move => {
            let (mut dx, mut dy) = (p[0] - start[0], p[1] - start[1]);
            if mods.shift {
                if dx.abs() > dy.abs() { dy = 0.0 } else { dx = 0.0 }
            }
            Affine::translate(dx.round(), dy.round()).mul(&f.transform)
        }
        EditMode::Rotate => {
            let center = f.transform.apply(Point::new(0.5, 0.5));
            let mut a = (p[1] - center.y).atan2(p[0] - center.x) - (start[1] - center.y).atan2(start[0] - center.x);
            if mods.shift {
                let step = std::f64::consts::PI / 12.;
                a = (a / step).round() * step;
            }
            Affine::translate(center.x, center.y).mul(&Affine::rotate(a)).mul(&Affine::translate(-center.x, -center.y)).mul(&f.transform)
        }
        EditMode::Resize(hx, hy) => {
            let Some(inv) = f.transform.inverse() else { return f.transform };
            let (s, c) = (inv.apply(Point::new(start[0], start[1])), inv.apply(Point::new(p[0], p[1])));
            let (dx, dy) = (c.x - s.x, c.y - s.y);
            let (mut x0, mut y0, mut x1, mut y1) = (0., 0., 1., 1.);
            if hx < 0 {
                x0 += dx;
                if mods.alt {
                    x1 -= dx;
                }
            } else if hx > 0 {
                x1 += dx;
                if mods.alt {
                    x0 -= dx;
                }
            }
            if hy < 0 {
                y0 += dy;
                if mods.alt {
                    y1 -= dy;
                }
            } else if hy > 0 {
                y1 += dy;
                if mods.alt {
                    y0 -= dy;
                }
            }
            if mods.shift {
                let k = if hx == 0 {
                    (y1 - y0).abs()
                } else if hy == 0 {
                    (x1 - x0).abs()
                } else {
                    (x1 - x0).abs().max((y1 - y0).abs())
                };
                let anchor = |lo: f64, hi: f64, h: i8| {
                    if mods.alt || h == 0 {
                        ((lo + hi - k) / 2., (lo + hi + k) / 2.)
                    } else if h < 0 {
                        (hi - k, hi)
                    } else {
                        (lo, lo + k)
                    }
                };
                (x0, x1) = anchor(x0, x1, hx);
                (y0, y1) = anchor(y0, y1, hy);
            }
            let [a, b, c, d, _, _] = f.transform.m;
            let minimum = |n: f64, size: f64| n.signum().copysign(if n == 0.0 { 1.0 } else { n }) * n.abs().max(1.0 / size.max(1.0));
            let (w, h) = (minimum(x1 - x0, a.hypot(b)), minimum(y1 - y0, c.hypot(d)));
            f.transform.mul(&Affine { m: [w, 0., 0., h, x0, y0] })
        }
    }
}
fn update_preview(app: &mut PhotocraftApp) {
    // A frame's image being dragged shows where it moves, as the Move tool's drag does (#13878).
    if let Some(FrameDrag::Content { layer, doc, revision, start, cur, .. }) = app.ui.frame_tool.drag.as_ref() {
        let (layer, revision, (dx, dy)) = (*layer, *revision, content_delta(*start, *cur, f64::from(app.point_zoom())));
        let shown = app
            .session
            .active()
            .filter(|st| st.doc.id == *doc && st.revision == revision)
            .and_then(|st| photocraft_engine::layer_multi_cmds::moved(&st.doc, &[layer], dx, dy).ok());
        app.frame_preview = shown.map(|shown| {
            let key = app.frame_preview.as_ref().map_or(1, |p| p.key.wrapping_add(1));
            Preview { doc: std::sync::Arc::new(shown), revision, key }
        });
        return;
    }
    let Some(FrameDrag::Edit { layer, doc, revision, proposed, .. }) = app.ui.frame_tool.drag.as_ref() else { return };
    let Some(st) = app.session.active().filter(|st| st.doc.id == *doc && st.revision == *revision) else {
        app.frame_preview = None;
        return;
    };
    let (id, revision, proposed) = (*layer, *revision, *proposed);
    let mut shown = (*st.doc).clone();
    if let Some(l) = shown.layer_mut(id) {
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
    app.frame_preview = Some(Preview { doc: std::sync::Arc::new(shown), revision, key });
}
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(std::sync::Arc<photocraft_doc::Document>, u64)> {
    if let Some(t) = app.ui.transform.clone().filter(frame_transform) {
        let transform = transformed_frame(app, &t).ok()?;
        let st = app.session.documents().get(idx)?;
        if app.transform_preview.as_ref().is_none_or(|pv| pv.doc.id != st.doc.id) {
            return None;
        }
        use std::hash::Hasher;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        hash.write_u64(t.session);
        for value in transform.m {
            hash.write_u64(value.to_bits());
        }
        let key = hash.finish();
        if app.frame_preview.as_ref().is_none_or(|p| p.doc.id != st.doc.id || p.revision != st.revision || p.key != key) {
            let mut doc = (*st.doc).clone();
            let l = doc.layer_mut(LayerId(t.layer))?;
            l.frame.as_mut()?.transform = transform;
            photocraft_engine::frame_cmds::refresh(l);
            app.frame_preview = Some(Preview { doc: std::sync::Arc::new(doc), revision: st.revision, key });
        }
    }
    let st = app.session.documents().get(idx)?;
    let p = app.frame_preview.as_ref()?;
    (app.ui.tool == Tool::Frame && st.doc.id == p.doc.id && st.revision == p.revision).then(|| (p.doc.clone(), (1 << 57) | p.key))
}
pub fn cancel(app: &mut PhotocraftApp) {
    app.ui.frame_tool.drag = None;
    app.frame_preview = None;
}
/// Space (the reposition key) for this canvas frame (#13867). Whether a frame is being drawn, which
/// Space then moves: the canvas keeps the key from the temporary Hand tool meanwhile.
pub fn set_space(app: &mut PhotocraftApp, down: bool) -> bool {
    app.ui.frame_tool.space = down;
    matches!(app.ui.frame_tool.drag, Some(FrameDrag::New { .. }))
}
#[cfg(test)]
mod space_move_tests;
/// The selected frame (its id, frame and target) and the handles a press at `p` hits on it: a
/// frame control (resize or rotate) when the frame itself is targeted, and whether it hits its
/// image's handles when its content is.
#[allow(clippy::type_complexity)]
fn press_controls(app: &PhotocraftApp, p: [f64; 2], zoom: f64) -> (Option<(LayerId, Option<Box<Frame>>, Target)>, Option<(LayerId, EditMode)>, bool) {
    let active = selected(app).map(|l| (l.id, l.frame.clone(), target(app, l)));
    let selected_control = active.as_ref().and_then(|(id, f, t)| (*t == Target::Frame).then(|| Some((*id, control_hit(f.as_ref()?, p, zoom)?))).flatten());
    let content_control = active.as_ref().is_some_and(|(id, _, t)| {
        if *t != Target::Content {
            return false;
        }
        let Some(st) = app.session.active() else { return false };
        let Some(child) = st.doc.layer(*id).and_then(|l| l.children()).and_then(|c| c.first()) else { return false };
        let b = photocraft_engine::transform_cmds::transform_frame(&st.doc, child);
        let (x0, y0, x1, y1) = (f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1));
        [(x0, y0), ((x0 + x1) / 2., y0), (x1, y0), (x1, (y0 + y1) / 2.), (x1, y1), ((x0 + x1) / 2., y1), (x0, y1), (x0, (y0 + y1) / 2.)]
            .iter()
            .any(|(x, y)| (x - p[0]).hypot(y - p[1]) * zoom <= HANDLE_PX)
    });
    (active, selected_control, content_control)
}

/// What a Frame tool press at `p` snaps (#13871), as the press is read in [`tool_event`]: a new
/// frame snaps its corners (no bounds), and moving an empty frame snaps its edges (its bounds);
/// with the layers that aren't targets: the frame itself, and the Background, whose edges are the
/// document's (not measured, so not snapped to, as today). Handle drags, image moves and frames
/// holding an image weren't measured and don't snap.
pub(crate) fn snap_press(app: &PhotocraftApp, p: [f64; 2]) -> Option<(Option<[f64; 4]>, Vec<LayerId>)> {
    let zoom = f64::from(app.point_zoom());
    let (_, selected_control, content_control) = press_controls(app, p, zoom);
    if selected_control.is_some() || content_control {
        return None;
    }
    let st = app.session.active()?;
    let mut exclude: Vec<LayerId> = st.doc.layers.iter().filter(|l| st.doc.is_background(l.id)).map(|l| l.id).collect();
    let Some(id) = frame_at(app, p) else { return Some((None, exclude)) };
    // A press on a shape lying above an empty frame drags the shape (#13882): the frame's bounds are no
    // gesture for it, so it moves by the pointer's offset, not one corrected to the frame's edges.
    if shape_above(app, id, p).is_some() {
        return None;
    }
    let l = st.doc.layer(id)?;
    let f = l.frame.as_ref()?;
    let locks = st.doc.effective_locks(id);
    if locks.all || locks.position || !l.children().is_none_or(<[Layer]>::is_empty) {
        return None;
    }
    let b = f.bounds();
    exclude.push(id);
    Some((Some([f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)]), exclude))
}
#[cfg(test)]
mod snap_tests;

/// Escape during a drag. A frame being drawn is made at the pointer, as the reference application
/// does (#13866); the mouse movement and release after it change nothing. Moving, resizing or
/// rotating an existing frame is cancelled.
pub fn escape(app: &mut PhotocraftApp) {
    let Some(FrameDrag::New { start, cur, square, centred, doc, revision }) = app.ui.frame_tool.drag.clone() else {
        cancel(app);
        return;
    };
    cancel(app);
    if app.session.active().is_some_and(|st| st.doc.id == doc && st.revision == revision) {
        commit_new(app, start, cur, square, centred);
    }
}
/// Make the frame drawn from `start` to `cur`, unless the drag was a click or drew nothing.
fn commit_new(app: &mut PhotocraftApp, start: [f64; 2], cur: [f64; 2], square: bool, centred: bool) {
    if (cur[0] - start[0]).hypot(cur[1] - start[1]) * f64::from(app.point_zoom()) < CLICK_PX {
        return;
    }
    let r = drawn(start, cur, square, centred);
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
#[cfg(test)]
mod escape_commit_tests;
/// A native double click (also driven by ui.click count=2) toggles content-only selection.
pub fn double_click(app: &mut PhotocraftApp, p: [f64; 2]) {
    let Some(id) = frame_at(app, p) else { return };
    let next = if selected(app).is_some_and(|l| l.id == id && target(app, l) == Target::Content) { Target::Both } else { Target::Content };
    select(app, id, next);
    cancel(app);
}
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    if app.ui.tool != Tool::Frame {
        return false;
    }
    if app.ui.frame_tool.drag.is_none() && crate::canvas::event_tool(app, mods) != Tool::Frame {
        return false;
    }
    let (x, y) = match ev {
        ToolEvent::Down { x, y, .. } | ToolEvent::Move { x, y, .. } | ToolEvent::Up { x, y } => (x, y),
    };
    if !x.is_finite() || !y.is_finite() || x.abs() > 600_000.0 || y.abs() > 600_000.0 {
        cancel(app);
        app.ui.status = tl!("Frame coordinates must be finite and within 600000 pixels").into();
        app.ui.status_error = true;
        return true;
    }
    if app.ui.transform.is_none() {
        app.ui.frame_tool.auto_transform = false;
    }
    match ev {
        ToolEvent::Down { x, y, .. } => {
            cancel(app);
            let p = [x, y];
            // What was pressed is decided where the pointer was, not where Snap moved the corner of a new
            // frame to: a press just off an existing frame must not become an edit of it.
            let raw = app.prefs_rt.snap.as_ref().map_or(p, |s| s.start);
            let zoom = f64::from(app.point_zoom());
            let (active, selected_control, content_control) = press_controls(app, raw, zoom);
            let id = selected_control
                .map(|(id, _)| id)
                .or_else(|| content_control.then(|| active.as_ref().map(|(id, _, _)| *id)).flatten())
                .or_else(|| frame_at(app, raw));
            // A shape lying above an empty frame where the press is: the drag moves the shape, one
            // "Move" step, and the frame stays (#13882).
            if selected_control.is_none()
                && !content_control
                && let Some(shape) = id.and_then(|frame| shape_above(app, frame, raw))
                && let Some(st) = app.session.active()
            {
                app.ui.frame_tool.drag = Some(FrameDrag::Content { frame: None, layer: shape, doc: st.doc.id, revision: st.revision, start: p, cur: p });
                return true;
            }
            if let Some(id) = id {
                let Some(st) = app.session.active() else { return true };
                let Some(l) = st.doc.layer(id) else { return true };
                let Some(f) = l.frame.clone() else { return true };
                let locks = st.doc.effective_locks(id);
                if locks.all || locks.position {
                    app.ui.status = tl!("The frame is locked").into();
                    app.ui.status_error = true;
                    return true;
                }
                let t = if content_control {
                    Target::Content
                } else if selected_control.is_some() || on_border(&f, raw, zoom) || l.children().is_none_or(|c| c.is_empty()) {
                    Target::Frame
                } else if active.as_ref().is_some_and(|(prev, _, t)| *prev == id && *t == Target::Content) {
                    Target::Content
                } else {
                    Target::Both
                };
                let content = l.children().and_then(|c| c.first()).map(|c| c.id);
                select(app, id, t);
                // Inside a frame with an image: a drag moves the image alone, one "Move" step on
                // release, and leaves it the active layer (#13878). A click keeps both selected.
                if t == Target::Both
                    && let Some(content) = content
                {
                    let Some(st) = app.session.active() else { return true };
                    app.ui.frame_tool.drag =
                        Some(FrameDrag::Content { frame: Some(id), layer: content, doc: st.doc.id, revision: st.revision, start: p, cur: p });
                    return true;
                }
                if t != Target::Frame {
                    // The canvas's own context: preview textures loaded into another never draw.
                    let Some(ctx) = app.frame_tool_ctx.clone() else { return true };
                    match crate::transform_tool::begin(app, &ctx) {
                        Ok(()) => {
                            app.ui.frame_tool.auto_transform = true;
                            crate::transform_tool::pointer(app, ev, mods);
                        }
                        Err(e) => {
                            app.ui.status = e;
                            app.ui.status_error = true;
                        }
                    }
                    return true;
                }
                let Some(st) = app.session.active() else { return true };
                app.ui.frame_tool.drag = Some(FrameDrag::Edit {
                    layer: id,
                    doc: st.doc.id,
                    revision: st.revision,
                    original: f.clone(),
                    proposed: f.transform,
                    start: p,
                    mode: selected_control.map_or(EditMode::Move, |(_, m)| m),
                });
            } else if let Some(st) = app.session.active() {
                app.ui.frame_tool.drag =
                    Some(FrameDrag::New { start: p, cur: p, square: mods.shift, centred: mods.alt, doc: st.doc.id, revision: st.revision });
            }
        }
        ToolEvent::Move { x, y, .. } => {
            let space = app.ui.frame_tool.space;
            if let Some(d) = &mut app.ui.frame_tool.drag {
                match d {
                    FrameDrag::New { start, cur, square, centred, .. } => {
                        // Space held moves the whole frame with the pointer (#13867).
                        if space {
                            *start = [start[0] + x - cur[0], start[1] + y - cur[1]];
                        }
                        *cur = [x, y];
                        *square = mods.shift;
                        *centred = mods.alt;
                    }
                    FrameDrag::Edit { original, proposed, start, mode, .. } => *proposed = edit_transform(original, *mode, *start, [x, y], mods),
                    FrameDrag::Content { cur, .. } => *cur = [x, y],
                }
            }
            update_preview(app);
        }
        ToolEvent::Up { x, y } => {
            let Some(d) = app.ui.frame_tool.drag.take() else { return true };
            app.frame_preview = None;
            let (doc, revision) = match &d {
                FrameDrag::New { doc, revision, .. } | FrameDrag::Edit { doc, revision, .. } | FrameDrag::Content { doc, revision, .. } => (*doc, *revision),
            };
            if !app.session.active().is_some_and(|st| st.doc.id == doc && st.revision == revision) {
                return true;
            }
            match d {
                FrameDrag::New { start, cur, .. } => {
                    // Released with Space still down: the last step moved the frame too (#13867).
                    let start = if app.ui.frame_tool.space { [start[0] + x - cur[0], start[1] + y - cur[1]] } else { start };
                    commit_new(app, start, [x, y], mods.shift, mods.alt)
                }
                FrameDrag::Edit { layer, original, start, mode, .. } => {
                    let transform = edit_transform(&original, mode, start, [x, y], mods);
                    if transform != original.transform {
                        let mut p = json!({"layer":layer.0,"transform":transform.m});
                        // Moving an empty frame is a "Move" step (#13880). Handle drags, and a
                        // frame with an image (not measured yet), keep "Frame Properties".
                        let empty = app.session.active().and_then(|st| st.doc.layer(layer)).is_some_and(|l| l.children().is_none_or(<[Layer]>::is_empty));
                        if mode == EditMode::Move && empty {
                            p["historyName"] = json!("Move");
                        }
                        let r = app.run("layer.frame.set", p);
                        report(app, r);
                    }
                }
                FrameDrag::Content { frame, layer, start, .. } => {
                    let (dx, dy) = content_delta(start, [x, y], f64::from(app.point_zoom()));
                    if (dx, dy) != (0, 0) {
                        if let Some(frame) = frame {
                            select(app, frame, Target::Content);
                        }
                        let r = app.run("layer.translate", json!({"layer": layer.0, "dx": dx, "dy": dy}));
                        report(app, r);
                    }
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod content_drag_tests;
/// The whole-pixel move of a frame's image dragged from `start` to `at` (#13878). A press and release
/// within [`CLICK_PX`] screen pixels is a click, which moves nothing (the frame keeps both selected).
fn content_delta(start: [f64; 2], at: [f64; 2], zoom: f64) -> (i32, i32) {
    if (at[0] - start[0]).hypot(at[1] - start[1]) * zoom < CLICK_PX {
        return (0, 0);
    }
    let d = |a: f64, b: f64| (b - a).round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    (d(start[0], at[0]), d(start[1], at[1]))
}
fn paint_path(painter: &egui::Painter, xf: &ViewXform, path: &photocraft_doc::Path, stroke: Stroke) {
    for line in photocraft_vector::flatten::flatten_path(path, 0.5 / f64::from(xf.zoom).max(0.01)) {
        let mut points: Vec<Pos2> = line.pts.iter().map(|&(x, y)| xf.to_screen(x as f32, y as f32)).collect();
        if line.closed
            && let Some(first) = points.first().copied()
        {
            points.push(first);
        }
        painter.add(egui::Shape::line(points, stroke));
    }
}
pub fn draw_overlay(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let t = Tokens::get(painter.ctx());
    let stroke = Stroke::new(1.5, t.accent);
    if let Some(st) = app.session.active() {
        for (path, _, l) in st.doc.walk() {
            if !path_visible(&st.doc, &path) {
                continue;
            }
            let Some(f) = &l.frame else { continue };
            if l.children().is_some_and(|c| c.is_empty()) {
                let q = f.corners().map(|p| xf.to_screen(p.x as f32, p.y as f32));
                paint_path(painter, xf, &f.outline(), Stroke::new(1., t.text_faint));
                painter.line_segment([q[0], q[2]], Stroke::new(1., t.text_faint));
                painter.line_segment([q[1], q[3]], Stroke::new(1., t.text_faint));
            }
        }
    }
    if app.ui.tool != Tool::Frame {
        return;
    }
    if let Some(FrameDrag::New { start, cur, square, centred, .. }) = &app.ui.frame_tool.drag {
        let r = drawn(*start, *cur, *square, *centred);
        if let Some(path) = outline(app, r) {
            paint_path(painter, xf, &path, stroke);
        }
        // The size the release would make, beside the pointer, like the marquee's (#13870).
        let at = painter.ctx().input(|i| i.pointer.latest_pos()).unwrap_or_else(|| xf.to_screen(cur[0] as f32, cur[1] as f32));
        let size = crate::canvas::size_readout(app, [0.0, 0.0, (r[2] - r[0]) as f64, (r[3] - r[1]) as f64]);
        crate::canvas::draw_value_readout(app, painter.ctx(), "frame-size-readout", at, ["W:", "H:"], size);
        return;
    }
    let Some(l) = selected(app) else { return };
    if !visible(app, l.id) {
        return;
    }
    let Some(original) = &l.frame else { return };
    let mut f = original.clone();
    if let Some(t) = app.ui.transform.as_ref().filter(|t| frame_transform(t))
        && let Ok(transform) = transformed_frame(app, t)
    {
        f.transform = transform;
    }
    if let Some(FrameDrag::Edit { proposed, .. }) = &app.ui.frame_tool.drag {
        f.transform = *proposed;
    }
    if !f.valid() {
        return;
    }
    paint_path(painter, xf, &f.outline(), stroke);
    if app.ui.transform.is_some() {
        return;
    }
    let target = target(app, l);
    if target == Target::Content {
        if let Some(st) = app.session.active()
            && let Some(child) = l.children().and_then(|c| c.first())
        {
            let b = photocraft_engine::transform_cmds::transform_frame(&st.doc, child);
            let r = egui::Rect::from_two_pos(xf.to_screen(b.x0 as f32, b.y0 as f32), xf.to_screen(b.x1 as f32, b.y1 as f32));
            painter.rect_stroke(r, 0., stroke, StrokeKind::Inside);
            for p in [r.left_top(), r.center_top(), r.right_top(), r.right_center(), r.right_bottom(), r.center_bottom(), r.left_bottom(), r.left_center()] {
                paint_handle(painter, p, &t);
            }
        }
    } else if target == Target::Frame {
        for (p, _, _) in handles(&f) {
            paint_handle(painter, xf.to_screen(p.x as f32, p.y as f32), &t);
        }
    }
}
fn paint_handle(p: &egui::Painter, at: Pos2, t: &Tokens) {
    let r = egui::Rect::from_center_size(at, egui::vec2(6., 6.));
    p.rect_filled(r, 0., t.card);
    p.rect_stroke(r, 0., Stroke::new(1., t.accent), StrokeKind::Inside);
}
pub fn cursor(app: &PhotocraftApp, p: [f64; 2]) -> Option<CursorIcon> {
    let zoom = f64::from(app.point_zoom());
    if let Some(l) = selected(app)
        && let Some(f) = &l.frame
        && target(app, l) == Target::Frame
        && let Some(hit) = control_hit(f, p, zoom)
    {
        return Some(match hit {
            EditMode::Resize(0, _) => CursorIcon::ResizeVertical,
            EditMode::Resize(_, 0) => CursorIcon::ResizeHorizontal,
            EditMode::Resize(x, y) if x == y => CursorIcon::ResizeNwSe,
            EditMode::Resize(..) => CursorIcon::ResizeNeSw,
            _ => CursorIcon::Crosshair,
        });
    }
    frame_at(app, p).map(|_| CursorIcon::Move)
}
fn stroke_json(s: &photocraft_doc::ShapeStroke) -> Value {
    let fill = match &s.paint {
        photocraft_doc::Fill::Solid(c) => {
            let [r, g, b] = c.to_rgb();
            json!([r, g, b, c.alpha])
        }
        _ => Value::Null,
    };
    json!({"color":fill,"width":s.width,"opacity":s.opacity*100.,"align":match s.align{photocraft_doc::StrokeAlign::Inside=>"inside",photocraft_doc::StrokeAlign::Outside=>"outside",_=>"center"}})
}
fn stroke_controls(ui: &mut egui::Ui, value: &mut Option<photocraft_doc::ShapeStroke>) -> bool {
    let before = value.clone();
    let mut enabled = value.is_some();
    if crate::widgets::checkbox(ui, &mut enabled, tl!("Stroke")).changed() {
        *value = enabled.then(photocraft_doc::ShapeStroke::default);
    }
    if let Some(s) = value {
        let c = match s.paint {
            photocraft_doc::Fill::Solid(c) => c,
            _ => photocraft_doc::Color::BLACK,
        };
        let [r, g, b] = c.to_rgb();
        let mut rgba = [r, g, b, c.alpha];
        if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
            s.paint = photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(rgba[0], rgba[1], rgba[2], rgba[3]));
        }
        crate::widgets::value_field_named(ui, tl!("Stroke:"), &mut s.width, 0.0..=1000.0, "px", 58.0);
        craft_ui::ComboBox::from_id_salt("frame-stroke-align")
            .selected_text(match s.align {
                photocraft_doc::StrokeAlign::Inside => tl!("Inside"),
                photocraft_doc::StrokeAlign::Outside => tl!("Outside"),
                _ => tl!("Center"),
            })
            .show_ui(ui, |ui| {
                for (a, label) in [
                    (photocraft_doc::StrokeAlign::Inside, tl!("Inside")),
                    (photocraft_doc::StrokeAlign::Center, tl!("Center")),
                    (photocraft_doc::StrokeAlign::Outside, tl!("Outside")),
                ] {
                    ui.selectable_value(&mut s.align, a, label);
                }
            });
    }
    *value != before
}
/// The stroke after the options bar's swatch answered `change` (#13873): a colour turns it on with
/// that colour and `style`'s width and alignment, No Color turns it off. Gradients aren't a frame
/// stroke yet (not measured), and the Color Picker button opens nothing here: both change nothing.
pub(crate) fn stroke_after_swatch(
    style: &photocraft_doc::ShapeStroke,
    change: &crate::shape_paint_ui::PaintChange,
) -> Option<Option<photocraft_doc::ShapeStroke>> {
    use crate::shape_paint_ui::PaintChange;
    match change {
        PaintChange::Color([r, g, b, a]) => {
            Some(Some(photocraft_doc::ShapeStroke { paint: photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(*r, *g, *b, *a)), ..style.clone() }))
        }
        PaintChange::None => Some(None),
        PaintChange::Gradient(_) | PaintChange::OpenPicker => None,
    }
}

/// The Frame tool's options-bar stroke (#13873): the stroke swatch (No Color by default), the
/// width field (0–288 px, decimals kept) and the alignment, always shown. `stroke` is the stroke
/// shown (`None`: off); `style` keeps the width, alignment and colour while it is off. Returns the
/// new stroke when it changed, and whether a width past the range was typed.
fn options_stroke_controls(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    stroke: &Option<photocraft_doc::ShapeStroke>,
) -> (Option<Option<photocraft_doc::ShapeStroke>>, bool) {
    let mut style = stroke.clone().unwrap_or_else(|| app.ui.frame_tool.stroke_style.clone());
    let mut changed: Option<Option<photocraft_doc::ShapeStroke>> = None;
    let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
    let lib = crate::paint_picker::library(&app.session, ui.ctx());
    let mut gradients = crate::shape_gradient_ui::GradientChoices {
        presets: &app.session.presets.gradients,
        current: &app.session.presets.gradient,
        recent: &mut app.ui.recent_gradients,
        fg,
        bg,
    };
    if let Some(change) = crate::shape_paint_ui::swatch(ui, &lib, stroke.as_ref().map(|s| &s.paint), fg, tl!("Set Frame stroke type"), &mut gradients) {
        changed = stroke_after_swatch(&style, &change);
    }
    let stored_width = style.width;
    let (r, alert) = crate::widgets::value_field_alerting(ui, tl!("Set Frame stroke width"), &mut style.width, STROKE_WIDTH, "px", 58.0);
    let r = r.on_hover_text(tl!("Set Frame stroke width"));
    // The field shows its value within 0–288, but only an edit changes the width: a frame that stores 500 px
    // keeps it when the alignment is changed, and the retained tool width is not rewritten by being drawn.
    if !r.changed() {
        style.width = stored_width;
    }
    let mut align = style.align;
    craft_ui::ComboBox::from_id_salt("frame-stroke-align")
        .selected_text(match style.align {
            photocraft_doc::StrokeAlign::Inside => tl!("Inside"),
            photocraft_doc::StrokeAlign::Outside => tl!("Outside"),
            _ => tl!("Center"),
        })
        .show_ui(ui, |ui| {
            for (a, label) in [
                (photocraft_doc::StrokeAlign::Inside, tl!("Inside")),
                (photocraft_doc::StrokeAlign::Center, tl!("Center")),
                (photocraft_doc::StrokeAlign::Outside, tl!("Outside")),
            ] {
                ui.selectable_value(&mut align, a, label);
            }
        })
        .response
        .on_hover_text(tl!("Set align type for stroke"));
    if changed.is_none() && (r.changed() || align != style.align) {
        style.align = align;
        // Width and alignment change the stroke when it is on, and are only kept for later (below)
        // when it is off.
        changed = stroke.as_ref().map(|_| Some(style.clone()));
    }
    // Whatever the stroke is now, its width, alignment and colour are the next ones to use.
    let kept = changed.clone().flatten().unwrap_or_else(|| style.clone());
    app.ui.frame_tool.stroke_style = photocraft_doc::ShapeStroke { align, ..kept };
    (changed, alert)
}

pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if tool != Tool::Frame {
        return false;
    }
    app.frame_tool_ctx = Some(ui.ctx().clone());
    for (icon, shape, label) in [
        ("square", "rectangle", tl!("Create a new rectangular frame")),
        ("circle", "ellipse", tl!("Create a new elliptical frame")),
        ("triangle", "triangle", tl!("Create a new triangular frame")),
        ("pentagon", "polygon", tl!("Create a new polygonal frame")),
        ("shapes", "custom", tl!("Create a new custom frame")),
    ] {
        let current = if app.ui.frame_tool.ellipse {
            "ellipse"
        } else if app.ui.frame_tool.shape.is_empty() {
            "rectangle"
        } else {
            &app.ui.frame_tool.shape
        };
        if crate::icons::button(ui, icon, 24., current == shape, label).clicked() {
            app.ui.frame_tool.shape = shape.into();
            app.ui.frame_tool.ellipse = shape == "ellipse";
        }
    }
    let shape = if app.ui.frame_tool.ellipse { "ellipse" } else { app.ui.frame_tool.shape.as_str() };
    if rounds(if shape.is_empty() { "rectangle" } else { shape }) && radius_field(app, ui) {
        crate::panels::options_bar::range_alert(&mut app.ui, RADIUS_ALERT);
    }
    if app.ui.frame_tool.shape == "polygon" {
        ui.label(tl!("Sides:"));
        let mut n = app.ui.frame_tool.sides.clamp(3, 100) as f32;
        crate::widgets::value_field_named(ui, tl!("Sides:"), &mut n, 3.0..=100.0, "", 44.);
        app.ui.frame_tool.sides = n as u32;
        // The star ratio (whole percents 1–100) and Smooth Star Indents (#13876).
        let t = Tokens::get(ui.ctx());
        let (rect, icon) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        let (c, r) = (rect.center(), rect.width() * 0.45);
        let star: Vec<Pos2> = (0..10)
            .map(|i| {
                let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * i as f32 / 10.0;
                let k = if i % 2 == 0 { r } else { r * 0.45 };
                c + egui::vec2(a.cos(), a.sin()) * k
            })
            .collect();
        ui.painter().add(egui::Shape::closed_line(star, Stroke::new(1.0, t.text_dim)));
        icon.on_hover_text(tl!("Star Ratio"));
        let (_, alert) = crate::widgets::value_field_whole_alerting(ui, tl!("Star Ratio"), &mut app.ui.frame_tool.star_ratio, 1.0..=100.0, "%", 52.0);
        if alert {
            crate::panels::options_bar::range_alert(&mut app.ui, STAR_RATIO_ALERT);
        }
        crate::widgets::checkbox(ui, &mut app.ui.frame_tool.smooth_indents, tl!("Smooth Star Indents"));
    }
    if app.ui.frame_tool.shape == "custom" {
        crate::preset_panels::shape_picker(app, ui);
    }
    crate::widgets::vline(ui, 22.);
    // The tool's own stroke, the one the next frame gets (#13874): a selected frame keeps its own,
    // which the Properties panel's frame section edits.
    let stroke = app.ui.frame_tool.stroke.clone();
    let (changed, alert) = options_stroke_controls(app, ui, &stroke);
    if alert {
        crate::panels::options_bar::range_alert(&mut app.ui, STROKE_WIDTH_ALERT);
    }
    if let Some(stroke) = changed {
        app.ui.frame_tool.stroke = stroke;
    }
    true
}
/// Frame-aware Properties: W/H/X/Y edit the container; the content can be selected/fitted/replaced.
pub fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, l: &Layer) {
    let Some(f) = &l.frame else { return };
    if app.session.active().is_some_and(|st| {
        let locks = st.doc.effective_locks(l.id);
        locks.all || locks.position
    }) {
        ui.disable();
    }
    let gesture = crate::vector_ui::paint_gesture(ui, ui.scope_id().with("frame-properties-gesture"));
    let coalesce = format!("frame-properties-{}-{gesture}", l.id.0);
    let [a, b, c, d, x, y] = f.transform.m;
    let (w, h) = (a.hypot(b), c.hypot(d));
    let mut values = [w as f32, h as f32, x as f32, y as f32];
    let mut changed = false;
    ui.label(tl!("Frame"));
    ui.horizontal(|ui| {
        for (i, label) in [(0, tl!("W")), (1, tl!("H"))] {
            ui.label(label);
            if let Some(v) = values.get_mut(i) {
                changed |= crate::widgets::value_field_named(ui, label, v, 1.0..=300_000.0, "px", 66.).changed();
            }
        }
    });
    ui.horizontal(|ui| {
        for (i, label) in [(2, tl!("X")), (3, tl!("Y"))] {
            ui.label(label);
            if let Some(v) = values.get_mut(i) {
                changed |= crate::widgets::value_field_named(ui, label, v, -300_000.0..=300_000.0, "px", 66.).changed();
            }
        }
    });
    if changed && w > 0. && h > 0. {
        let [nw, nh, nx, ny] = values;
        let r = app.run(
            "layer.frame.set",
            json!({"layer":l.id.0,"transform":[a*f64::from(nw)/w,b*f64::from(nw)/w,c*f64::from(nh)/h,d*f64::from(nh)/h,nx,ny],"coalesce":coalesce}),
        );
        report(app, r);
    }
    let mut stroke = f.stroke.clone();
    ui.horizontal_wrapped(|ui| {
        if stroke_controls(ui, &mut stroke) {
            let r = app.run("layer.frame.set", json!({"layer":l.id.0,"stroke":stroke.as_ref().map(stroke_json).unwrap_or(Value::Null),"coalesce":coalesce}));
            report(app, r);
        }
    });
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        if ui.selectable_label(target(app, l) == Target::Frame, tl!("Frame")).clicked() {
            select(app, l.id, Target::Frame);
        }
        if l.children().is_some_and(|c| !c.is_empty()) {
            if ui.selectable_label(target(app, l) == Target::Content, tl!("Image")).clicked() {
                select(app, l.id, Target::Content);
            }
            if ui.selectable_label(target(app, l) == Target::Both, tl!("Both")).clicked() {
                select(app, l.id, Target::Both);
            }
        }
    });
    if ui.button(if l.children().is_some_and(|c| !c.is_empty()) { tl!("Replace Image…") } else { tl!("Import Image…") }).clicked() {
        select(app, l.id, Target::Frame);
        let ctx = ui.ctx().clone();
        let r = crate::menus::invoke(app, &ctx, "file.placeEmbedded", json!({}));
        report(app, r);
    }
    if l.children().is_some_and(|c| !c.is_empty()) {
        for (mode, label) in [("contentToFrame", tl!("Fit Content to Frame")), ("frameToContent", tl!("Fit Frame to Content"))] {
            if ui.button(label).clicked() {
                let r = app.run("layer.frame.fit", json!({"layer":l.id.0,"mode":mode}));
                report(app, r);
            }
        }
    }
}
#[cfg(test)]
mod tests;

/// A compact on-canvas import/replace bar, following the currently selected frame.
pub fn task_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, xf: &ViewXform) {
    if app.ui.tool != Tool::Frame || app.ui.frame_tool.drag.is_some() || app.ui.transform.is_some() || !app.ui.dialogs.is_empty() {
        return;
    }
    let Some(l) = selected(app) else { return };
    if !visible(app, l.id) {
        return;
    }
    let (id, empty) = (l.id, l.children().is_none_or(|c| c.is_empty()));
    let Some(f) = &l.frame else { return };
    let bounds = f.bounds();
    let p = xf.to_screen((f64::from(bounds.x0) + f64::from(bounds.x1)) as f32 / 2., bounds.y1 as f32);
    let width = if empty { 170.0 } else { 290.0 };
    let at = egui::pos2(
        (p.x - width / 2.).clamp(xf.rect.left() + 8., (xf.rect.right() - width - 8.).max(xf.rect.left() + 8.)),
        (p.y + 18.).min(xf.rect.bottom() - 42.).max(xf.rect.top() + 8.),
    );
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    egui::Area::new(egui::Id::unique("frame-context-bar")).order(egui::Order::Middle).fixed_pos(at).show(&ctx, |ui| {
        egui::Frame::NONE.fill(t.card).stroke(Stroke::new(1., t.field_border)).corner_radius(t.radius).inner_margin(6.).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button(if empty { tl!("Import Image…") } else { tl!("Replace Image…") }).clicked() {
                    select(app, id, Target::Frame);
                    let r = crate::menus::invoke(app, &ctx, "file.placeEmbedded", json!({}));
                    report(app, r);
                }
                if !empty && ui.button(tl!("Fit Content to Frame")).clicked() {
                    let r = app.run("layer.frame.fit", json!({"layer":id.0}));
                    report(app, r);
                }
            });
        });
    });
}

/// Free Transform on the frame thumbnail edits the clipping geometry, leaving its image intact.
pub(crate) fn frame_transform(t: &crate::state::TransformSession) -> bool {
    t.target.as_ref().and_then(|v| v.get("frame")).and_then(Value::as_u64) == Some(t.layer)
}
pub(crate) fn transformed_frame(app: &PhotocraftApp, t: &crate::state::TransformSession) -> Result<Affine, String> {
    if t.warp.is_some() {
        return Err(tl!("A frame supports affine transforms; select its image to warp it").into());
    }
    let [x0, y0, x1, y1] = t.rect;
    let (w, h) = (x1 - x0, y1 - y0);
    let [q0, q1, q2, q3] = t.quad;
    if w <= 0.
        || h <= 0.
        || !t.quad.iter().flatten().all(|v| v.is_finite())
        || ((q1[0] + q3[0] - q0[0]) - q2[0]).abs() > 0.001
        || ((q1[1] + q3[1] - q0[1]) - q2[1]).abs() > 0.001
    {
        return Err(tl!("A frame supports affine transforms; select its image for perspective transforms").into());
    }
    let (a, b, c, d) = ((q1[0] - q0[0]) / w, (q1[1] - q0[1]) / w, (q3[0] - q0[0]) / h, (q3[1] - q0[1]) / h);
    let map = Affine { m: [a, b, c, d, q0[0] - a * x0 - c * y0, q0[1] - b * x0 - d * y0] };
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

/// Arrow keys respect which thumbnail is selected; Shift moves ten document pixels.
pub fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    if app.ui.tool != Tool::Frame
        || app.ui.transform.is_some()
        || app.ui.frame_tool.drag.is_some()
        || ctx.input(|i| i.modifiers.command || i.modifiers.ctrl || i.modifiers.alt)
    {
        return false;
    }
    let Some(l) = selected(app) else { return false };
    let (id, t, frame, child) = (l.id, target(app, l), l.frame.clone(), l.children().and_then(|c| c.first()).map(|l| l.id));
    for (key, dx, dy) in [(egui::Key::ArrowLeft, -1., 0.), (egui::Key::ArrowRight, 1., 0.), (egui::Key::ArrowUp, 0., -1.), (egui::Key::ArrowDown, 0., 1.)] {
        for mods in [egui::Modifiers::SHIFT, egui::Modifiers::NONE] {
            if ctx.input_mut(|i| i.consume_key(mods, key)) {
                let k = if mods.shift { 10. } else { 1. };
                app.nudge_bundle.now = ctx.input(|i| i.time);
                let (command, params) = if t == Target::Frame {
                    let Some(mut f) = frame.clone() else { return true };
                    f.transform = Affine::translate(dx * k, dy * k).mul(&f.transform);
                    ("layer.frame.set", json!({"layer":id.0,"transform":f.transform.m}))
                } else {
                    ("layer.translate", json!({"layer":if t==Target::Content{child.unwrap_or(id).0}else{id.0},"dx":dx*k,"dy":dy*k}))
                };
                let r = crate::nudge_bundle::run(app, "frame", command, params);
                report(app, r);
                return true;
            }
        }
    }
    false
}
#[cfg(test)]
mod move_history_tests;
