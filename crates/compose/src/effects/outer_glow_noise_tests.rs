//! #14952 - BUG-LAYER-STYLE-OUTER-GLOW-NOISE-TOO-WEAK.
//!
//! Measured on the issue's test document: 400 x 300 px, RGB 8 bit, white Background, an orange
//! (255, 128, 0) pixel layer covering x 100..299, y 75..224. Outer Glow black, Normal, 100 %,
//! Softer, Spread 0, Size 10, Contour Linear, Range 50 %. The issue counts the R channel of the
//! flattened band just above the rectangle, y 66..74, x 110..289 (1620 px): R 255 is the white
//! backdrop (no glow), R 0 is full black glow. Today `noise` only scales the coverage by
//! `1 + (n - 0.5) * 2 * amount`, so Noise 50 removes the glow nowhere (0 px at R 255) while the
//! reference application shows 314 px without glow there.

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

/// The issue's Outer Glow settings: black, Normal, 100 %, Softer, Spread 0, Size 10, Contour
/// Linear, Range 50 %, Jitter 0, Anti-aliased off, Noise `noise`.
fn glow(noise: f32) -> Glow {
    Glow {
        common: photocraft_doc::FxCommon::new(BlendMode::Normal, 1.0),
        paint: FxPaint::Color(photocraft_color::Color::BLACK),
        technique: GlowTechnique::Softer,
        spread: 0.0,
        size: 10.0,
        contour: Contour::Linear,
        anti_alias: false,
        range: 0.5,
        jitter: 0.0,
        noise,
        source: GlowSource::Edge,
    }
}

fn map(noise: f32) -> Map {
    glow_map(&document_shape(), &glow(noise), false, (0, 0))
}

/// The R channel of the white backdrop under a black Normal outer glow of coverage `c`.
fn r_u8(c: f32) -> u8 {
    (255.0 * (1.0 - c)).round().clamp(0.0, 255.0) as u8
}

/// The issue's band, y 66..=74 and x 110..=289 (1620 px), as R channel values.
fn band(m: &Map) -> Vec<u8> {
    let mut out = Vec::with_capacity(180 * 9);
    for y in 66..=74 {
        for x in 110..=289 {
            out.push(r_u8(m.get(x, y)));
        }
    }
    out
}

/// Done when: Noise 50 on that band leaves 240..400 pixels at R 255 (no glow at all), at most
/// 30 pixels equal to the noiseless result, and reaches R 0. Today the multiplicative law
/// removes the glow nowhere: 0 pixels sit at R 255.
#[test]
fn noise_fifty_scatters_the_glow_across_the_band() {
    let plain = band(&map(0.0));
    let noisy = band(&map(0.5));
    let gone = noisy.iter().filter(|v| **v == 255).count();
    let equal = noisy.iter().zip(&plain).filter(|(a, b)| a == b).count();
    let min = noisy.iter().copied().min().unwrap_or(255);
    assert!((240..=400).contains(&gone), "Noise 50: {gone} pixels without glow, want 240..400 (reference: 314)");
    assert!(equal <= 30, "Noise 50: {equal} pixels equal to the noiseless result, want at most 30 (reference: 9)");
    assert_eq!(min, 0, "Noise 50: minimum R {min}, want 0");
}

/// Done when: Noise 100 leaves 400..600 pixels at R 255 (reference: 498). Today: 14.
#[test]
fn noise_hundred_thins_the_glow_further() {
    let noisy = band(&map(1.0));
    let gone = noisy.iter().filter(|v| **v == 255).count();
    let min = noisy.iter().copied().min().unwrap_or(255);
    assert!((400..=600).contains(&gone), "Noise 100: {gone} pixels without glow, want 400..600 (reference: 498)");
    assert_eq!(min, 0, "Noise 100: minimum R {min}, want 0");
}

/// Done when: the noise does not reach beyond the glow's falloff: the row y 40 (25 px above the
/// rectangle, ten times the Size 10 reach) stays 255.
#[test]
fn noise_keeps_the_backdrop_beyond_the_glow_clean() {
    let plain = map(0.0);
    for noise in [0.5f32, 1.0] {
        let noisy = map(noise);
        for y in (16..=64).step_by(6) {
            for x in (110..=289).step_by(7) {
                assert_eq!(plain.get(x, y), 0.0, "noiseless coverage at ({x},{y}) is not empty");
                assert_eq!(noisy.get(x, y), 0.0, "noise {noise} lit up ({x},{y}): {}", noisy.get(x, y));
                assert_eq!(r_u8(noisy.get(x, y)), 255, "noise {noise} moved the backdrop at ({x},{y}) off 255");
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

/// Noise 0 stays bit-identical: this is the whole-map FNV hash `glow_map` gives for the
/// issue's document with Noise 0 on upstream/main, so the new law must not touch it. Re-pinned
/// for #15871, whose Range rule moves this Range 50 % map towards the measured #14958 profile.
#[test]
fn noise_zero_maps_stay_bit_identical() {
    assert_eq!(bit_hash(&map(0.0)), 0x49c1_01e2_0b3d_c395, "the Noise 0 outer glow map moved");
}
