use super::{sponge_affine_scale, sponge_rgb};

const COLORS: [[u8; 3]; 9] =
    [[200, 40, 40], [40, 160, 60], [40, 60, 200], [220, 170, 140], [230, 200, 40], [90, 50, 30], [180, 200, 230], [128, 64, 192], [255, 0, 0]];

pub(super) fn check_reference(saturate: bool, vibrance: bool, flow: f32, expected: [[u8; 3]; 9]) {
    for (input, expected) in COLORS.into_iter().zip(expected) {
        let actual = sponge_rgb(input.map(|v| f32::from(v) / 255.0), flow, 0.25, saturate, vibrance).map(|v| (v * 255.0).round() as u8);
        assert!(
            actual.into_iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 2),
            "RGB {input:?}, Flow {flow}, Vibrance {vibrance}: {actual:?} vs {expected:?}"
        );
    }
}

#[test]
fn desaturate_matches_measured_single_dabs() {
    // One 40 px hard round dab at 25% spacing, measured in #12972.
    check_reference(
        false,
        true,
        0.5,
        [[193, 40, 40], [41, 155, 60], [40, 59, 193], [213, 170, 144], [226, 197, 42], [87, 49, 31], [184, 200, 225], [126, 66, 185], [248, 0, 0]],
    );
    check_reference(
        false,
        true,
        1.0,
        [[187, 40, 40], [42, 149, 60], [40, 59, 187], [206, 170, 149], [223, 195, 44], [83, 48, 31], [187, 200, 219], [123, 69, 178], [240, 0, 0]],
    );
    check_reference(
        false,
        false,
        0.5,
        [[189, 44, 44], [47, 155, 65], [43, 61, 188], [216, 171, 144], [226, 199, 54], [87, 51, 33], [182, 200, 227], [125, 67, 183], [237, 7, 7]],
    );
    check_reference(
        false,
        false,
        1.0,
        [[178, 49, 49], [54, 151, 70], [46, 62, 175], [212, 172, 147], [222, 198, 69], [84, 52, 36], [183, 199, 223], [122, 71, 174], [220, 15, 15]],
    );
}

#[test]
fn greys_are_fixed_for_both_modes_and_every_flow() {
    for level in 0..=255u8 {
        let grey = [f32::from(level) / 255.0; 3];
        for flow in 0..=100u8 {
            for saturate in [false, true] {
                for vibrance in [false, true] {
                    assert_eq!(sponge_rgb(grey, f32::from(flow) / 100.0, 0.25, saturate, vibrance), grey);
                }
            }
        }
    }
}

#[test]
fn flow_changes_chroma_monotonically_and_preserves_hue_in_the_rgb_gamut() {
    let levels: [f32; 9] = [0.0, 0.02, 0.12, 0.3, 0.5, 0.7, 0.9, 0.99, 1.0];
    for r in levels {
        for g in levels {
            for b in levels {
                let input = [r, g, b];
                let lo = r.min(g).min(b);
                let hi = r.max(g).max(b);
                let chroma = hi - lo;
                if chroma == 0.0 {
                    continue;
                }
                let hue = input.map(|v| (v - lo) / chroma);
                for saturate in [false, true] {
                    for vibrance in [false, true] {
                        assert_eq!(sponge_rgb(input, 0.0, 0.25, saturate, vibrance), input);
                        let mut previous = chroma;
                        for amount in [0.1, 0.5, 1.0] {
                            let actual = sponge_rgb(input, amount, 0.25, saturate, vibrance);
                            assert!(actual.into_iter().all(|v| (-1e-6..=1.0 + 1e-6).contains(&v)), "{input:?} became {actual:?}");
                            let lo = actual.into_iter().fold(1.0, f32::min);
                            let hi = actual.into_iter().fold(0.0, f32::max);
                            let span = hi - lo;
                            assert!(if saturate { span >= previous - 1e-6 } else { span <= previous + 1e-6 });
                            for (out, normalized) in actual.into_iter().zip(hue) {
                                assert!((out - lo - normalized * span).abs() < 1e-6, "{input:?} became {actual:?}");
                            }
                            previous = span;
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn equal_distance_non_vibrance_passes_have_equal_strength() {
    let input = [0.58, 0.51, 0.42];
    for flow in [0.2, 0.5, 1.0] {
        for saturate in [false, true] {
            let pass = |spacing, dabs| (0..dabs).fold(input, |rgb, _| sponge_rgb(rgb, flow, spacing, saturate, false));
            let reference = pass(0.25, 4);
            for (spacing, dabs) in [(0.125, 8), (0.5, 2), (1.0, 1)] {
                for (actual, expected) in pass(spacing, dabs).into_iter().zip(reference) {
                    assert!((actual - expected).abs() < 1e-6, "Flow {flow}, spacing {spacing}, saturate {saturate}: {actual} vs {expected}");
                }
            }
        }
    }
}

#[test]
fn vibrance_at_other_spacings_stays_in_gamut_and_preserves_hue() {
    for input in COLORS.map(|rgb| rgb.map(|v| f32::from(v) / 255.0)) {
        let lo = input.into_iter().fold(1.0, f32::min);
        let hi = input.into_iter().fold(0.0, f32::max);
        let hue = input.map(|v| (v - lo) / (hi - lo));
        for spacing in [0.01, 0.125, 0.5, 1.0] {
            for saturate in [false, true] {
                let out = sponge_rgb(input, 1.0, spacing, saturate, true);
                let a = out.into_iter().fold(1.0, f32::min);
                let b = out.into_iter().fold(0.0, f32::max);
                assert!(out.into_iter().all(|v| (-1e-6..=1.0 + 1e-6).contains(&v)), "{input:?} became {out:?}");
                assert!(if saturate { b - a >= hi - lo - 1e-6 } else { b - a <= hi - lo + 1e-6 });
                for (v, h) in out.into_iter().zip(hue) {
                    assert!((v - a - h * (b - a)).abs() < 1e-6, "{input:?} became {out:?}");
                }
            }
        }
    }
}

#[test]
fn fractional_vibrance_gain_stays_accurate_near_zero() {
    for steps in [0.04, 0.5, 0.68, 1.32, 2.0, 3.0, 4.0] {
        for change in [-0.3, -0.002, -0.0001, 0.0, 0.0001, 0.002, 0.3] {
            let (x, n) = (f64::from(change), f64::from(steps));
            let expected = if x == 0.0 { n } else { (n * x.ln_1p()).exp_m1() / x };
            let actual = f64::from(sponge_affine_scale(change, steps));
            assert!((actual - expected).abs() < 0.0001, "change {change}, steps {steps}: {actual} vs {expected}");
        }
    }
}
