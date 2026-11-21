//! #14855: a split Blend If slider fades with both ends inclusive in 8-bit documents, measured in
//! the reference application on the test document of [`band_doc`].

use crate::render;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{BlendRange, Document, Layer};
use photocraft_geom::{Rect, Size};

/// The issue's test document at `depth`: 400x300 px, a Background of 16 vertical grey bands of
/// 25 px (band `i` has the value 17·i), a blue Base rectangle from (0,150) to (400,300) and the
/// orange (255,128,0) Top rectangle from (50,50) to (350,250), which carries `this` (This Layer)
/// and `under` (Underlying Layer) as its Gray Blend If entry.
fn band_doc(depth: SampleType, this: BlendRange, under: BlendRange) -> Document {
    let mut d = Document::new("t", Size::new(400, 300), ColorMode::Rgb, depth);
    let fmt = d.pixel_format();
    let mut bg = Layer::raster("Background", fmt);
    for i in 0..16 {
        let v = (17 * i) as f32 / 255.0;
        bg.surface_mut().unwrap().fill_rect(Rect::from_xywh(25 * i, 0, 25, 300), &[v, v, v, 1.0]);
    }
    let mut base = Layer::raster("Base", fmt);
    base.surface_mut().unwrap().fill_rect(Rect::new(0, 150, 400, 300), &[0.0, 0.0, 1.0, 1.0]);
    let mut top = Layer::raster("Top", fmt);
    top.surface_mut().unwrap().fill_rect(Rect::new(50, 50, 350, 250), &[1.0, 128.0 / 255.0, 0.0, 1.0]);
    top.blend_if.set(0, [this, under]);
    d.layers.extend([bg, base, top]);
    d
}

/// The flattened pixel at (`x`, `y`) on the 0..=255 scale, rounded.
fn rgb8(d: &Document, x: i32, y: i32) -> [i32; 3] {
    let p = render(d, Rect::from_xywh(x, y, 1, 1)).px[0];
    [0, 1, 2].map(|c| (p[c] * 255.0).round() as i32)
}

/// Asserts every sample `(x, want)` on row y = 100 of `d` within ±`tol` levels.
fn assert_row(d: &Document, what: &str, samples: &[(i32, [i32; 3])], tol: i32) {
    for &(x, want) in samples {
        let got = rgb8(d, x, 100);
        assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= tol), "{what} at ({x},100): {got:?}, want {want:?} ±{tol}");
    }
}

/// A Blend If range with the black point `black` and the white point `white`, each (low, high).
fn range(black: [u8; 2], white: [u8; 2]) -> BlendRange {
    BlendRange { black, white }
}

#[test]
fn eight_bit_weight_counts_both_ends_of_a_split_point() {
    // Black side (v − low + 1) / (high − low + 1): 0 only below the low half.
    let black = range([68, 136], [255, 255]);
    assert_eq!(black.weight_8bit(67.0), 0.0);
    assert_eq!(black.weight_8bit(68.0), 1.0 / 69.0);
    assert_eq!(black.weight_8bit(85.0), 18.0 / 69.0);
    assert_eq!(black.weight_8bit(136.0), 1.0);
    // White side (high − v + 1) / (high − low + 1): 0 only above the high half.
    let white = range([0, 0], [140, 160]);
    assert_eq!(white.weight_8bit(140.0), 1.0);
    assert_eq!(white.weight_8bit(152.0), 9.0 / 21.0);
    assert_eq!(white.weight_8bit(160.0), 1.0 / 21.0);
    assert_eq!(white.weight_8bit(161.0), 0.0);
    // Deep documents keep (v − low) / (high − low), and unsplit points are hard either way.
    assert_eq!(white.weight(152.0), 0.4);
    assert_eq!(black.weight(68.0), 0.0);
    let hard = range([152, 152], [152, 152]);
    assert_eq!([hard.weight_8bit(151.0), hard.weight_8bit(152.0), hard.weight_8bit(153.0)], [0.0, 1.0, 0.0]);
}

#[test]
fn split_this_layer_white_point_fades_with_both_ends_inclusive() {
    // Top's Gray is 152: alpha (160 − 152 + 1) / (160 − 140 + 1) = 9/21, not (160 − 152) / 20.
    let d = band_doc(SampleType::U8, range([0, 0], [140, 160]), BlendRange::FULL);
    assert_row(&d, "Current Layer Gray white 140/160", &[(62, [128, 74, 19]), (337, [236, 181, 127])], 1);
}

#[test]
fn split_underlying_black_point_fades_from_its_low_level() {
    // Over 68 (the low half itself) the layer already shows at 1/69; over 85 at 18/69.
    let d = band_doc(SampleType::U8, BlendRange::FULL, range([68, 136], [255, 255]));
    assert_row(&d, "Underlying Gray black 68/136", &[(112, [70, 69, 67]), (137, [129, 96, 63])], 1);
}

#[test]
fn split_underlying_white_point_fades_to_its_high_level() {
    // Over 153 the layer shows at 52/69; over 204 (the high half itself) at 1/69.
    let d = band_doc(SampleType::U8, BlendRange::FULL, range([0, 0], [136, 204]));
    assert_row(&d, "Underlying Gray white 136/204", &[(237, [230, 134, 38]), (312, [205, 203, 202])], 1);
}

#[test]
fn unsplit_points_stay_hard_at_the_layers_own_level() {
    let shown = [255, 128, 0];
    let hidden = [34, 34, 34];
    for (black, white, want) in [(152, 255, shown), (153, 255, hidden), (0, 152, shown), (0, 150, hidden), (0, 153, shown)] {
        let d = band_doc(SampleType::U8, range([black, black], [white, white]), BlendRange::FULL);
        assert_row(&d, &format!("This Layer Gray {black}/{white}"), &[(62, want)], 0);
    }
}

#[test]
fn split_points_in_deep_documents_keep_their_ramp() {
    // Not measured at 16 or 32 bits: the ramp stays (v − low) / (high − low), so Top shows as at
    // 40 % Opacity where 8 bits show 9/21, and not at all over the low half, where 8 bits show 1/69.
    for depth in [SampleType::U16, SampleType::F32] {
        for (this, under, x, alpha) in [
            (range([0, 0], [140, 160]), BlendRange::FULL, 62, 0.4),
            (range([0, 0], [140, 160]), BlendRange::FULL, 337, 0.4),
            (BlendRange::FULL, range([68, 136], [255, 255]), 112, 0.0),
        ] {
            let d = band_doc(depth, this, under);
            let mut faded = band_doc(depth, BlendRange::FULL, BlendRange::FULL);
            faded.layers[2].opacity = alpha;
            let r = Rect::from_xywh(x, 100, 1, 1);
            let (got, want) = (render(&d, r).px[0], render(&faded, r).px[0]);
            assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-4), "{depth:?} {this:?}/{under:?} at ({x},100): {got:?}, want {want:?}");
        }
    }
}

#[test]
fn split_points_in_grayscale_documents_keep_their_ramp() {
    // Only RGB was measured: a Grayscale value of 128 under a split black point at 128/129 stays
    // hidden, where an RGB document would show it at 1/2.
    let mut d = Document::with_background("g", Size::new(1, 1), ColorMode::Grayscale, SampleType::U8, photocraft_color::Color::WHITE);
    let mut l = Layer::raster("k", d.pixel_format());
    l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 1, 1), &[128.0 / 255.0, 1.0]);
    l.blend_if.set(1, [range([128, 129], [255, 255]), BlendRange::FULL]);
    d.layers.push(l);
    let p = render(&d, Rect::from_xywh(0, 0, 1, 1)).px[0];
    assert!(p[..3].iter().all(|&c| c > 0.999), "{p:?}");
}
