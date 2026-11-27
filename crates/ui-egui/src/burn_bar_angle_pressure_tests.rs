//! #12667: run the shared toning controls through Burn's bar and paint command.

use crate::Tool;
use crate::dodge_bar_angle_pressure_tests::{check_angle_range, check_bar_edits, check_stroke};

#[test]
fn burn_bar_edits_its_own_brush_and_records_each_gesture() {
    check_bar_edits(Tool::Burn, Tool::Dodge);
}

#[test]
fn burn_angle_range_clamps_and_opens_one_alert() {
    check_angle_range(Tool::Burn);
}

#[test]
fn burn_stroke_uses_the_bar_angle_and_pressure() {
    check_stroke(Tool::Burn, "paint.burn");
}
