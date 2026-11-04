//! Ordinary Smart-filter strength and blend mixing, without adding coverage twice.
//!
//! Callers own operation admission and authority. Mixing uses bounded tile scratch and
//! publishes its staged surface only on success; it never reads a source document.

use std::fmt;

use photocraft_color::{BlendMode, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{AllocationError, Interrupt, Surface};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Cancelled,
    Allocation(AllocationError),
    FormatMismatch,
    InvalidOpacity,
    NonFiniteSample,
}

impl fmt::Display for Error {
    /// Describe native Smart-filter mixing failures without altering or interpreting source configuration.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Smart filter mixing cancelled"),
            Self::Allocation(error) => error.fmt(f),
            Self::FormatMismatch => f.write_str("Smart filter input and result formats differ"),
            Self::InvalidOpacity => f.write_str("Smart filter opacity must be finite and between zero and one"),
            Self::NonFiniteSample => f.write_str("Smart filter samples must be finite"),
        }
    }
}

impl std::error::Error for Error {}

impl From<AllocationError> for Error {
    /// Preserve a fallible raster allocation failure in the native mixing error type.
    fn from(error: AllocationError) -> Self {
        Self::Allocation(error)
    }
}

/// Convert cancellation into the shared mix error before another tile is processed.
fn check(ctl: &Interrupt<'_>) -> Result<(), Error> {
    ctl.check().map_err(|_| Error::Cancelled)
}

/// Mix an ordinary filter result with its input over `area`, preserving base data outside it.
///
/// `area` is the caller's native mixing extent, normally the union of input/result content
/// bounds. Non-Normal coverage changes select source-over only at the affected pixel.
/// Alpha-preserving filters instead edit existing coverage, including at fractional opacity.
/// Processed F32 samples are validated before blending and after native color conversion.
/// The caller must admit the retained output tiles; scratch is bounded to three tile buffers.
pub fn mix(base: &Surface, filtered: &Surface, area: Rect, mode: BlendMode, opacity: f32, ctl: &Interrupt<'_>) -> Result<Surface, Error> {
    check(ctl)?;
    if base.format() != filtered.format() {
        return Err(Error::FormatMismatch);
    }
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return Err(Error::InvalidOpacity);
    }
    if opacity == 0.0 || area.is_empty() {
        return Ok(base.clone());
    }
    let fmt = base.format();
    let n = fmt.channels();
    let mut out = base.clone();
    for tile in area.tiles() {
        check(ctl)?;
        let rect = tile.rect().intersect(&area);
        let b = base.try_read_region(rect)?;
        let t = filtered.try_read_region(rect)?;
        let mut values = Vec::new();
        let bytes = b.len().checked_mul(std::mem::size_of::<f32>()).ok_or(AllocationError::SizeOverflow)?;
        values.try_reserve_exact(b.len()).map_err(|_| AllocationError::NotEnoughMemory { bytes })?;
        values.resize(b.len(), 0.);
        for ((bp, tp), op) in b.chunks_exact(n).zip(t.chunks_exact(n)).zip(values.chunks_exact_mut(n)) {
            if fmt.sample == SampleType::F32 && bp.iter().chain(tp).any(|value| !value.is_finite()) {
                return Err(Error::NonFiniteSample);
            }
            let b = photocraft_raster::to_rgba(&fmt, bp);
            let t = photocraft_raster::to_rgba(&fmt, tp);
            if mode != BlendMode::Normal && fmt.alpha && bp.last() != tp.last() {
                let rgba =
                    if fmt.sample == SampleType::F32 { composite_hdr(mode, b, t, opacity) } else { photocraft_color::blend::composite(mode, b, t, opacity) };
                photocraft_raster::from_rgba_into(&fmt, rgba, op);
                if fmt.sample == SampleType::F32 && op.iter().any(|value| !value.is_finite()) {
                    return Err(Error::NonFiniteSample);
                }
                continue;
            }
            let cb = [b[0], b[1], b[2]];
            let ct = [t[0], t[1], t[2]];
            let color = if fmt.sample == SampleType::F32 {
                photocraft_color::blend::blend_rgb_hdr(mode, cb, ct)
            } else {
                photocraft_color::blend::blend_rgb(mode, cb, ct)
            };
            let alpha = b[3] + (t[3] - b[3]) * opacity;
            let mut rgba = [0., 0., 0., alpha];
            if alpha > 0. {
                for c in 0..3 {
                    rgba[c] = (cb[c] * b[3] * (1. - opacity) + color[c] * t[3] * opacity) / alpha;
                    if fmt.sample == SampleType::F32 && !rgba[c].is_finite() && [cb[c], color[c], b[3], t[3], alpha].into_iter().all(f32::is_finite) {
                        // Even an identical MAX-valued pixel can overflow after f32 weighted
                        // division. Recompute only that exceptional channel with wide weights.
                        let before = f64::from(b[3]) * (1. - f64::from(opacity));
                        let after = f64::from(t[3]) * f64::from(opacity);
                        let coverage = before + after;
                        let value = if coverage > 0. { (f64::from(cb[c]) * before + f64::from(color[c]) * after) / coverage } else { 0. };
                        rgba[c] = value.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32;
                    }
                }
            }
            photocraft_raster::from_rgba_into(&fmt, rgba, op);
            if fmt.sample == SampleType::F32 && op.iter().any(|value| !value.is_finite()) {
                return Err(Error::NonFiniteSample);
            }
        }
        check(ctl)?;
        out.try_write_region(rect, &values)?;
    }
    check(ctl)?;
    out.prune();
    check(ctl)?;
    Ok(out)
}

/// Source-over in the float document domain; wide accumulators keep finite weighted HDR blends finite.
fn composite_hdr(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4], opacity: f32) -> [f32; 4] {
    let ab = f64::from(backdrop[3]);
    let alpha = f64::from(source[3]) * f64::from(opacity);
    if alpha <= 0. {
        return backdrop;
    }
    let output_alpha = alpha + ab * (1. - alpha);
    if output_alpha <= 0. {
        return [0.; 4];
    }
    let blend = photocraft_color::blend::blend_rgb_hdr(mode, [backdrop[0], backdrop[1], backdrop[2]], [source[0], source[1], source[2]]);
    let mut output = [0.; 4];
    for c in 0..3 {
        let color = ((1. - alpha) * ab * f64::from(backdrop[c]) + (1. - ab) * alpha * f64::from(source[c]) + alpha * ab * f64::from(blend[c])) / output_alpha;
        output[c] = color.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32;
    }
    output[3] = output_alpha as f32;
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Create original native-format samples for direct premultiplied-mixing expectations.
    fn surface(format: PixelFormat, rgba: [f32; 4], rect: Rect) -> Surface {
        let mut surface = Surface::new(format);
        surface.write_region(rect, &photocraft_raster::from_rgba(&format, rgba).repeat((rect.width() * rect.height()) as usize));
        surface
    }

    /// Choose comparison tolerance from actual native sample quantization rather than imported source precision.
    fn tolerance(sample: SampleType) -> f32 {
        match sample {
            SampleType::U8 => 2. / 255.,
            SampleType::U16 => 2. / 65535.,
            SampleType::F32 => 0.000001,
        }
    }

    #[test]
    /// Pin Normal filter strength against independent changed-alpha premultiplied arithmetic at every depth.
    fn normal_strength_keeps_coverage_and_uses_premultiplied_changed_alpha_at_every_depth() {
        let rect = Rect::new(-1, -1, 2, 1);
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for sample in SampleType::ALL {
                let format = PixelFormat::new(mode, sample, true);
                let base = surface(format, [0.25, 0.25, 0.25, 0.5], rect);
                let filtered = surface(format, [0.75, 0.75, 0.75, 0.5], rect);
                let mixed = mix(&base, &filtered, rect, BlendMode::Normal, 0.25, &Interrupt::NONE).unwrap();
                let actual = photocraft_raster::to_rgba(&format, &mixed.pixel(0, 0));
                assert!((actual[0] - 0.375).abs() <= tolerance(sample));
                assert!((actual[3] - 0.5).abs() <= tolerance(sample));
                assert_eq!(mix(&base, &filtered, rect, BlendMode::Normal, 0., &Interrupt::NONE).unwrap(), base);
                let changed = surface(format, [0.75, 0.75, 0.75, 0.25], rect);
                let mixed = mix(&base, &changed, rect, BlendMode::Normal, 0.5, &Interrupt::NONE).unwrap();
                let actual = photocraft_raster::to_rgba(&format, &mixed.pixel(0, 0));
                assert!((actual[0] - 5. / 12.).abs() <= tolerance(sample));
                assert!((actual[3] - 0.375).abs() <= tolerance(sample));
                for (base_alpha, top_alpha, expected_color, expected_alpha) in [(0.5, 0., 0.25, 0.25), (0., 0.5, 0.75, 0.25), (1., 1., 0.5, 1.)] {
                    let base = surface(format, [0.25, 0.25, 0.25, base_alpha], rect);
                    let filtered = surface(format, [0.75, 0.75, 0.75, top_alpha], rect);
                    let result = mix(&base, &filtered, rect, BlendMode::Normal, 0.5, &Interrupt::NONE).unwrap();
                    let actual = photocraft_raster::to_rgba(&format, &result.pixel(0, 0));
                    assert!((actual[0] - expected_color).abs() <= tolerance(sample));
                    assert!((actual[3] - expected_alpha).abs() <= tolerance(sample));
                }
            }
        }
    }

    #[test]
    /// A changed-alpha edge in another tile must not increase untouched interior coverage.
    fn non_normal_coverage_is_classified_per_pixel_across_tiles() {
        let area = Rect::new(0, 0, 257, 1);
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for sample in SampleType::ALL {
                let format = PixelFormat::new(mode, sample, true);
                let mut base = surface(format, [0.2, 0.4, 0.6, 0.5], Rect::new(0, 0, 1, 1));
                let mut filtered = surface(format, [0.8, 0.6, 0.4, 0.5], Rect::new(0, 0, 1, 1));
                base.write_pixel(256, 0, &photocraft_raster::from_rgba(&format, [0.2, 0.4, 0.6, 1.]));
                filtered.write_pixel(256, 0, &photocraft_raster::from_rgba(&format, [0.8, 0.6, 0.4, 0.25]));
                for blend in [BlendMode::Multiply, BlendMode::Screen, BlendMode::Luminosity] {
                    let mixed = mix(&base, &filtered, area, blend, 0.5, &Interrupt::NONE).unwrap();
                    for x in [0, 256] {
                        let b = photocraft_raster::to_rgba(&format, &base.pixel(x, 0));
                        let t = photocraft_raster::to_rgba(&format, &filtered.pixel(x, 0));
                        let expected = if x == 0 {
                            let color = photocraft_color::blend::blend_rgb(blend, [b[0], b[1], b[2]], [t[0], t[1], t[2]]);
                            [0.5 * (b[0] + color[0]), 0.5 * (b[1] + color[1]), 0.5 * (b[2] + color[2]), b[3]]
                        } else {
                            photocraft_color::blend::composite(blend, b, t, 0.5)
                        };
                        let expected = surface(format, expected, Rect::new(x, 0, x + 1, 1));
                        for (got, want) in mixed.pixel(x, 0).into_iter().zip(expected.pixel(x, 0)) {
                            assert!((got - want).abs() <= tolerance(sample), "{mode:?}/{sample:?}/{blend:?}/x{x}: {got}/{want}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn changed_coverage_uses_hdr_nonseparable_blending() {
        let area = Rect::new(0, 0, 1, 1);
        let base = surface(PixelFormat::RGBA32F, [4., 4., 4., 0.5], area);
        let filtered = surface(PixelFormat::RGBA32F, [8., 8., 8., 0.25], area);
        let out = mix(&base, &filtered, area, BlendMode::Luminosity, 1., &Interrupt::NONE).unwrap();
        for (got, expected) in out.pixel(0, 0).into_iter().zip([5.6, 5.6, 5.6, 0.625]) {
            assert!((got - expected).abs() < 1e-5);
        }
    }

    #[test]
    /// Keep native HDR and opaque formats while limiting changes to the caller's explicit mixing extent.
    fn float_hdr_and_alpha_absent_native_data_are_preserved_and_extent_is_explicit() {
        let area = Rect::new(0, 0, 1, 1);
        for alpha in [false, true] {
            let format = PixelFormat::new(ColorMode::Rgb, SampleType::F32, alpha);
            let mut base = surface(format, [-0.5, 2., 0., 0.5], area);
            let filtered = surface(format, [1.5, 0.25, 3., 0.5], area);
            base.write_pixel(4, 0, &photocraft_raster::from_rgba(&format, [0.2, 0.3, 0.4, 0.5]));
            let mixed = mix(&base, &filtered, area, BlendMode::Normal, 0.25, &Interrupt::NONE).unwrap();
            let rgba = photocraft_raster::to_rgba(&format, &mixed.pixel(0, 0));
            assert_eq!(&rgba[..3], &[0., 1.5625, 0.75]);
            assert_eq!(rgba[3], if alpha { 0.5 } else { 1. });
            assert_eq!(mixed.pixel(4, 0), base.pixel(4, 0));
        }
    }

    #[test]
    /// Finite HDR multiplication saturates instead of publishing infinity through native Smart-filter mixing.
    fn hdr_multiply_overflow_stays_finite() {
        let area = Rect::new(0, 0, 1, 1);
        for alpha in [false, true] {
            let format = PixelFormat::new(ColorMode::Rgb, SampleType::F32, alpha);
            let base = surface(format, [2e19, -2e19, -2e19, 0.5], area);
            for filtered_alpha in [0.5, 0.25] {
                let filtered = surface(format, [2e19, 2e19, -2e19, filtered_alpha], area);
                let result = mix(&base, &filtered, area, BlendMode::Multiply, 1., &Interrupt::NONE).unwrap();
                let expected = if alpha && filtered_alpha == 0.25 { f32::MAX * 0.2 } else { f32::MAX };
                for (value, sign) in result.pixel(0, 0).iter().take(3).zip([1., -1., 1.]) {
                    assert!((*value - expected * sign).abs() <= f32::MAX * 1e-6);
                }
                assert!(result.pixel(0, 0).iter().all(|value| value.is_finite()));
            }
        }
    }

    #[test]
    fn normal_fractional_strength_preserves_identical_signed_max_pixels() {
        let area = Rect::new(0, 0, 1, 1);
        for value in [-f32::MAX, f32::MAX] {
            let base = surface(PixelFormat::RGBA32F, [value, value, value, 0.01], area);
            let result = mix(&base, &base, area, BlendMode::Normal, 0.03, &Interrupt::NONE).unwrap();
            let pixel = result.pixel(0, 0);
            assert!(pixel.iter().all(|value| value.is_finite()));
            assert_eq!(pixel, base.pixel(0, 0));
        }
    }

    #[test]
    fn nonfinite_native_samples_fail_atomically_even_in_later_tiles() {
        let area = Rect::new(0, 0, 257, 1);
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Multichannel] {
            for alpha in [false, true] {
                let format = PixelFormat::new(mode, SampleType::F32, alpha);
                let mut bp = vec![0.25; format.channels()];
                let mut tp = vec![0.75; format.channels()];
                if alpha {
                    *bp.last_mut().unwrap() = 0.5;
                    *tp.last_mut().unwrap() = 0.25;
                }
                let mut base = Surface::new(format);
                let mut filtered = Surface::new(format);
                base.fill_rect(area, &bp);
                filtered.fill_rect(area, &tp);
                for channel in 0..format.channels() {
                    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                        for bad_base in [false, true] {
                            let (mut b, mut t) = (base.clone(), filtered.clone());
                            let mut bad = if bad_base { bp.clone() } else { tp.clone() };
                            bad[channel] = value;
                            if bad_base { &mut b } else { &mut t }.write_pixel(256, 0, &bad);
                            let before = (b.clone(), t.clone());
                            for blend in [BlendMode::Normal, BlendMode::Multiply] {
                                assert_eq!(
                                    mix(&b, &t, area, blend, 0.5, &Interrupt::NONE),
                                    Err(Error::NonFiniteSample),
                                    "{format:?} channel={channel} base={bad_base} {blend:?}"
                                );
                            }
                            assert_eq!((b, t), before);
                        }
                    }
                }
            }
        }
    }

    #[test]
    /// Reject mismatched formats, invalid opacity and delayed cancellation without modifying either input surface.
    fn invalid_arguments_and_mid_work_cancellation_do_not_mutate_inputs() {
        let area = Rect::new(0, 0, 513, 1);
        let format = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
        let base = surface(format, [0.2, 0.4, 0.6, 0.5], area);
        let filtered = surface(format, [0.8, 0.6, 0.4, 0.5], area);
        let before = base.clone();
        for opacity in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            assert_eq!(mix(&base, &filtered, area, BlendMode::Normal, opacity, &Interrupt::NONE), Err(Error::InvalidOpacity));
        }
        assert_eq!(mix(&base, &Surface::new(PixelFormat::GRAY8), area, BlendMode::Normal, 0.5, &Interrupt::NONE), Err(Error::FormatMismatch));
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 3;
        assert_eq!(mix(&base, &filtered, area, BlendMode::Normal, 0.5, &Interrupt::cancel_only(&cancel)), Err(Error::Cancelled));
        assert_eq!(base, before);
        assert!(checks.load(Ordering::Relaxed) > 3);
    }
}
