//! Resizable modal dialogs keep an honest minimum size in both reading directions: the minimum
//! handed to `egui::Resize` is the dialog's configured minimum (capped by its maximum), never
//! the window's current size, so a shrink drag survives the fork's clamp in LTR and in RTL.
//!
//! The twins below replay the callsite math at HEAD (`dialogs.rs:257`, the minimum clamped
//! through `direction::clamp_window_min_size(...).size().min(max)`) and the fork's drag handling
//! (`resize.rs`: `at_least(min_size).at_most(max_size)`, right-edge anchor under RTL) with the
//! w26 report's numbers, changing only the direction.

use egui::{NumExt, Rect, Vec2, pos2, vec2};
use photocraft_ui_egui::direction::{self, Dir};

/// The dialog's available rect and configured size limits, as in the w26 report section 6.
fn avail() -> Vec2 {
    vec2(700.0, 500.0)
}
fn min() -> Vec2 {
    vec2(560.0, 400.0)
}
fn max() -> Vec2 {
    vec2(1200.0, 900.0)
}

fn available_rect() -> Rect {
    Rect::from_min_size(pos2(0.0, 0.0), avail())
}

/// The callsite math at HEAD (`dialogs.rs:257`). Call inside a `direction_scope`.
fn callsite_min_size() -> Vec2 {
    direction::clamp_window_min_size(available_rect(), min()).size().min(max())
}

/// The fork's per-frame clamp of the dragged size (`resize.rs:303-306`).
fn fork_clamp(min_size: Vec2, user_requested: Vec2) -> Vec2 {
    user_requested.at_least(min_size).at_most(max())
}

#[test]
fn min_size_handed_to_resize_is_the_configured_min_not_the_current_size_in_both_dirs() {
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _scope = direction::direction_scope(dir);
        assert_eq!(direction::is_rtl(), dir == Dir::Rtl, "{dir:?}: the scope drives is_rtl");
        let rect = available_rect();
        let clamped = direction::clamp_window_min_size(rect, min());
        assert_eq!(clamped.size(), min(), "{dir:?}: size is exactly the configured min, never the current size");
        assert_eq!(callsite_min_size(), min().min(max()), "{dir:?}: the callsite caps the min at the max");
        // The anchored edge stays put: left in LTR, right in RTL; the top is fixed in both.
        if dir == Dir::Rtl {
            assert_eq!(clamped.right(), rect.right(), "{dir:?}: the anchored right edge stays put");
        } else {
            assert_eq!(clamped.min, rect.min, "{dir:?}: the anchored left edge stays put");
        }
        assert_eq!(clamped.top(), rect.top(), "{dir:?}: the top edge stays put");
    }
}

#[test]
fn shrink_and_grow_drags_survive_the_forks_clamp_identically_in_both_dirs() {
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _scope = direction::direction_scope(dir);
        let min_size = callsite_min_size();
        // (drag delta, expected size after the fork's at_least(min_size).at_most(max)).
        let cases = [
            (vec2(-100.0, 0.0), vec2(600.0, 500.0)),   // a shrink drag shrinks the dialog
            (vec2(200.0, 100.0), vec2(900.0, 600.0)),   // a grow drag grows it
            (vec2(-600.0, -400.0), vec2(560.0, 400.0)), // over-shrink clamps to exactly the min
            (vec2(600.0, 500.0), vec2(1200.0, 900.0)),  // over-grow clamps to exactly the max
        ];
        for (delta, expected) in cases {
            let user_requested = avail() + delta;
            assert_eq!(
                fork_clamp(min_size, user_requested),
                expected,
                "{dir:?}: drag {delta:?} yields {expected:?}"
            );
        }
    }
}

#[test]
fn the_fork_anchors_the_resized_rect_on_the_reading_start_edge_in_both_dirs() {
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _scope = direction::direction_scope(dir);
        let available = available_rect();
        // resize.rs:228-234: grip_on_left under RTL; anchor = available.right_top() (RTL) or
        // available.min (LTR).
        let grip_on_left = direction::is_rtl();
        let anchor = if grip_on_left { available.right_top() } else { available.min };
        // A drag in the reading direction (+x grows the window). resize.rs:275-279:
        // size = pointer - anchor, with x negated under RTL.
        let drag = vec2(50.0, 30.0);
        let pointer = if grip_on_left {
            pos2(anchor.x - drag.x, anchor.y + drag.y)
        } else {
            pos2(anchor.x + drag.x, anchor.y + drag.y)
        };
        let mut size = pointer - anchor;
        if grip_on_left {
            size.x = -size.x;
        }
        assert_eq!(size, drag, "{dir:?}: the same reading-direction drag yields the same size");
        // anchored_rect (resize.rs:217-223) keeps the reading-start edge fixed while resizing.
        let size = fork_clamp(callsite_min_size(), avail() + vec2(-100.0, 0.0));
        let rect = if grip_on_left {
            Rect::from_min_size(pos2(anchor.x - size.x, anchor.y), size)
        } else {
            Rect::from_min_size(anchor, size)
        };
        if dir == Dir::Rtl {
            assert_eq!(rect.right(), available.right(), "{dir:?}: RTL keeps the right edge fixed");
        } else {
            assert_eq!(rect.left(), available.left(), "{dir:?}: LTR keeps the left edge fixed");
        }
        assert_eq!(rect.top(), available.top(), "{dir:?}: the top edge stays fixed");
    }
}

/// w38: the minimum floor our clamp hands to the fork's `Resize` is identical in both reading
/// directions, so a resizable window clamps to the same min in LTR and RTL (the fork's own
/// content re-pin is tracked separately; this asserts the floor enforced by our clamp).
#[test]
fn w38_the_enforced_min_floor_is_direction_independent() {
    let floor = |dir: Dir| {
        let _scope = direction::direction_scope(dir);
        direction::clamp_window_min_size(available_rect(), min()).size().min(max())
    };
    let ltr = floor(Dir::Ltr);
    let rtl = floor(Dir::Rtl);
    assert_eq!(ltr, min().min(max()), "LTR: the floor is the configured min capped at the max");
    assert_eq!(rtl, ltr, "RTL: the same floor as LTR");
    assert!((ltr - rtl).abs().max_elem() <= 1.0, "w38: clamp floor LTR {ltr:?} vs RTL {rtl:?}");
}
