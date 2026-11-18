//! #11978: the Perspective Crop options bar shows its Cancel and Commit buttons only during a
//! session (a quad, or corners being clicked).

use craft::behavior::kurbo::Point;
use egui::Modifiers;
use serde_json::json;

use super::*;
use crate::canvas::testing::{ToolEvent, tool_event};

/// A 200 × 150 document with the Perspective Crop tool chosen and no session.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PerspectiveCrop;
    app
}

#[test]
fn commit_and_cancel_show_only_while_something_is_pending() {
    let mut app = app();
    assert!(!commit_shown(&app), "nothing pending");

    // A box drag in progress.
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 120.0, y: 90.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(commit_shown(&app), "a drag");
    tool_event(&mut app, ToolEvent::Up { x: 120.0, y: 90.0 }, Modifiers::NONE);
    assert!(commit_shown(&app), "the quad it drew");
    cancel(&mut app);
    assert!(!commit_shown(&app), "gone after cancel");

    // A quad.
    app.canvas_tools.perspective.tool.set(Some([Point::new(20.0, 20.0), Point::new(180.0, 20.0), Point::new(180.0, 130.0), Point::new(20.0, 130.0)]));
    assert!(commit_shown(&app), "a quad");
    apply(&mut app);
    assert!(!commit_shown(&app), "gone after commit");
    assert_eq!(app.session.active().unwrap().doc.size.width, 160, "Commit cropped to the quad");

    // Clicked corners.
    let mut app = self::app();
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 20.0 }, Modifiers::NONE);
    assert!(commit_shown(&app), "clicked corners");
    cancel(&mut app);
    assert!(!commit_shown(&app), "gone after cancel");
    assert_eq!(app.session.active().unwrap().doc.size.width, 200, "nothing cropped");
}
