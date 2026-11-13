//! A brush tip too small to read on its own (#13602): as in Photoshop, the cursor shows the tip at
//! its real size with a crosshair around it whose arms stop short of it, instead of a solid + that
//! covers it. Tests drive the real canvas (`document_area`) and read the OS cursor bitmap it sets.

use egui_kittest::Harness;
use serde_json::json;

use super::remove_cursor_tests::{harness, set_pref};
use crate::PhotocraftApp;
use crate::state::Tool;

/// The alpha of the cursor bitmap's pixels from its hotspot outward, along each of the four
/// arm directions (right, left, down, up), out to the bitmap's edge.
fn rays(h: &Harness<'_, PhotocraftApp>) -> [Vec<u8>; 4] {
    let out = h.output();
    let image = out.platform_output.cursor_image.as_ref().expect("a cursor bitmap");
    let [w, rows] = image.size.map(usize::from);
    let [x, y] = image.hotspot.map(usize::from);
    let at = |col: usize, row: usize| image.rgba[(row * w + col) * 4 + 3];
    [
        (x..w).map(|c| at(c, y)).collect(),
        (0..=x).rev().map(|c| at(c, y)).collect(),
        (y..rows).map(|r| at(x, r)).collect(),
        (0..=y).rev().map(|r| at(x, r)).collect(),
    ]
}

/// Whether one ray runs along an arm that only begins after at least one fully transparent pixel:
/// the crosshair leaves the space round the tip clear. An arm is four or more pixels in a row with
/// alpha > 230 (its light core); a ring or outline crosses a ray in a pixel or two.
fn arm_after_a_gap(ray: &[u8]) -> bool {
    ray.iter().position(|&a| a == 0).is_some_and(|gap| ray[gap..].windows(4).any(|w| w.iter().all(|&a| a > 230)))
}

/// Whether every ray of the cursor bitmap has an arm that starts clear of the hotspot.
fn clear_arms(h: &Harness<'_, PhotocraftApp>) -> bool {
    rays(h).iter().all(|ray| arm_after_a_gap(ray))
}

/// The canvas with `tool` and the `brush` fields given (its size among them) at `zoom`, with Full
/// Size Brush Tip cursors so the outline is the tip's own radius.
fn tip(tool: Tool, brush: serde_json::Value, zoom: f32) -> Harness<'static, PhotocraftApp> {
    let mut h = harness(tool);
    set_pref(&mut h, "cursors.painting", json!("fullSizeTip"));
    h.state_mut().run("tools.setBrush", json!({ "brush": brush })).unwrap();
    h.state_mut().ui.views[0].zoom = zoom;
    h.run_steps(3);
    h
}

#[test]
fn a_tiny_round_tip_shows_inside_arms_that_leave_it_clear() {
    for size in [1, 2, 4] {
        for centre in [false, true] {
            for painting in ["normalTip", "fullSizeTip"] {
                let mut h = tip(Tool::Brush, json!({"size": size}), 1.0);
                set_pref(&mut h, "cursors.showCrosshairInBrushTip", json!(centre));
                set_pref(&mut h, "cursors.painting", json!(painting));
                let rays = rays(&h);
                let what = format!("{size} px at 100 %, {painting}, Show Crosshair {centre}: {rays:?}");
                assert!(rays.iter().all(|r| arm_after_a_gap(r)), "{what}: arms stop short of the tip");
                // The tip itself, at the hotspot or within its 1-2 px radius: the arms (5 px
                // clear of it) cannot reach there.
                assert!(rays.iter().all(|r| r[..=size / 2].iter().any(|&a| a > 0)), "{what}: the tip is drawn");
            }
        }
    }
}

#[test]
fn a_round_tip_three_points_across_or_more_keeps_its_plain_circle() {
    // 4 px is a 2 pt radius at 100 % (small) and 4 pt at 200 % (a readable circle), so zooming
    // in and out switches between the two.
    let small = tip(Tool::Brush, json!({"size": 4}), 1.0);
    assert!(clear_arms(&small), "2 pt radius: the small tip cursor");
    let readable = tip(Tool::Brush, json!({"size": 4}), 2.0);
    assert!(!clear_arms(&readable), "4 pt radius: the plain circle, no arms");
    let back = {
        let mut h = readable;
        h.state_mut().ui.views[0].zoom = 1.0;
        h.run_steps(3);
        h
    };
    assert!(clear_arms(&back), "zoomed back out: the small tip cursor again");
    assert!(!clear_arms(&tip(Tool::Brush, json!({"size": 6}), 1.0)), "a 3 pt radius is readable");
}

#[test]
fn a_tiny_shaped_tip_shows_inside_arms_that_leave_it_clear() {
    // A flattened round tip traces its outline rather than drawing the plain circle.
    let tiny = tip(Tool::Brush, json!({"size": 4, "roundness": 0.25}), 1.0);
    assert!(clear_arms(&tiny), "a shaped tip under 3 pt: {:?}", rays(&tiny));
    let large = tip(Tool::Brush, json!({"size": 40, "roundness": 0.25}), 1.0);
    assert!(!clear_arms(&large), "a 20 pt shaped tip keeps its outline alone");
}

#[test]
fn the_pencil_arms_clear_its_tiny_square_and_its_centre_mark_stays_for_larger_ones() {
    for size in [1, 2] {
        let h = tip(Tool::Pencil, json!({"size": size}), 1.0);
        let rays = rays(&h);
        assert!(rays.iter().all(|r| arm_after_a_gap(r)), "{size} px Pencil: {rays:?}");
        // The square is at most 2 pt across and may sit a pixel off the pointer: nothing of the
        // crosshair within 2 pt of the hotspot covers it.
        assert!(rays.iter().all(|r| r[..=2].iter().all(|&a| a == 0)), "{size} px Pencil: the square stays uncovered: {rays:?}");
    }
    let mut h = tip(Tool::Pencil, json!({"size": 8}), 1.0);
    assert!(h.output().platform_output.cursor_image.is_none(), "an 8 pt square needs no crosshair");
    set_pref(&mut h, "cursors.showCrosshairInBrushTip", json!(true));
    let rays = rays(&h);
    assert!(rays.iter().all(|r| r[0] > 230), "Show Crosshair in Brush Tip: the solid centre + over a readable square");
}
