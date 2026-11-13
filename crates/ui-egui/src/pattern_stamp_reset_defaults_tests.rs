use super::*;
use crate::canvas::paint_mouse;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 60})).unwrap();
    app
}

/// #12868: After Reset Tool the Pattern Stamp gets a 21 px soft round tip at 25 % spacing.
#[test]
fn pattern_stamp_reset_sets_tip() {
    let mut app = app();
    app.ui.tool = Tool::PatternStamp;
    let brush = paint_mouse::fresh_brush(Tool::PatternStamp, &app.session.tools.brush);
    assert_eq!(brush.size, 21.0);
    assert_eq!(brush.hardness, 0.0);
    assert_eq!(brush.spacing, 0.25);
    assert_eq!(brush.smoothing.amount, 0.0);
}

/// #12868: After Reset Tool the Pattern Stamp's pattern is the first of the first default group.
#[test]
fn pattern_stamp_reset_selects_first_pattern() {
    let mut app = app();
    app.ui.tool = Tool::PatternStamp;
    // Pick some other pattern first.
    app.session.presets.pattern = app.session.patterns.items.last().map(|p| p.id.clone());
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, crate::tools::tool_reset::RESET, json!({})).unwrap();
    let first = photocraft_engine::presets::patterns::builtin_groups()
        .first()
        .and_then(|g| g.items.first())
        .cloned();
    assert_eq!(app.session.presets.pattern, first);
}
