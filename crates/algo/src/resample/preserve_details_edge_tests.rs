//! Preserve Details enlarges hard edges without a ringing halo (#14794).

use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};

const CANVAS: Rect = Rect { x0: 0, y0: 0, x1: 400, y1: 300 };

/// The issue's flattened document: white, a red square at (0,0), a blue one at (300,200) and a
/// green one at (175,125), each opaque.
fn document(sample: SampleType, alpha: bool) -> Surface {
    let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, alpha));
    let px = |r: f32, g: f32, b: f32| if alpha { vec![r, g, b, 1.0] } else { vec![r, g, b] };
    s.fill_rect(CANVAS, &px(1.0, 1.0, 1.0));
    s.fill_rect(Rect::new(0, 0, 100, 100), &px(1.0, 0.0, 0.0));
    s.fill_rect(Rect::new(300, 200, 400, 300), &px(0.0, 0.0, 1.0));
    s.fill_rect(Rect::new(175, 125, 225, 175), &px(0.0, 1.0, 0.0));
    s
}

/// The green channel at y, x in `xs`, in 8-bit levels.
fn green(s: &Surface, y: i32, xs: std::ops::Range<i32>) -> Vec<i32> {
    xs.map(|x| (s.pixel(x, y)[1] * 255.0).round() as i32).collect()
}

fn within(actual: &[i32], expected: &[i32], tol: i32) -> bool {
    actual.len() == expected.len() && actual.iter().zip(expected).all(|(a, e)| (a - e).abs() <= tol)
}

/// The reference application enlarges the red/white edge 2x to `0,0,0,8,247,255,255,255,255`
/// (green channel, y=20, x 196..204) with Preserve Details: no overshoot on either side, a
/// steep edge. Measured at 8 bits; the same edge holds at 16 and 32 bits.
#[test]
fn enlarging_a_hard_edge_does_not_ring() {
    let reference = [0, 0, 0, 8, 247, 255, 255, 255, 255];
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for alpha in [false, true] {
            let o = resize_surface_in_canvas(&document(sample, alpha), 2.0, 2.0, Resample::PreserveDetails, CANVAS);
            let row = green(&o, 20, 196..205);
            assert!(within(&row, &reference, 2), "{sample:?} alpha={alpha}: {row:?}, reference {reference:?}");
        }
    }
}

/// Reducing with Preserve Details stays Bicubic Sharper: `0,0,0,13,242,255,255,255` at 200x150.
#[test]
fn reducing_stays_bicubic_sharper() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let s = document(sample, false);
        let pd = resize_surface_in_canvas(&s, 0.5, 0.5, Resample::PreserveDetails, CANVAS);
        let sharper = resize_surface_in_canvas(&s, 0.5, 0.5, Resample::BicubicSharper, CANVAS);
        let r = Rect::new(0, 0, 200, 150);
        assert_eq!(pd.read_region(r), sharper.read_region(r), "{sample:?}");
        if sample == SampleType::U8 {
            assert_eq!(green(&pd, 10, 46..54), [0, 0, 0, 13, 242, 255, 255, 255]);
        }
    }
}

/// The other methods enlarge the edge as before (#14794 changes Preserve Details only).
#[test]
fn other_methods_keep_their_edge() {
    let s = document(SampleType::U8, false);
    let cases: [(Resample, [i32; 9]); 7] = [
        (Resample::Nearest, ROWS[0]),
        (Resample::Bilinear, ROWS[1]),
        (Resample::Bicubic, ROWS[2]),
        (Resample::BicubicSmoother, ROWS[3]),
        (Resample::BicubicSharper, ROWS[4]),
        (Resample::BicubicAutomatic, ROWS[5]),
        (Resample::Lanczos, ROWS[6]),
    ];
    for (f, expected) in cases {
        let o = resize_surface_in_canvas(&s, 2.0, 2.0, f, CANVAS);
        assert_eq!(green(&o, 20, 196..205), expected, "{f:?}");
    }
}

/// The rows main 406ac677e1 gives, for [`other_methods_keep_their_edge`].
const ROWS: [[i32; 9]; 7] = [
    [0, 0, 0, 0, 255, 255, 255, 255, 255],
    [0, 0, 0, 64, 191, 255, 255, 255, 255],
    [0, 0, 0, 58, 197, 255, 255, 255, 255],
    [0, 0, 0, 66, 189, 255, 255, 255, 255],
    [0, 0, 0, 66, 189, 255, 255, 255, 255],
    [0, 0, 0, 66, 189, 255, 255, 255, 255],
    [8, 0, 0, 54, 201, 255, 255, 247, 253],
];

/// At any scale and depth the enlargement stays within the source's levels: no halo darker than
/// the red or brighter than the white, and flat areas stay flat.
#[test]
fn enlargements_never_overshoot() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let s = document(sample, true);
        for k in [1.37, 2.0, 3.0] {
            let o = resize_surface_in_canvas(&s, k, k, Resample::PreserveDetails, CANVAS);
            let dst = scaled_rect(CANVAS, k, k);
            assert_eq!(o.content_bounds(), dst, "{sample:?} {k}");
            for v in o.read_region(dst) {
                assert!((0.0..=1.0).contains(&v), "{sample:?} {k}: {v}");
            }
            // Inside the red square, away from its edge.
            let p = o.pixel((50.0 * k) as i32, (50.0 * k) as i32);
            assert_eq!(p, vec![1.0, 0.0, 0.0, 1.0], "{sample:?} {k}");
        }
    }
}

/// A layer smaller than the canvas, on transparency: the clamp doesn't pull its colour towards
/// the transparent pixels, and the stepped resize gives the same pixels as the one-shot one.
#[test]
fn transparent_surroundings_and_steps_match() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
        s.fill_rect(Rect::new(10, 10, 30, 30), &[0.2, 0.6, 1.0, 1.0]);
        let o = resize_surface_in_canvas(&s, 2.5, 2.5, Resample::PreserveDetails, CANVAS);
        // On the layer's edge the colour stays the layer's.
        let edge = o.pixel(25, 50);
        assert!(edge[3] > 0.0 && edge[3] < 1.0, "{sample:?} {edge:?}");
        let off = edge.iter().take(3).zip([0.2, 0.6, 1.0]).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(off < 0.01, "{sample:?} {edge:?}");
        let mut steps = ResizeSteps::new(s.clone(), 2.5, 2.5, Resample::PreserveDetails, Some(CANVAS));
        let stepped = loop {
            if let Some(out) = steps.step(&mut || true) {
                break out;
            }
        };
        let r = o.content_bounds().union(&stepped.content_bounds());
        assert_eq!(stepped.read_region(r), o.read_region(r), "{sample:?}");
    }
}
