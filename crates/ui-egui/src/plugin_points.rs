//! Canvas handles for `point` parameters of an open filter dialog (WebAssembly plug-ins declare
//! them in their manifest): each point is a handle on the image that can be clicked or dragged,
//! and the plug-in's optional `guide` draws a dotted line between two of them. The value is the
//! dialog field itself (`[x, y]` in percent of the canvas), so the dialog's numeric fields and the
//! live preview follow the handle exactly as they follow a slider.

use egui::{Color32, Pos2, Stroke};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::filter_dialog::Kind;

/// Screen distance (points) within which a press grabs a handle.
const HIT: f32 = 10.0;
/// Handle radius (points).
const RADIUS: f32 = 6.0;
/// Which handle a drag holds, across frames.
const DRAG: &str = "pc-plugin-point-drag";

/// The points of a dialog, in dialog order, and the guide between two of them (indices).
#[derive(Clone, Debug, PartialEq)]
pub struct Handles {
    pub points: Vec<(String, [f32; 2])>,
    pub guide: Option<(usize, usize)>,
}

/// A point field's value, `[x, y]` clamped to 0-100 (`None` when it is not two finite numbers).
pub fn value(f: &Map<String, Value>, key: &str) -> Option<[f32; 2]> {
    let a = f.get(key)?.as_array()?;
    let [x, y] = a.as_slice() else { return None };
    let p = photocraft_plugins::clamp_point(x.as_f64()?, y.as_f64()?)?;
    Some([p[0] as f32, p[1] as f32])
}

/// The point params of a dialog's spec, with their current values (or defaults); `None` without any.
pub fn handles(f: &Map<String, Value>) -> Option<Handles> {
    let spec = crate::filter_dialog::spec_of(f)?;
    let points: Vec<(String, [f32; 2])> = crate::filter_dialog::parse_spec(&spec)
        .into_iter()
        .filter_map(|p| match p.kind {
            Kind::Point(default) => {
                let v = value(f, &p.key).unwrap_or(default);
                Some((p.key, v))
            }
            _ => None,
        })
        .collect();
    if points.is_empty() {
        return None;
    }
    let names = f.get("__guide").and_then(Value::as_array);
    let index = |i: usize| names.and_then(|n| n.get(i)).and_then(Value::as_str).and_then(|k| points.iter().position(|p| p.0 == k));
    let guide = match (index(0), index(1)) {
        (Some(a), Some(b)) if a != b => Some((a, b)),
        _ => None,
    };
    Some(Handles { points, guide })
}

/// A point in percent of the canvas → document pixels.
pub fn to_doc(p: [f32; 2], size: [u32; 2]) -> [f32; 2] {
    [p[0] / 100.0 * size[0] as f32, p[1] / 100.0 * size[1] as f32]
}

/// Document pixels → percent of the canvas, clamped to it and rounded to 0.01 %.
pub fn to_pct(d: [f64; 2], size: [u32; 2]) -> [f32; 2] {
    let pct = |v: f64, n: u32| if n == 0 || !v.is_finite() { 0.0 } else { ((v / f64::from(n) * 100.0).clamp(0.0, 100.0) * 100.0).round() / 100.0 };
    [pct(d[0], size[0]) as f32, pct(d[1], size[1]) as f32]
}

/// The handle a press at `p` takes: the nearest within [`HIT`], else the first point (it jumps to
/// the press, so clicking the image places the start of a bolt; the end is then dragged).
pub fn pick(screen: &[Pos2], p: Pos2) -> usize {
    screen.iter().enumerate().map(|(i, s)| (i, s.distance(p))).filter(|(_, d)| *d <= HIT).min_by(|a, b| a.1.total_cmp(&b.1)).map_or(0, |(i, _)| i)
}

/// The open filter dialog with point params, when it edits document `idx`.
fn dialog(app: &PhotocraftApp, idx: usize) -> Option<usize> {
    if app.session.active_index() != Some(idx) {
        return None;
    }
    let i = app.ui.dialogs.iter().position(|d| d.fields.contains_key("__filter"))?;
    handles(&app.ui.dialogs.get(i)?.fields).is_some().then_some(i)
}

/// Whether document `idx` shows point handles for an open dialog.
pub fn active(app: &PhotocraftApp, idx: usize) -> bool {
    dialog(app, idx).is_some()
}

/// Pointer handling on the free canvas under the dialog: a press takes a handle ([`pick`]), and the
/// handle follows the pointer until release, writing its dialog field every frame.
pub fn interact(app: &mut PhotocraftApp, ctx: &egui::Context, xf: &ViewXform, size: [u32; 2], rect: egui::Rect, idx: usize) {
    let Some(di) = dialog(app, idx) else { return };
    let Some(h) = app.ui.dialogs.get(di).and_then(|d| handles(&d.fields)) else { return };
    let id = egui::Id::new(DRAG);
    let screen: Vec<Pos2> = h.points.iter().map(|(_, p)| to_doc(*p, size)).map(|d| xf.to_screen(d[0], d[1])).collect();
    let Some(p) = crate::dialogs::free_press(ctx, rect) else {
        if !ctx.input(|i| i.pointer.primary_down()) {
            ctx.data_mut(|d| d.remove::<usize>(id));
        }
        if let Some(hover) = crate::dialogs::free_pointer_over(ctx, rect) {
            let near = screen.iter().any(|s| s.distance(hover) <= HIT);
            ctx.set_cursor_icon(if near { egui::CursorIcon::Grab } else { egui::CursorIcon::Crosshair });
        }
        return;
    };
    let held = if ctx.input(|i| i.pointer.primary_pressed()) { None } else { ctx.data(|d| d.get_temp::<usize>(id)) };
    let i = held.filter(|i| *i < h.points.len()).unwrap_or_else(|| pick(&screen, p));
    ctx.data_mut(|d| d.insert_temp(id, i));
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    let (Some((key, _)), Some(d)) = (h.points.get(i), app.ui.dialogs.get_mut(di)) else { return };
    d.fields.insert(key.clone(), json!(to_pct(xf.to_doc(p), size)));
}

/// Draws the guide (a dashed line) and a handle at every point.
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, size: [u32; 2], idx: usize) {
    let Some(h) = dialog(app, idx).and_then(|i| app.ui.dialogs.get(i)).and_then(|d| handles(&d.fields)) else { return };
    let accent = crate::theme::Tokens::get(painter.ctx()).accent;
    let screen: Vec<Pos2> = h.points.iter().map(|(_, p)| to_doc(*p, size)).map(|d| xf.to_screen(d[0], d[1])).collect();
    if let Some((a, b)) = h.guide
        && let (Some(a), Some(b)) = (screen.get(a), screen.get(b))
    {
        // Dark under light, so the dashes read on any image.
        painter.line_segment([*a, *b], Stroke::new(3.0, Color32::from_black_alpha(90)));
        painter.add(egui::Shape::dashed_line(&[*a, *b], Stroke::new(1.5, Color32::WHITE), 6.0, 5.0));
    }
    for (i, s) in screen.iter().enumerate() {
        painter.circle_filled(*s, RADIUS + 1.5, Color32::from_black_alpha(160));
        painter.circle(*s, RADIUS, if i == 0 { accent } else { Color32::from_gray(30) }, Stroke::new(1.5, Color32::WHITE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(spec: &str, guide: Option<[&str; 2]>) -> Map<String, Value> {
        let mut f = Map::new();
        f.insert("__command".into(), json!("plugin.run"));
        f.insert("__filter".into(), json!(true));
        f.insert("__spec".into(), json!(spec));
        if let Some(g) = guide {
            f.insert("__guide".into(), json!(g));
        }
        f
    }

    const SPEC: &str = r#"{"start":point=[12,12],"end":point=[88,62],"amount":0..100=50}"#;

    #[test]
    fn handles_follow_the_spec_and_the_guide() {
        let mut f = fields(SPEC, Some(["start", "end"]));
        let h = handles(&f).unwrap();
        assert_eq!(h.points, vec![("start".to_string(), [12.0, 12.0]), ("end".to_string(), [88.0, 62.0])]);
        assert_eq!(h.guide, Some((0, 1)));
        // Field values win over defaults, and are clamped; a malformed one falls back to the default.
        f.insert("start".into(), json!([150, -4]));
        f.insert("end".into(), json!("junk"));
        assert_eq!(handles(&f).unwrap().points, vec![("start".to_string(), [100.0, 0.0]), ("end".to_string(), [88.0, 62.0])]);
        // No guide, an unknown or a repeated name: handles but no line.
        assert_eq!(handles(&fields(SPEC, None)).unwrap().guide, None);
        assert_eq!(handles(&fields(SPEC, Some(["start", "nope"]))).unwrap().guide, None);
        assert_eq!(handles(&fields(SPEC, Some(["end", "end"]))).unwrap().guide, None);
        // No point params: no handles.
        assert!(handles(&fields(r#"{"amount":0..100=50}"#, None)).is_none());
    }

    #[test]
    fn percent_and_document_coordinates_round_trip_and_clamp() {
        let size = [400, 200];
        assert_eq!(to_doc([25.0, 50.0], size), [100.0, 100.0]);
        assert_eq!(to_pct([100.0, 100.0], size), [25.0, 50.0]);
        assert_eq!(to_pct([-30.0, 900.0], size), [0.0, 100.0]);
        assert_eq!(to_pct([f64::NAN, 1.0], [0, 0]), [0.0, 0.0]);
        assert_eq!(to_pct([133.0, 7.0], size), [33.25, 3.5]);
    }

    /// The real canvas (`document_area`) at 100 % with a plug-in dialog open whose spec has two
    /// points (the Invert fixture runs the preview; it ignores the extra params).
    fn harness() -> egui_kittest::Harness<'static, PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../plugins/tests/fixtures/invert.wasm");
        app.run("plugin.install", json!({"path": path})).unwrap();
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        app.sync_views();
        app.ui.extras.rulers = false;
        let mut fixed = Map::new();
        fixed.insert("id".into(), json!("org.photocraft.example.invert"));
        fixed.insert("__guide".into(), json!(["start", "end"]));
        crate::filter_dialog::open_with_spec(&mut app, "plugin.run", "Bolt", r#"{"start":point=[10,10],"end":point=[90,90]}"#, fixed);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(4);
        let v = &mut h.state_mut().ui.views[0];
        v.zoom = 1.0;
        v.center = [200.0, 100.0];
        v.fit_pending = false;
        h.run_steps(2);
        h
    }

    fn screen(h: &egui_kittest::Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
        ViewXform::active(h.state()).unwrap().to_screen(x, y)
    }

    /// Primary button down or up at document point `(x, y)`.
    fn button(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, (x, y): (f32, f32), down: bool) {
        let p = screen(h, x, y);
        h.event(egui::Event::PointerMoved(p));
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: down, modifiers: Default::default() });
        h.run_steps(2);
    }

    fn move_to(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, x: f32, y: f32) {
        let p = screen(h, x, y);
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(2);
    }

    fn field(h: &egui_kittest::Harness<'static, PhotocraftApp>, key: &str) -> Value {
        h.state().ui.dialogs.last().unwrap().fields[key].clone()
    }

    #[test]
    fn dragging_a_handle_on_the_canvas_sets_its_dialog_field() {
        let mut h = harness();
        // Drag the end handle (90 %, 90 % of 400 × 200 = 360, 180) to document (100, 50).
        button(&mut h, (360.0, 180.0), true);
        for (x, y) in [(300.0, 150.0), (200.0, 100.0), (100.0, 50.0)] {
            move_to(&mut h, x, y);
            // Live: the field follows during the drag, not only on release.
            assert_eq!(field(&h, "end"), json!([x / 4.0, y / 2.0]));
        }
        button(&mut h, (100.0, 50.0), false);
        assert_eq!(field(&h, "end"), json!([25.0, 25.0]));
        assert_eq!(field(&h, "start"), json!([10.0, 10.0]), "the other point stays");
        // The live preview runs with the dragged value, as it does for a slider (the canvas asks
        // for it only with a GPU texture to show it on, so ask directly here).
        crate::canvas::ensure_filter_preview(h.state_mut(), 0, &egui::Context::default());
        let previewed = h.state().filter_preview.as_ref().map(|p| p.key.params["end"].clone());
        assert_eq!(previewed, Some(json!([25.0, 25.0])));
        assert!(h.state().filter_preview.as_ref().is_some_and(|p| p.result.is_some()), "the plug-in ran for the preview");
        // A click away from both handles places the start point there.
        button(&mut h, (300.0, 40.0), true);
        button(&mut h, (300.0, 40.0), false);
        assert_eq!(field(&h, "start"), json!([75.0, 20.0]));
        assert_eq!(field(&h, "end"), json!([25.0, 25.0]));
        // Dragged off the image, the point stops at its edge.
        button(&mut h, (300.0, 40.0), true);
        move_to(&mut h, 420.0, -15.0);
        button(&mut h, (420.0, -15.0), false);
        assert_eq!(field(&h, "start"), json!([100.0, 0.0]));
        // The plug-in gets the arrays as given.
        let params = crate::filter_dialog::params_of(&h.state().ui.dialogs.last().unwrap().fields);
        assert_eq!(params["start"], json!([100.0, 0.0]));
        assert_eq!(params["end"], json!([25.0, 25.0]));
        assert!(params.get("__guide").is_none());
    }

    #[test]
    fn a_press_takes_the_nearest_handle_or_moves_the_first() {
        let s = [Pos2::new(10.0, 10.0), Pos2::new(100.0, 100.0)];
        assert_eq!(pick(&s, Pos2::new(98.0, 103.0)), 1);
        assert_eq!(pick(&s, Pos2::new(12.0, 9.0)), 0);
        assert_eq!(pick(&s, Pos2::new(50.0, 50.0)), 0, "away from both: the first point jumps there");
        assert_eq!(pick(&[], Pos2::ZERO), 0);
    }
}
