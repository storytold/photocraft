//! #12900: the Pen's gear holds a Rubber Band check item, off by default as in Photoshop. Off,
//! nothing is drawn from the last anchor to the pointer while a path is drawn; on, the preview
//! line is. The setting is a tool option: saved, reset by Reset Tool, reported by `ui.inspect`.

use egui::Pos2;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::PenPath;
use super::add_anchor_options_bar_tests::harness;
use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::control::{ControlRequest, Outcome, handle};
use crate::state::{Tool, ToolOptions};

const GEAR: &str = "Set additional pen and path options";
const RUBBER_BAND: &str = "Rubber Band";

/// A 200x200 document with a two-anchor path being drawn by `tool` and the pointer away from it.
fn drawing(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app.ui.pen = Some(PenPath { knots: vec![[[20.0, 20.0]; 3], [[120.0, 30.0]; 3]], ..Default::default() });
    app.hover_doc = Some([60.0, 150.0]);
    app
}

/// The dashes the vector overlay draws: the preview from the last anchor to the pointer is the
/// only dashed line of a path being drawn.
fn dashes(app: &PhotocraftApp) -> usize {
    segments(app).len()
}

/// The line segments the vector overlay draws (document = screen px here), with their strokes.
fn segments(app: &PhotocraftApp) -> Vec<([Pos2; 2], egui::Stroke)> {
    let xf = ViewXform {
        rect: egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(200., 200.)),
        zoom: 1.0,
        center: [100., 100.],
        flip: false,
        rotation: 0.,
        aspect: 1.,
    };
    let doc = app.session.active().unwrap().doc.clone();
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(Default::default(), |ui| super::draw_overlay(app, ui.painter(), &xf, &doc));
    out.textures_delta.clear();
    let mut stack: Vec<egui::Shape> = out.shapes.into_iter().map(|s| s.shape).collect();
    let mut out = Vec::new();
    while let Some(shape) = stack.pop() {
        match shape {
            egui::Shape::Vec(v) => stack.extend(v),
            egui::Shape::LineSegment { points, stroke } => out.push((points, stroke)),
            _ => {}
        }
    }
    out
}

/// #9713: with Rubber Band on, the preview is the segment the next click makes: it leaves the
/// last anchor along its outgoing handle (a dragged, smooth point), ends at the pointer, and is
/// drawn in the Path Options' colour and thickness.
#[test]
fn rubber_band_previews_the_segment_the_next_click_makes() {
    let mut app = drawing(Tool::Pen);
    app.ui.tool_options.pen_rubber_band = true;
    app.ui.view.path_options = crate::view_cmds::PathOptions { color: "red".into(), thickness: 2.0 };
    // The last anchor was dragged out: its outgoing handle points right, away from the pointer.
    let (last, out, to) = ([120.0, 30.0], [170.0, 30.0], [60.0, 150.0]);
    app.ui.pen.as_mut().unwrap().knots[1] = [last, [70.0, 30.0], out];
    // Its handle lines are 1 px; the band has the Path Options' 2 px.
    let band: Vec<([Pos2; 2], egui::Stroke)> = segments(&app).into_iter().filter(|(_, s)| s.width == 2.0).collect();
    assert!(band.len() > 10, "the dashed preview: {band:?}");
    assert!(band.iter().all(|(_, s)| s.color == egui::Color32::from_rgb(0xff, 0x2a, 0x2a)), "in the Path Options' colour");
    let pts: Vec<Pos2> = band.iter().flat_map(|(p, _)| *p).collect();
    // The click's segment: the cubic from the last anchor over its handle to the pointer.
    let curve: Vec<Pos2> = (0..=2000)
        .map(|i| {
            let t = f64::from(i) / 2000.0;
            let u = 1.0 - t;
            let at = |k: usize| u * u * u * last[k] + 3.0 * u * u * t * out[k] + 3.0 * u * t * t * to[k] + t * t * t * to[k];
            Pos2::new(at(0) as f32, at(1) as f32)
        })
        .collect();
    let off_curve = |p: &Pos2| curve.iter().map(|c| c.distance(*p)).fold(f32::INFINITY, f32::min);
    assert!(pts.iter().all(|p| off_curve(p) < 0.6), "every dash lies on the segment the click makes");
    let near = |q: [f64; 2]| pts.iter().map(|p| p.distance(Pos2::new(q[0] as f32, q[1] as f32))).fold(f32::INFINITY, f32::min);
    assert!(near(last) < 0.5 && near(to) < 4.0, "from the last anchor to the pointer");
    // It bends with the handle: a straight band would stay on the chord.
    let chord = |p: &Pos2| ((to[0] - last[0]) * (last[1] - f64::from(p.y)) - (last[0] - f64::from(p.x)) * (to[1] - last[1])).abs() / 120f64.hypot(60.0);
    assert!(pts.iter().map(chord).fold(0.0, f64::max) > 12.0, "bent by the outgoing handle");

    // Off, nothing follows the pointer.
    app.ui.tool_options.pen_rubber_band = false;
    assert!(segments(&app).iter().all(|(_, s)| s.width != 2.0));
}

#[test]
fn off_by_default_the_pen_draws_no_line_to_the_pointer() {
    let mut app = drawing(Tool::Pen);
    assert!(!app.ui.tool_options.pen_rubber_band, "off by default, as in Photoshop");
    assert_eq!(dashes(&app), 0, "no preview line");
    app.ui.tool_options.pen_rubber_band = true;
    assert!(dashes(&app) > 5, "Rubber Band on: the dashed preview line");
    app.ui.tool_options.pen_rubber_band = false;
    assert_eq!(dashes(&app), 0);
}

#[test]
fn the_other_pens_keep_their_preview() {
    // The Curvature Pen's preview is #12756; a path of another pen is not the Pen's to hide.
    let mut app = drawing(Tool::CurvaturePen);
    if let Some(pen) = app.ui.pen.as_mut() {
        pen.curvature = true;
    }
    assert!(dashes(&app) > 5, "Curvature Pen");
    assert!(dashes(&drawing(Tool::FreeformPen)) > 5, "Freeform Pen");
}

#[test]
fn gear_popup_has_the_rubber_band_check_item() {
    let mut h = harness(Tool::Pen);
    assert!(h.query_by_label(RUBBER_BAND).is_none(), "only in the gear's popup");
    let gear = h.get_by_label(GEAR).rect();
    let auto = h.get_by_label("Auto Add/Delete").rect();
    assert!(gear.right() <= auto.left() && (gear.center().y - auto.center().y).abs() < 4.0, "the gear sits before Auto Add/Delete: {gear:?} {auto:?}");

    h.get_by_label(GEAR).click();
    h.run_steps(4);
    h.get_by_label(RUBBER_BAND).click();
    h.run_steps(4);
    assert!(h.state().ui.tool_options.pen_rubber_band, "the check item turns the option on");
    h.get_by_label(RUBBER_BAND).click();
    h.run_steps(4);
    assert!(!h.state().ui.tool_options.pen_rubber_band, "and off again");
}

#[test]
fn only_the_pen_has_the_gear() {
    for tool in [Tool::FreeformPen, Tool::CurvaturePen, Tool::AddAnchorPoint] {
        assert!(harness(tool).query_by_label(GEAR).is_none(), "{tool:?}");
    }
}

#[test]
fn the_setting_is_saved_inspected_and_reset() {
    // Saved with the tool options; a save from before the option reads as off.
    let mut v = serde_json::to_value(ToolOptions::default()).unwrap();
    assert_eq!(v["pen_rubber_band"], json!(false));
    v["pen_rubber_band"] = json!(true);
    let on: ToolOptions = serde_json::from_value(v.clone()).unwrap();
    assert!(on.pen_rubber_band);
    v.as_object_mut().unwrap().remove("pen_rubber_band");
    let old: ToolOptions = serde_json::from_value(v).unwrap();
    assert!(!old.pen_rubber_band);

    let mut app = drawing(Tool::Pen);
    let ctx = egui::Context::default();
    let inspect = |app: &mut PhotocraftApp| {
        let (req, _rx) = ControlRequest::new("ui.inspect", json!({}));
        match handle(app, &ctx, &req) {
            Outcome::Done(v) => v["result"]["toolOptions"]["pen_rubber_band"].clone(),
            _ => panic!("ui.inspect: expected an immediate reply"),
        }
    };
    assert_eq!(inspect(&mut app), json!(false));
    app.ui.tool_options.pen_rubber_band = true;
    app.ui.tool_options.feather = 12.0;
    assert_eq!(inspect(&mut app), json!(true));

    // Reset Tool turns it off and leaves another tool's option alone.
    crate::menus::invoke(&mut app, &ctx, "tool.reset", json!({})).unwrap();
    assert!(!app.ui.tool_options.pen_rubber_band);
    assert_eq!(app.ui.tool_options.feather, 12.0);
    assert_eq!(inspect(&mut app), json!(false));
}
