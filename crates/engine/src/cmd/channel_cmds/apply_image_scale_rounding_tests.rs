//! Apply Image / Calculations: Add and Subtract divide by Scale, and the reference application
//! truncates that quotient at 8 bit instead of rounding it to the nearest level (#14490).
//!
//! The measured table comes from the issue. Target document (8-bit RGB): left half 200/100/50,
//! right half 50/150/250. Source document: left half 128/128/128, right half 255/255/255. Only
//! case F's right half has a fractional quotient, and there the fraction is dropped:
//! (50 + 255) / 2 = 152.5 becomes 152.

use serde_json::{Value, json};

use crate::Session;

const LEFT: i32 = 1;
const RIGHT: i32 = 4;

fn fill_halves(s: &mut Session, left: &str, right: &str) {
    for (x, color) in [(0, left), (3, right)] {
        s.execute("select.rect", json!({"x": x, "y": 0, "width": 3, "height": 4, "mode": "replace"})).unwrap();
        s.execute("edit.fill", json!({"color": color})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
}

/// The issue's pair of documents at `depth` bit: target (document 0, filled with `target_left` /
/// `target_right`) and source (document 1, 128 on the left half, 255 on the right half).
fn pair(depth: u32, target_left: &str, target_right: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 6, "height": 4, "depth": depth})).unwrap();
    fill_halves(&mut s, target_left, target_right);
    s.execute("file.new", json!({"width": 6, "height": 4, "depth": depth})).unwrap();
    fill_halves(&mut s, "#808080", "#ffffff");
    s.set_active(0);
    s
}

fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
}

fn chan(s: &Session, i: usize, x: i32, y: i32) -> f32 {
    s.active().unwrap().doc.channels[i].surface.sample_channel(x, y, 0)
}

/// A normalised sample as whole levels of an 8-bit ramp, as the issue measures them.
fn lv8(v: f32) -> u32 {
    (v * 255.0 + 0.5) as u32
}

/// The target layer's three colour channels, in 8-bit levels.
fn levels(s: &Session, x: i32, y: i32) -> [u32; 3] {
    let p = px(s, x, y);
    [lv8(p[0]), lv8(p[1]), lv8(p[2])]
}

/// Apply Image of document 1 (the issue's source) onto the active document's layer, with the
/// case's own blending, Scale and Offset.
fn apply(s: &mut Session, params: Value) {
    s.execute("image.applyImage", params).unwrap();
}

#[test]
fn add_scale_two_truncates_the_fractional_quotient() {
    // Case F, the measured half: (50 + 255) / 2 = 152.5, (150 + 255) / 2 = 202.5,
    // (250 + 255) / 2 = 252.5. The reference application drops the fraction; main rounds it up.
    let mut s = pair(8, "#c86432", "#3296fa");
    apply(&mut s, json!({"source": {"document": 1}, "blending": "add", "scale": 2, "offset": 0}));
    assert_eq!(levels(&s, RIGHT, 1), [152, 202, 252], "case F right half");
}

#[test]
fn add_scale_two_keeps_a_whole_quotient() {
    // Case F's other half: (200 + 128) / 2 = 164, (100 + 128) / 2 = 114, (50 + 128) / 2 = 89.
    // These already match on main and must stay: the truncation may not drop a quotient that is a
    // whole level down to the one below it through floating-point noise.
    let mut s = pair(8, "#c86432", "#3296fa");
    apply(&mut s, json!({"source": {"document": 1}, "blending": "add", "scale": 2, "offset": 0}));
    assert_eq!(levels(&s, LEFT, 1), [164, 114, 89], "case F left half");
}

#[test]
fn add_scale_two_truncates_other_fractional_quotients_too() {
    // The same rule one level up: (52 + 255) / 2 = 153.5 becomes 153, and so on.
    let mut s = pair(8, "#c86432", "#3498fc");
    apply(&mut s, json!({"source": {"document": 1}, "blending": "add", "scale": 2, "offset": 0}));
    assert_eq!(levels(&s, RIGHT, 1), [153, 203, 253], "right half of 52/152/252");
    assert_eq!(levels(&s, LEFT, 1), [164, 114, 89], "the untouched left half");
}

#[test]
fn add_scale_one_with_offset_keeps_its_values() {
    // Case F2: t + s + offset, clamped. No quotient to truncate.
    let mut s = pair(8, "#c86432", "#3296fa");
    apply(&mut s, json!({"source": {"document": 1}, "blending": "add", "scale": 1, "offset": -100}));
    assert_eq!(levels(&s, LEFT, 1), [228, 128, 78], "case F2 left half");
    assert_eq!(levels(&s, RIGHT, 1), [205, 255, 255], "case F2 right half");
}

#[test]
fn subtract_scale_one_with_offset_keeps_its_values() {
    // Case G: t - s + offset, clamped.
    let mut s = pair(8, "#c86432", "#3296fa");
    apply(&mut s, json!({"source": {"document": 1}, "blending": "subtract", "scale": 1, "offset": 128}));
    assert_eq!(levels(&s, LEFT, 1), [200, 100, 50], "case G left half");
    assert_eq!(levels(&s, RIGHT, 1), [0, 23, 123], "case G right half");
}

#[test]
fn subtract_scale_two_keeps_its_values() {
    // Case G2: (t - s) / 2. Its quotients are fractional and negative; a negative quotient is not
    // measured (#14490) and stays clamped to black here either way.
    let mut s = pair(8, "#c86432", "#3296fa");
    apply(&mut s, json!({"source": {"document": 1}, "blending": "subtract", "scale": 2, "offset": 0}));
    assert_eq!(levels(&s, LEFT, 1), [36, 0, 0], "case G2 left half");
    assert_eq!(levels(&s, RIGHT, 1), [0, 0, 0], "case G2 right half");
}

#[test]
fn a_blend_mode_is_not_truncated() {
    // Only Add and Subtract divide by Scale, so a separable mode is untouched: Multiply of the
    // source's left half (128) onto the target's left half stays 200 * 128/255 and so on.
    let mut s = pair(8, "#c86432", "#3296fa");
    apply(&mut s, json!({"source": {"document": 1}, "blending": "multiply"}));
    let want = [lv8(200.0 / 255.0 * (128.0 / 255.0)), lv8(100.0 / 255.0 * (128.0 / 255.0)), lv8(50.0 / 255.0 * (128.0 / 255.0))];
    assert_eq!(levels(&s, LEFT, 1), want, "multiply at the left half");
}

#[test]
fn a_sixteen_bit_target_keeps_todays_rounding() {
    // 16 bit is not measured (#14490), so the truncation must stay out of its write. Scale 1.6
    // keeps the comparison away from a half level: (50 * 257 + 65535) / 1.6 = 48990.625 and
    // (150 * 257 + 65535) / 1.6 = 65053.125, which today round to 48991 and 65053.
    let mut s = pair(16, "#c86432", "#3296fa");
    apply(&mut s, json!({"source": {"document": 1}, "blending": "add", "scale": 1.6, "offset": 0}));
    let p = px(&s, RIGHT, 1);
    let got = [(p[0] * 65535.0 + 0.5) as u32, (p[1] * 65535.0 + 0.5) as u32];
    assert_eq!(got, [48991, 65053], "16-bit right half keeps rounding to the nearest level");
}

#[test]
fn a_thirty_two_bit_target_keeps_its_fraction() {
    // 32 bit is not measured either: (50 + 255) / 2 = 152.5 stays 152.5, and 152 would be the bug.
    let mut s = pair(32, "#c86432", "#3296fa");
    apply(&mut s, json!({"source": {"document": 1}, "blending": "add", "scale": 2, "offset": 0}));
    let p = px(&s, RIGHT, 1);
    assert!(((p[0] * 255.0) - 152.5).abs() < 0.05 && ((p[1] * 255.0) - 202.5).abs() < 0.05, "32-bit right half: {p:?}");
}

#[test]
fn calculations_keeps_todays_rounding() {
    // Image > Calculations shares `Blending::apply` and is not measured (#14490): Add, Scale 1.6
    // of a white source onto 100% gray gives (100 + 255) / 1.6 = 221.875, which today lands on 222
    // in the new channel. Apply Image at 8 bit gives 221 for the same numbers.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 6, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#646464"})).unwrap();
    s.execute("file.new", json!({"width": 6, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
    s.set_active(0);
    s.execute("image.calculations", json!({"source1": {"document": 1}, "source2": {"channel": "composite"}, "blending": "add", "scale": 1.6, "offset": 0}))
        .unwrap();
    assert_eq!(lv8(chan(&s, 0, 1, 1)), 222, "Calculations keeps rounding to the nearest level");
}

#[test]
fn half_opacity_keeps_todays_rounding() {
    // Opacity below 100 % is not measured (#14490): the blend is mixed with the target before the
    // write, so it keeps today's rounding. (50 + 152.5) / 2 = 101.25 stays 101.
    let mut s = pair(8, "#c86432", "#3296fa");
    apply(&mut s, json!({"source": {"document": 1}, "blending": "add", "scale": 2, "offset": 0, "opacity": 50}));
    assert_eq!(levels(&s, RIGHT, 1), [101, 176, 251], "right half at 50 % opacity");
}
