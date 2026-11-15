//! Inner Glow at Range 1 % (#14929): the issue's document (400 × 300, an orange 200 × 150
//! rectangle at 100,75), a black Softer Edge glow, size 10, Linear. The reference's G channel from
//! the edge inwards (no glow = 128).

use super::*;

pub(super) const W: usize = 400;
pub(super) const H: usize = 300;

pub(super) fn rect() -> Map {
    let mut m = Map::new(W, H, 0.0);
    for y in 75..225 {
        for x in 100..300 {
            m.v[y * W + x] = 1.0;
        }
    }
    m
}

pub(super) fn glow(range: f32) -> Glow {
    Glow {
        common: photocraft_doc::FxCommon::new(BlendMode::Normal, 1.0),
        paint: FxPaint::Color(photocraft_color::Color::BLACK),
        technique: GlowTechnique::Softer,
        spread: 0.0,
        size: 10.0,
        contour: Contour::Linear,
        anti_alias: false,
        range,
        jitter: 0.0,
        noise: 0.0,
        source: GlowSource::Edge,
    }
}

/// The four edge profiles, each from the edge outwards or inwards: top, left, bottom, right.
pub(super) fn profiles(m: &Map, inner: bool, level: f32) -> [Vec<i32>; 4] {
    let at = |x: i32, y: i32| (level * (1.0 - m.v[y as usize * W + x as usize])).round() as i32;
    let (n, s) = if inner { (75, 224) } else { (74, 225) };
    let (we, ea) = if inner { (100, 299) } else { (99, 300) };
    let d = if inner { 1 } else { -1 };
    [
        (0..10).map(|i| at(200, n + d * i)).collect(),
        (0..10).map(|i| at(we + d * i, 150)).collect(),
        (0..10).map(|i| at(200, s - d * i)).collect(),
        (0..10).map(|i| at(ea - d * i, 150)).collect(),
    ]
}

pub(super) fn close(got: &[i32], want: &[i32], tol: i32) -> bool {
    got.len() == want.len() && got.iter().zip(want).all(|(a, b)| (a - b).abs() <= tol)
}

const RANGE_1: [i32; 10] = [0, 0, 0, 0, 0, 0, 0, 0, 19, 128];
const RANGE_25: [i32; 10] = [0, 0, 0, 21, 52, 77, 97, 113, 123, 128];
const RANGE_50: [i32; 10] = [13, 36, 57, 74, 90, 102, 113, 120, 125, 128];
const RANGE_100: [i32; 10] = [70, 82, 92, 101, 109, 115, 120, 124, 126, 128];

#[test]
fn range_one_is_eight_solid_pixels_then_a_light_one() {
    let m = glow_map(&rect(), &glow(0.01), true, (0, 0));
    for (i, p) in profiles(&m, true, 128.0).iter().enumerate() {
        assert!(close(p, &RANGE_1, 3), "edge {i}: {p:?}");
    }
}

#[test]
fn ranges_25_50_100_are_unchanged() {
    for (range, want) in [(0.25, RANGE_25), (0.5, RANGE_50), (1.0, RANGE_100)] {
        let m = glow_map(&rect(), &glow(range), true, (0, 0));
        for (i, p) in profiles(&m, true, 128.0).iter().enumerate() {
            assert!(close(p, &want, 2), "range {range} edge {i}: {p:?}");
        }
    }
}

#[test]
fn glow_lut_at_range_one_matches_the_measured_table() {
    // `photocraft-gpu` builds its glow table from this function too, so Range 1 % reads alike there.
    let l = glow_lut(&glow(0.01), true).unwrap();
    assert_eq!(lut_at(&l, 0.0025), 0.0);
    assert!((lut_at(&l, 0.0125) - 0.85).abs() < 0.02);
    assert_eq!(lut_at(&l, 0.0325), 1.0);
}

/// The coverage at which the table first reaches full strength.
fn full_at(l: &[f32]) -> f32 {
    l.iter().position(|v| *v >= 1.0).map_or(f32::INFINITY, |k| k as f32 / (l.len() - 1) as f32)
}

#[test]
fn raising_the_range_never_shortens_the_glow() {
    let reach: Vec<f32> = [0.010, 0.012, 0.0147, 0.016, 0.02].iter().map(|r| full_at(&glow_lut(&glow(*r), true).unwrap())).collect();
    assert!(reach.windows(2).all(|w| w[0] <= w[1] + 1e-6), "full-strength coverage must not decrease: {reach:?}");
    assert!((reach[0] - reach[1]).abs() < 1e-6, "1 % and 1.2 % render alike: {reach:?}");
}

#[test]
fn an_inner_center_glow_at_range_one_keeps_the_plain_transfer() {
    let mut g = glow(0.01);
    g.source = GlowSource::Center;
    assert_eq!(glow_lut(&g, true), ranged_lut(&g.contour, 0.01), "unmeasured: unchanged");
    // The Center source means nothing to an outer glow, which keeps the measured Range 1 %.
    assert_ne!(glow_lut(&g, false), ranged_lut(&g.contour, 0.01));
    g.source = GlowSource::Edge;
    assert_ne!(glow_lut(&g, true), ranged_lut(&g.contour, 0.01));
}
