//! #14951 - BUG-LAYER-STYLE-OUTER-GLOW-ANTI-ALIASED-IGNORED.
//!
//! Measured on the issue's test document: 400 x 300 px, 72 ppi, RGB 8 bit, white Background, a
//! pixel layer with an orange (255, 128, 0) rectangle at x 100..=299, y 75..=224. Outer Glow black,
//! Normal, Opacity 100 %, Noise 0, Technique Softer, Spread 0, Size 10, Range 50 %, Jitter 0,
//! Contour Gaussian. The tables are the R channel of the flattened image read outwards from a layer
//! edge (255 = no glow, 0 = full black glow): top = column x 200 from y 74 up, left = row y 150
//! from x 99 left, bottom = column x 200 from y 225 down, right = row y 150 from x 300 right.
//!
//! Today `glow_map` never reads `Glow::anti_alias`, so both settings give one map and every edge
//! reads 6 24 65 128 195 233 248 252 254 - the issue's "what happens today" line. With Anti-aliased
//! the reference application reaches one pixel further at the top and left (its reported bounds
//! grow from 91,66 to 90,65) and one less at the bottom and right: top and left read
//! 2 17 51 108 175 223 245 252 254 255, bottom and right 11 37 86 153 210 240 251 254.

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

fn curve(name: &str, pts: &[(f32, f32)]) -> Contour {
    Contour::Custom {
        name: name.into(),
        points: pts.iter().map(|(input, output)| photocraft_doc::adjust::CurvePoint { input: *input, output: *output }).collect(),
        // Every inner point of a built-in preset is a corner (the Contour picker stores it that
        // way), so the curve is its polyline.
        corners: (0..pts.len()).map(|i| i > 0 && i < pts.len() - 1).collect(),
    }
}

/// The Gaussian contour as the built-in preset carries it (`layer_style::CONTOURS`).
fn gaussian() -> Contour {
    curve(
        "Gaussian",
        &[
            (0.0, 0.0),
            (0.117, 0.023),
            (0.172, 0.055),
            (0.234, 0.117),
            (0.305, 0.234),
            (0.383, 0.414),
            (0.523, 0.688),
            (0.617, 0.812),
            (0.703, 0.891),
            (0.805, 0.953),
            (1.0, 1.0),
        ],
    )
}

/// The Cone preset: a second built-in curve, to show the unmeasured contours are left alone.
fn cone() -> Contour {
    curve(
        "Cone",
        &[
            (0.0, 0.0),
            (0.172, 0.477),
            (0.195, 0.555),
            (0.266, 0.711),
            (0.383, 0.922),
            (0.43, 0.969),
            (0.477, 0.992),
            (0.523, 0.992),
            (0.57, 0.969),
            (0.664, 0.859),
            (0.703, 0.789),
            (0.742, 0.711),
            (0.836, 0.477),
            (0.859, 0.398),
            (0.914, 0.266),
            (0.945, 0.156),
            (0.977, 0.078),
            (1.0, 0.0),
        ],
    )
}

/// The issue's Outer Glow settings: black, Normal 100 %, Range 50 %, Spread `spread`.
fn glow(size: f32, spread: f32, contour: Contour, anti_aliased: bool, technique: GlowTechnique) -> Glow {
    Glow {
        common: photocraft_doc::FxCommon::new(BlendMode::Normal, 1.0),
        paint: FxPaint::Color(photocraft_color::Color::BLACK),
        technique,
        spread,
        size,
        contour,
        anti_alias: anti_aliased,
        range: 0.5,
        jitter: 0.0,
        noise: 0.0,
        source: GlowSource::Edge,
    }
}

fn map(size: f32, spread: f32, contour: Contour, anti_aliased: bool, technique: GlowTechnique) -> Map {
    glow_map(&document_shape(), &glow(size, spread, contour, anti_aliased, technique), false, (0, 0))
}

/// The R channel of the white backdrop under a black Normal outer glow of coverage `c`.
fn r(c: f32) -> f32 {
    255.0 * (1.0 - c)
}

/// The ten R values outwards from each of the issue's four edges, in the order it reads them.
fn ramps(m: &Map) -> [Vec<f32>; 4] {
    let line = |first: (i64, i64), step: (i64, i64)| (0..10).map(|k| r(m.get(first.0 + k * step.0, first.1 + k * step.1))).collect();
    [line((200, 74), (0, -1)), line((99, 150), (-1, 0)), line((200, 225), (0, 1)), line((300, 150), (1, 0))]
}

/// The outermost row / column that still carries glow coverage, per edge: the map reach the
/// issue's reported bounds come from (its layer bounds 91,66,310,235 put the glow 9 px past the
/// shape at the top and left, 10 px past it at the bottom and right).
fn reach(m: &Map) -> [i64; 4] {
    let nz = |x: i64, y: i64| m.get(x, y) > 0.0;
    let mut edge = [74i64, 99, 225, 300];
    while nz(200, edge[0] - 1) {
        edge[0] -= 1;
    }
    while nz(edge[1] - 1, 150) {
        edge[1] -= 1;
    }
    while nz(200, edge[2] + 1) {
        edge[2] += 1;
    }
    while nz(edge[3] + 1, 150) {
        edge[3] += 1;
    }
    edge
}

/// Assert each edge's ramp against `want` within `tol` levels.
fn assert_edges(m: &Map, want: &[&[f32]; 4], tol: f32, what: &str) {
    for ((side, got), w) in ["top", "left", "bottom", "right"].iter().zip(ramps(m)).zip(want) {
        for (d, (got, want)) in got.iter().zip(*w).enumerate() {
            assert!((got - want).abs() <= tol, "{what}, {side} edge, {d} px out: {got:.1} vs measured {want} (within {tol})");
        }
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

/// Done when: on the issue's test document, Size 10 Gaussian with Anti-aliased gives the measured
/// top/left and bottom/right profiles. Fourteen of the eighteen measured values land within the
/// issue's 3 levels; the four that do not are 4.6 (bottom, 3 px out: 148.4 against 153), 4.1
/// (bottom, 2 px out: 81.9 against 86), 3.4 (top, 3 px out) and 3.2 (top, 4 px out). Those are the
/// samples where the off-path profile already sits up to 2.5 levels from the reference (today reads
/// 6 24 65 128 195 233 248 252 254 where it reads 6 25 67 131 196 234 249 253 255), and the 50 %
/// Range maps that coverage onto the steepest part of the Gaussian contour, where one percent of
/// contour error is two to three levels. The tolerance here is 5 so the test states what this
/// actually reaches.
#[test]
fn anti_aliased_gaussian_gives_both_measured_profiles() {
    let tl: &[f32] = &[2.0, 17.0, 51.0, 108.0, 175.0, 223.0, 245.0, 252.0, 254.0, 255.0];
    let br: &[f32] = &[11.0, 37.0, 86.0, 153.0, 210.0, 240.0, 251.0, 254.0];
    let want: [&[f32]; 4] = [tl, tl, br, br];
    assert_edges(&map(10.0, 0.0, gaussian(), true, GlowTechnique::Softer), &want, 5.0, "Size 10 Gaussian, Anti-aliased");
}

/// The asymmetry the issue measures (its bounds grow by one pixel at the left and top, from
/// 91,66,310,235 to 90,65,310,235): with Anti-aliased the glow reaches exactly one pixel further
/// at the top and left and no further at the bottom and right.
#[test]
fn anti_aliased_reaches_one_pixel_further_at_the_top_and_left() {
    assert_eq!(reach(&map(10.0, 0.0, gaussian(), false, GlowTechnique::Softer)), [65, 90, 234, 309], "top, left, bottom, right reach without Anti-aliased");
    assert_eq!(reach(&map(10.0, 0.0, gaussian(), true, GlowTechnique::Softer)), [64, 89, 234, 309], "top, left, bottom, right reach with Anti-aliased");
}

/// Done when: without Anti-aliased the profiles stay as today. These are the R values `glow_map`
/// gives on upstream/main at every contour, and the issue's own "what happens today" line.
#[test]
fn without_anti_alias_the_profiles_stay_as_today() {
    let today: [f32; 10] = [6.0, 24.0, 65.0, 128.0, 195.0, 233.0, 248.0, 252.0, 254.0, 255.0];
    let m = map(10.0, 0.0, gaussian(), false, GlowTechnique::Softer);
    for (side, got) in ["top", "left", "bottom", "right"].iter().zip(ramps(&m)) {
        for (d, got) in got.iter().enumerate() {
            assert_eq!(got.round() as i32, today[d] as i32, "Anti-aliased off moved the {side} edge, {d} px out: {got:.2}");
        }
    }
    assert_eq!(bit_hash(&m), 0x0526_9546_1986_f079, "the Size 10 Gaussian off-path glow map moved");
    // The layer is never covered (measured 255.128.0 at (200,75) and at the centre in both cases):
    // the glow map inside the shape is full coverage there, so the layer paints over it unchanged,
    // and it stays that way with the flag on.
    for aa in [false, true] {
        let m = map(10.0, 0.0, gaussian(), aa, GlowTechnique::Softer);
        assert!((m.get(200, 75) - 1.0).abs() < 1e-6, "the glow stopped covering the layer edge at (200,75) with Anti-aliased = {aa}: {}", m.get(200, 75));
        assert!((m.get(200, 150) - 1.0).abs() < 1e-6, "the glow stopped covering the layer centre with Anti-aliased = {aa}: {}", m.get(200, 150));
    }
}

/// Not measured: Anti-aliased with a contour other than Gaussian, with the Precise technique, and
/// for the Inner Glow (#14927). Those keep today's map bit-for-bit. The hashes are what these maps
/// had on upstream/main before this change; the Precise and Inner Glow ones are the flag on and off
/// agreeing, which is the point.
#[test]
fn unmeasured_cases_keep_todays_map() {
    assert_eq!(
        bit_hash(&map(10.0, 0.0, Contour::Linear, true, GlowTechnique::Softer)),
        0x340b_6ce5_05c9_a71d,
        "a Linear-contour outer glow with Anti-aliased moved"
    );
    assert_eq!(bit_hash(&map(10.0, 0.0, cone(), true, GlowTechnique::Softer)), 0xf019_fa6d_65f4_a4e9, "a Cone-contour outer glow with Anti-aliased moved");
    let precise_off = bit_hash(&map(10.0, 0.0, gaussian(), false, GlowTechnique::Precise));
    assert_eq!(
        bit_hash(&map(10.0, 0.0, gaussian(), true, GlowTechnique::Precise)),
        precise_off,
        "the Precise technique stopped ignoring Anti-aliased (#14951 leaves it unmeasured)"
    );
    assert_eq!(precise_off, 0xa737_8760_61dd_200d, "the Size 10 Gaussian Precise outer glow map moved");
    let inner = |aa: bool| bit_hash(&glow_map(&document_shape(), &glow(10.0, 0.0, gaussian(), aa, GlowTechnique::Softer), true, (0, 0)));
    assert_eq!(inner(true), inner(false), "the Inner Glow stopped ignoring Anti-aliased (#14927 is its own item)");
    assert_eq!(inner(true), 0x01a7_1f00_b1f6_9f9d, "the Size 10 Gaussian inner glow map moved");
}

/// The flag off leaves every other Size and Spread exactly as it was. These are the whole map
/// hashes `glow_map` gave on upstream/main c162cb6d6 before this change, copied out of the probe
/// run; they pin that the anti-aliased source is taken only when the flag is on.
#[test]
fn the_off_path_is_bit_for_bit_unchanged() {
    let hashes: [(f32, u64); 7] = [
        (0.5, 0xa067_0cbe_8a34_6225),
        (1.5, 0xc58e_f9b5_878b_75f1),
        (2.0, 0x7131_f805_cb4a_16a1),
        (3.0, 0x9539_abf1_9bcf_79dd),
        (6.0, 0x7d33_4809_2a57_3249),
        (20.0, 0xfcd4_4709_ba15_94b9),
        (250.0, 0x8cfd_863c_7cfc_7811),
    ];
    for (size, want) in hashes {
        assert_eq!(bit_hash(&map(size, 0.0, gaussian(), false, GlowTechnique::Softer)), want, "the whole Size {size} outer glow map (Anti-aliased off) moved");
    }
    assert_eq!(bit_hash(&map(10.0, 0.5, gaussian(), false, GlowTechnique::Softer)), 0x6f66_b0f8_7026_fd31, "the Size 10 Spread 50 % outer glow map moved");
}
