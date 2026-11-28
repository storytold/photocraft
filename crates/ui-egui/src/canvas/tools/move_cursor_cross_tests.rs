//! The Move tool's cursor (#16819): the arrow with a small four-way cross below right of it, the hotspot on the arrow
//! tip, over the canvas, the pasteboard and locked pixels alike, and for ⌘ held with a painting tool. The plain arrow
//! inside a selection, and inside the transform box while Auto-Select and Show Transform Controls are both on. Driven
//! through real egui input on the real canvas; the assertions read the cursor the tool (or its stand-in) asked the
//! tool host for.

use craft::behavior::{Cursor, CursorIcon, PaintedCursor, PointerGlyph};
use egui::Modifiers;
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::guide_cursor_tests::{harness, hover};
use crate::state::Tool;

/// The cursor the tool host was asked for at document point (`x`, `y`) with `mods` held. The harness's guide is at
/// x = 200, so no point here is on it.
fn cursor_at(h: &mut Harness<'static, Box<PhotocraftApp>>, x: f32, y: f32, mods: Modifiers) -> Option<Cursor> {
    hover(h, x, y, mods);
    h.state().canvas_tools.last_response.as_ref().and_then(|r| r.cursor.clone())
}

fn cross() -> Option<Cursor> {
    Some(Cursor::Painted(PaintedCursor::pointer_tool(PointerGlyph::Arrow, crate::icons::tool_icon(Tool::Move))))
}

fn arrow() -> Option<Cursor> {
    Some(Cursor::System(CursorIcon::Default))
}

/// A shape layer over 150..250 × 100..200, selected, with `show_transform` for Show Transform Controls.
fn with_shape(show_transform: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = harness(Tool::Move);
    let app = h.state_mut();
    app.run("shape.create", json!({"kind": "rect", "rect": [150, 100, 100, 100], "fill": "#ff0000"})).unwrap();
    app.ui.tool_options.move_show_transform = show_transform;
    h.run_steps(2);
    h
}

#[test]
fn the_move_tool_shows_the_arrow_with_the_cross_over_the_canvas_and_the_pasteboard() {
    let mut h = harness(Tool::Move);
    assert_eq!(cursor_at(&mut h, 100.0, 150.0, Modifiers::NONE), cross(), "over the canvas");
    assert_eq!(cursor_at(&mut h, -40.0, 150.0, Modifiers::NONE), cross(), "over the pasteboard");
}

#[test]
fn inside_the_transform_box_it_is_the_plain_arrow_only_with_auto_select_and_the_controls_on() {
    let mut h = with_shape(true);
    assert!(h.state().ui.tool_options.move_auto_select, "Auto-Select is on by default");
    assert_eq!(cursor_at(&mut h, 175.0, 160.0, Modifiers::NONE), arrow(), "inside the box");
    assert_eq!(cursor_at(&mut h, 320.0, 160.0, Modifiers::NONE), cross(), "outside the box");
    h.state_mut().ui.tool_options.move_auto_select = false;
    assert_eq!(cursor_at(&mut h, 175.0, 160.0, Modifiers::NONE), cross(), "Auto-Select off: the cross inside the box too");
    let mut h = with_shape(false);
    assert_eq!(cursor_at(&mut h, 175.0, 160.0, Modifiers::NONE), cross(), "Show Transform Controls off: the cross on the layer");
}

#[test]
fn inside_a_selection_it_is_the_plain_arrow() {
    let mut h = harness(Tool::Move);
    h.state_mut().run("select.rect", json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
    assert_eq!(cursor_at(&mut h, 100.0, 100.0, Modifiers::NONE), arrow(), "inside the selection");
    assert_eq!(cursor_at(&mut h, 300.0, 100.0, Modifiers::NONE), cross(), "outside it");
}

#[test]
fn command_with_a_painting_tool_shows_the_same_move_cursor() {
    let mut h = harness(Tool::Brush);
    let brush = cursor_at(&mut h, 100.0, 150.0, Modifiers::NONE);
    assert_ne!(brush, cross(), "the Brush shows its own cursor");
    assert_eq!(cursor_at(&mut h, 100.0, 150.0, Modifiers::COMMAND), cross(), "⌘: the Move cursor");
}

#[test]
fn a_locked_layers_pixels_show_the_cross_and_no_not_allowed() {
    let mut h = with_shape(false);
    h.state_mut().run("layer.lockLayers", json!({"all": true})).unwrap();
    assert_eq!(cursor_at(&mut h, 175.0, 160.0, Modifiers::NONE), cross());
}
