//! #14950 — BUG-LAYER-STYLE-OUTER-GLOW-SIZE-ONE.
//!
//! Measured on the issue's test document: 400 × 300 px, 72 ppi, RGB 8 bit, white Background, an
//! orange (255, 128, 0) pixel layer covering x 100..299, y 75..224. Outer Glow black, Normal,
//! Opacity 100 %, Noise 0, Technique Softer, Spread 0, Contour Linear, Range 50. The issue's
//! table is the R channel of the flattened image read outwards from a layer edge: 255 is the white
//! backdrop (no glow), 0 is a full black glow. The layer itself is not covered by its glow: it
//! paints over it.

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

/// The issue's Outer Glow settings: black, Normal, 100 %, Softer, Spread `spread`, Range 50 %.
fn glow(size: f32, spread: f32) -> Glow {
    Glow {
        common: photocraft_doc::FxCommon::new(BlendMode::Normal, 1.0),
        paint: FxPaint::Color(photocraft_color::Color::BLACK),
        technique: GlowTechnique::Softer,
        spread,
        size,
        contour: Contour::Linear,
        anti_alias: false,
        range: 0.5,
        jitter: 0.0,
        noise: 0.0,
        source: GlowSource::Edge,
    }
}

fn map(size: f32, spread: f32) -> Map {
    glow_map(&document_shape(), &glow(size, spread), false, (0, 0))
}

/// The R channel of the white backdrop under a black Normal outer glow of coverage `c`.
fn r(c: f32) -> f32 {
    255.0 * (1.0 - c)
}

/// The twelve R values outwards from each of the four edges, in the order the issue reads them:
/// top = column x 200 from y 74 up, left = row y 150 from x 99 left, bottom = column x 200 from
/// y 225 down, right = row y 150 from x 300 right.
fn ramps(m: &Map) -> [Vec<f32>; 4] {
    let line = |first: (i64, i64), step: (i64, i64)| (0..12).map(|k| r(m.get(first.0 + k * step.0, first.1 + k * step.1))).collect();
    [line((200, 74), (0, -1)), line((99, 150), (-1, 0)), line((200, 225), (0, 1)), line((300, 150), (1, 0))]
}

/// Assert the R ramp of every edge against `want`, within 2.5 levels (the issue measures these
/// profiles to 2 levels; the widest gap today is 2.1, from the Range LUT's 4096-step table).
fn assert_edges(m: &Map, want: &[f32], what: &str) {
    let [top, left, bottom, right] = ramps(m);
    for (side, g) in ["top", "left", "bottom", "right"].iter().zip([top.as_slice(), left.as_slice(), bottom.as_slice(), right.as_slice()]) {
        for (d, w) in want.iter().enumerate() {
            assert!((g[d] - *w).abs() <= 2.5, "{what}, {side} edge, {d} px out: {} vs measured {w}", g[d]);
        }
    }
}

/// Done when: Size 1 puts R 128 (within 2 levels) on the first pixel outside every edge and leaves
/// the second one at the backdrop's own 255. Today `glow_map` blurs the layer alpha with a width-1
/// box, which is the identity, so the glow stays the hard alpha and never reaches past the edge:
/// every pixel outside stays 255.
#[test]
fn size_one_draws_the_measured_ring() {
    let m = map(1.0, 0.0);
    assert_edges(&m, &[128.0, 255.0], "Size 1");
    // The pixels the issue names: (200, 74), (200, 225), (99, 150), (300, 150), and the ones after.
    for (x, y) in [(200i64, 74), (200, 225), (99, 150), (300, 150)] {
        assert!((r(m.get(x, y)) - 128.0).abs() <= 2.0, "({x},{y}): {} vs measured 128", r(m.get(x, y)));
    }
    for (x, y) in [(200i64, 73), (200, 226), (98, 150), (301, 150)] {
        assert!((r(m.get(x, y)) - 255.0).abs() <= 2.0, "({x},{y}): {} vs measured 255", r(m.get(x, y)));
    }
    // Nothing lands on the backdrop beyond the glow's one pixel.
    assert_eq!(m.get(200, 70), 0.0, "the ring is one pixel wide");
}

/// Done when: Size 0, 3 and 10 keep the measured profiles (they already match today). The widest
/// gap here is 2.1 levels at Size 10, two pixels out — the Range LUT is a 4096-step table — so the
/// tolerance is 2.5 rather than the issue's 2.
#[test]
fn sizes_zero_three_and_ten_keep_the_measured_profiles() {
    let table: [(f32, &[f32]); 3] =
        [(0.0, &[255.0]), (3.0, &[86.0, 199.0, 255.0]), (10.0, &[26.0, 72.0, 113.0, 148.0, 179.0, 204.0, 225.0, 240.0, 250.0, 255.0])];
    for (size, want) in table {
        assert_edges(&map(size, 0.0), want, &format!("Size {size}"));
    }
}

/// Every other case is bit-for-bit what it was: these are the coverage ramps and whole-map hashes
/// `glow_map` gave on upstream/main c162cb6d6 before the fix, copied out of the probe run. They pin
/// that the Size 1 kernel is taken only by the Softer Outer Glow of Size 1 with Spread 0 — Sizes 0,
/// 0.5, 1.5, 2, 3, 10, 20 and 250, the Precise technique at Size 1, a Size 1 glow with Spread 50 %
/// and the Inner Glow of Size 1 (#14924 keeps its own ticket) all keep today's map.
#[test]
fn other_cases_are_unchanged() {
    let pin = |m: &Map, want: &[f64]| {
        let got: Vec<f32> = (0..12).map(|k| m.get(200, 74 - k)).collect();
        for (d, (g, w)) in got.iter().zip(want).enumerate() {
            assert!((f64::from(*g) - *w).abs() <= 1e-6, "{d} px out: {g:.8} vs {w:.8} on main");
        }
    };
    let sizes: [(f32, &[f64]); 8] = [
        (0.0, &[0.0; 12]),
        (0.5, &[0.0; 12]),
        (1.5, &[0.5, 0.05555556, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        (2.0, &[0.625, 0.125, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        (3.0, &[0.66666669, 0.22222222, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        (10.0, &[0.90499997, 0.72500002, 0.56500006, 0.42500001, 0.30500001, 0.20500000, 0.12500000, 0.06500000, 0.02500000, 0.00500000, 0.0, 0.0]),
        (
            20.0,
            &[0.95125002, 0.85624999, 0.76625001, 0.68125004, 0.60124999, 0.52625000, 0.45625001, 0.39125001, 0.33125001, 0.27625000, 0.22624999, 0.18125001],
        ),
        (
            250.0,
            &[0.53605729, 0.53298539, 0.52991337, 0.52684140, 0.52376950, 0.52069747, 0.51762557, 0.51455355, 0.51148164, 0.50840968, 0.50533766, 0.50226575],
        ),
    ];
    let hashes: [(f32, u64); 8] = [
        (0.0, 0xa067_0cbe_8a34_6225),
        (0.5, 0xa067_0cbe_8a34_6225),
        (1.5, 0x0a7c_e787_7ddd_4f09),
        (2.0, 0xc982_9687_8f28_6225),
        (3.0, 0x21e7_8d7a_c19d_e549),
        (10.0, 0x340b_6ce5_05c9_a71d),
        (20.0, 0x739f_4889_d735_7df9),
        (250.0, 0x51ed_b979_e767_1c79),
    ];
    for (size, want) in sizes {
        pin(&map(size, 0.0), want);
    }
    for (size, want) in hashes {
        let got = bit_hash(&map(size, 0.0));
        assert_eq!(got, want, "the whole Size {size} outer glow map moved");
    }
    // The Precise technique at Size 1 is not measured: today it still ends at the layer edge.
    let precise = glow_map(&document_shape(), &Glow { technique: GlowTechnique::Precise, ..glow(1.0, 0.0) }, false, (0, 0));
    assert_eq!(precise.get(200, 74), 0.5, "Precise Size 1 keeps its measured 0.5 at the edge");
    assert_eq!(precise.get(200, 73), 0.0, "and nothing beyond it");
    assert_eq!(bit_hash(&precise), 0x6a8e_83ba_430a_c571, "the Precise Size 1 map moved");
    assert_eq!(
        bit_hash(&glow_map(&document_shape(), &Glow { technique: GlowTechnique::Precise, ..glow(10.0, 0.0) }, false, (0, 0))),
        0xeb92_52e0_0904_52f1,
        "the Precise Size 10 map moved"
    );
    // Size 1 with Spread 50 % is not measured either: today a hard 1 px ring.
    let spread = map(1.0, 0.5);
    assert_eq!(spread.get(200, 74), 1.0, "a spread Size 1 glow keeps its hard ring");
    assert_eq!(spread.get(200, 73), 0.0);
    assert_eq!(bit_hash(&spread), 0xac14_bc4f_e034_6225, "the spread Size 1 map moved");
    // The Inner Glow of Size 1 (#14924) is not this ticket: it still draws nothing.
    let inner = glow_map(&document_shape(), &glow(1.0, 0.0), true, (0, 0));
    assert_eq!(inner.get(200, 75), 0.0, "an inner glow of Size 1 still stays empty inside the edge");
    assert_eq!(bit_hash(&inner), 0xe94d_157b_1934_6225, "the Size 1 inner glow map moved");
    assert_eq!(bit_hash(&glow_map(&document_shape(), &glow(7.0, 0.0), true, (0, 0))), 0x8af6_374a_dbf8_ef49, "the Size 7 inner glow map moved");
}

/// A hash over every coverage value of a map: the map is bit-for-bit unchanged iff this is.
fn bit_hash(m: &Map) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in &m.v {
        h = (h ^ u64::from(v.to_bits())).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
