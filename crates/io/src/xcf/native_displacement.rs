//! One-time conversion of native XY pixels into an ordinary embedded Displace resource.
//! The resulting Smart Filter contains only the native command and native PSD map data.
use super::effects::{NativeMapCoordinates, NativeMapping, native_filter};
use super::{IoError, Result, invalid};
use photocraft_algo::Image;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use serde_json::json;

#[derive(Clone, Copy)]
pub(super) struct Controls {
    pub coordinates: NativeMapCoordinates,
    pub scale: f32,
    pub wrap: bool,
    pub metadata_limit: usize,
}

pub(super) fn mapping(surface: &Surface, bounds: Rect, controls: Controls, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<NativeMapping> {
    let Controls { coordinates, scale, wrap, metadata_limit } = controls;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if bounds.is_empty() || !scale.is_finite() || !(0.0..=5000.0).contains(&scale) {
        return Err(invalid("native displacement requires a nonempty frame and finite bounded scale"));
    }
    let width = usize::try_from(bounds.width()).map_err(|_| invalid("native map width exceeds platform limits"))?;
    let height = usize::try_from(bounds.height()).map_err(|_| invalid("native map height exceeds platform limits"))?;
    let pixels = width.checked_mul(height).ok_or_else(|| invalid("native map area overflows"))?;
    let storage = pixels.checked_mul(96).and_then(|n| n.checked_add(32_768)).ok_or_else(|| invalid("native map admission overflows"))?;
    // Native embedded maps are two U8 planes encoded as hex in native params.
    if pixels > (1 << 24) || pixels.checked_mul(4).and_then(|n| n.checked_add(16_384)).is_none_or(|n| n > metadata_limit.min(super::MAX_METADATA_BYTES)) {
        return Err(IoError::Unsupported("native embedded displacement map exceeds native parameter storage limit".into()));
    }
    let remaining = budget.checked_sub(storage).ok_or_else(|| invalid("native displacement map exceeds remaining import allowance"))?;
    let count = pixels.checked_mul(3).ok_or_else(|| invalid("native map sample count overflows"))?;
    let mut data = Vec::new();
    data.try_reserve_exact(count).map_err(|_| invalid("native displacement map allocation failed"))?;
    data.resize(count, 0.5);
    let color_channels = surface.format().mode.color_channels();
    let channels = surface.channels();
    let mut maximum = [0.0_f64; 2];
    for y in bounds.y0..bounds.y1 {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let row = Rect::new(bounds.x0, y, bounds.x1, y.saturating_add(1));
        let samples = surface.try_read_region(row).map_err(|_| invalid("native displacement row allocation failed"))?;
        for (column, sample) in samples.chunks_exact(channels).enumerate() {
            let horizontal = f64::from(*sample.first().ok_or_else(|| invalid("native map has no first component"))?);
            let vertical = f64::from(*sample.get(if color_channels >= 2 { 1 } else { 0 }).ok_or_else(|| invalid("native map has no vertical component"))?);
            let x = f64::from(bounds.x0) + column as f64 + 0.5;
            let y_center = f64::from(y) + 0.5;
            let values = match coordinates {
                NativeMapCoordinates::Offsets => [horizontal * f64::from(scale), vertical * f64::from(scale)],
                NativeMapCoordinates::Positions => [(horizontal - x) * f64::from(scale), (vertical - y_center) * f64::from(scale)],
            };
            let line = usize::try_from(y - bounds.y0).map_err(|_| invalid("native map row index invalid"))?;
            let index = line
                .checked_mul(width)
                .and_then(|n| n.checked_add(column))
                .and_then(|n| n.checked_mul(3))
                .ok_or_else(|| invalid("native map index overflows"))?;
            let out = data.get_mut(index..index + 3).ok_or_else(|| invalid("native map output is short"))?;
            for axis in 0..2 {
                let value = *values.get(axis).ok_or_else(|| invalid("native map coordinate disappeared"))?;
                if !value.is_finite() || value.abs() > 999.0 * 1.28 {
                    return Err(IoError::Unsupported("native displacement offsets exceed editable native Displace range (+/-1278.72 pixels)".into()));
                }
                let maximum = maximum.get_mut(axis).ok_or_else(|| invalid("native map axis disappeared"))?;
                *maximum = maximum.max(value.abs());
                *out.get_mut(axis).ok_or_else(|| invalid("native map output axis disappeared"))? = value as f32;
            }
        }
    }
    let controls = maximum.map(|n| (n / 1.28).ceil().min(999.0));
    for (index, sample) in data.as_chunks_mut::<3>().0.iter_mut().enumerate() {
        if index % 4096 == 0 {
            ctl.check().map_err(|_| IoError::Cancelled)?;
        }
        for (axis, control) in controls.iter().enumerate() {
            let value = sample.get_mut(axis).ok_or_else(|| invalid("native map encoding axis disappeared"))?;
            *value = if *control == 0.0 { 0.5 } else { (0.5 + f64::from(*value) / (2.0 * control * 1.28)) as f32 };
        }
    }
    let native = json!({"horizontal":controls.first().copied().unwrap_or(0.0),"vertical":controls.get(1).copied().unwrap_or(0.0),"fit":"stretch","undefinedAreas":if wrap {"wrap"} else {"repeat"}});
    let params = crate::smart_map::displace_embed_map(&native, &Image { rect: bounds, ch: 3, data })
        .ok_or_else(|| invalid("native displacement map could not be embedded"))?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let result = NativeMapping {
        filters: vec![native_filter("filter.distort.displace", params)],
        notes: vec!["Actual current native auxiliary pixels were captured once into an ordinary embedded native Displace map. Absolute XY values use native pixel-center coordinates; relative values become native pixel offsets. Native two-plane U8 map quantization, integer scale controls, native bilinear sampling and native edge policy apply. XY resources become static editable native Smart Filter parameters; source graph bindings and source sampler/abyss behavior are not retained. Zero-displacement axes use native scale0 exactly.".into()],
        ..Default::default()
    };
    *budget = remaining;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    #[test]
    fn typed_native_embedded_map_endpoints_and_zero_axes() {
        let bounds = Rect::new(-2, -1, 1, 1);
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            let values = if depth == SampleType::F32 { [1.28, -1.28, 0., 0.4] } else { [1., 0., 0., 0.4] };
            source.try_write_region(bounds, &values.repeat(6)).unwrap();
            let mut budget = 1 << 24;
            let mapped = mapping(
                &source,
                bounds,
                Controls { coordinates: NativeMapCoordinates::Offsets, scale: 1., wrap: false, metadata_limit: 20_000 },
                &mut budget,
                &Interrupt::NONE,
            )
            .unwrap();
            let filter = &mapped.filters[0];
            assert_eq!(filter.command, "filter.distort.displace");
            assert_eq!(filter.params["horizontal"], 1.);
            assert_eq!(filter.params["vertical"], if depth == SampleType::F32 { 1. } else { 0. });
            let native = crate::smart_map::displace_embedded_map(&filter.params).unwrap();
            assert_eq!((native.rect.width(), native.rect.height()), (3, 2));
            if depth == SampleType::F32 {
                assert_eq!(&native.data[..3], &[1., 0., 0.5]);
            }
            assert!(budget < 1 << 24);
            let mut zero = Surface::new(source.format());
            zero.try_write_region(bounds, &[0., 0., 0., 1.].repeat(6)).unwrap();
            let zero = mapping(
                &zero,
                bounds,
                Controls { coordinates: NativeMapCoordinates::Offsets, scale: 1., wrap: true, metadata_limit: 20_000 },
                &mut budget,
                &Interrupt::NONE,
            )
            .unwrap();
            assert_eq!(zero.filters[0].params["horizontal"], 0.);
            assert_eq!(zero.filters[0].params["vertical"], 0.);
            assert_eq!(zero.filters[0].params["undefinedAreas"], "wrap");
        }
    }
    #[test]
    fn positions_are_one_time_native_resources_and_admission_is_atomic() {
        let bounds = Rect::new(-2, -1, 1, 1);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        let mut data = Vec::new();
        for y in bounds.y0..bounds.y1 {
            for x in bounds.x0..bounds.x1 {
                data.extend([x as f32 + 0.5, y as f32 + 0.5, 0., 1.]);
            }
        }
        source.try_write_region(bounds, &data).unwrap();
        let mut budget = 1 << 24;
        let native = mapping(
            &source,
            bounds,
            Controls { coordinates: NativeMapCoordinates::Positions, scale: 1., wrap: false, metadata_limit: 20_000 },
            &mut budget,
            &Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(native.filters[0].params["horizontal"], 0.);
        assert_eq!(native.filters[0].params["vertical"], 0.);
        let mut tiny = 0;
        assert!(
            mapping(
                &source,
                bounds,
                Controls { coordinates: NativeMapCoordinates::Offsets, scale: 1., wrap: false, metadata_limit: 20_000 },
                &mut tiny,
                &Interrupt::NONE
            )
            .is_err()
        );
        assert_eq!(tiny, 0);
        let before = budget;
        assert!(
            mapping(
                &source,
                bounds,
                Controls { coordinates: NativeMapCoordinates::Offsets, scale: 1., wrap: false, metadata_limit: 1 },
                &mut budget,
                &Interrupt::NONE
            )
            .is_err()
        );
        assert_eq!(budget, before);
        let cancelled = || true;
        assert!(matches!(
            mapping(
                &source,
                bounds,
                Controls { coordinates: NativeMapCoordinates::Positions, scale: 1., wrap: false, metadata_limit: 20_000 },
                &mut budget,
                &Interrupt::cancel_only(&cancelled)
            ),
            Err(IoError::Cancelled)
        ));
        assert_eq!(budget, before);
    }
}
