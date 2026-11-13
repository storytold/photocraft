//! #12929: Quick Selection starts at 30 px, and Reset Tool only resets its size,
//! keeping hardness, spacing and angle as they are.

use serde_json::json;

use super::*;
use crate::canvas::paint_mouse;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 60})).unwrap();
    app
}

/// #12929: Fresh Quick Selection gets size 30 px, keeping current hardness/spacing/angle.
#[test]
fn quick_selection_first_use_gets_30px_tip() {
    let mut app = app();
    app.ui.tool = Tool::QuickSelection;
    let brush = paint_mouse::fresh_brush(Tool::QuickSelection, &app.session.tools.brush);
    assert_eq!(brush.size, 30.0, "Quick Selection default size: {brush:?}");
    // Hardness, spacing and angle are preserved from current brush settings.
    assert_eq!(brush.hardness, app.session.tools.brush.hardness);
    assert_eq!(brush.spacing, app.session.tools.brush.spacing);
    assert_eq!(brush.angle, app.session.tools.brush.angle);
}

/// #12929: Other tools keep their default tips after Reset Tool.
#[test]
fn quick_selection_reset_does_not_affect_other_tools() {
    let app = app();
    let brush = paint_mouse::fresh_brush(Tool::Brush, &app.session.tools.brush);
    assert_eq!((brush.size, brush.hardness, brush.spacing), (13.0, 0.0, 0.25));
}
