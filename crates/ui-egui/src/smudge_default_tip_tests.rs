//! #12950: The Smudge tool gets a fresh 13 px tip with hardness 0 % and spacing 25 %.

use serde_json::json;

use super::*;
use crate::canvas::paint_mouse;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 60})).unwrap();
    app
}

/// #12950: Fresh Smudge gets its own tip (13 px, hardness 0 %, spacing 25 %).
#[test]
fn smudge_default_tip() {
    let app = app();
    let brush = paint_mouse::fresh_brush(Tool::Smudge, &app.session.tools.brush);
    assert_eq!(brush.size, 13.0);
    assert_eq!(brush.hardness, 0.0);
    assert_eq!(brush.spacing, 0.25);
    // Smoothing is untouched: keeps first_smoothing() (10 %).
    assert_eq!(brush.smoothing.amount, 0.1);
}

/// #12950: Smudge picked for the first time starts at its own tip, not the outgoing tool's brush.
#[test]
fn smudge_first_use_gets_own_tip() {
    let mut app = app();
    // Switch away from Smudge first so it has no stored brush.
    app.ui.tool = Tool::Brush;
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    // Now pick Smudge: first-use path through sync_tool_brush.
    app.ui.tool = Tool::Smudge;
    paint_mouse::sync_tool_brush(&mut app);
    let brush = app.session.tools.brush.clone();
    assert_eq!(brush.size, 13.0);
    assert_eq!(brush.hardness, 0.0);
    assert_eq!(brush.spacing, 0.25);
    assert_eq!(brush.smoothing.amount, 0.1);
}
