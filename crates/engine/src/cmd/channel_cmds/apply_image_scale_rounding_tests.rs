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
    // Only Add and Subtract divide by Scale, so a separable mode is untouched. The fixture must
    // DISCRIMINATE (review #14634 finding 4): the old one (source 128 onto 200/100/50) rounds
    // and truncates to the same levels. Multiply a source of 200 onto a target of 200/100/50:
    // red is 200 * 200 / 255 = 156.86, which today rounds to 157 while a truncation would store
    // 156. Green (100 * 200 / 255 = 78.43) and blue (50 * 200 / 255 = 39.22) pin the values too.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 6, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#c86432"})).unwrap();
    s.execute("file.new", json!({"width": 6, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#c8c8c8"})).unwrap();
    s.set_active(0);
    apply(&mut s, json!({"source": {"document": 1}, "blending": "multiply"}));
    assert_eq!(levels(&s, LEFT, 1), [157, 78, 39], "multiply at the left half: rounded, never truncated");
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

#[test]
fn scale_two_with_a_non_zero_offset_keeps_todays_rounding() {
    // Review #14634 (calc.rs:72): the truncation gate ignored Offset, but #14490 lists "Scale
    // with an Offset at the same time" as not measured, so those blends keep Blending::apply's
    // rounding. The review's exact inputs: Add, target 50, source 255, Scale 2, Offset 1 gives
    // (50 + 255) / 2 + 1 = 153.5 and must stay 154, not 153; Subtract, target 50, source 255,
    // Scale 2, Offset 128 gives (50 - 255) / 2 + 128 = 25.5 and must stay 26, not 25.
    let mut s = pair(8, "#c86432", "#3296fa");
    apply(&mut s, json!({"source": {"document": 1}, "blending": "add", "scale": 2, "offset": 1}));
    assert_eq!(levels(&s, RIGHT, 1), [154, 204, 254], "Add, Scale 2, Offset 1: right half");
    assert_eq!(levels(&s, LEFT, 1), [165, 115, 90], "Add, Scale 2, Offset 1: whole quotients on the left half");
    let mut s = pair(8, "#c86432", "#3296fa");
    apply(&mut s, json!({"source": {"document": 1}, "blending": "subtract", "scale": 2, "offset": 128}));
    assert_eq!(levels(&s, RIGHT, 1), [26, 76, 126], "Subtract, Scale 2, Offset 128: right half");
    assert_eq!(levels(&s, LEFT, 1), [164, 114, 89], "Subtract, Scale 2, Offset 128: whole quotients on the left half");
}

#[test]
fn a_full_weight_selection_keeps_todays_rounding() {
    // Review #14634 (calc.rs:90): inside a selection that covers the pixel the blend's effective
    // weight is still 1, but #14490 lists "a mask or a selection" as not measured, so a run with
    // an active selection keeps Blending::apply's rounding: target 50, source 255, Add at Scale 2
    // stays (50 + 255) / 2 = 152.5 -> 153 inside the selection, and the unselected left half is
    // untouched.
    let mut s = pair(8, "#c86432", "#3296fa");
    s.execute("select.rect", json!({"x": 3, "y": 0, "width": 3, "height": 4, "mode": "replace"})).unwrap();
    apply(&mut s, json!({"source": {"document": 1}, "blending": "add", "scale": 2, "offset": 0}));
    assert_eq!(levels(&s, RIGHT, 1), [153, 203, 253], "right half inside the full-weight selection");
    assert_eq!(levels(&s, LEFT, 1), [200, 100, 50], "the unselected left half stays as it was");
}

#[test]
fn an_all_one_mask_keeps_todays_rounding() {
    // Review #14634 (calc.rs:90), same finding through the mask parameter: a mask with value 1
    // at the pixel leaves the weight at 1, yet #14490 marks a masked run as not measured. Apply
    // Image with the source document's all-opaque transparency as a mask (value 1 everywhere)
    // must still round: (50 + 255) / 2 = 152.5 stays 153, and the whole left half stays 164/114/89.
    let mut s = pair(8, "#c86432", "#3296fa");
    apply(&mut s, json!({"source": {"document": 1}, "blending": "add", "scale": 2, "offset": 0, "mask": {"document": 1, "channel": "transparency"}}));
    assert_eq!(levels(&s, RIGHT, 1), [153, 203, 253], "right half through the all-one mask");
    assert_eq!(levels(&s, LEFT, 1), [164, 114, 89], "left half through the all-one mask");
}

#[test]
fn a_thirty_two_bit_source_keeps_todays_rounding() {
    // Review #14634 (calc.rs:99): the gate only looked at the target format, but a 16 or 32-bit
    // source keeps fractional samples, and the truncation assumes whole 8-bit levels. The
    // review's exact input: an opaque 8-bit RGB target at red 0, Add, Scale 1, Offset 0, and a
    // 32-bit source whose red is 100.75/255. Today's path writes 101/255; truncating the
    // fractional quotient drops it to 100/255. #14490 measured 8-bit sources only, so this
    // keeps Blending::apply.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 6, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    s.execute("file.new", json!({"width": 6, "height": 4, "depth": 32})).unwrap();
    s.execute("edit.fill", json!({"color": [100.75 / 255.0, 0.4, 0.4]})).unwrap();
    s.set_active(0);
    apply(&mut s, json!({"source": {"document": 1}, "blending": "add", "scale": 1, "offset": 0}));
    assert_eq!(lv8(px(&s, RIGHT, 1)[0]), 101, "32-bit source at red 100.75/255 keeps today's rounding");
}

#[test]
fn a_half_transparent_source_layer_keeps_todays_rounding() {
    // Independent review of #14634 (finding 1): the source DOCUMENT's pixel format says 8-bit,
    // but a merged composite blends its layers in f32 before anyone quantises again: a black
    // layer at 50 % over white reaches the loop as top = 127.5/255 inside a perfectly 8-bit RGB
    // document. #14490 measured the reference dividing whole levels, so this fractional sample
    // keeps Blending::apply: 0 + 127.5 -> 128 today, not 127.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 6, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    s.execute("file.new", json!({"width": 6, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "half black", "opacity": 0.5})).unwrap();
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    s.set_active(0);
    apply(&mut s, json!({"source": {"document": 1}, "blending": "add", "scale": 1, "offset": 0}));
    assert_eq!(levels(&s, RIGHT, 1), [128, 128, 128], "composite sample 127.5/255 keeps today's rounding");
}

#[test]
fn a_vector_mask_coverage_float_keeps_todays_rounding() {
    // Independent review of #14634 (finding 1), the other raw-float source: a vector mask reads
    // as coverage values of the shape, not as whole 8-bit levels (`ChanRef::VectorMask`). A path
    // edge at x=1.5 puts 0.5 coverage on pixel column 1; Apply Image of that channel onto a
    // black 8-bit RGB target must keep today's rounding: 0.5 * 255 = 127.5 -> 128, not 127.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 6, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    s.execute("file.new", json!({"width": 6, "height": 4})).unwrap();
    let masked = s.execute("layer.new.layer", json!({"name": "masked"})).unwrap()["layer"].as_u64().unwrap();
    let path = json!({"subpaths": [{"closed": true, "knots": [[0.0, 0.0], [1.5, 0.0], [1.5, 4.0], [0.0, 4.0]]}]});
    s.execute("layer.vectorMask.add", json!({"layer": masked, "path": path})).unwrap();
    s.set_active(0);
    apply(&mut s, json!({"source": {"document": 1, "channel": "vectorMask"}, "blending": "add", "scale": 1, "offset": 0}));
    assert_eq!(levels(&s, 1, 1), [128, 128, 128], "half coverage 127.5/255 keeps today's rounding");
    assert_eq!(levels(&s, 0, 1), [255, 255, 255], "full coverage is a whole level and lands on 255 either way");
    assert_eq!(levels(&s, 2, 1), [0, 0, 0], "outside the path stays black");
}

#[test]
fn an_indexed_color_target_keeps_todays_rounding() {
    // Independent review of #14634 (finding 2): an Indexed Color document keeps its pixels
    // RGB-expanded, so its PIXEL FORMAT is Rgb and a gate on the format alone takes the new path
    // for a mode #14490 never measured. The gate is on the document's own mode, so Indexed stays
    // on the old path: (50 + 255) / 2 = 152.5 stays 153.
    let mut s = pair(8, "#c86432", "#3296fa");
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none"})).unwrap();
    apply(&mut s, json!({"source": {"document": 1}, "blending": "add", "scale": 2, "offset": 0}));
    assert_eq!(levels(&s, RIGHT, 1), [153, 203, 253], "Indexed target keeps rounding");
    assert_eq!(levels(&s, LEFT, 1), [164, 114, 89], "the whole left half stays as measured for RGB");
}

#[test]
fn add_scale_one_point_six_also_truncates() {
    // Independent review of #14634 (finding 3): the truncation is a rule continuous in Scale, not
    // keyed to the two measured values. At Scale 1.6 the same rule truncates (100 + 255) / 1.6 =
    // 221.875 to 221 where main rounds to 222 -- the Calculations twin of this exact number is
    // pinned at 222 by `calculations_keeps_todays_rounding`. This test PASSES on 7e69ac000: it
    // pins the deliberate generalisation "every Scale in 1..=2 truncates at 8 bit" rather than a
    // measured value (the issue measured Scale 1 and 2 only).
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 6, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#646464"})).unwrap();
    s.execute("file.new", json!({"width": 6, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
    s.set_active(0);
    apply(&mut s, json!({"source": {"document": 1}, "blending": "add", "scale": 1.6, "offset": 0}));
    assert_eq!(levels(&s, RIGHT, 1), [221, 221, 221], "Apply Image at 8 bit truncates at Scale 1.6 too");
}

#[test]
fn an_eight_bit_grayscale_target_keeps_todays_rounding() {
    // Only RGB is measured (#14490), so another colour mode keeps rounding to the nearest level:
    // (50 + 255) / 2 = 152.5 lands on 153 in a Grayscale target.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 6, "height": 4, "mode": "grayscale"})).unwrap();
    s.execute("edit.fill", json!({"color": "#323232"})).unwrap();
    s.execute("file.new", json!({"width": 6, "height": 4, "mode": "grayscale"})).unwrap();
    s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
    s.set_active(0);
    apply(&mut s, json!({"source": {"document": 1}, "blending": "add", "scale": 2, "offset": 0}));
    assert_eq!(lv8(px(&s, RIGHT, 1)[0]), 153, "Grayscale keeps rounding");
}
