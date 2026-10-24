use super::*;
use photocraft_color::ColorMode;

#[test]
fn preserves_alpha_and_averages_hidden_rgb_at_each_depth() {
    let bounds = Rect::new(0, 0, 2, 1);
    for sample in SampleType::ALL {
        let fmt = PixelFormat::new(ColorMode::Rgb, sample, true);
        let mut surface = Surface::new(fmt);
        surface.write_region(bounds, &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0]);
        let out = apply(&surface, bounds, None, &Interrupt::NONE).unwrap();
        for (i, p) in out.read_region(bounds).as_chunks::<4>().0.iter().enumerate() {
            assert!((p[0] - 0.5).abs() <= 1.0 / 255.0);
            assert_eq!(p[1], 0.0);
            assert!((p[2] - 0.5).abs() <= 1.0 / 255.0);
            assert_eq!(p[3], if i == 0 { 1.0 } else { 0.0 });
        }
    }
}

#[test]
fn direct_kernel_uses_reference_bounds_for_each_output_tile() {
    let bounds = Rect::new(-1, 0, 1, 1);
    let src = Image { rect: bounds, ch: 4, data: vec![1.0, 0.0, 0.0, 0.25, 0.0, 0.0, 1.0, 0.75] };
    let ctx = Ctx { bounds, mode: ColorMode::Rgb, alpha: true };
    assert_eq!(kernel(&src, Rect::new(-1, 0, 0, 1), &ctx), vec![0.5, 0.0, 0.5, 0.25]);
    assert_eq!(kernel(&src, Rect::new(0, 0, 1, 1), &ctx), vec![0.5, 0.0, 0.5, 0.75]);
}

#[test]
fn rejects_extreme_geometry_and_honours_pre_cancel() {
    let surface = Surface::new(PixelFormat::RGBA8);
    let huge = Rect::new(i32::MIN, 0, i32::MAX, 1);
    assert!(apply(&surface, huge, None, &Interrupt::NONE).is_none());
    assert!(apply(&surface, Rect::new(0, 0, 5, 5), None, &Interrupt::cancel_only(&|| true)).is_none());
    assert_eq!(surface.tile_count(), 0);
}

// Reference arithmetic from map_pixels/Average before streaming. It deliberately keeps
// separate original/processed buffers so selection interpolation is independently exercised.
fn dense_reference(surface: &Surface, area: Rect, selection: Option<&Surface>) -> Surface {
    let fmt = surface.format();
    let n = fmt.channels();
    let mut raw = surface.read_region(area);
    let orig: Vec<_> = raw.chunks_exact(n).map(|p| to_rgba(&fmt, p)).collect();
    let w = area.width() as usize;
    let cover: Vec<_> = (0..orig.len()).map(|i| selection.map_or(1.0, |s| s.sample_channel(area.x0 + (i % w) as i32, area.y0 + (i / w) as i32, 0))).collect();
    let mut sum = [0.0f64; 3];
    let mut weight = 0.0f64;
    for (p, &k) in orig.iter().zip(&cover) {
        for c in 0..3 {
            sum[c] += f64::from(p[c]) * f64::from(k);
        }
        weight += f64::from(k);
    }
    if weight <= 0.0 {
        return surface.clone();
    }
    let mean = sum.map(|s| (s / weight) as f32);
    let mut processed = orig.clone();
    for px in &mut processed {
        px[..3].copy_from_slice(&mean);
    }
    for (i, px) in raw.chunks_exact_mut(n).enumerate() {
        if cover[i] <= 0.0 {
            continue;
        }
        let rgba = std::array::from_fn(|c| orig[i][c] + (processed[i][c] - orig[i][c]) * cover[i]);
        from_rgba_into(&fmt, rgba, px);
    }
    let mut out = surface.clone();
    out.write_region(area, &raw);
    out.prune();
    out
}

#[test]
fn streaming_matches_dense_across_formats_masks_and_tile_edges() {
    // Cross negative/global tile edges and the 4096-pixel reduction segment boundary.
    let area = Rect::new(-3, -1, 4203, 2);
    let check = area.inflate(2);
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for sample in SampleType::ALL {
            for alpha in [false, true] {
                let fmt = PixelFormat::new(mode, sample, alpha);
                let mut source = Surface::with_default(fmt, &vec![0.2; fmt.channels()]);
                let mut mask = Surface::new(PixelFormat::GRAY8);
                for y in check.y0..check.y1 {
                    for x in check.x0..check.x1 {
                        let seed = (x + 13 * y).rem_euclid(17) as usize;
                        if seed != 0 {
                            let px: Vec<_> = (0..fmt.channels()).map(|c| ((seed + 3 * c) % 19) as f32 / 13.0 - 0.1).collect();
                            source.write_pixel(x, y, &px);
                        }
                        mask.write_pixel(x, y, &[if seed == 0 { 0.0 } else { (seed % 5) as f32 / 4.0 }]);
                    }
                }
                for selection in [None, Some(&mask)] {
                    let expected = dense_reference(&source, area, selection);
                    let actual = apply(&source, area, selection, &Interrupt::NONE).unwrap();
                    for (i, (actual, expected)) in actual.read_region(check).iter().zip(expected.read_region(check)).enumerate() {
                        assert_eq!(actual.to_bits(), expected.to_bits(), "{fmt:?}, mask={}, sample={i}", selection.is_some());
                    }
                }
                let empty = Surface::new(PixelFormat::GRAY8);
                assert_eq!(apply(&source, area, Some(&empty), &Interrupt::NONE).unwrap(), source);
            }
        }
    }
}

#[test]
fn average_can_cancel_in_both_passes_without_mutating_input() {
    // Cancel in response to an intermediate progress report in each pass.
    use std::sync::atomic::{AtomicBool, Ordering};
    let area = Rect::new(0, 0, 1024, 1024);
    let source = Surface::with_default(PixelFormat::RGBA8, &[1.0, 0.0, 0.0, 0.5]);
    for threshold in [0.1, 0.6] {
        let cancelled = AtomicBool::new(false);
        let cancel = || cancelled.load(Ordering::Relaxed);
        let progress = |f: f32| {
            if f >= threshold && f < 1.0 {
                cancelled.store(true, Ordering::Relaxed);
            }
        };
        let before = source.clone();
        assert!(apply(&source, area, None, &Interrupt::new(&cancel, &progress)).is_none(), "threshold={threshold}");
        assert_eq!(source, before);
    }
}

#[test]
fn large_average_starts_with_a_bounded_cancellable_read() {
    use std::sync::atomic::{AtomicBool, Ordering};
    // 196 MP would need a 3.1 GB decoded RGBA buffer. Starting this operation must only
    // read a small segment, report progress, and observe cancellation before the next read.
    let source = Surface::with_default(PixelFormat::RGBA32F, &[1.0, 0.0, 0.0, 1.0]);
    let progressed = AtomicBool::new(false);
    let cancel = || progressed.load(Ordering::Relaxed);
    let progress = |f: f32| {
        if f > 0.0 && f < 1.0 {
            progressed.store(true, Ordering::Relaxed);
        }
    };
    let area = Rect::new(0, 0, 14000, 14000);
    assert!(crate::apply_in_with(&source, &crate::FilterParams::Average, area, area, None, area, &Interrupt::new(&cancel, &progress)).is_none());
    assert!(progressed.load(Ordering::Relaxed), "a large image should start processing, not be refused for a full-image float allocation");
    assert_eq!(source.tile_count(), 0);
}
