use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use std::sync::atomic::{AtomicUsize, Ordering};

fn params(softness: f32) -> FilterParams {
    FilterParams::Relight { angle: -43.0, elevation: 37.0, intensity: 73.0, ambient: 31.0, warmth: -22.0, softness }
}

fn fixture(fmt: PixelFormat, rect: Rect, hostile: bool) -> Surface {
    let ch = fmt.channels();
    let mut data = vec![0.0; rect.size().area() as usize * ch];
    for (i, px) in data.chunks_exact_mut(ch).enumerate() {
        let x = i % rect.width() as usize;
        let y = i / rect.width() as usize;
        for (c, v) in px.iter_mut().enumerate() {
            *v = ((x * 13 + y * 31 + c * 71) % 257) as f32 / 256.0;
        }
        if fmt.alpha {
            px[ch - 1] = match (x / 17 + y / 23) % 5 {
                0 => 0.0,
                1 => 0.25,
                _ => 1.0,
            };
        }
        if hostile && fmt.sample == SampleType::F32 {
            if i % 97 == 0 {
                px[0] = -2.5;
            }
            if i % 113 == 0 {
                px[0] = 4.0;
            }
            if i % 199 == 0 {
                px[0] = f32::NAN;
            }
            if i % 211 == 0 {
                px[0] = f32::INFINITY;
            }
            if fmt.alpha && i % 223 == 0 {
                px[ch - 1] = f32::NAN;
            }
        }
    }
    let mut s = Surface::new(fmt);
    s.write_region(rect, &data);
    s
}

fn compare(a: &Surface, b: &Surface, area: Rect, tolerance: f32) -> f32 {
    let aa = a.read_region(area);
    let bb = b.read_region(area);
    let mut maximum = 0.0f32;
    for (i, (&a, &b)) in aa.iter().zip(&bb).enumerate() {
        if a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()) {
            continue;
        }
        assert!(a.is_finite() && b.is_finite(), "sample {i}: {a} != {b}");
        let d = (a - b).abs();
        maximum = maximum.max(d);
        assert!(d <= tolerance, "sample {i}: {a} != {b}, error {d}, tolerance {tolerance}");
    }
    maximum
}

fn reference(s: &Surface, p: &FilterParams, area: Rect, bounds: Rect, selection: Option<&Surface>, extent: Option<Rect>) -> Surface {
    crate::apply_tiled_with_impl(s, p, area, bounds, selection, crate::auto_tile(p, bounds), extent, &Interrupt::NONE, false).unwrap()
}

#[test]
fn stream_matches_reference_across_depths_models_and_transparency() {
    let area = Rect::new(-19, 13, 581, 493);
    let bounds = Rect::new(-19, 13, 5981, 4013); // sigma reaches 96 even on this cropped output.
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for alpha in [false, true] {
                let fmt = PixelFormat::new(mode, sample, alpha);
                let s = fixture(fmt, area, false);
                for softness in [1.0, 25.0, 100.0] {
                    let p = params(softness);
                    let fast = apply(&s, &p, area, bounds, None, Some(area), &Interrupt::NONE).unwrap().unwrap();
                    let old = reference(&s, &p, area, bounds, None, Some(area));
                    // Running sums now start at the shared field's origin. At stored integer
                    // depths permit one quantization step; float paths stay within 2e-5.
                    let tolerance = match sample {
                        SampleType::U8 => 1.0 / 255.0 + 1e-7,
                        SampleType::U16 => 1.0 / 65535.0 + 1e-7,
                        _ => 2e-5,
                    };
                    compare(&fast, &old, area, tolerance);
                    if alpha {
                        for (a, b) in fast.read_region(area).chunks_exact(fmt.channels()).zip(s.read_region(area).chunks_exact(fmt.channels())) {
                            assert_eq!(a[fmt.channels() - 1].to_bits(), b[fmt.channels() - 1].to_bits());
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn stream_matches_reference_at_strip_edges_selection_and_nonfinite_samples() {
    let area = Rect::new(0, 0, 2101, 129); // crosses the old block boundary and many strip boundaries.
    let bounds = Rect::new(0, 0, 6000, 4000);
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    let s = fixture(fmt, area, true);
    let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    selection.fill_rect(Rect::new(7, 15, 2095, 112), &[0.4]);
    selection.fill_rect(Rect::new(91, 32, 1703, 96), &[1.0]);
    for extent in [None, Some(area)] {
        for sel in [None, Some(&selection)] {
            let p = params(100.0);
            let fast = apply(&s, &p, area, bounds, sel, extent, &Interrupt::NONE).unwrap().unwrap();
            let old = reference(&s, &p, area, bounds, sel, extent);
            compare(&fast, &old, area, 2e-5);
        }
    }
}

#[test]
fn vertical_ring_matches_the_original_pass_bit_for_bit() {
    for (w, h) in [(1, 257), (7, 333), (65, 211)] {
        for r in [0, 1, 2, 31, 95, 96] {
            let src: Vec<f32> = (0..w * h * 2).map(|i| ((i * 73 % 1021) as f32 - 400.0) / 127.0).collect();
            let mut stage = BoxRows::new(w * 2, r).unwrap();
            let mut row = vec![0.0; w * 2];
            let mut got = Vec::new();
            for input in src.chunks_exact(w * 2) {
                if stage.push(input, &mut row) {
                    got.extend_from_slice(&row);
                }
            }
            for _ in 0..r {
                stage.repeat(&mut row);
                got.extend_from_slice(&row);
            }
            let norm = 1.0 / (2 * r + 1) as f64;
            let mut expected = vec![0.0f32; src.len()];
            for x in 0..w * 2 {
                let at = |y: isize| f64::from(src[y.clamp(0, h as isize - 1) as usize * w * 2 + x]);
                let mut sum: f64 = (-(r as isize)..=r as isize).map(at).sum();
                for y in 0..h {
                    expected[y * w * 2 + x] = (sum * norm) as f32;
                    sum += at((y + r + 1) as isize) - at(y as isize - r as isize);
                }
            }
            assert_eq!(got.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), "{w}x{h} radius {r}");
        }
    }
}

#[test]
fn preparation_is_bounded_and_independent_of_height() {
    let bounds = Rect::new(0, 0, 6000, 4000);
    let p = params(100.0);
    let a = working_bytes(&p, bounds, bounds, 4);
    let b = working_bytes(&p, Rect::new(0, 0, 6000, 400_000), bounds, 4);
    assert_eq!(a, b);
    assert!(a < 128 * 1024 * 1024, "{a} bytes");
    assert!(crate::working_bytes(&p, bounds, bounds, 4) >= a);
    assert!(geometry(&p, Rect::new(0, 0, 300_000, 4000), bounds).is_none());
    assert!(geometry(&p, Rect::new(i32::MIN, 0, i32::MIN + 600, 480), bounds).is_none());
    assert!(geometry(&p, Rect::new(0, i32::MAX - 480, 600, i32::MAX), bounds).is_none());
    assert!(geometry(&params(f32::NAN), bounds, bounds).is_none());
}

#[test]
fn cancellation_discards_the_result_at_each_streaming_phase() {
    let area = Rect::new(0, 0, 600, 480);
    let s = fixture(PixelFormat::RGBA8, area, false);
    let original = s.read_region(area);
    for point in [1, 2, 4, 20, 100, 400, 1000] {
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= point;
        let ctl = Interrupt::cancel_only(&cancel);
        assert!(apply(&s, &params(100.0), area, area, None, Some(area), &ctl).unwrap().is_none(), "check {point}");
    }
    assert_eq!(s.read_region(area), original);
}

#[test]
fn parallel_slabs_merge_partial_tiles_at_negative_coordinates() {
    let area = Rect::new(-719, -51, 1382, 78);
    let bounds = Rect::new(-719, -51, 5281, 3949);
    let s = fixture(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true), area.inflate(3), false);
    let p = params(100.0);
    let fast = apply(&s, &p, area, bounds, None, None, &Interrupt::NONE).unwrap().unwrap();
    let old = reference(&s, &p, area, bounds, None, None);
    compare(&fast, &old, area.inflate(3), 2e-5);
}

#[test]
fn one_pixel_axis_and_large_softness_keep_canvas_edge_semantics() {
    for area in [Rect::new(-3, 7, -2, 270_007), Rect::new(-3, 7, 270_997, 8)] {
        // The very wide case deliberately falls back; the narrow/tall case streams.
        let s = fixture(PixelFormat::RGBA8, area, false);
        let p = params(100.0);
        let fast = crate::apply_tiled_with_impl(&s, &p, area, area, None, 704, Some(area), &Interrupt::NONE, true).unwrap();
        let old = reference(&s, &p, area, area, None, Some(area));
        compare(&fast, &old, area, 1.0 / 255.0 + 1e-7);
    }
}

/// A full photo oracle is opt-in so ordinary test runs do not multiply its memory use.
#[test]
#[ignore = "24 MP float comparison against the original tiled path"]
fn full_photo_matches_reference() {
    let area = Rect::new(0, 0, 6000, 4000);
    let s = fixture(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true), area, true);
    let p = params(100.0);
    let old = reference(&s, &p, area, area, None, Some(area));
    let fast = apply(&s, &p, area, area, None, Some(area), &Interrupt::NONE).unwrap().unwrap();
    let mut maximum = 0.0f32;
    for y in (0..4000).step_by(128) {
        maximum = maximum.max(compare(&fast, &old, Rect::new(0, y, 6000, (y + 128).min(4000)), 2e-5));
    }
    eprintln!("24 MP float maximum absolute sample error: {maximum}");
}

#[test]
fn a_one_shot_cancellation_never_writes_an_incomplete_strip() {
    let area = Rect::new(0, 0, 512, 64);
    let surface = fixture(PixelFormat::RGBA8, area, false);
    let ctx = Ctx { bounds: area, mode: ColorMode::Rgb, alpha: true };
    let p = Params { angle: 45.0, elevation: 40.0, intensity: 40.0, ambient: 55.0, warmth: 0.0, softness: 25.0 };
    let step = 2;
    let shading_rows = 2 * step as usize + STRIP;
    let mut output = Output {
        surface: &surface,
        selection: None,
        area,
        field: area,
        ctx,
        p,
        step,
        shading: vec![0.5; area.width() as usize * shading_rows],
        shading_rows,
        received: STRIP - 1 + step as usize,
        next_y: 0,
        result: surface.clone(),
    };
    let checks = AtomicUsize::new(0);
    let once = || checks.fetch_add(1, Ordering::Relaxed) == 0;
    let ctl = Interrupt::cancel_only(&once);
    // One worker leaves an empty result, but the check after the join says false again.
    assert!(!output.push(&vec![1.0; area.width() as usize * 2], &ctl));
    assert_eq!(output.result.read_region(area), surface.read_region(area));
}

#[test]
fn stream_tile_indexes_are_local_and_keep_nonzero_defaults_and_edge_pixels() {
    let area = Rect::new(-19, 13, 581, 493);
    let bounds = Rect::new(0, 0, 6000, 4000);
    let far = Rect::new(100_000, 100_000, 100_001, 100_001).tiles().next().unwrap();
    for sample in SampleType::ALL {
        let fmt = PixelFormat::new(ColorMode::Rgb, sample, true);
        let mut source = Surface::with_default(fmt, &[0.2, 0.4, 0.1, 0.37]);
        source.write_pixel(-25, 15, &[0.9, 0.1, 0.7, 1.0]); // Outside the area, in its edge tile.
        source.write_pixel(30, 40, &[0.7, 0.5, 0.3, 0.6]);
        source.write_pixel(100_000, 100_000, &[0.8, 0.7, 0.6, 0.5]);
        let p = params(100.0);
        let stream = apply_stream(&source, &p, area, bounds, None, None, &Interrupt::NONE, true).unwrap().unwrap();
        assert!(stream.tile(far).is_none(), "a stream must not retain unrelated tile entries");
        assert!(stream.tile_count() as u64 <= area.tile_count());
        assert_eq!(stream.default_bytes(), source.default_bytes());
        let merged = apply(&source, &p, area, bounds, None, None, &Interrupt::NONE).unwrap().unwrap();
        let expected = reference(&source, &p, area, bounds, None, None);
        compare(&merged, &expected, area.inflate(8), 2e-5);
        assert_eq!(merged.pixel(100_000, 100_000), source.pixel(100_000, 100_000));
    }
}
