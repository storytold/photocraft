//! #12035: Image Rotation › Arbitrary pre-fills the ruler's straightening angle at the
//! precision the reference application shows. A ruler from 100,100 to 400,300 (A -33.7° on
//! the Info panel) opens the Rotate Canvas dialog at 33.69°, Counter clockwise, and OK runs
//! `image.rotation.arbitrary` with that two-decimal angle, not the field's old one-decimal
//! round. The Angle range is -359.99..359.99, wide, so today's wide-range rounding to one
//! decimal decided the dialog's value; only this field now keeps hundredths.

use super::filter_dialog::{body, open, params_of};
use crate::PhotocraftApp;
use serde_json::{Map, Value, json};

/// Draw a dialog body once, as a live dialog does every frame: the drawn fields are what
/// the dialog shows and what OK sends.
fn drawn(f: &mut Map<String, Value>) {
    egui::Context::default().run_ui(Default::default(), |ui| body(ui, f)).textures_delta.clear();
}

/// A 318 px square document (the size `ok_runs_the_two_decimal_angle` measures on), a ruler
/// from 100,100 to `end`, the opened Arbitrary dialog and its fields after one drawn frame.
fn ruler_dialog(end: [i32; 2]) -> (PhotocraftApp, u64, Map<String, Value>) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 318, "height": 318})).unwrap();
    app.run("image.analysis.rulerTool", json!({"start": [100, 100], "end": end})).unwrap();
    let id = open(&mut app, "image.rotation.arbitrary").unwrap();
    let mut f = app.ui.dialog_mut(id).unwrap().fields.clone();
    drawn(&mut f);
    (app, id, f)
}

/// The measured case: the Rotate Canvas dialog opens at 33.69°, Counter clockwise — the
/// angle keeps two decimals where the wide-range rule used to round it to 33.7.
#[test]
fn ruler_prefill_keeps_the_measured_two_decimal_angle() {
    let (_app, _id, f) = ruler_dialog([400, 300]);
    let a = f["angle"].as_f64().unwrap();
    assert!((a - 33.69).abs() < 1e-3, "the dialog opens at {a}, want the measured 33.69");
    assert_eq!(crate::widgets::fmt_num2(a), "33.69", "the Angle field shows the measured string");
    assert_eq!(f["direction"], json!("ccw"), "a ruler falling to the right straightens counter-clockwise");
}

/// A ruler that needs a clockwise turn pre-fills Clockwise, at the same two decimals;
/// with no ruler the dialog opens at today's default (0, Clockwise).
#[test]
fn clockwise_ruler_prefills_clockwise_and_no_ruler_keeps_the_default() {
    // (100,100) → (400,50) rises to the right: a 9.46° clockwise turn.
    let (_app, _id, f) = ruler_dialog([400, 50]);
    let a = f["angle"].as_f64().unwrap();
    assert_eq!(f["direction"], json!("cw"), "a rising ruler pre-fills Clockwise");
    assert!((a - 9.46).abs() < 1e-3, "the clockwise pre-fill is {a}, want 9.46 to two decimals");
    // No ruler: today's default, unchanged.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 318, "height": 318})).unwrap();
    let id = open(&mut app, "image.rotation.arbitrary").unwrap();
    let mut f = app.ui.dialog_mut(id).unwrap().fields.clone();
    drawn(&mut f);
    assert_eq!(f["angle"], json!(0.0), "without a ruler the dialog opens at today's 0");
    assert_eq!(f["direction"], json!("cw"), "without a ruler the dialog opens Clockwise");
}

/// OK runs `image.rotation.arbitrary` with the two-decimal angle: a 318 px square rotated by
/// 33.69° measures 441 px across, by 33.7° 442 px — the dialog's rounding must not decide.
#[test]
fn ok_runs_the_two_decimal_angle() {
    let (mut app, id, f) = ruler_dialog([400, 300]);
    let params = params_of(&f);
    assert!((params["angle"].as_f64().unwrap() - 33.69).abs() < 1e-3, "OK sends {params}");
    app.ui.dialog_mut(id).unwrap().fields = f;
    let r = crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(441), Some(441)), "442 would mean the 33.7 round ran");
}

/// The f32 double-rounding the review caught: a 43x27 ruler line (here 100,100 to 143,127)
/// is 32.1249984 degrees, and the f32 cast of that full-precision pre-fill lands exactly on
/// 32.125 - rounding the widened value half away from zero opened the dialog at 32.13. The
/// hundredths round in f64 before the field narrows, so it opens at 32.12, Counter clockwise.
#[test]
fn the_ruler_angle_rounds_at_full_precision_before_the_field_narrows() {
    let (_app, _id, f) = ruler_dialog([143, 127]);
    let a = f["angle"].as_f64().unwrap();
    assert!((a - 32.12).abs() < 1e-3, "the 43x27 ruler opens at {a}, want 32.12");
    assert_eq!(crate::widgets::fmt_num2(a), "32.12", "the Angle field shows 32.12, not the 32.13 the f32 round gives");
    assert!((a - 32.13).abs() > 1e-3, "the f32 double-round must not reach the dialog: {a}");
    assert_eq!(f["direction"], json!("ccw"));
}

/// The fix is per-field: unrelated number fields keep the precision they have today —
/// wide ranges round to one decimal (in another dialog, and under the same key name),
/// small ranges keep their two.
#[test]
fn other_number_fields_keep_their_precision() {
    // A wide range in another dialog: Gaussian Blur's radius (0.1..1000) rounds to one decimal.
    let mut f = Map::new();
    f.insert("__command".into(), json!("filter.blur.gaussianBlur"));
    f.insert("radius".into(), json!(7.26));
    drawn(&mut f);
    let r = f["radius"].as_f64().unwrap();
    assert!((r - 7.3).abs() < 1e-5, "the Gaussian Blur radius {r} must stay at one decimal");
    // The same key the fix touches, in another dialog: Twirl's angle (-999..999) rounds to one.
    let mut f = Map::new();
    f.insert("__command".into(), json!("filter.distort.twirl"));
    f.insert("angle".into(), json!(33.69));
    drawn(&mut f);
    let a = f["angle"].as_f64().unwrap();
    assert!((a - 33.7).abs() < 1e-5, "Twirl's angle {a} keeps the one-decimal round");
    // A small range keeps its two decimals (Radial Blur spins, so the centre fields stay plain).
    let mut f = Map::new();
    f.insert("__command".into(), json!("filter.blur.radialBlur"));
    f.insert("method".into(), json!("spin"));
    f.insert("centerX".into(), json!(0.123));
    drawn(&mut f);
    let x = f["centerX"].as_f64().unwrap();
    assert!((x - 0.12).abs() < 1e-5, "the Radial Blur centre {x} keeps its two decimals");
}
