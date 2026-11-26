use super::*;
use photocraft_color::{ColorMode, PixelFormat};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[test]
fn checked_coverage_matches_native_mix_at_integer_and_hdr_depths() {
    // Cross native tile and negative-coordinate boundaries; cover alpha/opaque
    // and channel layouts without borrowing expected output from the new path.
    let area = Rect::from_xywh(-1, -1, 260, 3);
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            for alpha in [false, true] {
                let format = PixelFormat::new(mode, depth, alpha);
                let mut base = Surface::new(format);
                let mut filtered = Surface::new(format);
                let mut mask = LayerMask::hide_all();
                mask.surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
                for y in -1..2 {
                    for x in -1..259 {
                        let before = if depth == SampleType::F32 { -0.25 } else { 0.125 };
                        let after = if depth == SampleType::F32 { 2.5 } else { 0.875 };
                        let mut b = vec![before; format.channels()];
                        let mut f = vec![after; format.channels()];
                        if alpha {
                            *b.last_mut().unwrap() = if x == 0 { f32::from_bits(1) } else { 0.25 };
                            *f.last_mut().unwrap() = if x == 0 { 2. * f32::from_bits(1) } else { 0.75 };
                        }
                        base.write_pixel(x, y, &b);
                        filtered.write_pixel(x, y, &f);
                        mask.surface.write_pixel(
                            x,
                            y,
                            &[match x.rem_euclid(4) {
                                0 => 0.,
                                1 => 0.25,
                                2 => 0.75,
                                _ => 1.,
                            }],
                        );
                    }
                }
                for enabled in [false, true] {
                    for density in [0., 0.625, 1.] {
                        mask.enabled = enabled;
                        mask.density = density;
                        let before = (base.clone(), filtered.clone(), mask.clone());
                        let expected = mix(&base, &filtered, &mask);
                        let actual = mix_in(&base, &filtered, &mask, area, &Interrupt::NONE).unwrap();
                        assert_eq!(
                            actual.to_interleaved(area),
                            expected.to_interleaved(area),
                            "{depth:?} {mode:?} alpha={alpha} enabled={enabled} density={density}"
                        );
                        assert_eq!((base.clone(), filtered.clone(), mask.clone()), before);
                    }
                }
            }
        }
    }
}

#[test]
fn checked_coverage_cancels_inside_a_tile_without_publishing() {
    let area = Rect::from_xywh(0, 0, 2, 4);
    let mut base = Surface::new(PixelFormat::RGBA16);
    let mut filtered = base.clone();
    base.fill_rect(area, &[0.125, 0.25, 0.5, 1.]);
    filtered.fill_rect(area, &[0.75, 0.5, 0.25, 1.]);
    let mask = LayerMask::reveal_all();
    let before = (base.clone(), filtered.clone());
    let cancelled = AtomicBool::new(false);
    let rows = AtomicUsize::new(0);
    let cancel = || cancelled.load(Ordering::Relaxed);
    let progress = |_: f32| {
        rows.fetch_add(1, Ordering::Relaxed);
        cancelled.store(true, Ordering::Relaxed);
    };
    assert!(matches!(mix_in(&base, &filtered, &mask, area, &Interrupt::new(&cancel, &progress)), Err(MixError::Cancelled)));
    assert_eq!(rows.load(Ordering::Relaxed), 1, "cancellation after the first completed row must stop before row two");
    assert_eq!((base, filtered), before);
}

#[test]
fn checked_coverage_rejects_invalid_samples_and_density() {
    let area = Rect::from_xywh(0, 0, 1, 1);
    let mut base = Surface::new(PixelFormat::RGBA32F);
    base.fill_rect(area, &[0., 0.25, 2., 1.]);
    let mut filtered = base.clone();
    filtered.fill_rect(area, &[-1., 0.5, 4., 1.]);
    let mut mask = LayerMask::reveal_all();
    let before = (base.clone(), filtered.clone());
    mask.density = f32::NAN;
    assert!(matches!(mix_in(&base, &filtered, &mask, area, &Interrupt::NONE), Err(MixError::InvalidDensity)));
    mask.density = 1.;
    mask.surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    mask.surface.fill_rect(area, &[f32::INFINITY]);
    assert!(matches!(mix_in(&base, &filtered, &mask, area, &Interrupt::NONE), Err(MixError::NonFiniteSample)));
    mask.surface.fill_rect(area, &[0.5]);
    filtered.fill_rect(area, &[f32::NAN, 0., 0., 1.]);
    assert!(matches!(mix_in(&base, &filtered, &mask, area, &Interrupt::NONE), Err(MixError::NonFiniteSample)));
    assert_eq!(base, before.0);
}
