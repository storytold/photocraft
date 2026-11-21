//! #12868: Reset Tool gives the Pattern Stamp a 21 px soft round tip at 25 % spacing with no
//! smoothing, and selects the first pattern of the first default group for that tool only.
//! Fill and the Paint Bucket keep the shared pattern selection.

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

#[test]
fn reset_tool_gives_the_pattern_stamp_its_tip() {
    let mut app = app();
    pick(&mut app, Tool::PatternStamp);
    let b = &mut app.session.tools.brush;
    (b.size, b.hardness, b.spacing) = (57.0, 1.0, 0.6);
    b.smoothing.amount = 0.1;
    invoke(&mut app, "tool.reset", json!({}));
    let b = &app.session.tools.brush;
    assert_eq!((b.size, b.hardness, b.spacing), (21.0, 0.0, 0.25));
    assert_eq!(b.smoothing.amount, 0.0, "smoothing is measured at 0 after Reset Tool");
    assert_eq!(b.size.round(), 21.0, "the options-bar brush chip reads the rounded size");
}

#[test]
fn reset_tool_selects_the_first_pattern_for_the_tool_only() {
    let mut app = app();
    // Fill and the Paint Bucket read this shared selection.
    invoke(&mut app, "pattern.presets.select", json!({"pattern": "Bricks"}));
    let shared = app.session.presets.pattern.clone();
    assert!(shared.is_some(), "a shared pattern is selected");
    pick(&mut app, Tool::PatternStamp);
    let other = app.session.patterns.items.iter().find(|p| Some(&p.id) != shared.as_ref()).map(|p| p.id.clone());
    app.ui.tool_options.pattern_stamp_pattern = other.clone();
    assert_ne!(crate::retouch_ui::pattern_stamp_pattern_id(&app), shared);

    invoke(&mut app, "tool.reset", json!({}));

    let first = photocraft_engine::presets::patterns::builtin_groups().first().and_then(|g| g.items.first()).cloned();
    assert_eq!(app.ui.tool_options.pattern_stamp_pattern, first, "the tool's own pattern is the first default one");
    assert_eq!(app.session.presets.pattern, shared, "the shared selection Fill and the Paint Bucket read is unchanged");
    assert_eq!(crate::retouch_ui::pattern_stamp_pattern_id(&app), first, "the stroke paints with it");
    let stroke = crate::retouch_ui::pattern_stamp_params(&app, &[[1.0, 1.0, 1.0]]);
    assert_eq!(stroke["pattern"], json!(first));
}

#[test]
fn the_tools_own_pattern_is_what_the_stroke_uses() {
    let mut app = app();
    invoke(&mut app, "pattern.presets.select", json!({"pattern": "Bricks"}));
    let shared = crate::retouch_ui::pattern_stamp_pattern_id(&app);
    let own = app.session.patterns.items.iter().find(|p| Some(&p.id) != shared.as_ref()).map(|p| p.id.clone());
    app.ui.tool_options.pattern_stamp_pattern = own.clone();
    assert_eq!(crate::retouch_ui::pattern_stamp_pattern_id(&app), own, "the tool's choice wins over the shared one");
    // The options bar still shows the shared selection, so Fill and the Paint Bucket are unaffected.
    assert_ne!(app.session.presets.pattern.clone(), own);
}

#[test]
fn a_recorded_pattern_the_library_no_longer_holds_falls_back_to_the_shared_one() {
    let mut app = app();
    invoke(&mut app, "pattern.presets.select", json!({"pattern": "Bricks"}));
    let shared = crate::retouch_ui::pattern_stamp_pattern_id(&app);
    app.ui.tool_options.pattern_stamp_pattern = Some("no such pattern".into());
    assert_eq!(crate::retouch_ui::pattern_stamp_pattern_id(&app), shared, "a stale id never reaches the stroke");
}
