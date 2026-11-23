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
/// steep edge. Measured at 8 bits only (16 and 32 bits keep the previous result, see
/// [`unmeasured_enlargements_keep_the_previous_sharpening`]).
#[test]
fn enlarging_a_hard_edge_does_not_ring() {
    let reference = [0, 0, 0, 8, 247, 255, 255, 255, 255];
    for alpha in [false, true] {
        let o = resize_surface_in_canvas(&document(SampleType::U8, alpha), 2.0, 2.0, Resample::PreserveDetails, CANVAS);
        let row = green(&o, 20, 196..205);
        assert!(within(&row, &reference, 2), "alpha={alpha}: {row:?}, reference {reference:?}");
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

/// The measured enlargement (8 bits, 2x) stays within the source's levels: no halo darker than
/// the red or brighter than the white, and flat areas stay flat.
#[test]
fn the_measured_enlargement_never_overshoots() {
    let s = document(SampleType::U8, true);
    let o = resize_surface_in_canvas(&s, 2.0, 2.0, Resample::PreserveDetails, CANVAS);
    let dst = scaled_rect(CANVAS, 2.0, 2.0);
    assert_eq!(o.content_bounds(), dst);
    for v in o.read_region(dst) {
        assert!((0.0..=1.0).contains(&v), "{v}");
    }
    // Inside the red square, away from its edge.
    assert_eq!(o.pixel(100, 100), vec![1.0, 0.0, 0.0, 1.0]);
}

/// A layer smaller than the canvas, on transparency: in the measured case the clamp doesn't pull
/// its colour towards the transparent pixels, and at any scale and depth the stepped resize gives
/// the same pixels as the one-shot one.
#[test]
fn transparent_surroundings_and_steps_match() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
        s.fill_rect(Rect::new(10, 10, 30, 30), &[0.2, 0.6, 1.0, 1.0]);
        for k in [2.0, 2.5] {
            let o = resize_surface_in_canvas(&s, k, k, Resample::PreserveDetails, CANVAS);
            if sample == SampleType::U8 && k == 2.0 {
                // On the layer's edge the colour stays the layer's.
                let edge = o.pixel(20, 40);
                assert!(edge[3] > 0.0 && edge[3] < 1.0, "{edge:?}");
                let off = edge.iter().take(3).zip([0.2, 0.6, 1.0]).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
                assert!(off < 0.01, "{edge:?}");
            }
            let mut steps = ResizeSteps::new(s.clone(), k, k, Resample::PreserveDetails, Some(CANVAS));
            let stepped = loop {
                if let Some(out) = steps.step(&mut || true) {
                    break out;
                }
            };
            let r = o.content_bounds().union(&stepped.content_bounds());
            assert_eq!(stepped.read_region(r), o.read_region(r), "{sample:?} {k}");
        }
    }
}

/// Preserve Details before #14794: Lanczos, then an Unsharp Mask of 30 % at radius 0.6 x scale.
fn lanczos_and_unsharp_mask(s: &Surface, sx: f64, sy: f64) -> Surface {
    let out = resize_surface(s, sx, sy, Resample::Lanczos);
    let dst = out.content_bounds();
    let p = crate::FilterParams::UnsharpMask { amount: 30.0, radius: 0.6 * sx.max(sy) as f32, threshold: 0.0 };
    let area = crate::output_area(&p, dst, dst, None);
    let mut out = crate::apply(&out, &p, area, dst, None);
    out.prune();
    out
}

/// The issue measured only 8-bit 2x enlargements; other scales and 16/32-bit documents keep
/// Preserve Details' previous result (#14794, "not measured ... keep today's behaviour").
#[test]
fn unmeasured_enlargements_keep_the_previous_sharpening() {
    let cases = [
        (SampleType::U8, 1.37, 1.37),
        (SampleType::U8, 3.0, 3.0),
        (SampleType::U8, 2.0, 1.5),
        (SampleType::U8, 1.0, 2.0),
        (SampleType::U16, 2.0, 2.0),
        (SampleType::F32, 2.0, 2.0),
    ];
    for (sample, sx, sy) in cases {
        let s = document(sample, false);
        let actual = resize_surface(&s, sx, sy, Resample::PreserveDetails);
        let expected = lanczos_and_unsharp_mask(&s, sx, sy);
        let r = actual.content_bounds().union(&expected.content_bounds());
        assert!(actual.read_region(r) == expected.read_region(r), "{sample:?} {sx}x{sy}: differs from Lanczos + Unsharp Mask 30 %");
    }
}

/// One axis shrinking and the other growing: the columns black, black, black, white reduced to a
/// quarter keep the white averaged in by the reduction instead of being clamped back to black.
#[test]
fn mixed_axis_resize_keeps_reduced_detail() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
        for x in 0..64 {
            let v = if x % 4 == 3 { 1.0 } else { 0.0 };
            s.fill_rect(Rect::new(x, 0, x + 1, 16), &[v, v, v, 1.0]);
        }
        let o = resize_surface(&s, 0.25, 2.0, Resample::PreserveDetails);
        let v = o.pixel(8, 16)[1];
        assert!(v > 0.1, "{sample:?}: green {v} at (8,16), the white columns were clamped away");
        let expected = lanczos_and_unsharp_mask(&s, 0.25, 2.0);
        let r = o.content_bounds().union(&expected.content_bounds());
        assert!(o.read_region(r) == expected.read_region(r), "{sample:?}: differs from Lanczos + Unsharp Mask 30 %");
    }
}

/// A band of output rows that spans thousands of source rows (6000x4000 at 2 x 0.01 spans about
/// 3900 per band) reads them a row at a time, so a read stays within the read budget.
#[test]
fn dering_reads_a_bounded_number_of_source_rows() {
    let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, false));
    s.fill_rect(Rect::new(0, 0, 8, 4000), &[0.5, 0.5, 0.5]);
    let mut o = resize_surface(&s, 2.0, 0.01, Resample::Lanczos);
    dering(&mut o, &s, 2.0, 0.01, None);
    let rows = DERING_MAX_READ_ROWS.load(std::sync::atomic::Ordering::Relaxed);
    assert!(rows <= 1, "one dering read spanned {rows} source rows");
}

/// Dering writes its bands a group at a time, like `resize_pass`, instead of holding them all.
#[test]
fn dering_holds_a_bounded_group_of_bands() {
    let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, false));
    s.fill_rect(Rect::new(0, 0, 2, 6400), &[0.5, 0.5, 0.5]);
    let mut o = resize_surface(&s, 2.0, 2.0, Resample::Lanczos);
    dering(&mut o, &s, 2.0, 2.0, None);
    let held = DERING_MAX_HELD_BANDS.load(std::sync::atomic::Ordering::Relaxed);
    let group = rayon::current_num_threads().max(1) * 2;
    assert!(held <= group, "dering held {held} of {} bands at once (group {group})", 12800usize.div_ceil(BAND));
}

/// Without a canvas, positions outside the content read the default pixel, as the resize does:
/// an opaque square on transparency keeps its partly transparent fringe instead of the clamp
/// making it opaque, and the result equals the one in a canvas of transparency around it.
#[test]
fn enlarging_without_a_canvas_keeps_the_transparent_fringe() {
    let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    s.fill_rect(Rect::new(10, 10, 30, 30), &[0.2, 0.6, 1.0, 1.0]);
    let o = resize_surface(&s, 2.0, 2.0, Resample::PreserveDetails);
    // The square's first output column: Lanczos leaves it partly transparent (about 0.79).
    let fringe = o.pixel(20, 40)[3];
    assert!(fringe > 0.0 && fringe < 1.0, "alpha {fringe} on the square's edge");
    let framed = resize_surface_in_canvas(&s, 2.0, 2.0, Resample::PreserveDetails, CANVAS);
    let r = o.content_bounds().union(&framed.content_bounds());
    assert!(o.read_region(r) == framed.read_region(r), "differs from the same square in a transparent canvas");
}

/// With a canvas much wider than the layer, dering reads only the source columns the output
/// samples use, not the whole canvas width.
#[test]
fn dering_reads_only_the_columns_it_uses() {
    let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    s.fill_rect(Rect::new(1000, 10, 1020, 30), &[0.2, 0.6, 1.0, 1.0]);
    let canvas = Rect::new(0, 0, 20_000, 40);
    let mut o = resize_surface_in_canvas(&s, 2.0, 2.0, Resample::Lanczos, canvas);
    // One worker thread, so the thread-local counter sees every read.
    let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    let cols = pool.install(|| {
        DERING_READ_COLUMNS.with(|c| c.set(0));
        dering(&mut o, &s, 2.0, 2.0, Some(canvas));
        DERING_READ_COLUMNS.with(std::cell::Cell::get)
    });
    assert!(cols <= 40, "one dering read spanned {cols} source columns");
}
