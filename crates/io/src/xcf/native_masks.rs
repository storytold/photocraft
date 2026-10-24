//! Static ordinary native channels and masks from already prepared native pixels.

use super::effects::NativeMaskComponent;
use super::{Result, coverage, invalid};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::LayerMask;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

fn check(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| super::IoError::Cancelled)
}

fn extract(
    source: &Surface,
    bounds: Rect,
    component: NativeMaskComponent,
    output_format: PixelFormat,
    adjustment: (bool, f32),
    budget: &mut usize,
    ctl: &Interrupt<'_>,
) -> Result<Surface> {
    check(ctl)?;
    let (invert, scale) = adjustment;
    if !scale.is_finite() || !(0.0..=1.0).contains(&scale) {
        return Err(invalid("native mask scale must be finite in 0..1"));
    }
    let format = source.format();
    let colors = format.mode.color_channels();
    if colors == 0 || source.channels() != colors + usize::from(format.alpha) {
        return Err(invalid("native channel source has an inconsistent format"));
    }
    let mut remaining = *budget;
    coverage::reserve_native_frame(bounds, output_format, &mut remaining)?;
    let mut output = Surface::new(output_format);
    for tile in bounds.tiles() {
        check(ctl)?;
        let rect = tile.rect().intersect(&bounds);
        let input = source.try_read_region(rect).map_err(|error| invalid(format!("native channel source read: {error}")))?;
        if input.iter().any(|sample| !sample.is_finite()) {
            return Err(invalid("native channel source contains nonfinite samples"));
        }
        let pixels = input.len() / source.channels();
        let count = pixels.checked_mul(output_format.channels()).ok_or_else(|| invalid("native channel sample count overflows"))?;
        let bytes = count.checked_mul(std::mem::size_of::<f32>()).ok_or_else(|| invalid("native channel scratch bytes overflow"))?;
        let mut values = Vec::new();
        values.try_reserve_exact(count).map_err(|_| invalid(format!("native channel scratch allocation unavailable ({bytes} bytes)")))?;
        for pixel in input.chunks_exact(source.channels()) {
            let selected = match component {
                NativeMaskComponent::Alpha => {
                    if format.alpha {
                        pixel.get(colors).copied().ok_or_else(|| invalid("native alpha component is absent"))?
                    } else {
                        1.
                    }
                }
                NativeMaskComponent::Composite => {
                    let color = pixel.get(..colors).ok_or_else(|| invalid("native composite color components are absent"))?;
                    // Keep ordinary native Apply Image arithmetic for representable sums.
                    // A finite HDR mean can exist even if its intermediate f32 sum overflows.
                    let sum = color.iter().sum::<f32>();
                    if sum.is_finite() { sum / colors as f32 } else { (color.iter().map(|value| f64::from(*value)).sum::<f64>() / colors as f64) as f32 }
                }
            }
            .clamp(0., 1.);
            let value = (if invert { 1. - selected } else { selected }) * scale;
            values.extend(std::iter::repeat_n(value, output_format.mode.color_channels()));
            if output_format.alpha {
                values.push(1.);
            }
        }
        check(ctl)?;
        output.try_write_region(rect, &values).map_err(|error| invalid(format!("native channel output write: {error}")))?;
    }
    check(ctl)?;
    *budget = remaining;
    Ok(output)
}

/// Copy a native component into every native color plane, with opaque alpha when present.
/// `bounds` retains the actual owner frame, including transparent/default border pixels.
pub(super) fn channel(source: &Surface, bounds: Rect, component: NativeMaskComponent, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Surface> {
    extract(source, bounds, component, source.format(), (false, 1.), budget, ctl)
}

/// Create ordinary normalized GrayF32 mask data, with inversion before coverage scaling.
pub(super) fn mask(
    source: &Surface,
    bounds: Rect,
    component: NativeMaskComponent,
    invert: bool,
    scale: f32,
    budget: &mut usize,
    ctl: &Interrupt<'_>,
) -> Result<LayerMask> {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
    let surface = extract(source, bounds, component, format, (invert, scale), budget, ctl)?;
    Ok(LayerMask { surface, enabled: true, linked: true, density: 1., feather: 0. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn components_match_native_color_plane_mean_and_transparency_at_all_depths() {
        let bounds = Rect::new(-2, 3, 2, 5);
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for depth in SampleType::ALL {
                let format = PixelFormat::new(mode, depth, true);
                let mut source = Surface::new(format);
                let rgba = photocraft_raster::from_rgba(&format, [0.25, 0.5, 0.75, 0.5]);
                source.write_pixel(-1, 3, &rgba);
                let stored = source.pixel(-1, 3);
                let expected_mean = if mode == ColorMode::Rgb { (stored[0] + stored[1] + stored[2]) / 3. } else { stored[0] };
                for (component, expected) in [(NativeMaskComponent::Alpha, *stored.last().unwrap()), (NativeMaskComponent::Composite, expected_mean)] {
                    let mut budget = 1 << 28;
                    let selected = channel(&source, bounds, component, &mut budget, &Interrupt::NONE).unwrap();
                    assert_eq!(selected.format(), format);
                    let tolerance = match depth {
                        SampleType::U8 => 1. / 255.,
                        SampleType::U16 => 1. / 65535.,
                        SampleType::F32 => 0.000001,
                    };
                    let pixel = selected.pixel(-1, 3);
                    for value in &pixel[..mode.color_channels()] {
                        assert!((*value - expected).abs() <= tolerance);
                    }
                    assert_eq!(*pixel.last().unwrap(), 1.);
                    let mask = mask(&source, bounds, component, true, 0.4, &mut budget, &Interrupt::NONE).unwrap();
                    assert_eq!(mask.surface.format(), PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
                    assert!(mask.enabled && mask.linked);
                    assert_eq!((mask.density, mask.feather), (1., 0.));
                    assert!((mask.value(-1, 3) - (1. - expected) * 0.4).abs() < 0.000001);
                    assert_eq!(mask.surface.sample_channel(-2, 3, 0), 0.4, "transparent frame border must survive inversion");
                    // Ordinary LayerMask::value applies 1-density*(1-coverage), adding f32 roundoff.
                    assert!((mask.value(-2, 3) - 0.4).abs() <= f32::EPSILON);
                    assert_eq!(mask.value(2, 3), 0., "outside explicit frame remains hidden");
                    let mut doc = photocraft_doc::Document::new("Independent native static mask", photocraft_geom::Size::new(4, 2), mode, depth);
                    let mut layer = photocraft_doc::Layer::new("Original editable contents", photocraft_doc::LayerContent::Raster(source.clone()));
                    layer.mask = Some(mask.clone());
                    doc.layers.push(layer);
                    let reopened = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap()).unwrap();
                    assert_eq!(reopened.layers[0].mask, Some(mask));
                }
            }
        }
    }

    #[test]
    fn missing_alpha_is_opaque_and_hdr_mean_is_normalized_after_native_selection() {
        let bounds = Rect::new(0, 0, 2, 1);
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for depth in SampleType::ALL {
                let source = Surface::new(PixelFormat::new(mode, depth, false));
                let mut budget = 1 << 28;
                let alpha = channel(&source, bounds, NativeMaskComponent::Alpha, &mut budget, &Interrupt::NONE).unwrap();
                assert!(alpha.read_region(bounds).iter().all(|value| *value == 1.));
                let hidden = mask(&source, bounds, NativeMaskComponent::Alpha, true, 1., &mut budget, &Interrupt::NONE).unwrap();
                assert!(hidden.surface.read_region(bounds).iter().all(|value| *value == 0.));
            }
        }
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        source.write_pixel(0, 0, &[2., -1., 0.5, 0.25]);
        source.write_pixel(1, 0, &[f32::MAX, f32::MAX, -f32::MAX, 0.75]);
        let selected = channel(&source, bounds, NativeMaskComponent::Composite, &mut (1 << 28), &Interrupt::NONE).unwrap();
        assert_eq!(selected.pixel(0, 0), [0.5, 0.5, 0.5, 1.]);
        assert_eq!(selected.pixel(1, 0), [1., 1., 1., 1.]);
    }

    #[test]
    fn budget_invalid_data_and_cancellation_leave_original_inputs_and_ledgers_unchanged() {
        let bounds = Rect::new(0, 0, 513, 1);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        source.write_pixel(0, 0, &[0.2, 0.4, 0.6, 0.5]);
        let original = source.clone();
        let mut tiny = 1;
        assert!(channel(&source, bounds, NativeMaskComponent::Alpha, &mut tiny, &Interrupt::NONE).is_err());
        assert_eq!(tiny, 1);
        let mut budget = 1 << 28;
        let before = budget;
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 3;
        assert!(matches!(
            mask(&source, bounds, NativeMaskComponent::Alpha, true, 1., &mut budget, &Interrupt::cancel_only(&cancel)),
            Err(super::super::IoError::Cancelled)
        ));
        assert_eq!(budget, before);
        assert_eq!(source, original);
        for scale in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            assert!(mask(&source, bounds, NativeMaskComponent::Composite, false, scale, &mut budget, &Interrupt::NONE).is_err());
            assert_eq!(budget, before);
        }
        source.write_pixel(0, 0, &[f32::NAN, 0., 0., 1.]);
        assert!(channel(&source, bounds, NativeMaskComponent::Alpha, &mut budget, &Interrupt::NONE).is_err());
        assert_eq!(budget, before);
    }
}
