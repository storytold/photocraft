//! Ordinary Smart-filter strength and blend mixing, without adding coverage twice.
//!
//! Callers own operation admission and authority. Mixing uses bounded tile scratch and
//! publishes its staged surface only on success; it never reads a source document.

use std::fmt;

use photocraft_color::{BlendMode, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{AllocationError, Interrupt, Surface};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Cancelled,
    Allocation(AllocationError),
    FormatMismatch,
    InvalidOpacity,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Smart filter mixing cancelled"),
            Self::Allocation(error) => error.fmt(f),
            Self::FormatMismatch => f.write_str("Smart filter input and result formats differ"),
            Self::InvalidOpacity => f.write_str("Smart filter opacity must be finite and between zero and one"),
        }
    }
}

impl std::error::Error for Error {}

impl From<AllocationError> for Error {
    fn from(error: AllocationError) -> Self {
        Self::Allocation(error)
    }
}

fn check(ctl: &Interrupt<'_>) -> Result<(), Error> {
    ctl.check().map_err(|_| Error::Cancelled)
}

/// Mix an ordinary filter result with its input over `area`, preserving base data outside it.
///
/// `area` is the caller's native mixing extent, normally the union of input/result content
/// bounds. Non-Normal coverage changes classify the entire extent before mixing any pixel.
/// Alpha-preserving filters instead edit existing coverage, including at fractional opacity.
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
    // An unchanged-alpha pixel must use the same rule as a changed-alpha blur edge.
    // This classification is global, as in the original native Smart-filter mixer.
    let mut composite_coverage = false;
    if mode != BlendMode::Normal && fmt.alpha {
        for tile in area.tiles() {
            check(ctl)?;
            let rect = tile.rect().intersect(&area);
            let b = base.try_read_region(rect)?;
            let t = filtered.try_read_region(rect)?;
            if b.chunks_exact(n).zip(t.chunks_exact(n)).any(|(bp, tp)| bp.last() != tp.last()) {
                composite_coverage = true;
                break;
            }
        }
    }
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
            let b = photocraft_raster::to_rgba(&fmt, bp);
            let t = photocraft_raster::to_rgba(&fmt, tp);
            if composite_coverage {
                let rgba = photocraft_color::blend::composite(mode, b, t, opacity);
                photocraft_raster::from_rgba_into(&fmt, rgba, op);
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
                }
            }
            photocraft_raster::from_rgba_into(&fmt, rgba, op);
        }
        check(ctl)?;
        out.try_write_region(rect, &values)?;
    }
    check(ctl)?;
    out.prune();
    check(ctl)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn surface(format: PixelFormat, rgba: [f32; 4], rect: Rect) -> Surface {
        let mut surface = Surface::new(format);
        surface.write_region(rect, &photocraft_raster::from_rgba(&format, rgba).repeat((rect.width() * rect.height()) as usize));
        surface
    }

    fn tolerance(sample: SampleType) -> f32 {
        match sample {
            SampleType::U8 => 2. / 255.,
            SampleType::U16 => 2. / 65535.,
            SampleType::F32 => 0.000001,
        }
    }

    #[test]
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
    fn non_normal_coverage_classification_is_global_across_tiles_and_matches_native_composite() {
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
                        let expected = photocraft_color::blend::composite(blend, b, t, 0.5);
                        let expected = surface(format, expected, Rect::new(x, 0, x + 1, 1));
                        assert_eq!(mixed.pixel(x, 0), expected.pixel(x, 0), "{mode:?}/{sample:?}/{blend:?}/x{x}");
                    }
                }
            }
        }
    }

    #[test]
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
