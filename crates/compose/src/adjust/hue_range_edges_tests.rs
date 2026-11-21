//! #14744: colour ranges through the hue tables match the reference application's measurements on
//! a hue ramp of HSL(5i°, 100 % or 50 %, 50 %) cells in an 8-bit document, within 2 levels.

use super::*;
use photocraft_geom::Rect;

fn reds(bounds: [f32; 4], hue: f32, saturation: f32, lightness: f32) -> Adjustment {
    let mut ranges = HueRange::defaults();
    ranges[0] = HueRange { hue, saturation, lightness, bounds };
    Adjustment::HueSaturation { hue: 0.0, saturation: 0.0, lightness: 0.0, colorize: false, ranges }
}

fn run(adj: &Adjustment, rgb: [u8; 3]) -> [u8; 3] {
    let c = rgb.map(|v| f32::from(v) / 255.0);
    let mut b = Buffer { rect: Rect::new(0, 0, 1, 1), px: vec![[c[0], c[1], c[2], 1.0]] };
    apply_depth(adj, &mut b, Transfer::Srgb, Some(SampleType::U8));
    std::array::from_fn(|i| (b.px[0][i] * 255.0).round() as u8)
}

fn assert_near(adj: &Adjustment, rgb: [u8; 3], want: [u8; 3]) {
    let got = run(adj, rgb);
    assert!(got.iter().zip(want).all(|(&g, w)| g.abs_diff(w) <= 2), "{rgb:?}: {got:?}, measured {want:?}");
}

const DEFAULT_REDS: [f32; 4] = [315.0, 345.0, 15.0, 45.0];

#[test]
fn default_reds_ramp_matches_the_reference() {
    // Input, then Saturation -100, Hue +60, Lightness +100, Lightness -100.
    #[rustfmt::skip]
    let table: [[[u8; 3]; 5]; 12] = [
        [[255, 85, 0], [148, 120, 106], [211, 255, 0], [255, 227, 213], [42, 14, 0]],
        [[255, 106, 0], [169, 120, 85], [232, 255, 0], [255, 206, 171], [84, 35, 0]],
        [[255, 127, 0], [191, 127, 64], [254, 255, 0], [255, 191, 128], [128, 64, 0]],
        [[255, 148, 0], [212, 141, 43], [255, 234, 0], [255, 184, 86], [169, 98, 0]],
        [[255, 169, 0], [233, 162, 22], [255, 213, 0], [255, 184, 44], [211, 140, 0]],
        [[255, 190, 0], [254, 190, 1], [255, 192, 0], [255, 191, 2], [253, 189, 0]],
        [[255, 0, 169], [237, 18, 163], [255, 0, 133], [255, 36, 181], [219, 0, 145]],
        [[255, 0, 148], [215, 39, 141], [255, 0, 71], [255, 78, 181], [177, 0, 103]],
        [[255, 0, 127], [194, 60, 127], [255, 0, 8], [255, 120, 187], [135, 0, 67]],
        [[255, 0, 106], [173, 82, 120], [255, 59, 0], [255, 165, 202], [90, 0, 37]],
        [[255, 0, 85], [151, 103, 119], [255, 122, 0], [255, 207, 223], [48, 0, 16]],
        [[255, 0, 64], [130, 124, 125], [255, 184, 0], [255, 249, 250], [6, 0, 1]],
    ];
    let edits = [(0.0, -100.0, 0.0), (60.0, 0.0, 0.0), (0.0, 0.0, 100.0), (0.0, 0.0, -100.0)];
    for row in table {
        for (k, (h, s, l)) in edits.into_iter().enumerate() {
            assert_near(&reds(DEFAULT_REDS, h, s, l), row[0], row[k + 1]);
        }
    }
}

#[test]
fn narrow_and_wide_ranges_match_the_reference() {
    let narrow = reds([350.0, 355.0, 5.0, 10.0], 0.0, -100.0, 0.0);
    assert_near(&narrow, [255, 0, 21], [146, 109, 112]);
    assert_near(&narrow, [255, 42, 0], [249, 46, 6]);
    assert_near(&narrow, [255, 0, 0], [128, 128, 128]);
    assert_near(&narrow, [255, 21, 0], [128, 128, 128]);
    assert_near(&narrow, [255, 64, 0], [255, 64, 0]);
    assert_near(&narrow, [255, 0, 42], [255, 0, 42]);
    assert_near(&narrow, [191, 64, 74], [133, 121, 122]);
    assert_near(&narrow, [191, 85, 64], [188, 87, 67]);
    let wide = reds([270.0, 330.0, 30.0, 90.0], 0.0, -100.0, 0.0);
    assert_near(&wide, [191, 191, 64], [159, 159, 96]);
    assert_near(&wide, [191, 64, 191], [160, 95, 160]);
}

#[test]
fn hard_edge_stays_sharp_through_the_hue_tables() {
    let hard = reds([345.0, 345.0, 15.0, 15.0], 0.0, -100.0, 0.0);
    for rgb in [[255, 0, 42], [255, 0, 0], [255, 42, 0], [255, 64, 0]] {
        assert_near(&hard, rgb, [128, 128, 128]);
    }
    assert_eq!(run(&hard, [255, 0, 64]), [255, 0, 64]);
    assert_eq!(run(&hard, [255, 85, 0]), [255, 85, 0]);
}

#[test]
fn overlapping_ranges_still_add() {
    let mut ranges = HueRange::defaults();
    ranges[0].saturation = -100.0;
    ranges[1].saturation = -100.0;
    let both = hue_range_tables(&ranges);
    let k = (LUT_SIZE - 1) / 12; // about hue 30°, between reds and yellows
    let one = |i: usize| ranges[i].weight(k as f32 * 360.0 / (LUT_SIZE - 1) as f32);
    assert!((both[1][k] + one(0) + one(1)).abs() < 1e-5, "{}", both[1][k]);
}
