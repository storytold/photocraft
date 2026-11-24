//! #13602: a brush tip under 3 points across on screen shows as the framework's small tip (craft-libs'
//! `PaintedCursor::SmallTip`): the tip itself, at its own size, inside four crosshair arms that leave it clear, rather
//! than the plain crosshair. Tips 3 points across or more keep their outline, and a Pencil square under 3 points gets
//! the same arms around it. Driven through real egui input on the real canvas at 100 %; the assertions read the cursor
//! the tool asked the tool host for.

use craft::behavior::{Cursor, PaintedCursor};
use egui::Modifiers;
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::guide_cursor_tests::{harness, hover};
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The cursor the tool asked for over the canvas, away from the harness's guide, with a hard tip `size` px across.
fn cursor(h: &mut H, size: f32) -> PaintedCursor {
    h.state_mut().session.tools.brush.size = size;
    h.state_mut().session.tools.brush.hardness = 1.0;
    // Off the pixel grid, so a Pencil square sits off the pointer.
    hover(h, 100.3, 150.3, Modifiers::NONE);
    hover(h, 100.4, 150.4, Modifiers::NONE);
    match h.state().canvas_tools.last_response.as_ref().and_then(|r| r.cursor.clone()) {
        Some(Cursor::Painted(painted)) => painted,
        other => panic!("a painted cursor, not {other:?}"),
    }
}

/// The distance from the pointer to a small tip's farthest point; `None` for any other cursor.
fn small_tip(c: &PaintedCursor) -> Option<f32> {
    match c {
        PaintedCursor::SmallTip { extent, .. } => Some(*extent),
        _ => None,
    }
}

#[test]
fn a_round_brush_tip_under_three_points_shows_at_its_size_inside_the_arms() {
    let mut h = harness(Tool::Brush);
    for size in [1.0f32, 2.0] {
        // Normal Brush Tip draws where a dab of this hard tip reaches half coverage, at least the framework's dot.
        let extent = photocraft_paint::half_coverage_radius(size / 2.0, 1.0).max(PaintedCursor::SMALL_TIP_DOT);
        let c = cursor(&mut h, size);
        let at = small_tip(&c).unwrap_or_else(|| panic!("{size} px: a small tip, not {c:?}"));
        assert!((at - extent).abs() < 0.01, "{size} px: the tip's own radius {extent}, not {at}");
    }
    // The outline Normal Brush Tip draws is the half-coverage one: a 3 px hard tip's is just under 3 points across
    // (a small tip still), a 4 px one's just under 4.
    for size in [4.0f32, 6.0] {
        assert!(photocraft_paint::half_coverage_radius(size / 2.0, 1.0) >= PaintedCursor::SMALL_TIP / 2.0);
        let c = cursor(&mut h, size);
        assert!(matches!(c, PaintedCursor::Circle { .. }), "{size} px keeps the circle: {c:?}");
    }
}

#[test]
fn a_pencil_square_under_three_points_gets_the_arms_and_a_larger_one_hides_the_pointer() {
    let mut h = harness(Tool::Pencil);
    for size in [1.0f32, 2.0] {
        let c = cursor(&mut h, size);
        let at = small_tip(&c).unwrap_or_else(|| panic!("{size} px: a small tip, not {c:?}"));
        // The square is snapped to the pixel grid, so the arms clear its corner farthest from the pointer.
        assert!(at > size / 2.0 && at < size * std::f32::consts::SQRT_2 + 0.01, "{size} px: {at}");
    }
    assert_eq!(cursor(&mut h, 8.0), PaintedCursor::Hidden, "the canvas shows an 8 px square itself");
    h.state_mut().run("prefs.set", json!({"path": "cursors.showCrosshairInBrushTip", "value": true})).unwrap();
    assert_eq!(cursor(&mut h, 8.0), PaintedCursor::Crosshair, "with Show Crosshair in Brush Tip");
}

#[test]
fn caps_lock_and_precise_keep_the_crosshair_for_a_small_tip() {
    let mut h = harness(Tool::Brush);
    h.state_mut().run("prefs.set", json!({"path": "cursors.painting", "value": "precise"})).unwrap();
    assert_eq!(cursor(&mut h, 2.0), PaintedCursor::Crosshair);
}
