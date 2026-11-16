//! ⌥/Alt-click on the last anchor of the path being drawn takes away its outgoing handle only, as
//! in Photoshop: the incoming handle keeps shaping the curve into that anchor, stays on screen and
//! can be ⌥/Alt-dragged on its own. It was kept, but drawn only while the outgoing handle was
//! pulled out, so both handles seemed to go, and an ⌥-press on its end added an anchor there.

use egui::{Modifiers, Pos2, Rect, vec2};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform, tool_event};
use crate::state::Tool;

/// A 400 × 300 px document with the Pen at 100 %, snapping off so the presses land where they are.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Pen;
    app.ui.views[0].zoom = 1.0;
    app.ui.extras.snap = false;
    app
}

/// A press at `from`, a drag to `to` in a few moves and the release there, with `mods` held.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    if from != to {
        for i in 1..=4 {
            let t = f64::from(i) / 4.0;
            tool_event(app, ToolEvent::Move { x: from[0] + (to[0] - from[0]) * t, y: from[1] + (to[1] - from[1]) * t, pressure: 1.0 }, mods);
        }
    }
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

fn click(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    drag(app, at, at, mods);
}

/// Is `a` within 0.05 px of `b` on both axes?
fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05
}

/// The round ends of the direction handles the canvas draws over the path being drawn
/// (`draw_handle`), in document pixels: the view maps them one to one.
fn handle_ends(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    let xf =
        ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0)), zoom: 1.0, center: [200.0, 150.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput::default());
    let painter = ctx.layer_painter(egui::LayerId::background());
    crate::vector_ui::draw_overlay(app, &painter, &xf, &app.session.active().unwrap().doc);
    let mut out = ctx.end_pass();
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Circle(circle) if circle.radius == 3.0 => Some(xf.to_doc(circle.center)),
            _ => None,
        })
        .collect()
}

/// A corner anchor at (40, 200), then a smooth one at (210, 100) dragged out to (250, 140): its
/// incoming handle is at (170, 60), mirrored, and both handles show.
fn smooth_last_anchor() -> PhotocraftApp {
    let mut app = app();
    click(&mut app, [40.0, 200.0], Modifiers::NONE);
    drag(&mut app, [210.0, 100.0], [250.0, 140.0], Modifiers::NONE);
    let k = app.ui.pen.as_ref().unwrap().knots[1];
    assert!(near(k[0], [210.0, 100.0]) && near(k[1], [170.0, 60.0]) && near(k[2], [250.0, 140.0]), "a smooth anchor: {k:?}");
    let ends = handle_ends(&app);
    assert!(ends.len() == 2 && ends.iter().any(|e| near(*e, [170.0, 60.0])) && ends.iter().any(|e| near(*e, [250.0, 140.0])), "both handles show: {ends:?}");
    app
}

#[test]
fn alt_click_on_the_last_anchor_takes_its_outgoing_handle_and_shows_the_incoming_one() {
    let mut app = smooth_last_anchor();
    click(&mut app, [210.0, 100.0], Modifiers::ALT);
    let pen = app.ui.pen.clone().unwrap();
    assert_eq!(pen.knots.len(), 2, "no anchor was added");
    let k = pen.knots[1];
    assert!(near(k[2], k[0]), "the outgoing handle is gone: {k:?}");
    assert!(near(k[1], [170.0, 60.0]), "the incoming handle stays where it was: {k:?}");
    let ends = handle_ends(&app);
    assert!(ends.len() == 1 && near(ends[0], [170.0, 60.0]), "the incoming handle still shows, alone: {ends:?}");
}

#[test]
fn alt_drag_turns_the_incoming_handle_left_by_an_alt_click() {
    let mut app = smooth_last_anchor();
    click(&mut app, [210.0, 100.0], Modifiers::ALT);
    drag(&mut app, [170.0, 60.0], [150.0, 110.0], Modifiers::ALT);
    let pen = app.ui.pen.clone().unwrap();
    assert_eq!(pen.knots.len(), 2, "no anchor was added on the handle");
    let k = pen.knots[1];
    assert!(near(k[0], [210.0, 100.0]), "the anchor stayed put: {k:?}");
    assert!(near(k[1], [150.0, 110.0]), "the incoming handle followed the pointer: {k:?}");
    assert!(near(k[2], [210.0, 100.0]), "no outgoing handle came back: {k:?}");
    assert!(handle_ends(&app).iter().any(|e| near(*e, [150.0, 110.0])), "the turned handle shows");

    // The next anchor is a corner joined by a straight segment, and only its own handles show:
    // it has none.
    click(&mut app, [300.0, 250.0], Modifiers::NONE);
    assert!(handle_ends(&app).is_empty(), "the previous anchor's incoming handle no longer shows");
    assert!(crate::vector_ui::pen_commit(&mut app, false));
    let doc = &app.session.active().unwrap().doc;
    let k = &doc.work_path.as_ref().unwrap().subpaths[0].knots[1];
    assert!(!k.smooth, "a cusp in the work path");
    assert!(near([k.in_ctrl.x, k.in_ctrl.y], [150.0, 110.0]) && near([k.out_ctrl.x, k.out_ctrl.y], [210.0, 100.0]), "{k:?}");
}
