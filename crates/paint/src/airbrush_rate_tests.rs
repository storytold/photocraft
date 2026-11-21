//! #11886: with Airbrush on, a brush held still builds up at about ten dabs a second and a plain
//! click lays two dabs, as in Photoshop. Measured there with the default 13 px soft brush at Flow
//! 10 %, black on white: the dab centre reads 205 after a click and 26 after a 2 s hold, and 229
//! with Airbrush off however long the button is held.

use photocraft_color::PixelFormat;
use photocraft_raster::Surface;

use crate::dynamics::DabGenerator;
use crate::{BrushPreset, BrushSettings, Control, Dynamic, StrokePoint, Transfer, render_stroke};

fn brush(build_up: bool) -> BrushSettings {
    BrushSettings { size: 13.0, hardness: 0.0, flow: 0.1, pressure_size: false, build_up, color: [0.0, 0.0, 0.0, 1.0], ..BrushSettings::default() }
}

/// The 0–255 value at the dab centre over white after holding the button still for `ms`.
fn centre_after_hold(b: &BrushSettings, ms: f64) -> f32 {
    let a = StrokePoint { time: 0.0, ..StrokePoint::new(30.0, 30.0, 1.0) };
    let z = StrokePoint { time: ms, ..a };
    let mut s = Surface::new(PixelFormat::RGBA32F);
    render_stroke(&mut s, b, &[a, z], None, false, 1.0);
    255.0 * (1.0 - s.rgba(30, 30)[3])
}

#[test]
fn the_default_rate_is_ten_dabs_a_second() {
    assert_eq!(BrushSettings::default().build_up_rate, 10.0);
}

#[test]
fn a_click_lays_two_dabs() {
    let v = centre_after_hold(&brush(true), 0.0);
    assert!((v - 205.0).abs() <= 5.0, "{v}");
}

#[test]
fn a_two_second_hold_builds_up_to_the_reference_value() {
    let v = centre_after_hold(&brush(true), 2000.0);
    assert!((v - 26.0).abs() <= 8.0, "{v}");
}

#[test]
fn airbrush_off_keeps_one_dab_however_long_it_is_held() {
    for ms in [0.0, 500.0, 2000.0] {
        let v = centre_after_hold(&brush(false), ms);
        assert!((v - 229.0).abs() <= 3.0, "{ms} ms: {v}");
    }
}

#[test]
fn saved_airbrush_presets_default_to_the_legacy_rate() {
    for (source, build_up, rate) in [
        (r#"{"name":"Legacy Airbrush","brush":{"size":42,"buildUp":true}}"#, true, 20.0),
        (r#"{"name":"Custom Airbrush","brush":{"size":42,"buildUp":true,"buildUpRate":35}}"#, true, 35.0),
        (r#"{"name":"Regular Brush","brush":{"size":42}}"#, false, 10.0),
    ] {
        let preset: BrushPreset = serde_json::from_str(source).unwrap();
        assert_eq!(preset.brush.build_up, build_up);
        assert_eq!(preset.brush.size, 42.0);
        assert_eq!(preset.brush.build_up_rate, rate);
        assert_eq!(serde_json::to_value(&preset).unwrap()["brush"]["buildUpRate"], rate);
    }
    assert!(serde_json::from_str::<BrushPreset>(r#"{"name":"Invalid","brush":{"buildUp":true,"buildUpRate":null}}"#).is_err());
    let live: BrushSettings = serde_json::from_str(r#"{"buildUp":true}"#).unwrap();
    assert_eq!(live.build_up_rate, 10.0);
}

fn fade_dabs(build_up: bool, end: StrokePoint) -> Vec<(f64, f32)> {
    let brush = BrushSettings {
        size: 10.0,
        spacing: 1.0,
        flow: 1.0,
        build_up,
        transfer: Transfer { enabled: true, flow: Dynamic { control: Control::Fade, fade_steps: 2, ..Dynamic::default() }, ..Transfer::default() },
        ..BrushSettings::default()
    };
    let mut generator = DabGenerator::new(&brush, 1.0);
    let (mut dabs, mut dual) = (Vec::new(), Vec::new());
    generator.push(&[StrokePoint::new(10.0, 10.0, 1.0), end], &mut dabs, &mut dual);
    generator.finish(&mut dabs, &mut dual);
    dabs.into_iter().map(|dab| (dab.center.x, dab.alpha)).collect()
}

#[test]
fn the_extra_first_airbrush_dab_does_not_advance_movement_fade() {
    let end = StrokePoint::new(30.0, 10.0, 1.0);
    assert_eq!(fade_dabs(false, end), [(10.0, 1.0), (20.0, 0.5), (30.0, 0.0)]);
    assert_eq!(fade_dabs(true, end), [(10.0, 1.0), (10.0, 1.0), (20.0, 0.5), (30.0, 0.0)]);
}

#[test]
fn timed_airbrush_dabs_advance_fade_after_the_two_initial_dabs() {
    let end = StrokePoint { time: 200.0, ..StrokePoint::new(10.0, 10.0, 1.0) };
    assert_eq!(fade_dabs(true, end), [(10.0, 1.0), (10.0, 1.0), (10.0, 0.5), (10.0, 0.0)]);
}
