//! The Patch tool's cursor shows a right-pointing arrow over the selection (#12130).

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::Drag;
use crate::state::Tool;
use crate::tool_feedback::{Badge, badge, badge_glyph};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("select.rect", json!({"x": 20, "y": 20, "width": 30, "height": 30})).unwrap();
    app
}

const INSIDE: [f64; 2] = [30.0, 30.0];
const OUTSIDE: [f64; 2] = [80.0, 80.0];

#[test]
fn arrow_over_the_selection_without_modifiers() {
    let mut app = app();
    app.hover_doc = Some(INSIDE);
    assert_eq!(badge(&app, Tool::Patch, Modifiers::NONE), Some(Badge::Arrow));
    assert_ne!(Badge::Arrow, Badge::Move, "a new single-arrow glyph");
    let glyph = badge_glyph(Badge::Arrow, egui::pos2(0.0, 0.0), 4.0);
    let tip = glyph.iter().flatten().map(|p| p.x).fold(f32::MIN, f32::max);
    assert!(glyph.iter().flatten().all(|p| p.x <= tip) && glyph.len() == 3, "one shaft and two barbs pointing right");
}

#[test]
fn the_arrow_stays_during_a_drag_started_inside() {
    let mut app = app();
    app.drag = Some(Drag::new(Tool::Patch, INSIDE, vec![[INSIDE[0], INSIDE[1], 1.0]], Modifiers::NONE, false));
    app.hover_doc = Some(OUTSIDE);
    assert_eq!(badge(&app, Tool::Patch, Modifiers::NONE), Some(Badge::Arrow));
}

#[test]
fn outside_and_with_shift_or_alt_the_badges_are_as_before() {
    let mut app = app();
    app.hover_doc = Some(OUTSIDE);
    assert_eq!(badge(&app, Tool::Patch, Modifiers::NONE), None);
    app.hover_doc = Some(INSIDE);
    assert_eq!(badge(&app, Tool::Patch, Modifiers::SHIFT), Some(Badge::Add));
    assert_eq!(badge(&app, Tool::Patch, Modifiers::ALT), Some(Badge::Subtract));
}

#[test]
fn content_aware_move_and_the_curvature_pen_keep_their_cursors() {
    let mut app = app();
    app.hover_doc = Some(INSIDE);
    assert_eq!(badge(&app, Tool::ContentAwareMove, Modifiers::NONE), None);
    assert_eq!(badge_glyph(Badge::Move, egui::pos2(0.0, 0.0), 4.0).len(), 10, "four-arrow glyph unchanged");
}
