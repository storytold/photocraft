//! #14741: a colour range's Saturation multiplies the Master Saturation's factor instead of
//! adding to it.

use super::*;

fn hue_sat(master: f32, reds: f32) -> Adjustment {
    let mut ranges = HueRange::defaults();
    ranges[0].saturation = reds;
    Adjustment::HueSaturation { hue: 0.0, saturation: master, lightness: 0.0, colorize: false, ranges }
}

fn run(adj: &Adjustment, c: [f32; 3], depth: SampleType) -> [f32; 3] {
    let mut b = Buffer::filled(crate::Rect::new(0, 0, 1, 1), [c[0], c[1], c[2], 1.0]);
    apply_depth(adj, &mut b, Transfer::Gamma(1.0), Some(depth));
    [b.px[0][0], b.px[0][1], b.px[0][2]]
}

fn levels(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| (v * 255.0).round())
}

fn within(got: [f32; 3], want: [f32; 3], tolerance: f32) -> bool {
    levels(got).iter().zip(levels(want)).all(|(g, w)| (g - w).abs() <= tolerance)
}

/// The reference application, measured: Master -50 with Reds -50 turns (255, 0, 0) into
/// (159, 95, 95), a quarter of red's saturation. Adding the two made -100 and grey.
#[test]
fn master_and_reds_minus_fifty_keep_a_quarter_of_red() {
    let got = run(&hue_sat(-50.0, -50.0), [1.0, 0.0, 0.0], SampleType::U8);
    let want = [159.0, 95.0, 95.0].map(|v| v / 255.0);
    assert!(within(got, want, 1.0), "{:?}", levels(got));
}

/// The 72-hue ramp of the measurement at HSL saturation 100 % and 50 %: no Reds cell turns grey,
/// and away from the range's fades every cell has master × range of its saturation.
#[test]
fn ramp_cells_take_the_product_of_both_factors() {
    let adj = hue_sat(-50.0, -50.0);
    for s in [1.0, 0.5] {
        for step in 0..72 {
            let deg = step as f32 * 5.0;
            let c = hsl_to_rgb(deg / 360.0, s, 0.5);
            let got = run(&adj, c, SampleType::U8);
            let from_red = deg.min(360.0 - deg);
            if from_red < 45.0 {
                let chroma = got[0].max(got[1]).max(got[2]) - got[0].min(got[1]).min(got[2]);
                assert!(chroma * 255.0 >= 8.0, "hue {deg} at {s}: {:?} is grey", levels(got));
            }
            let factor = match from_red {
                d if d <= 15.0 => 0.25,
                d if d >= 45.0 => 0.5,
                _ => continue,
            };
            let want = hsl_to_rgb(deg / 360.0, s * factor, 0.5);
            assert!(within(got, want, 2.0), "hue {deg} at {s}: {:?}, want {:?}", levels(got), levels(want));
        }
    }
}

/// Raised and mixed settings combine as the same factors applied in turn: +50 doubles the
/// saturation, -50 halves it, +25 multiplies it by 4/3.
#[test]
fn raised_and_mixed_settings_multiply_their_factors() {
    let red = |s: f32| hsl_to_rgb(0.0, s, 0.5);
    for (master, reds, s, want) in
        [(50.0, -50.0, 0.4, 0.4), (-50.0, 50.0, 0.4, 0.4), (25.0, 25.0, 0.3, 0.3 * 16.0 / 9.0), (50.0, 50.0, 0.4, 1.0), (-100.0, 50.0, 0.4, 0.0)]
    {
        let got = run(&hue_sat(master, reds), red(s), SampleType::U8);
        assert!(within(got, red(want), 1.0), "master {master}, reds {reds}: {:?}, want {:?}", levels(got), levels(red(want)));
    }
}

/// 32 bits scale the chroma by the product of their own factors: a quarter of red's chroma for
/// -50 twice, and Reds -50 halves what Master +50 (510/256) or +100 (32767/256) alone gives.
#[test]
fn thirty_two_bits_multiply_too() {
    let chroma = |c: [f32; 3]| c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]);
    for (master, reds, want) in [(-50.0, -50.0, 0.25), (50.0, -50.0, 255.0 / 256.0), (100.0, -50.0, 32767.0 / 512.0)] {
        let got = run(&hue_sat(master, reds), [1.0, 0.0, 0.0], SampleType::F32);
        assert!((chroma(got) - want).abs() < 1e-3 * want, "master {master}, reds {reds}: {got:?}, chroma {want}");
        assert!(got[1..].iter().all(|v| (v - (1.0 - want) / 2.0).abs() < 1e-3 * want.max(1.0)), "{got:?}");
    }
}

/// A range alone reaches the saturation step as a master of the same value does, bit for bit, on
/// a colour inside the range: one setting alone is untouched by the change.
#[test]
fn one_setting_alone_passes_through_unchanged() {
    let c = hsl_to_rgb(2.0 / 360.0, 0.6, 0.45);
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for k in -20..=20 {
            let v = k as f32 * 5.0;
            let range = run(&hue_sat(0.0, v), c, depth).map(f32::to_bits);
            let master = run(&hue_sat(v, 0.0), c, depth).map(f32::to_bits);
            assert_eq!(range, master, "{depth:?}, {v}");
        }
    }
}

#[test]
fn hostile_values_stay_finite() {
    for depth in [SampleType::U8, SampleType::F32] {
        for (m, r) in [(f32::INFINITY, -50.0), (f32::NEG_INFINITY, 50.0), (100.0, 100.0), (-100.0, -100.0), (100.0, -100.0), (500.0, 500.0), (-500.0, 1e9)] {
            let got = run(&hue_sat(m, r), [0.8, 0.1, 0.2], depth);
            assert!(got.iter().all(|v| v.is_finite()), "{depth:?}, {m}, {r}: {got:?}");
        }
        let _ = run(&hue_sat(f32::NAN, f32::NAN), [0.8, 0.1, 0.2], depth);
    }
}
