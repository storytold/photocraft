//! #12631: the Art History Brush starts, and comes back after Reset Tool, with the reference
//! application's tip: a 21 px soft round (hardness 0 %, angle 0°, roundness 100 %) at 25 %
//! spacing and full opacity, with no smoothing. Other tools' starting tips are checked by `reset_tip_tests`.

use serde_json::json;

use crate::PhotocraftApp;
use crate::control::{ControlRequest, handle};
use crate::state::Tool;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app
}

/// Run a menu command as the control channel does, then the frame's sync.
fn invoke(app: &mut PhotocraftApp, id: &str, params: serde_json::Value) {
    let ctx = app.ctx().clone();
    let (req, _rx) = ControlRequest::new("ui.menu.invoke", json!({"id": id, "params": params}));
    handle(app, &ctx, &req);
    app.sync_views();
}

/// Pick `tool`, and swap its brush in as a frame does.
fn pick(app: &mut PhotocraftApp, tool: Tool) {
    app.ui.tool = tool;
    crate::paint_mouse::sync_tool_brush(app);
    app.sync_views();
}

/// Size, hardness, spacing, angle, roundness, opacity and smoothing of the live brush, as
/// `brush.get` reports the tip.
fn tip(app: &mut PhotocraftApp) -> [f64; 7] {
    let b = app.run("brush.get", json!({})).unwrap();
    let [size, hardness, spacing, angle, roundness] = ["size", "hardness", "spacing", "angle", "roundness"].map(|k| b[k].as_f64().unwrap());
    let brush = &app.session.tools.brush;
    let (opacity, smoothing) = (f64::from(brush.opacity), f64::from(brush.smoothing.amount));
    [size, hardness, spacing, angle, roundness, opacity, smoothing].map(|v| (v * 1e4).round() / 1e4)
}

/// A tip that is no tool's starting tip, so a reset has something to undo.
fn scramble(app: &mut PhotocraftApp) {
    let b = &mut app.session.tools.brush;
    (b.size, b.hardness, b.spacing, b.angle, b.roundness, b.opacity, b.smoothing.amount) = (57.0, 0.4, 0.6, 30.0, 0.5, 0.3, 0.3);
}

const ART_HISTORY_TIP: [f64; 7] = [21.0, 0.0, 0.25, 0.0, 1.0, 1.0, 0.0];

#[test]
fn reset_tool_gives_the_art_history_brush_a_21_px_soft_tip() {
    let mut app = app();
    pick(&mut app, Tool::ArtHistoryBrush);
    scramble(&mut app);
    invoke(&mut app, "tool.reset", json!({}));
    assert_eq!(tip(&mut app), ART_HISTORY_TIP);
    // The options-bar brush chip reads the live brush's rounded size.
    assert_eq!(app.session.tools.brush.size.round(), 21.0);
}

#[test]
fn a_first_use_starts_with_the_tip() {
    // No stored tool brushes, as on a fresh install: the Art History Brush's first pick, after
    // another tool's tip.
    let mut app = app();
    pick(&mut app, Tool::Brush);
    scramble(&mut app);
    pick(&mut app, Tool::ArtHistoryBrush);
    assert_eq!(tip(&mut app), ART_HISTORY_TIP);
    // Its own tip is then kept across tool switches.
    app.session.tools.brush.size = 42.0;
    pick(&mut app, Tool::Brush);
    pick(&mut app, Tool::ArtHistoryBrush);
    assert_eq!(tip(&mut app)[0], 42.0);
}

#[test]
fn a_first_use_keeps_the_outgoing_pressure_toggles() {
    // The pressure settings of the tip are not measured, so they stay what the first pick gave
    // before: the outgoing brush's.
    let mut app = app();
    pick(&mut app, Tool::Brush);
    (app.session.tools.brush.pressure_size, app.session.tools.brush.pressure_opacity) = (true, true);
    pick(&mut app, Tool::ArtHistoryBrush);
    let b = &app.session.tools.brush;
    assert_eq!((b.pressure_size, b.pressure_opacity), (true, true));
    assert_eq!(tip(&mut app), ART_HISTORY_TIP);
}

#[test]
fn reset_all_tools_gives_it_the_tip_active_or_picked_later() {
    let mut app = app();
    pick(&mut app, Tool::ArtHistoryBrush);
    scramble(&mut app);
    invoke(&mut app, "tool.resetAll", json!({"confirmed": true}));
    assert_eq!(tip(&mut app), ART_HISTORY_TIP, "active");
    let mut app = self::app();
    pick(&mut app, Tool::ArtHistoryBrush);
    scramble(&mut app);
    pick(&mut app, Tool::Pencil);
    invoke(&mut app, "tool.resetAll", json!({"confirmed": true}));
    pick(&mut app, Tool::ArtHistoryBrush);
    assert_eq!(tip(&mut app), ART_HISTORY_TIP, "picked after the reset");
}

/// The other tools' starting tips are checked by `reset_tip_tests`; the History Brush's first use
/// still takes the current tip, as before.
#[test]
fn the_history_brush_first_use_keeps_the_current_tip() {
    let mut a = app();
    pick(&mut a, Tool::Pencil);
    scramble(&mut a);
    pick(&mut a, Tool::HistoryBrush);
    assert_eq!(tip(&mut a)[0], 57.0, "History Brush, first use");
}
