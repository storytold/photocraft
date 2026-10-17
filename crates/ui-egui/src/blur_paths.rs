//! Filter › Blur Gallery › Path Blur… on the image (#1759), in the Blur Gallery workspace
//! ([`crate::blur_pins`]): the blur follows paths drawn over the image. A drag on the image draws
//! a new path from where it starts to where it ends; a drag on a point moves it, a click on the
//! selected path's line adds a point there, and an Alt-click on a point removes it (a path keeps
//! two). Delete or Backspace removes the selected path. The dialog shows the selected path's Speed
//! and Taper. Each path is drawn with an arrow at its end, where Taper fades the blur.
//!
//! The paths live in the dialog's fields as the command takes them, `paths: [{points, speed,
//! taper}]` with points as fractions of the filter's bounds, so the preview follows every change,
//! OK applies them in one step and automation sets them with `ui.dialog.set`.

use egui::{Color32, Pos2, Stroke};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::blur_pins::{bounds, from_doc};
use crate::canvas::ViewXform;

pub const COMMAND: &str = "filter.blurGallery.pathBlur";
/// Paths and points per path: more than anyone draws by hand, and the per-pixel loop stays bounded.
pub const MAX_PATHS: usize = 16;
pub const MAX_POINTS: usize = 32;
const MAX_SPEED: f64 = 500.0;
const DEFAULT_SPEED: f64 = 50.0;
/// Screen sizes (points): a point's handle, how near a press must be to grab a point or a line,
/// the shortest drag that draws a path, and the end arrow.
const HANDLE: f32 = 4.5;
const GRAB: f32 = 9.0;
const LINE_GRAB: f32 = 5.0;
const MIN_DRAW: f32 = 6.0;
const ARROW: f32 = 11.0;

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.get("__command").and_then(Value::as_str) == Some(COMMAND)
}

/// One path: points as fractions of the bounds, its blur length in pixels, and how much it fades
/// towards its end (0–100).
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub points: Vec<[f64; 2]>,
    pub speed: f64,
    pub taper: f64,
}

/// Turn a freshly opened Path Blur dialog into the workspace: its one straight path, selected.
pub fn init(fields: &mut Map<String, Value>) {
    let num = |k: &str, d: f64| fields.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(d);
    let path = Path {
        points: vec![[num("startX", 0.2), num("startY", 0.5)], [num("endX", 0.8), num("endY", 0.5)]],
        speed: num("speed", DEFAULT_SPEED),
        taper: num("taper", 0.0),
    };
    for k in ["startX", "startY", "endX", "endY"] {
        fields.remove(k);
    }
    set_paths(fields, &[path]);
    select(fields, Some(0));
}

/// The paths in `fields`, cleaned: finite, in range, at most [`MAX_PATHS`] of 2 to
/// [`MAX_POINTS`] points (shorter ones are dropped).
pub fn paths(fields: &Map<String, Value>) -> Vec<Path> {
    let Some(list) = fields.get("paths").and_then(Value::as_array) else { return Vec::new() };
    let num = |v: &Value, k: &str, d: f64| v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(d);
    list.iter()
        .filter_map(|v| {
            let points: Vec<[f64; 2]> = v
                .get("points")?
                .as_array()?
                .iter()
                .filter_map(|q| {
                    let (x, y) = (q.get(0)?.as_f64()?, q.get(1)?.as_f64()?);
                    (x.is_finite() && y.is_finite()).then(|| [x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)])
                })
                .take(MAX_POINTS)
                .collect();
            (points.len() >= 2).then(|| Path {
                points,
                speed: num(v, "speed", DEFAULT_SPEED).clamp(0.0, MAX_SPEED),
                taper: num(v, "taper", 0.0).clamp(0.0, 100.0),
            })
        })
        .take(MAX_PATHS)
        .collect()
}

fn set_paths(fields: &mut Map<String, Value>, paths: &[Path]) {
    let list: Vec<Value> = paths.iter().map(|p| json!({"points": p.points, "speed": p.speed, "taper": p.taper})).collect();
    fields.insert("paths".into(), Value::Array(list));
}

/// The selected path's index, if it exists.
pub fn selected(fields: &Map<String, Value>) -> Option<usize> {
    let n = paths(fields).len();
    fields.get("__selected").and_then(Value::as_u64).map(|i| i as usize).filter(|i| *i < n)
}

fn select(fields: &mut Map<String, Value>, i: Option<usize>) {
    fields.insert("__selected".into(), i.map_or(Value::Null, |i| json!(i)));
    // The dialog's Speed and Taper (remembered for next time) follow the selected path.
    if let Some(p) = i.and_then(|i| paths(fields).get(i).cloned()) {
        fields.insert("speed".into(), json!(p.speed));
        fields.insert("taper".into(), json!(p.taper));
    }
}

/// Remove the selected path; the one before it (or the new first) is selected.
pub fn remove_selected(fields: &mut Map<String, Value>) -> bool {
    let Some(i) = selected(fields) else { return false };
    let mut list = paths(fields);
    list.remove(i);
    set_paths(fields, &list);
    select(fields, if list.is_empty() { None } else { Some(i.saturating_sub(1)) });
    true
}

/// The dialog body: the selected path's Speed and Taper, how to draw paths, and Preview.
pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    match selected(f) {
        Some(i) => {
            let mut list = paths(f);
            if let Some(p) = list.get_mut(i) {
                let (mut speed, mut taper) = (p.speed as f32, p.taper as f32);
                let a = crate::widgets::slider_row(ui, tl!("Speed"), &mut speed, 0.0..=MAX_SPEED as f32, "px", None).changed();
                let b = crate::widgets::slider_row(ui, tl!("Taper"), &mut taper, 0.0..=100.0, "%", None).changed();
                if a || b {
                    (p.speed, p.taper) = (f64::from(speed.round()).clamp(0.0, MAX_SPEED), f64::from(taper.round()).clamp(0.0, 100.0));
                    set_paths(f, &list);
                    select(f, Some(i));
                }
            }
        }
        None => {
            ui.label(egui::RichText::new(tl!("No path selected")).color(t.text_dim));
        }
    }
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new(tl!(
                "Drag on the image to draw a path; drag its points to bend it, click its line to add a point, and Alt-click a point to remove it. Delete removes the selected path."
            ))
            .color(t.text_dim)
            .size(11.5),
        )
        .wrap(),
    );
    let typing = ui.ctx().memory(|m| m.focused().is_some());
    if !typing && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete) || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)) {
        remove_selected(f);
        ui.ctx().request_repaint();
    }
    if let Some(mut preview) = f.get("__preview").and_then(Value::as_bool) {
        ui.add_space(4.0);
        crate::widgets::checkbox(ui, &mut preview, tl!("Preview"));
        f.insert("__preview".into(), json!(preview));
    }
}

/// What a press grabs: a point (of the selected path first), the selected path's line between
/// points `k` and `k + 1`, or nothing (a new path).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    Point(usize, usize),
    Segment(usize, usize),
    New,
}

fn screen_points(b: [f64; 4], xf: &ViewXform, p: &Path) -> Vec<Pos2> {
    p.points.iter().map(|q| xf.to_screen((b[0] + q[0] * (b[2] - b[0])) as f32, (b[1] + q[1] * (b[3] - b[1])) as f32)).collect()
}

/// Distance in screen points from `at` to the segment from `a` to `b`.
fn segment_distance(at: Pos2, a: Pos2, b: Pos2) -> f32 {
    let d = b - a;
    let l2 = d.length_sq();
    let t = if l2 <= f32::EPSILON { 0.0 } else { ((at - a).dot(d) / l2).clamp(0.0, 1.0) };
    at.distance(a + d * t)
}

fn grab(fields: &Map<String, Value>, xf: &ViewXform, at: Pos2) -> Grab {
    let Some(b) = bounds(fields) else { return Grab::New };
    let list = paths(fields);
    let sel = selected(fields);
    // The selected path's points first, then any path's, nearest.
    let order: Vec<usize> = sel.into_iter().chain((0..list.len()).filter(|i| Some(*i) != sel)).collect();
    for &i in &order {
        let Some(p) = list.get(i) else { continue };
        let near = screen_points(b, xf, p).iter().enumerate().map(|(k, q)| (k, q.distance(at))).filter(|(_, d)| *d <= GRAB).min_by(|x, y| x.1.total_cmp(&y.1));
        if let Some((k, _)) = near {
            return Grab::Point(i, k);
        }
    }
    if let Some(i) = sel
        && let Some(p) = list.get(i)
        && p.points.len() < MAX_POINTS
    {
        let pts = screen_points(b, xf, p);
        if let Some(k) = pts.windows(2).position(|w| segment_distance(at, w[0], w[1]) <= LINE_GRAB) {
            return Grab::Segment(i, k);
        }
    }
    Grab::New
}

/// The pointer on the free image under the dialog (see [`crate::blur_pins::canvas`]).
pub fn canvas(app: &mut PhotocraftApp, ctx: &egui::Context, xf: &ViewXform, over: Pos2, press: Option<Pos2>) -> bool {
    let Some(d) = app.ui.dialogs.last_mut().filter(|d| owns(&d.fields)) else { return false };
    let f = &mut d.fields;
    let Some(b) = bounds(f) else { return false };
    let (start, down, alt) = ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.modifiers.alt));
    ctx.set_cursor_icon(match grab(f, xf, over) {
        Grab::Point(..) => egui::CursorIcon::Grab,
        Grab::Segment(..) => egui::CursorIcon::Copy,
        Grab::New => egui::CursorIcon::Crosshair,
    });
    let mut list = paths(f);
    if !down && press.is_none() {
        // A path drawn with a click (no drag) is dropped: there's nothing to blur along.
        if let Some(drag) = f.remove("__drag")
            && drag.get("new").and_then(Value::as_bool) == Some(true)
            && let Some(i) = drag.get("path").and_then(Value::as_u64).map(|i| i as usize)
            && let Some(p) = list.get(i)
            && let [a, c] = screen_points(b, xf, p)[..]
            && a.distance(c) < MIN_DRAW
        {
            list.remove(i);
            set_paths(f, &list);
            select(f, if list.is_empty() { None } else { Some(list.len() - 1) });
        }
        return true;
    }
    let Some(at) = press else { return true };
    let doc = xf.to_doc(at);
    if start {
        let drag = match grab(f, xf, at) {
            Grab::Point(i, k) if alt => {
                // Alt-click removes the point; a path keeps two.
                if let Some(p) = list.get_mut(i)
                    && p.points.len() > 2
                {
                    p.points.remove(k);
                    set_paths(f, &list);
                }
                select(f, Some(i));
                ctx.request_repaint();
                return true;
            }
            Grab::Point(i, k) => {
                select(f, Some(i));
                let q = list.get(i).and_then(|p| p.points.get(k)).copied().unwrap_or([0.0, 0.0]);
                let c = [b[0] + q[0] * (b[2] - b[0]), b[1] + q[1] * (b[3] - b[1])];
                json!({"path": i, "point": k, "dx": c[0] - doc[0], "dy": c[1] - doc[1]})
            }
            Grab::Segment(i, k) => {
                let (x, y) = from_doc(b, doc);
                if let Some(p) = list.get_mut(i) {
                    p.points.insert(k + 1, [x, y]);
                }
                set_paths(f, &list);
                select(f, Some(i));
                json!({"path": i, "point": k + 1, "dx": 0.0, "dy": 0.0})
            }
            Grab::New if list.len() < MAX_PATHS && (b[0]..=b[2]).contains(&doc[0]) && (b[1]..=b[3]).contains(&doc[1]) => {
                // A new path takes the selected path's Speed and Taper, as the dialog shows them.
                let num = |k: &str, d: f64, max: f64| f.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(d).clamp(0.0, max);
                let (x, y) = from_doc(b, doc);
                list.push(Path { points: vec![[x, y], [x, y]], speed: num("speed", DEFAULT_SPEED, MAX_SPEED), taper: num("taper", 0.0, 100.0) });
                set_paths(f, &list);
                select(f, Some(list.len() - 1));
                json!({"path": list.len() - 1, "point": 1, "dx": 0.0, "dy": 0.0, "new": true})
            }
            Grab::New => return true,
        };
        f.insert("__drag".into(), drag);
        ctx.request_repaint();
        return true;
    }
    let Some(drag) = f.get("__drag").cloned() else { return true };
    let index = |k: &str| drag.get(k).and_then(Value::as_u64).map(|i| i as usize);
    let num = |k: &str| drag.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    if let (Some(i), Some(k)) = (index("path"), index("point"))
        && let Some(q) = list.get_mut(i).and_then(|p| p.points.get_mut(k))
    {
        let (x, y) = from_doc(b, [doc[0] + num("dx"), doc[1] + num("dy")]);
        *q = [x, y];
        set_paths(f, &list);
        ctx.request_repaint();
    }
    true
}

/// Draw the paths while the workspace is open: each as a line with an arrow at its end, the
/// selected one brighter and with its points.
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(d) = app.ui.dialogs.last().filter(|d| owns(&d.fields)) else { return };
    let Some(b) = bounds(&d.fields) else { return };
    let sel = selected(&d.fields);
    for (i, p) in paths(&d.fields).iter().enumerate() {
        let pts = screen_points(b, xf, p);
        let on = sel == Some(i);
        let (dark, light) = if on {
            (Stroke::new(3.5, Color32::from_black_alpha(150)), Stroke::new(1.75, Color32::WHITE))
        } else {
            (Stroke::new(2.5, Color32::from_black_alpha(110)), Stroke::new(1.0, Color32::from_white_alpha(170)))
        };
        painter.add(egui::Shape::line(pts.clone(), dark));
        painter.add(egui::Shape::line(pts.clone(), light));
        // The arrow at the end: the blur runs along the path and Taper fades it towards here.
        if let [.., a, c] = pts[..] {
            let dir = (c - a).normalized();
            if dir.x.is_finite() && dir.y.is_finite() && dir != egui::Vec2::ZERO {
                let side = egui::vec2(-dir.y, dir.x);
                let head = [c - dir * ARROW + side * ARROW * 0.5, c, c - dir * ARROW - side * ARROW * 0.5];
                painter.add(egui::Shape::line(head.to_vec(), dark));
                painter.add(egui::Shape::line(head.to_vec(), light));
            }
        }
        if on {
            for q in &pts {
                painter.circle(*q, HANDLE, Color32::WHITE, Stroke::new(1.0, Color32::from_black_alpha(180)));
            }
        }
    }
}
