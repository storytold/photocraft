use super::{Contour, Map, satin_map};
use photocraft_color::{BlendMode, Color};
use photocraft_doc::adjust::CurvePoint;
use photocraft_doc::{FxCommon, Satin};

/// The built-in Gaussian contour (input, output, smooth).
const GAUSSIAN: [(f32, f32, bool); 11] = [
    (0.0, 0.0, false),
    (0.117, 0.023, true),
    (0.172, 0.055, true),
    (0.234, 0.117, true),
    (0.305, 0.234, true),
    (0.383, 0.414, true),
    (0.523, 0.688, true),
    (0.617, 0.812, true),
    (0.703, 0.891, true),
    (0.805, 0.953, true),
    (1.0, 1.0, false),
];

fn gaussian() -> Contour {
    Contour::Custom {
        name: "Gaussian".into(),
        points: GAUSSIAN.iter().map(|&(input, output, _)| CurvePoint { input, output }).collect(),
        corners: GAUSSIAN.iter().map(|p| p.2).collect(),
    }
}

/// #14981: black Satin (Angle 90, Distance 20, Size 10) over an orange rectangle, G channel down
/// its centre column x 200 from y 75 (128 = no satin, 0 = full black).
fn green_column(contour: Contour, invert: bool) -> Vec<u8> {
    let mut shape = Map::new(400, 300, 0.0);
    for row in shape.v.as_chunks_mut::<400>().0.iter_mut().skip(75).take(150) {
        row[100..300].fill(1.0);
    }
    let satin = Satin {
        common: FxCommon::new(BlendMode::Normal, 1.0),
        color: Color::BLACK,
        angle: 90.0,
        distance: 20.0,
        size: 10.0,
        contour,
        anti_alias: false,
        invert,
    };
    let coverage = satin_map(&shape, &satin);
    (75..225).map(|y| (128.0 * (1.0 - coverage.get(200, y))).round() as u8).collect()
}

/// `edge` from y 75 inwards, the middle value up to the mirrored edge.
fn mirrored(edge: &[u8], middle: u8) -> Vec<u8> {
    let mut column = edge.to_vec();
    column.resize(150 - edge.len(), middle);
    column.extend(edge.iter().rev());
    column
}

fn assert_close(actual: &[u8], expected: &[u8], tolerance: u8, case: &str) {
    for (i, (a, e)) in actual.iter().zip(expected).enumerate() {
        assert!(a.abs_diff(*e) <= tolerance, "{case}, y {}: expected G {e}, got {a}", i + 75);
    }
}

#[test]
fn inverted_gaussian_satin_is_the_opposite_of_the_plain_one() {
    let mut edge = vec![128; 13];
    edge.extend([127, 126, 123, 116, 103, 82, 58, 35, 21, 13, 8, 5, 3, 2, 1, 1]);
    let inverted = green_column(gaussian(), true);
    assert_close(&inverted, &mirrored(&edge, 0), 3, "Gaussian, Invert on");
    let plain = green_column(gaussian(), false);
    let opposite: Vec<u8> = plain.iter().map(|g| 128 - g).collect();
    assert_close(&inverted, &opposite, 1, "Gaussian, Invert on vs 128 - Invert off");
}

#[test]
fn inverted_linear_satin_keeps_its_profile() {
    let mut edge = vec![128; 11];
    edge.extend([127, 124, 120, 115, 109, 101, 92, 82, 71, 58, 46, 36, 27, 20, 13, 8, 4, 2]);
    assert_close(&green_column(Contour::Linear, true), &mirrored(&edge, 0), 2, "Linear, Invert on");
}
