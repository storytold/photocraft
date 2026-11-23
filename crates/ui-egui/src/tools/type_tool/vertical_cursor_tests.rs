//! #12063: the Vertical Type tool's pointer over the canvas is an I-beam on its side inside a dotted
//! box, the Vertical Type Mask tool's the system's sideways I-beam, and the horizontal tools keep the
//! upright I-beam. Driven through real egui input on the real canvas; the assertions read the cursor
//! the tool asked the tool host for.

use craft::behavior::{Cursor, CursorIcon, PaintedCursor};
use egui::Modifiers;
use egui_kittest::Harness;

use super::vertical_cursor::rings;
use crate::PhotocraftApp;
use crate::guide_cursor_tests::{harness, hover};
use crate::state::Tool;

/// The cursor `tool` asked for with the pointer over an empty part of the page.
fn cursor_of(tool: Tool) -> Option<Cursor> {
    let mut h: Harness<'static, Box<PhotocraftApp>> = harness(tool);
    hover(&mut h, 100.0, 100.0, Modifiers::NONE);
    h.state().canvas_tools.last_response.as_ref().and_then(|r| r.cursor.clone())
}

#[test]
fn vertical_type_shows_a_sideways_i_beam_in_a_dotted_box() {
    assert_eq!(cursor_of(Tool::VerticalType), Some(Cursor::Painted(PaintedCursor::outline(rings()))));
}

#[test]
fn the_rings_are_a_sideways_i_beam_and_a_dotted_box() {
    let rings = rings();
    let beam = &rings[0];
    let (min_x, max_x) = beam.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.x), b.max(p.x)));
    let (min_y, max_y) = beam.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
    assert!(max_x - min_x > max_y - min_y, "the I-beam lies across, not up and down");
    assert!(min_x < 0.0 && max_x > 0.0 && min_y < 0.0 && max_y > 0.0, "the hotspot is inside the I-beam");
    // The dots stand at the box's four corners: 9 either side of the hotspot, 4 above and 12 below.
    let near = |x: f32, y: f32| {
        rings[1..].iter().any(|r| r.iter().map(|p| p.x).sum::<f32>() / r.len() as f32 == x && r.iter().map(|p| p.y).sum::<f32>() / r.len() as f32 == y)
    };
    for (x, y) in [(-9.0, -4.0), (9.0, -4.0), (-9.0, 12.0), (9.0, 12.0)] {
        assert!(near(x, y), "a corner dot at ({x}, {y})");
    }
    // Dotted, not a line: the beam, four dots on each of the top and bottom edges and two on each side.
    assert_eq!(rings.len(), 1 + 8 + 4);
}

#[test]
fn vertical_type_mask_shows_the_system_sideways_i_beam() {
    assert_eq!(cursor_of(Tool::VerticalTypeMask), Some(Cursor::System(CursorIcon::VerticalText)));
}

#[test]
fn the_horizontal_type_tools_keep_the_upright_i_beam() {
    for tool in [Tool::Type, Tool::HorizontalTypeMask] {
        assert_eq!(cursor_of(tool), Some(Cursor::System(CursorIcon::Text)), "{tool:?}");
    }
}
