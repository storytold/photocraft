//! #14934 - BUG-LAYER-STYLE-INNER-SHADOW-ANTI-ALIASED-IGNORED.
//!
//! Measured on the issue's test document: 400 x 300 px, 72 ppi, RGB 8 bit, white Background, a
//! pixel layer with an orange (255, 128, 0) rectangle at x 100..=299, y 75..=224. Black Inner
//! Shadow, Normal 100 %, Use Global Light off, Size 10, Distance 0, Choke 0, Noise 0, Contour
//! Gaussian. The tables are the G channel of the flattened image read from a layer edge inwards
//! (128 = the layer's own orange, 0 = full shadow): top = column x 200 from y 75 down, left = row
//! y 150 from x 100, bottom = column x 200 from y 224 up, right = row y 150 from x 299.
//!
//! Today `shadow_map` never reads `Shadow::anti_alias`, so both settings give one map: every edge
//! reads 58 81 103 116 122 125 126 127 128, and the issue's "with and without Anti-aliased" line
//! is exactly that. The reference application's Anti-aliased profile sits on an inner shadow whose
//! source sits a third of a pixel towards the top-left: the top and left edges read
//! 65 89 107 118 124 126 127 127 128, the bottom and right 49 74 96 112 121 125 126 127 128.

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

/// A second built-in preset, to show the gate is "the measured contour", not "any curve".
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

/// The issue's Inner Shadow settings: black, Normal, 100 %, global light off, Choke `spread`.
fn shadow(size: f32, distance: f32, angle: f32, spread: f32, contour: Contour, anti_aliased: bool) -> Shadow {
    Shadow {
        common: photocraft_doc::FxCommon::new(BlendMode::Normal, 1.0),
        color: photocraft_color::Color::BLACK,
        angle,
        use_global_light: false,
        distance,
        spread,
        size,
        contour,
        anti_alias: anti_aliased,
        noise: 0.0,
        knocks_out: false,
    }
}

fn map(size: f32, distance: f32, angle: f32, spread: f32, contour: Contour, anti_aliased: bool) -> Map {
    shadow_map(&document_shape(), &shadow(size, distance, angle, spread, contour, anti_aliased), &GlobalLight::default(), true, (0, 0))
}

/// The G channel of the orange layer under a black Normal inner shadow of coverage `c`.
fn g(c: f32) -> f32 {
    128.0 * (1.0 - c)
}

/// The nine G values inwards from each of the issue's four edges, in the order it reads them.
fn ramps(m: &Map) -> [Vec<f32>; 4] {
    let line = |first: (i64, i64), step: (i64, i64)| (0..9).map(|k| g(m.get(first.0 + k * step.0, first.1 + k * step.1))).collect();
    [line((200, 75), (0, 1)), line((100, 150), (1, 0)), line((200, 224), (0, -1)), line((299, 150), (-1, 0))]
}

/// Assert each edge's ramp against `want` within `tol` levels.
fn assert_edges(m: &Map, want: &[&[f32]; 4], tol: f32, what: &str) {
    for ((side, r), w) in ["top", "left", "bottom", "right"].iter().zip(ramps(m)).zip(want) {
        for (d, (got, want)) in r.iter().zip(*w).enumerate() {
            assert!((got - want).abs() <= tol, "{what}, {side} edge, {d} px in: {got:.1} vs measured {want} (within {tol})", got = got);
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
/// top/left and bottom/right profiles within 3 levels. The two pairs of edges differ because the
/// source sits off-centre; the issue leaves *why* they differ unmeasured, and this test pins the
/// measured profiles rather than a rule for the unmeasured ones.
#[test]
fn anti_aliased_gaussian_gives_both_measured_profiles() {
    let want: [&[f32]; 4] = [
        &[65.0, 89.0, 107.0, 118.0, 124.0, 126.0, 127.0, 127.0, 128.0],
        &[65.0, 89.0, 107.0, 118.0, 124.0, 126.0, 127.0, 127.0, 128.0],
        &[49.0, 74.0, 96.0, 112.0, 121.0, 125.0, 126.0, 127.0, 128.0],
        &[49.0, 74.0, 96.0, 112.0, 121.0, 125.0, 126.0, 127.0, 128.0],
    ];
    assert_edges(&map(10.0, 0.0, 0.0, 0.0, gaussian(), true), &want, 3.0, "Size 10 Gaussian, Anti-aliased");
}

/// Done when: without Anti-aliased the profiles stay as today. These are the G values `shadow_map`
/// gives on upstream/main for every Size 10 contour, and the issue's own "what happens today" line.
#[test]
fn without_anti_alias_the_profiles_stay_as_today() {
    let today: [f32; 9] = [58.0, 81.0, 103.0, 116.0, 122.0, 125.0, 126.0, 127.0, 128.0];
    let m = map(10.0, 0.0, 0.0, 0.0, gaussian(), false);
    for (side, r) in ["top", "left", "bottom", "right"].iter().zip(ramps(&m)) {
        for (d, got) in r.iter().enumerate() {
            assert_eq!(got.round() as i32, today[d] as i32, "Anti-aliased off moved the {side} edge, {d} px in: {got:.2}");
        }
    }
    // And the whole map is what it was: the issue's measured off-path, unchanged by the flag.
    assert_eq!(bit_hash(&m), 0x2cf1_991d_b2c1_a195, "the Size 10 Gaussian off-path map moved");
    // The centre of the layer keeps its own colour with the flag on as with it off (measured
    // 255.128.0 in both cases): the shadow map is empty there.
    for aa in [false, true] {
        assert_eq!(map(10.0, 0.0, 0.0, 0.0, gaussian(), aa).get(200, 150), 0.0, "the layer centre gained a shadow with Anti-aliased = {aa}");
    }
}

/// The sub-pixel source sits inside the layer: an inner shadow still paints nothing outside the
/// shape, whatever the flag. A third of a pixel must not pull coverage through the layer edge.
#[test]
fn anti_aliased_still_paints_nothing_outside_the_layer() {
    let m = map(10.0, 0.0, 0.0, 0.0, gaussian(), true);
    for (x, y) in [(200i64, 74), (200, 225), (99, 150), (300, 150), (99, 74), (300, 225)] {
        assert_eq!(m.get(x, y), 0.0, "inner shadow outside the layer at ({x},{y}) with Anti-aliased on");
    }
    // The measured reach inwards is unchanged: 9 px in every edge still reads the layer's own 128.
    for (side, first, step) in [("top", (200i64, 75), (0i64, 1)), ("left", (100, 150), (1, 0)), ("bottom", (200, 224), (0, -1)), ("right", (299, 150), (-1, 0))]
    {
        let deep = g(m.get(first.0 + 8 * step.0, first.1 + 8 * step.1));
        assert_eq!(deep.round() as i32, 128, "the {side} edge shadow reached further in with Anti-aliased on: {deep:.1} at 8 px");
    }
}

/// Not measured: Anti-aliased with a contour other than Gaussian. Those keep today's map
/// bit-for-bit - the Linear default, a second built-in curve, and the Drop Shadow (its own item is
/// #14881). Hashes are the ones these maps had on upstream/main before the change.
#[test]
fn other_contours_and_the_drop_shadow_keep_todays_map() {
    assert_eq!(bit_hash(&map(10.0, 0.0, 0.0, 0.0, Contour::Linear, true)), 0x2200_2b2d_61bc_9c2d, "a Linear-contour inner shadow with Anti-aliased moved");
    assert_eq!(bit_hash(&map(10.0, 0.0, 0.0, 0.0, cone(), true)), 0xb86c_68a7_8a2f_e689, "a Cone-contour inner shadow with Anti-aliased moved");
    let drop = |aa: bool| shadow_map(&document_shape(), &shadow(10.0, 0.0, 0.0, 0.0, gaussian(), aa), &GlobalLight::default(), false, (0, 0));
    assert_eq!(bit_hash(&drop(false)), 0x5e49_20b6_9751_cc45, "the Drop Shadow Gaussian map (Anti-aliased off) moved");
    assert_eq!(bit_hash(&drop(true)), 0x5e49_20b6_9751_cc45, "the Drop Shadow still ignores Anti-aliased (#14881)");
}

/// The flag off leaves every other Size, Distance and Choke exactly as it was. These are the whole
/// map hashes `shadow_map` gave on upstream/main c162cb6d6 before this change, copied out of the
/// probe run; they pin that the anti-aliased source is taken only when the flag is on.
#[test]
fn the_off_path_is_bit_for_bit_unchanged() {
    let hashes: [(f32, u64); 8] = [
        (0.5, 0xe94d_157b_1934_6225),
        (1.5, 0x87a1_3f9d_d043_dcb9),
        (2.0, 0xbe9c_6dd8_2c2a_fead),
        (3.0, 0x3208_8f2d_83f7_d11d),
        (6.0, 0xda06_d874_0203_1369),
        (10.0, 0x2cf1_991d_b2c1_a195),
        (20.0, 0xfdf3_096c_5583_d1c1),
        (250.0, 0x48ce_bfb8_fbe0_5379),
    ];
    for (size, want) in hashes {
        assert_eq!(bit_hash(&map(size, 0.0, 0.0, 0.0, gaussian(), false)), want, "the whole Size {size} inner shadow map (Anti-aliased off) moved");
    }
    assert_eq!(bit_hash(&map(10.0, 5.0, 90.0, 0.0, gaussian(), false)), 0xe8fb_ce94_b436_f35b, "the Size 10 Distance 5 inner shadow map moved");
    assert_eq!(bit_hash(&map(10.0, 0.0, 0.0, 0.5, gaussian(), false)), 0x013f_77f0_d6ef_b7c1, "the Size 10 Choke 50 % inner shadow map moved");
}

// ---------------------------------------------------------------------------------------------
// Independent review of this PR: the offset was a constant applied at every Size, Choke and
// Distance with the flag on, and only Size 10 / Choke 0 was measured. The four probes below are
// the review's break inputs, written before the code changed.

/// The review's first break: the issue's document with SIZE 1 (Angle 0, Distance 0, Choke 0,
/// Gaussian, Anti-aliased on). `spread_split(1, 0)` is `(0, 1)`, so `dilate` no-ops at a radius of
/// 0 and `blur(m, 1.0)` is the identity (`tent_kernel(1).0 == 0`, and `tent` returns at once): the
/// third-of-a-pixel source offset would become the whole map. Coverage 1/3 through the Gaussian
/// contour (0.30) would then land on the bottom and right inner edges only, G 90 where the layer
/// reads its own 128, and nothing at the top and left - a one-sided black line that no measurement
/// shows, because Size 1 was never measured with the flag on. So the offset stays off at Size 1.
/// (Written when today's Size 1 map was empty; #14933 merged in after it and gave Size 1 the
/// quarter-strength edge pixel the reference shows - with the flag off as with it on, so the
/// invariant this test pins, that Anti-aliased does not move the Size 1 map, stands.)
#[test]
fn anti_aliased_size_one_leaves_the_map_the_other_pr_fixed() {
    let off = map(1.0, 0.0, 0.0, 0.0, gaussian(), false);
    let on = map(1.0, 0.0, 0.0, 0.0, gaussian(), true);
    assert_eq!(bit_hash(&on), bit_hash(&off), "the Size 1 inner shadow map moved with Anti-aliased on");
    // The map #14933 fixed: a darkened first pixel inside every edge, the same on all four sides
    // (the one-sided line the offset would paint is dark on two and clean on the other two), and
    // the layer's own 128 one pixel deeper. This file's contour is the Gaussian curve, so the
    // first pixel reads darker than the 96 the Size 1 test's default contour gives.
    let mut firsts = [0.0f32; 4];
    for (k, (side, x, y, dx, dy)) in
        [("bottom", 200i64, 224, 0, -1), ("right", 299, 150, -1, 0), ("top", 200, 75, 0, 1), ("left", 100, 150, 1, 0)].into_iter().enumerate()
    {
        let got = g(on.get(x, y));
        firsts[k] = got;
        assert!(got < 126.0, "Size 1 left the {side} inner edge clean: G {got:.1}");
        let deeper = g(on.get(x + dx, y + dy));
        assert!((deeper - 128.0).abs() <= 2.0, "Size 1 beyond the first pixel at the {side} inner edge: G {deeper:.1}");
    }
    for pair in [(0usize, 1usize), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)] {
        assert!((firsts[pair.0] - firsts[pair.1]).abs() < 0.05, "the Size 1 inner shadow is not symmetric: {firsts:?}");
    }
}

/// The second degeneracy the review names: Size 2 with Choke 50 %. `spread_split(2, 0.5)` is
/// `(1, 1.0)`, a 1 px dilation plus a blur of width exactly 1 - the identity again - so the
/// offset would be the only soft thing in the map. That neighbour keeps today's output.
#[test]
fn anti_aliased_size_two_choke_half_keeps_todays_map() {
    let off = map(2.0, 0.0, 0.0, 0.5, gaussian(), false);
    let on = map(2.0, 0.0, 0.0, 0.5, gaussian(), true);
    assert_eq!(bit_hash(&on), bit_hash(&off), "the Size 2 Choke 50 % inner shadow map moved with Anti-aliased on (its blur width is 1, the identity)");
}

/// The parity finding: with Choke above 0 the CPU recomputes its chamfer dilation distance on the
/// offset map (`blur::dilate`), while the GPU dilates with a whole-pixel-shifted distance field of
/// the unshifted shape (`fx::builders::spread_blur`), so the two compositors part by exactly the
/// third of a pixel the offset adds. #14934 measured Choke 0, so every Choke that dilates keeps
/// today's map. The boundary is the map both compositors can build alike: no dilation
/// (`spread_split`'s radius 0) and a blur wider than the identity.
#[test]
fn anti_aliased_choke_keeps_todays_map() {
    let off = map(10.0, 0.0, 0.0, 0.5, gaussian(), false);
    assert_eq!(bit_hash(&off), 0x013f_77f0_d6ef_b7c1, "the Size 10 Choke 50 % inner shadow map (Anti-aliased off) moved");
    assert_eq!(
        bit_hash(&map(10.0, 0.0, 0.0, 0.5, gaussian(), true)),
        0x013f_77f0_d6ef_b7c1,
        "Anti-aliased moved the Size 10 Choke 50 % map, where CPU and GPU cannot be made equal"
    );
    // Choke 5 % dilates too (spread_split(10, 0.05) = (1, 9.5)) and stays on today's map.
    assert_eq!(
        bit_hash(&map(10.0, 0.0, 0.0, 0.05, gaussian(), true)),
        bit_hash(&map(10.0, 0.0, 0.0, 0.05, gaussian(), false)),
        "Anti-aliased moved the Size 10 Choke 5 % map"
    );
}

/// Pattern 13 the other way round: the rule may not be keyed on the one measured document's
/// Size 10 either. Every Size whose blur is wider than the identity takes the offset at Choke 0,
/// and so does a Distance other than 0 - the offset is a rule in the inputs. Guard test: green
/// before and after, it states where the boundary must NOT sit.
#[test]
fn the_offset_follows_the_inputs_not_the_measured_size() {
    for size in [1.5, 2.0, 3.0, 6.0, 20.0, 250.0] {
        let off = bit_hash(&map(size, 0.0, 0.0, 0.0, gaussian(), false));
        assert_ne!(
            bit_hash(&map(size, 0.0, 0.0, 0.0, gaussian(), true)),
            off,
            "Anti-aliased did nothing at Size {size}, Choke 0: the rule would be keyed on the measured Size 10"
        );
    }
    let dist_off = bit_hash(&map(10.0, 5.0, 90.0, 0.0, gaussian(), false));
    assert_ne!(bit_hash(&map(10.0, 5.0, 90.0, 0.0, gaussian(), true)), dist_off, "Anti-aliased did nothing at Size 10, Distance 5, Choke 0");
}
