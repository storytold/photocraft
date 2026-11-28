//! #14744: a colour range's weight follows the reference application's ramp edges, measured on a
//! hue ramp of HSL(5i°, 100 %, 50 %) cells at 8 bits. Hues are the cells' own (60 · 64 / 255 =
//! 15.06° for the "15°" cell), weights within 0.02 (2 levels of a Saturation -100 swing).

use super::HueRange;

/// The hue in degrees of a red-dominant 8-bit colour (one of the other channels is 0).
fn hue(rgb: [u8; 3]) -> f32 {
    let [_, g, b] = rgb.map(f32::from);
    if b > 0.0 { 360.0 - 60.0 * b / 255.0 } else { 60.0 * g / 255.0 }
}

fn range(bounds: [f32; 4]) -> HueRange {
    HueRange { saturation: -100.0, bounds, ..HueRange::neutral(0) }
}

fn assert_weight(r: &HueRange, deg: f32, want: f32) {
    let got = r.weight_8bit(deg);
    assert!((got - want).abs() <= 0.02, "{:?} at {deg}: {got}, measured {want}", r.bounds);
}

#[test]
fn default_ranges_follow_the_measured_ramp() {
    #[rustfmt::skip]
    let measured: [([u8; 3], f32); 12] = [
        ([255, 85, 0], 0.84), ([255, 106, 0], 0.67), ([255, 127, 0], 0.50),
        ([255, 148, 0], 0.34), ([255, 169, 0], 0.17), ([255, 190, 0], 0.01),
        ([255, 0, 169], 0.14), ([255, 0, 148], 0.31), ([255, 0, 127], 0.47),
        ([255, 0, 106], 0.64), ([255, 0, 85], 0.81), ([255, 0, 64], 0.98),
    ];
    for (rgb, want) in measured {
        assert_weight(&HueRange::neutral(0), hue(rgb), want);
        // The other five default ranges have the same shape, 60° apart.
        for i in 1..6 {
            let r = HueRange::neutral(i);
            assert_weight(&r, hue(rgb) + 60.0 * i as f32, want);
        }
    }
}

#[test]
fn default_ranges_are_fully_on_from_350_to_15() {
    for i in 0..6 {
        let r = HueRange::neutral(i);
        for k in 0..=50 {
            let deg = 350.0 + 0.5 * k as f32 + 60.0 * i as f32;
            assert_eq!(r.weight_8bit(deg), 1.0, "range {i} at {deg}");
        }
    }
}

#[test]
fn narrow_range_matches_the_measured_edges() {
    let r = range([350.0, 355.0, 5.0, 10.0]);
    assert_weight(&r, hue([255, 0, 21]), 0.85);
    assert_weight(&r, hue([255, 42, 0]), 0.05);
    assert_eq!(r.weight_8bit(hue([255, 0, 0])), 1.0);
    assert_eq!(r.weight_8bit(hue([255, 21, 0])), 1.0);
    assert_eq!(r.weight_8bit(hue([255, 64, 0])), 0.0);
    assert_eq!(r.weight_8bit(hue([255, 0, 42])), 0.0);
}

#[test]
fn hard_edged_range_takes_hue_15_but_not_345() {
    let r = range([345.0, 345.0, 15.0, 15.0]);
    for rgb in [[255, 0, 42], [255, 0, 21], [255, 0, 0], [255, 21, 0], [255, 42, 0], [255, 64, 0]] {
        assert_eq!(r.weight_8bit(hue(rgb)), 1.0, "{rgb:?}");
    }
    assert_eq!(r.weight_8bit(hue([255, 0, 64])), 0.0);
    assert_eq!(r.weight_8bit(hue([255, 85, 0])), 0.0);
}

/// A range narrower than the 0.77° between the shifted fade-in and fade-out still takes its hues.
#[test]
fn a_range_narrower_than_the_fade_lags_still_takes_its_hues() {
    let r = range([10.0, 10.0, 10.5, 10.5]);
    assert!(r.weight_8bit(10.25) > 0.0, "{}", r.weight_8bit(10.25));
    assert_eq!(r.weight_8bit(20.0), 0.0);
}
