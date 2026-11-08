//! A loaded Mixer Brush lasts for a length of stroke, not a number of dabs (#11943): measured with
//! Wet 0, Mix 0, Flow 100, a black load and a 35 px soft round tip at 25 % spacing, a stroke from
//! x 30 to 770 on an empty layer paints solid (alpha 252 or more) to about x 420 at Load 1 %, x 600
//! at Load 5 % and to the end at Load 50 %; then the alpha steps down to about 68 % and fades.

use crate::mixer::{MixerSettings, MixerState, apply_mixer_stroke};
use crate::{BrushSettings, Stroke, StrokePoint};
use photocraft_color::PixelFormat;
use photocraft_raster::Surface;

const Y: f64 = 100.0;

fn stroke(x0: f64, x1: f64, spacing: f32) -> Stroke {
    Stroke {
        brush: BrushSettings { size: 35.0, hardness: 0.0, spacing, pressure_size: false, ..Default::default() },
        points: vec![StrokePoint::new(x0, Y, 1.0), StrokePoint::new(x1, Y, 1.0)],
    }
}

/// Paints the stroke with a fresh black load and returns the alpha (0–255) along its centre line,
/// and the reservoir amount left.
fn paint(load: f32, x0: f64, x1: f64, spacing: f32) -> (impl Fn(i32) -> i32, f32) {
    let mut s = Surface::new(PixelFormat::RGBA8);
    let mut st = MixerState::default();
    st.load([0.0, 0.0, 0.0, 1.0]);
    let m = MixerSettings { wet: 0.0, load, mix: 0.0, flow: 1.0, ..Default::default() };
    apply_mixer_stroke(&mut s, None, &stroke(x0, x1, spacing), &m, &mut st, None, false);
    (move |x: i32| (s.rgba(x, Y as i32)[3] * 255.0).round() as i32, st.amount)
}

/// The first x at or after `from` whose alpha falls below 250.
fn runs_out(alpha: &impl Fn(i32) -> i32, from: i32) -> Option<i32> {
    (from..=770).find(|&x| alpha(x) < 250)
}

#[test]
fn the_measured_strokes_stay_solid_as_far_as_the_reference_and_then_drop() {
    for (load, solid_to) in [(0.5, 760), (0.05, 590), (0.01, 410)] {
        let (alpha, _) = paint(load, 30.0, 770.0, 0.25);
        for x in 40..=solid_to {
            assert!(alpha(x) >= 250, "Load {load}: alpha {} at x {x}", alpha(x));
        }
    }
    // After the supply runs out the alpha steps down to 60–75 % and then fades.
    let (alpha, amount) = paint(0.01, 30.0, 770.0, 0.25);
    assert!((150..=200).contains(&alpha(450)), "Load 1 %: alpha {} at x 450", alpha(450));
    assert!((25..=55).contains(&alpha(750)), "Load 1 %: alpha {} at x 750", alpha(750));
    assert!(alpha(480) > alpha(570) && alpha(570) > alpha(690), "it keeps fading");
    assert!(amount < 0.2, "{amount}");
    let (alpha, _) = paint(0.05, 30.0, 770.0, 0.25);
    assert!((153..=191).contains(&alpha(640)), "Load 5 %: alpha {} at x 640", alpha(640));
    assert!(alpha(760) < alpha(640), "Load 5 % fades towards the end");
}

#[test]
fn the_supply_is_a_length_of_stroke() {
    // A 470 px stroke at Load 5 % stays solid to its end.
    let (alpha, amount) = paint(0.05, 300.0, 770.0, 0.25);
    for x in 310..=760 {
        assert!(alpha(x) >= 250, "alpha {} at x {x}", alpha(x));
    }
    assert_eq!(amount, 1.0);
    // At spacing 10 % (two and a half times the dabs) the load runs out at the same place.
    for load in [0.01, 0.05] {
        let wide = runs_out(&paint(load, 30.0, 770.0, 0.25).0, 40).unwrap();
        let dense = runs_out(&paint(load, 30.0, 770.0, 0.10).0, 40).unwrap();
        assert!((wide - dense).abs() <= 30, "Load {load}: spacing 25 % runs out at x {wide}, 10 % at x {dense}");
    }
}

#[test]
fn load_100_never_runs_out() {
    let (alpha, amount) = paint(1.0, 30.0, 770.0, 0.25);
    assert_eq!(runs_out(&alpha, 40).filter(|&x| x < 765), None);
    assert_eq!(amount, 1.0);
}

#[test]
fn each_stroke_measures_its_own_length() {
    // With the automatic Load off, a second stroke starts from what the first left and measures
    // its supply from its own start.
    let mut s = Surface::new(PixelFormat::RGBA8);
    let mut st = MixerState::default();
    st.load([0.0, 0.0, 0.0, 1.0]);
    let m = MixerSettings { wet: 0.0, load: 0.05, mix: 0.0, flow: 1.0, ..Default::default() };
    apply_mixer_stroke(&mut s, None, &stroke(30.0, 330.0, 0.25), &m, &mut st, None, false);
    assert_eq!(st.amount, 1.0, "300 px of a 570 px supply");
    apply_mixer_stroke(&mut s, None, &stroke(30.0, 330.0, 0.25), &m, &mut st, None, false);
    assert_eq!(st.amount, 1.0, "the second stroke starts its own 570 px");
}
