use photocraft_algo::{
    transform::{Homography, Interp, warp_surface, warp_surface_clipped},
    warp::place_source,
};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Affine, Rect};
use photocraft_raster::Surface;

fn stripes(sample: SampleType) -> Surface {
    let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
    for y in 0..80 {
        // Horizontal detail must survive a reduction of the width alone.
        let v = if y % 3 == 0 { 0.8 } else { 0.2 };
        src.fill_rect(Rect::new(0, y, 96, y + 1), &[v, 1.0 - v, 0.4, 1.0]);
    }
    src
}

#[test]
fn reducing_width_preserves_detail_on_the_unscaled_axis() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let src = stripes(sample);
        let h = Homography([0.5, 0.0, 0.25, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        for interp in [Interp::Bicubic, Interp::BicubicCoverage] {
            let out = warp_surface(&src, Rect::new(0, 0, 96, 80), &h, interp).unwrap();
            for y in 4..76 {
                let got = out.pixel(20, y);
                let expected = src.pixel(40, y);
                for (a, b) in got.iter().zip(&expected) {
                    assert!((a - b).abs() < 1e-6, "{sample:?} {interp:?} row {y}: {got:?} != {expected:?}");
                }
            }
        }
    }
}

#[test]
fn reorigining_source_does_not_change_placed_detail() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let bounds = Rect::new(0, 0, 96, 80);
        let mut src = stripes(sample);
        for x in 0..96 {
            let a = if x % 5 == 0 { 0.35 } else { 1.0 };
            src.fill_rect(Rect::new(x, 16, x + 1, 60), &[0.1, (x % 7) as f32 / 7.0, 0.6, a]);
        }
        let moved = photocraft_algo::resample::translate_surface(&src, 13, -7);
        for (sx, sy) in [(0.625, 0.625), (0.48, 0.73), (0.99, 0.985)] {
            let t = Affine { m: [sx, 0.0, 0.0, sy, 251.25, 249.75] };
            let compensating = Affine { m: [sx, 0.0, 0.0, sy, 251.25 - 13.0 * sx, 249.75 + 7.0 * sy] };
            let out = place_source(&src, bounds, &t, None).unwrap();
            let other = place_source(&moved, Rect::new(13, -7, 109, 73), &compensating, None).unwrap();
            let crop = Rect::new(255, 255, 285, 285);
            let max = out.read_region(crop).iter().zip(other.read_region(crop)).map(|(a, b)| (a - b).abs()).fold(0.0_f32, f32::max);
            assert!(max < 1e-5, "{sample:?} {sx}x{sy}: max {max}");
            let h = Homography([sx, 0.0, 251.25, 0.0, sy, 249.75, 0.0, 0.0, 1.0]);
            let clipped = warp_surface_clipped(&src, bounds, &h, Interp::BicubicCoverage, crop).unwrap();
            assert_eq!(out.read_region(crop), clipped.read_region(crop));
        }
    }
}

#[test]
fn fractional_reductions_preserve_constant_translucent_colour_and_hdr() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let fmt = PixelFormat::new(ColorMode::Rgb, sample, true);
        let mut src = Surface::new(fmt);
        let colour = if sample == SampleType::F32 { [-0.2, 2.5, 0.6, 0.3] } else { [0.1, 0.7, 0.6, 0.3] };
        src.fill_rect(Rect::new(-20, -20, 80, 80), &colour);
        for (sx, sy) in [(0.7, 0.6), (-0.7, 0.6), (0.6, -0.7), (1.4, 0.7)] {
            let h = Homography([sx, 0.0, 256.25, 0.0, sy, 256.75, 0.0, 0.0, 1.0]);
            let out = warp_surface(&src, Rect::new(-20, -20, 80, 80), &h, Interp::BicubicCoverage).unwrap();
            let expected = src.pixel(0, 0);
            for (x, y) in [(255, 255), (256, 255), (255, 256), (256, 256)] {
                let got = out.pixel(x, y);
                for (a, b) in got.iter().zip(&expected) {
                    assert!((a - b).abs() < 1e-6, "{sample:?} {sx}x{sy}: {got:?} != {expected:?}");
                }
            }
        }
    }
}

#[test]
fn downscale_samples_the_original_pixel_grid_once() {
    // A half-width horizontal ramp translated by a quarter destination pixel samples at
    // source x = 2*x_out, exactly. The interior widened cubic reproduces that linear ramp.
    // A prefitered half-size raster samples at 2*x_out + 0.5, then interpolates again.
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
        for x in 0..80 {
            src.fill_rect(Rect::new(x, 0, x + 1, 16), &[x as f32 / 255.0, 0.4, 0.7, 1.0]);
        }
        let h = Homography([0.5, 0.0, 0.25, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        let out = warp_surface(&src, src.content_bounds(), &h, Interp::Bicubic).unwrap();
        for x in 3..37 {
            assert!((out.pixel(x, 6)[0] - src.pixel(2 * x, 6)[0]).abs() < 1e-6, "{sample:?} {x}");
        }
    }
}

#[test]
#[ignore = "24 MP release benchmark; run explicitly with --release --ignored --nocapture"]
fn affine_reduction_24mp() {
    let mut src = Surface::new(PixelFormat::RGBA8);
    let bounds = Rect::new(0, 0, 6000, 4000);
    for y in 0..4000 {
        src.fill_rect(Rect::new(0, y, 6000, y + 1), &[0.2, (y % 17) as f32 / 17.0, 0.7, 1.0]);
    }
    let h = Homography([0.5, 0.0, 0.25, 0.0, 0.5, 0.75, 0.0, 0.0, 1.0]);
    for round in 0..3 {
        let start = std::time::Instant::now();
        let out = warp_surface(&src, bounds, &h, Interp::BicubicCoverage).unwrap();
        eprintln!("round {round}: {:?}, {} tiles", start.elapsed(), out.tiles().count());
        assert!(out.pixel(100, 100)[3] > 0.99);
    }
}

#[test]
fn reduced_coverage_keeps_authored_faint_pixels_and_rejects_negative_lobe_products() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
        src.write_pixel(12, 12, &[0.1, 0.5, 0.85, 1.0]);
        src.fill_rect(Rect::new(0, 0, 8, 8), &[0.1, 0.5, 0.85, 1.0 / 255.0]);
        let h = Homography([0.75, 0.0, 0.5, 0.0, 0.75, 0.5, 0.0, 0.0, 1.0]);
        let regular = warp_surface(&src, Rect::new(0, 0, 24, 24), &h, Interp::Bicubic).unwrap();
        let covered = warp_surface(&src, Rect::new(0, 0, 24, 24), &h, Interp::BicubicCoverage).unwrap();
        assert!(regular.pixel(8, 8)[3] > 0.0, "fixture must have a negative-lobe product: {sample:?}");
        assert_eq!(covered.pixel(8, 8)[3], 0.0, "{sample:?}");
        assert_eq!(covered.pixel(4, 4)[3], src.pixel(4, 4)[3], "authored faint coverage: {sample:?}");
        assert!(covered.pixel(9, 9)[3] > 0.1, "positive footprint: {sample:?}");
    }
}

#[test]
fn direct_reduction_cancels_and_preserves_non_rgb_channels() {
    use photocraft_algo::transform::warp_surface_with;
    use photocraft_raster::Interrupt;
    use std::sync::atomic::{AtomicBool, Ordering};
    for mode in [ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::new(mode, sample, true);
            let mut src = Surface::new(fmt);
            let colour: Vec<_> = (0..fmt.channels()).map(|c| (c + 1) as f32 / (fmt.channels() + 1) as f32).collect();
            let rect = Rect::new(0, 0, 600, 520);
            src.fill_rect(rect, &colour);
            let h = Homography([0.6, 0.0, 0.25, 0.0, 0.7, 0.75, 0.0, 0.0, 1.0]);
            let out = warp_surface(&src, rect, &h, Interp::BicubicCoverage).unwrap();
            for (a, b) in out.pixel(50, 50).iter().zip(src.pixel(50, 50)) {
                assert!((a - b).abs() < 1e-6, "{mode:?} {sample:?}: {a} != {b}");
            }
            let cancelled = AtomicBool::new(false);
            let check = || cancelled.load(Ordering::Relaxed);
            let progress = |_| cancelled.store(true, Ordering::Relaxed);
            assert!(warp_surface_with(&src, rect, &h, Interp::BicubicCoverage, &Interrupt::new(&check, &progress)).unwrap().is_none());
            assert!(cancelled.load(Ordering::Relaxed));
        }
    }
}

#[test]
fn implicit_pixels_and_sparse_reductions_keep_their_coverage() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let fmt = PixelFormat::new(ColorMode::Rgb, sample, true);
        let rect = Rect::new(0, 0, 700, 600);
        let h = Homography([0.6, 0.0, 0.25, 0.0, 0.7, 0.75, 0.0, 0.0, 1.0]);
        let colour = [0.2, 0.5, 0.7, 0.3];
        let implicit = Surface::with_default(fmt, &colour);
        let mut explicit = Surface::new(fmt);
        explicit.fill_rect(rect, &colour);
        let a = warp_surface(&implicit, rect, &h, Interp::BicubicCoverage).unwrap();
        let b = warp_surface(&explicit, rect, &h, Interp::BicubicCoverage).unwrap();
        let dst = a.content_bounds().union(&b.content_bounds());
        assert_eq!(a.read_region(dst), b.read_region(dst), "{sample:?}: implicit default was discarded");
        let mut sparse = Surface::new(fmt);
        sparse.fill_rect(Rect::new(16, 18, 40, 50), &colour);
        let out = warp_surface(&sparse, rect, &h, Interp::BicubicCoverage).unwrap();
        assert_eq!(out.tiles().count(), 1, "{sample:?}: empty destination tiles were retained");
        assert!(out.pixel(15, 20)[3] > 0.0);
    }
}

#[test]
fn subpixel_rectangles_cover_only_their_geometric_area() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for (width, height, sx, sy) in [(1, 16, 0.5_f64, 1.0_f64), (16, 1, 1.0, -0.5), (1, 1, -0.5, 0.5), (1, 1, 1.0 / 16.0, -1.0 / 16.0)] {
            let rect = Rect::new(-7, 11, -7 + width, 11 + height);
            let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
            src.fill_rect(rect, &[0.25, 0.5, 0.75, 1.0]);
            let tx = 10.5 - sx * (f64::from(rect.x0) + f64::from(width) / 2.0);
            let ty = 15.5 - sy * (f64::from(rect.y0) + f64::from(height) / 2.0);
            let h = Homography([sx, 0.0, tx, 0.0, sy, ty, 0.0, 0.0, 1.0]);
            let expected = ((f64::from(width) * sx.abs()).min(1.0) * (f64::from(height) * sy.abs()).min(1.0)) as f32;
            for interp in [Interp::Bicubic, Interp::BicubicCoverage] {
                let out = warp_surface(&src, rect, &h, interp).unwrap();
                let got = out.pixel(10, 15);
                let tolerance = match sample {
                    SampleType::U8 => 1.0 / 255.0,
                    SampleType::U16 => 1.0 / 65535.0,
                    SampleType::F32 => 1e-6,
                };
                assert!((got[3] - expected).abs() <= tolerance, "{sample:?} {width}x{height} {sx}x{sy} {interp:?}: {got:?}, alpha {expected}");
                assert!(got[..3].iter().zip(src.pixel(rect.x0, rect.y0)).all(|(a, b)| (a - b).abs() < 1e-6));
            }
        }
    }
}

#[test]
fn invalid_alpha_affects_only_the_sampled_pixel_footprint() {
    let rect = Rect::new(0, 0, 64, 64);
    let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
    src.write_pixel(12, 12, &[0.1, 0.5, 0.85, 1.0]);
    src.write_pixel(60, 60, &[0.1, 0.5, 0.85, -0.1]);
    let h = Homography([0.75, 0.0, 0.5, 0.0, 0.75, 0.5, 0.0, 0.0, 1.0]);
    let full = warp_surface(&src, rect, &h, Interp::BicubicCoverage).unwrap();
    let clipped = warp_surface_clipped(&src, rect, &h, Interp::BicubicCoverage, Rect::new(8, 8, 9, 9)).unwrap();
    let shifted = warp_surface(&src, rect, &Homography([0.75, 0.0, 244.5, 0.0, 0.75, 0.5, 0.0, 0.0, 1.0]), Interp::BicubicCoverage).unwrap();
    assert_eq!(full.pixel(8, 8)[3], 0.0, "distant signed alpha enabled a negative-lobe product");
    assert_eq!(full.pixel(8, 8), clipped.pixel(8, 8));
    assert_eq!(full.pixel(8, 8), shifted.pixel(252, 8));
    // Signed alpha inside the actual kernel still needs the conservative fallback: a
    // negative sample times a negative weight can legitimately produce positive alpha.
    let mut signed = Surface::new(src.format());
    signed.write_pixel(12, 12, &[0.1, 0.5, 0.85, -0.1]);
    let regular = warp_surface(&signed, rect, &h, Interp::Bicubic).unwrap();
    let covered = warp_surface(&signed, rect, &h, Interp::BicubicCoverage).unwrap();
    assert!(regular.pixel(8, 9)[3] > 0.0);
    assert_eq!(covered.pixel(8, 9), regular.pixel(8, 9));
}

#[test]
fn equivalent_homogeneous_matrices_have_identical_reduced_bounds_and_pixels() {
    let rect = Rect::new(0, 0, 16, 16);
    let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
    src.fill_rect(rect, &[0.25, 0.5, 0.75, 1.0]);
    let h = Homography([0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 1.0]);
    for interp in [Interp::Bicubic, Interp::BicubicCoverage] {
        let expected = warp_surface(&src, rect, &h, interp).unwrap();
        for factor in [0.00015, -0.00015, 0.5, 1024.0] {
            let other = warp_surface(&src, rect, &Homography(h.0.map(|v| v * factor)), interp).unwrap();
            assert_eq!(other.content_bounds(), expected.content_bounds(), "homogeneous factor {factor}");
            assert_eq!(other.read_region(rect), expected.read_region(rect), "homogeneous factor {factor}");
        }
    }
}
