//! #12929: Quick Selection starts, and comes back after either reset, at 30 px. Only its size is
//! measured: hardness, spacing and angle survive, while its selection mode and options reset.

use serde_json::json;

use super::*;
use crate::control::{ControlRequest, handle};

/// A 200 × 200 document, synced as a frame would.
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

/// Size and the geometry that Quick Selection keeps across a reset, as `brush.get` reports them.
fn tip(app: &mut PhotocraftApp) -> [f64; 4] {
    let b = app.run("brush.get", json!({})).unwrap();
    ["size", "hardness", "spacing", "angle"].map(|k| (b[k].as_f64().unwrap() * 1e4).round() / 1e4)
}

const RESET: [f64; 4] = [30.0, 0.0, 0.01, 180.0];

/// A tip that makes a reset or a first pick observable, and non-default Quick Selection options.
fn scramble(app: &mut PhotocraftApp) {
    let b = &mut app.session.tools.brush;
    (b.size, b.hardness, b.spacing, b.angle) = (57.0, 0.0, 0.01, 180.0);
    b.opacity = 0.4;
    b.flow = 0.6;
    app.ui.selection_mode = 2;
    app.ui.tool_options.sample_all.quick_selection = true;
    app.ui.tool_options.enhance_edge = true;
}

/// The measured tip geometry survives; Opacity and Flow keep the reset defaults from main.
fn assert_reset(app: &mut PhotocraftApp) {
    assert_eq!(tip(app), RESET);
    assert_eq!(app.ui.selection_mode, 0, "New selection");
    assert!(!app.ui.tool_options.sample_all.quick_selection, "Sample All Layers is off");
    assert!(!app.ui.tool_options.enhance_edge, "Enhance Edge is off");
    let b = &app.session.tools.brush;
    assert_eq!((b.opacity, b.flow), (1.0, 1.0), "reset Opacity and Flow as on main");
    assert_eq!(b.size.round(), 30.0, "the options-bar brush chip's source");
}

#[test]
fn first_pick_starts_at_30_px_and_keeps_the_outgoing_geometry() {
    let mut app = app();
    pick(&mut app, Tool::Pencil);
    scramble(&mut app);
    pick(&mut app, Tool::QuickSelection);
    assert_eq!(tip(&mut app), RESET);
    // First use still inherits the outgoing stroke-level settings, as it does on main.
    let b = &app.session.tools.brush;
    assert_eq!((b.opacity, b.flow), (0.4, 0.6));
    // It is a starting size, not an override on every subsequent switch to the tool.
    app.session.tools.brush.size = 42.0;
    pick(&mut app, Tool::Pencil);
    pick(&mut app, Tool::QuickSelection);
    assert_eq!(tip(&mut app)[0], 42.0);
}

#[test]
fn reset_tool_and_reset_all_keep_the_active_quick_selection_geometry() {
    let mut app = app();
    pick(&mut app, Tool::QuickSelection);
    for command in ["tool.reset", "tool.resetAll"] {
        scramble(&mut app);
        invoke(&mut app, command, json!({"confirmed": true}));
        assert_reset(&mut app);
    }
}

#[test]
fn reset_all_then_a_later_pick_keeps_the_reset_quick_selection_geometry() {
    let mut app = app();
    pick(&mut app, Tool::QuickSelection);
    scramble(&mut app);
    pick(&mut app, Tool::Pencil);
    invoke(&mut app, "tool.resetAll", json!({"confirmed": true}));
    pick(&mut app, Tool::QuickSelection);
    assert_reset(&mut app);
}

#[test]
fn reset_all_with_another_tool_active_keeps_the_remembered_quick_selection_geometry() {
    let mut app = app();
    pick(&mut app, Tool::QuickSelection);
    scramble(&mut app);
    pick(&mut app, Tool::Pencil);
    let b = &mut app.session.tools.brush;
    (b.size, b.hardness, b.spacing, b.angle) = (17.0, 0.7, 0.8, 35.0);
    invoke(&mut app, "tool.resetAll", json!({"confirmed": true}));
    pick(&mut app, Tool::QuickSelection);
    assert_reset(&mut app);
}
