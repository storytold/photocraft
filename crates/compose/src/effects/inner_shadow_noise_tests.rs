//! #14936 - BUG-LAYER-STYLE-INNER-SHADOW-NOISE-TOO-WEAK.
//!
//! Measured on the issue's test document: 400 x 300 px, RGB 8 bit, white Background, an orange
//! (255, 128, 0) pixel layer covering x 100..299, y 75..224. Inner Shadow black, Normal, 100 %,
//! Use Global Light off, Distance 0, Choke 0, Contour Linear, Anti-aliased off. The issue counts
//! the G channel of the flattened top band y 75..83, x 110..289 (1620 px): G 128 is the layer's
//! own orange (no shadow), G 0 is full black shadow. Today `noise` only scales the coverage by
//! `1 + (n - 0.5) * 2 * amount`, so Noise 50 removes the shadow nowhere (0 px at G 128) while the
//! reference application shows 491 px without shadow there.

use super::*;

const DOC_W: usize = 400;
const DOC_H: usize = 300;

/// The layer's alpha over the whole document: solid over x 100..=299, y 75..=224.
fn document_shape() -> Map {
    let mut m = Map::new(DOC_W, DOC_H, 0.0);
    for y in 75..=224 {
        for x in 100..=299 {
            m.v[y * DOC_W + x] = 1.0;
        }
    }
    m
}

/// The issue's Inner Shadow settings: black, Normal, 100 %, global light off, Distance 0,
/// Choke 0, Size 10, Contour Linear, Anti-aliased off, Noise `noise`.
fn shadow(noise: f32) -> Shadow {
    Shadow {
        common: photocraft_doc::FxCommon::new(BlendMode::Normal, 1.0),
        color: photocraft_color::Color::BLACK,
        angle: 0.0,
        use_global_light: false,
        distance: 0.0,
        spread: 0.0,
        size: 10.0,
        contour: Contour::Linear,
        anti_alias: false,
        noise,
        knocks_out: false,
    }
}

fn map(noise: f32) -> Map {
    shadow_map(&document_shape(), &shadow(noise), &GlobalLight::default(), true, (0, 0))
}

/// The G channel of the orange layer under a black Normal inner shadow of coverage `c`.
fn g_u8(c: f32) -> u8 {
    (128.0 * (1.0 - c)).round().clamp(0.0, 128.0) as u8
}

/// The issue's band, y 75..=83 and x 110..=289 (1620 px), as G channel values.
fn band(m: &Map) -> Vec<u8> {
    let mut out = Vec::with_capacity(180 * 9);
    for y in 75..=83 {
        for x in 110..=289 {
            out.push(g_u8(m.get(x, y)));
        }
    }
    out
}

/// Done when: Noise 50 on that band leaves 400..580 pixels at G 128 (no shadow at all), at
/// most 30 pixels equal to the noiseless result, and a minimum G of at most 15. Today the
/// multiplicative law never removes coverage: 0 pixels sit at G 128.
#[test]
fn noise_fifty_scatters_the_shadow_across_the_band() {
    let plain = band(&map(0.0));
    let noisy = band(&map(0.5));
    let gone = noisy.iter().filter(|v| **v == 128).count();
    let equal = noisy.iter().zip(&plain).filter(|(a, b)| a == b).count();
    let min = noisy.iter().copied().min().unwrap_or(255);
    assert!((400..=580).contains(&gone), "Noise 50: {gone} pixels without shadow, want 400..580 (reference: 491)");
    assert!(equal <= 30, "Noise 50: {equal} pixels equal to the noiseless result, want at most 30 (reference: 11)");
    assert!(min <= 15, "Noise 50: minimum G {min}, want at most 15 (reference: 7)");
}

/// Done when: Noise 100 leaves 550..720 pixels at G 128 and reaches G 0 (reference: 632 px
/// without shadow, minimum G 0). Today: 55 pixels at G 128 and minimum G 13.
#[test]
fn noise_hundred_spans_full_shadow_and_no_shadow() {
    let noisy = band(&map(1.0));
    let gone = noisy.iter().filter(|v| **v == 128).count();
    let min = noisy.iter().copied().min().unwrap_or(255);
    assert!((550..=720).contains(&gone), "Noise 100: {gone} pixels without shadow, want 550..720 (reference: 632)");
    assert_eq!(min, 0, "Noise 100: minimum G {min}, want 0");
}

/// Done when: the interior beyond the shadow's reach stays 128. Every pixel at least 10 px in
/// from all four edges (y 85..214, x 115..285 inside the layer) has zero coverage without noise
/// (the Size 10 falloff ends at depth 10, see the #14933 ramp), and noise must not light any of
/// them up.
#[test]
fn noise_keeps_the_unshadowed_interior_clean() {
    let plain = map(0.0);
    for noise in [0.5f32, 1.0] {
        let noisy = map(noise);
        for y in (85..=214).step_by(3) {
            for x in (115..=285).step_by(7) {
                assert_eq!(plain.get(x, y), 0.0, "noiseless coverage at ({x},{y}) is not empty");
                assert_eq!(noisy.get(x, y), 0.0, "noise {noise} lit up ({x},{y}): {}", noisy.get(x, y));
                assert_eq!(g_u8(noisy.get(x, y)), 128, "noise {noise} moved the interior at ({x},{y}) off 128");
            }
        }
    }
}

/// Done when: the same settings give the identical image twice.
#[test]
fn the_same_noise_renders_identically_twice() {
    for noise in [0.5f32, 1.0] {
        let a = map(noise);
        let b = map(noise);
        assert_eq!(a.v, b.v, "noise {noise} is not deterministic");
    }
}

/// A hash over every coverage value of a map: the map is bit-for-bit unchanged iff this is.
fn bit_hash(m: &Map) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in &m.v {
        h = (h ^ u64::from(v.to_bits())).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Noise 0 stays bit-identical: this is the whole-map FNV hash `shadow_map` gives for the
/// issue's document with Noise 0 on upstream/main, so the new law must not touch it.
#[test]
fn noise_zero_maps_stay_bit_identical() {
    assert_eq!(bit_hash(&map(0.0)), 0x2200_2b2d_61bc_9c2d, "the Noise 0 inner shadow map moved");
}

/// The issues measure falloff bands only. A big Drop Shadow's solid interior (coverage exactly
/// 1.0 in the Size 40 blur plateau) keeps today's multiplicative jitter: at Noise 100 the core
/// still spans both sides of 0.5 and still holds full coverage.
/// Guard test, passes before and after the law change - it pins the unmeasured case.
#[test]
fn solid_core_keeps_todays_jitter() {
    let mut shape = Map::new(DOC_W, DOC_H, 0.0);
    for y in 75..=224 {
        for x in 100..=299 {
            shape.v[y * DOC_W + x] = 1.0;
        }
    }
    // The plateau of the Size 40 blur at Distance 0 (measured: coverage is exactly 1.0 on these
    // samples, at least 45 px in from every edge).
    let core = |noise: f32| {
        let s = Shadow { size: 40.0, noise, ..shadow(0.0) };
        let m = shadow_map(&shape, &s, &GlobalLight::default(), false, (0, 0));
        (145..=176).step_by(3).flat_map(|y| (150..=250).step_by(5).map(move |x| (x, y))).map(|(x, y)| m.get(x, y)).collect::<Vec<f32>>()
    };
    let plain = core(0.0);
    assert!(plain.iter().all(|v| *v == 1.0), "the Size 40 drop shadow plateau is solid: {plain:?}");
    let half = core(0.5);
    assert!(half.iter().any(|v| *v < 1.0) && half.contains(&1.0), "Noise 50 speckles the solid core: {half:?}");
    let heavy = core(1.0);
    assert!(heavy.iter().any(|v| *v < 0.5) && heavy.contains(&1.0), "Noise 100 spans both sides of 0.5 in the solid core: {heavy:?}");
}
