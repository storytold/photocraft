//! #14933 — BUG-LAYER-STYLE-INNER-SHADOW-SIZE-ONE.
//!
//! Measured on the issue's test document: 400 × 300 px, 72 ppi, RGB 8 bit, white Background, an
//! orange (255, 128, 0) pixel layer covering x 100..299, y 75..224. Inner Shadow black, Normal,
//! 100 %, Use Global Light off, Distance 0, Choke 0, Contour Linear, Anti-aliased off, Noise 0.
//! The issue's table is the G channel of the flattened image read from a layer edge inwards:
//! 128 is the layer's own orange (no shadow), 0 is full shadow.

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

/// The issue's Inner Shadow settings: black, Normal, 100 %, global light off, Choke `spread`.
fn shadow(size: f32, distance: f32, angle: f32, spread: f32) -> Shadow {
    Shadow {
        common: photocraft_doc::FxCommon::new(BlendMode::Normal, 1.0),
        color: photocraft_color::Color::BLACK,
        angle,
        use_global_light: false,
        distance,
        spread,
        size,
        contour: Contour::Linear,
        anti_alias: false,
        noise: 0.0,
        knocks_out: false,
    }
}

fn map(size: f32, distance: f32, angle: f32, spread: f32) -> Map {
    shadow_map(&document_shape(), &shadow(size, distance, angle, spread), &GlobalLight::default(), true, (0, 0))
}

/// The G channel of the orange layer under a black Normal inner shadow of coverage `c`.
fn g(c: f32) -> f32 {
    128.0 * (1.0 - c)
}

/// The twenty G values inwards from each of the four edges, in the order the issue reads them:
/// top = column x 200 from y 75 down, left = row y 150 from x 100, bottom = column x 200 from
/// y 224 up, right = row y 150 from x 299 left.
fn ramps(m: &Map) -> [Vec<f32>; 4] {
    let line = |first: (i64, i64), step: (i64, i64)| (0..20).map(|k| g(m.get(first.0 + k * step.0, first.1 + k * step.1))).collect();
    [line((200, 75), (0, 1)), line((100, 150), (1, 0)), line((200, 224), (0, -1)), line((299, 150), (-1, 0))]
}

/// Assert the ten-pixel G ramp of every edge against `want`, within 2 levels (the issue's
/// tolerance) and the same on all four sides.
fn assert_edges(m: &Map, want: &[f32], what: &str) {
    let [top, left, bottom, right] = ramps(m);
    for (side, r) in ["top", "left", "bottom", "right"].iter().zip([top.as_slice(), left.as_slice(), bottom.as_slice(), right.as_slice()]) {
        for (d, w) in want.iter().enumerate() {
            assert!((r[d] - *w).abs() <= 2.0, "{what}, {side} edge, {d} px in: {} vs measured {w}", r[d]);
        }
    }
}

/// Done when: Size 1, Distance 0 puts G 96 (within 2 levels) on the first pixel inside every edge
/// and leaves the second one at the layer's own 128. Today `shadow_map` blurs with a width-1 box,
/// which is the identity, so the map is still the hard inverted alpha and the clip to the layer
/// removes the shadow completely: every pixel stays 128.
#[test]
fn size_one_draws_the_measured_edge_pixel() {
    assert_edges(&map(1.0, 0.0, 0.0, 0.0), &[96.0, 128.0], "Size 1");
    // Outside the layer nothing changes: the map is still clipped to the shape, so white stays white
    // at y 74 and x 99.
    let m = map(1.0, 0.0, 0.0, 0.0);
    for (x, y) in [(200i64, 74), (99, 150), (200, 225), (300, 150)] {
        assert_eq!(m.get(x, y), 0.0, "no inner shadow outside the layer at ({x},{y})");
    }
}

/// Done when: Sizes 3, 10 and 20 keep the measured profiles (they already match today).
#[test]
fn sizes_three_ten_and_twenty_keep_the_measured_profiles() {
    let table: [(f32, &[f32]); 3] = [
        (3.0, &[85.0, 114.0, 128.0]),
        (10.0, &[70.0, 82.0, 92.0, 101.0, 109.0, 115.0, 120.0, 124.0, 126.0, 128.0]),
        (20.0, &[67.0, 73.0, 79.0, 84.0, 89.0, 94.0, 99.0, 103.0, 107.0, 110.0, 113.0, 116.0, 119.0, 121.0, 123.0, 125.0, 126.0, 127.0, 127.0, 128.0]),
    ];
    for (size, want) in table {
        assert_edges(&map(size, 0.0, 0.0, 0.0), want, &format!("Size {size}"));
    }
}

/// Done when: Size 0 keeps the hard band of Distance pixels. Measured: Size 0, Distance 5,
/// Angle 90 reads 0 for five pixels from the top edge and 128 after that; the other edges stay 128.
#[test]
fn size_zero_keeps_the_hard_band_of_distance() {
    let m = map(0.0, 5.0, 90.0, 0.0);
    let col: Vec<f32> = (75..81).map(|y| g(m.get(200, y))).collect();
    assert_eq!(&col[..5], &[0.0f32; 5][..], "five dark pixels then 128: {col:?}");
    assert_eq!(col[5], 128.0, "the band ends after Distance pixels: {col:?}");
    // The other three edges read their own colour at the pixel the issue reads them from.
    for k in 0..6 {
        assert_eq!(g(m.get(100 + k, 150)), 128.0, "left edge keeps its own colour, x {}", 100 + k);
        assert_eq!(g(m.get(299 - k, 150)), 128.0, "right edge keeps its own colour, x {}", 299 - k);
        assert_eq!(g(m.get(200, 224 - k)), 128.0, "bottom edge keeps its own colour, y {}", 224 - k);
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

/// Every other Size 1 and every other size is bit-for-bit what it was: these are the coverage
/// ramps and whole-map hashes `shadow_map` gave on upstream/main c162cb6d6 before the fix, copied
/// out of the probe run. They pin that the Size 1 kernel is taken only by the Inner Shadow of
/// Size 1 with Choke 0 — sub-1 and fractional sizes (which the issue leaves unmeasured), sizes
/// 1.5..250, the Drop Shadow of the same size (#14880 keeps its own ticket) and a choked Size 1
/// all keep today's map.
#[test]
fn other_sizes_are_unchanged() {
    let pin = |m: &Map, want: &[f64]| {
        let got: Vec<f32> = (75..87).map(|y| m.get(200, y)).collect();
        for (d, (g, w)) in got.iter().zip(want).enumerate() {
            assert!((f64::from(*g) - *w).abs() <= 1e-6, "{d} px in: {g:.8} vs {w:.8} on main");
        }
    };
    let sizes: [(f32, &[f64]); 8] = [
        (0.5, &[0.0; 12]),
        (1.5, &[0.25, 0.02777778, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        (2.0, &[0.3125, 0.0625, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        (3.0, &[0.33333334, 0.11111111, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        (6.0, &[0.42361110, 0.28472221, 0.17361110, 0.09027778, 0.03472222, 0.00694444, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        (10.0, &[0.45249999, 0.36250001, 0.28250000, 0.21250001, 0.15250000, 0.10250000, 0.06250000, 0.03250000, 0.01250000, 0.00250000, 0.0, 0.0]),
        (
            20.0,
            &[0.47562501, 0.42812499, 0.38312501, 0.34062499, 0.30062500, 0.26312500, 0.22812501, 0.19562501, 0.16562501, 0.13812500, 0.11312500, 0.09062500],
        ),
        (
            250.0,
            &[0.45157880, 0.45233655, 0.45308408, 0.45382136, 0.45454839, 0.45526519, 0.45597175, 0.45666808, 0.45735416, 0.45802999, 0.45869559, 0.45935094],
        ),
    ];
    let hashes: [(f32, u64); 8] = [
        (0.5, 0xe94d_157b_1934_6225),
        (1.5, 0xbbca_5e75_8b9f_520d),
        (2.0, 0x6bd5_69ac_da8a_6225),
        (3.0, 0x3109_0d74_08b1_8c1d),
        (6.0, 0xe5dc_989a_bcfb_ed89),
        (10.0, 0x2200_2b2d_61bc_9c2d),
        (20.0, 0x8ba0_be17_19fa_7b25),
        (250.0, 0x460f_fa2e_3da8_0c4d),
    ];
    for (size, want) in sizes {
        pin(&map(size, 0.0, 0.0, 0.0), want);
    }
    for (size, want) in hashes {
        let got = bit_hash(&map(size, 0.0, 0.0, 0.0));
        assert_eq!(got, want, "the whole Size {size} inner shadow map moved");
    }
    // A Drop Shadow of Size 1 keeps today's hard inverted alpha (#14880 is its own ticket).
    let drop = |size: f32| shadow_map(&document_shape(), &shadow(size, 0.0, 0.0, 0.0), &GlobalLight::default(), false, (0, 0));
    let one = drop(1.0);
    let got: Vec<f32> = (72..84).map(|y| one.get(200, y)).collect();
    assert_eq!(&got[..3], &[0.0f32; 3][..], "a Size 1 drop shadow still paints nothing outside the layer: {got:?}");
    assert_eq!(&got[3..], &[1.0f32; 9][..], "and stays hard inside it: {got:?}");
    assert_eq!(bit_hash(&one), 0xa067_0cbe_8a34_6225, "the whole Size 1 drop shadow map moved");
    assert_eq!(bit_hash(&drop(3.0)), 0xc097_cc3a_25df_6ae5, "the whole Size 3 drop shadow map moved");
    // Size 1 with Choke 50 % is not measured either: today it is a hard 1 px band.
    let choked = map(1.0, 0.0, 0.0, 0.5);
    assert_eq!(choked.get(200, 74), 0.0);
    assert_eq!(choked.get(200, 75), 1.0, "a choked Size 1 inner shadow keeps its hard band");
    assert_eq!(choked.get(200, 76), 0.0);
    assert_eq!(bit_hash(&choked), 0xde5c_6e74_9734_6225, "the choked Size 1 map moved");
}

/// The kernel is distance-independent, so a Size 1 inner shadow away from the light keeps the
/// band of Distance pixels and gains the same ¾ / ¼ coverage where the band ends. That
/// combination is not in the issue's measured table; it is spelled out here so the consequence of
/// the kernel is visible rather than assumed.
#[test]
fn size_one_at_a_distance_adds_the_ramp_at_the_band_edge() {
    let m = map(1.0, 5.0, 90.0, 0.0);
    let col: Vec<f32> = (75..83).map(|y| g(m.get(200, y))).collect();
    assert_eq!(&col[..4], &[0.0f32; 4][..], "the band of Distance pixels: {col:?}");
    assert!((col[4] - 32.0).abs() <= 2.0 && (col[5] - 96.0).abs() <= 2.0, "the quarter ramp where the band ends: {col:?}");
    assert_eq!(&col[6..], &[128.0f32; 2][..], "and nothing deeper: {col:?}");
}
