//! Source relationships are translated once, never resource permissions or a live GIMP link.

use super::{Cursor, Pixels, Result, invalid};

/// Declarative XCF 3.2 link metadata. The locator is deliberately never resolved here.
pub(super) struct Link {
    pub flags: u32,
    pub locator: String,
    pub width: u32,
    pub height: u32,
}

pub(super) fn link(bytes: &[u8]) -> Result<Link> {
    let mut c = Cursor { b: bytes, pos: 0, wide: false };
    let flags = c.u32()?;
    let locator = c.string()?;
    let (width, height) = (c.u32()?, c.u32()?);
    if c.pos != bytes.len() || locator.is_empty() || width == 0 || height == 0 {
        return Err(invalid("invalid link-layer locator or dimensions"));
    }
    Ok(Link { flags, locator, width, height })
}

pub(super) struct Transform {
    pub offset: [i32; 2],
    pub interpolation: u32,
    pub matrix: [f32; 9],
}

pub(super) fn transform(bytes: &[u8]) -> Result<Transform> {
    let mut c = Cursor { b: bytes, pos: 0, wide: false };
    let offset = [i32::from_be_bytes(c.u32()?.to_be_bytes()), i32::from_be_bytes(c.u32()?.to_be_bytes())];
    let interpolation = c.u32()?;
    let mut matrix = [0.0; 9];
    for value in &mut matrix {
        *value = f32::from_bits(c.u32()?);
        if !value.is_finite() {
            return Err(invalid("a link-layer transform is not finite"));
        }
    }
    if c.pos != bytes.len() {
        return Err(invalid("invalid link-layer transform length"));
    }
    Ok(Transform { offset, interpolation, matrix })
}

pub(super) fn attachment(bytes: &[u8], wide: bool) -> Result<usize> {
    let mut c = Cursor { b: bytes, pos: 0, wide };
    let attachment = c.ptr()?;
    if c.pos != bytes.len() {
        return Err(invalid("invalid floating-selection attachment length"));
    }
    Ok(attachment)
}

/// A floating selection attached to a channel/mask has gray coverage even in an RGB image.
/// Repeating the channel creates an ordinary editable RGB layer; it does not modify its owner.
pub(super) fn gray_to_rgb(pixels: Pixels, budget: &mut usize, ctl: &photocraft_raster::Interrupt<'_>) -> Result<Pixels> {
    ctl.check().map_err(|_| crate::IoError::Cancelled)?;
    if !matches!(pixels.channels, 1 | 2) {
        return Err(invalid("floating gray pixels have an invalid channel count"));
    }
    let samples = usize::try_from(pixels.width)
        .ok()
        .and_then(|w| usize::try_from(pixels.height).ok().and_then(|h| w.checked_mul(h)))
        .ok_or_else(|| invalid("floating pixel dimensions overflow"))?;
    let expected = samples.checked_mul(pixels.channels).ok_or_else(|| invalid("floating pixel count overflow"))?;
    if pixels.width == 0 || pixels.height == 0 || pixels.data.len() != expected {
        return Err(invalid("floating gray buffer does not match its complete dimensions and channels"));
    }
    let extra = samples.checked_mul(2).and_then(|n| n.checked_mul(4)).ok_or_else(|| invalid("floating pixel count overflow"))?;
    let remaining = budget.checked_sub(extra).ok_or_else(|| invalid("the layers exceed the 1 GiB pixel limit"))?;
    let channels = pixels.channels + 2;
    let count = samples.checked_mul(channels).ok_or_else(|| invalid("floating pixel count overflow"))?;
    let mut data = Vec::new();
    data.try_reserve_exact(count).map_err(|_| invalid("floating gray expansion allocation is unavailable"))?;
    for (index, pixel) in pixels.data.chunks_exact(pixels.channels).enumerate() {
        if index.is_multiple_of(2048) {
            ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        }
        let gray = pixel.first().copied().ok_or_else(|| invalid("missing floating gray sample"))?;
        data.extend_from_slice(&[gray, gray, gray]);
        if let Some(alpha) = pixel.get(1) {
            data.push(*alpha);
        }
    }
    ctl.check().map_err(|_| crate::IoError::Cancelled)?;
    *budget = remaining;
    Ok(Pixels { channels, data, ..pixels })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xcf::{
        import,
        tests::{TestImage, TestLayer},
    };
    use photocraft_doc::{LayerContent, import_conversion::ImportConversion};
    use photocraft_geom::Rect;
    use photocraft_raster::Interrupt;

    fn source_link(locator: &str, flags: u32) -> Vec<u8> {
        let mut bytes = flags.to_be_bytes().to_vec();
        bytes.extend_from_slice(&(locator.len() as u32 + 1).to_be_bytes());
        bytes.extend_from_slice(locator.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&27u32.to_be_bytes());
        bytes.extend_from_slice(&19u32.to_be_bytes());
        bytes
    }

    fn source_transform() -> Vec<u8> {
        let mut bytes = (-3i32).to_be_bytes().to_vec();
        bytes.extend_from_slice(&7i32.to_be_bytes());
        bytes.extend_from_slice(&4u32.to_be_bytes());
        for value in [2.0f32, 0.0, 5.0, 0.0, 3.0, -4.0, 0.01, 0.02, 1.0] {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        bytes
    }

    #[test]
    fn link_configuration_is_bounded_and_validates_all_optional_fields() {
        let bytes = source_link("../../untrusted/source.png", 3);
        let metadata = link(&bytes).unwrap();
        assert_eq!((metadata.flags, metadata.width, metadata.height), (3, 27, 19));
        assert_eq!(metadata.locator, "../../untrusted/source.png");
        for length in 0..bytes.len() {
            assert!(link(&bytes[..length]).is_err());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(link(&trailing).is_err());
        assert!(link(&source_link("", 0)).is_err());
        let mut oversized = vec![0; 4];
        oversized.extend_from_slice(&u32::MAX.to_be_bytes());
        assert!(link(&oversized).is_err());
        let transform = source_transform();
        let decoded = super::transform(&transform).unwrap();
        assert_eq!(decoded.offset, [-3, 7]);
        assert_eq!(decoded.interpolation, 4);
        assert_eq!(decoded.matrix[6], 0.01);
        for length in 0..transform.len() {
            assert!(super::transform(&transform[..length]).is_err());
        }
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut bad = transform.clone();
            bad[12..16].copy_from_slice(&value.to_be_bytes());
            assert!(super::transform(&bad).is_err());
        }
        assert_eq!(attachment(&73u32.to_be_bytes(), false).unwrap(), 73);
        assert_eq!(attachment(&73u64.to_be_bytes(), true).unwrap(), 73);
        assert!(attachment(&73u32.to_be_bytes(), true).is_err());
    }

    #[test]
    fn floating_rgb_pixels_keep_negative_position_attachment_and_native_reopen() {
        for version in [0, 10, 11, 26] {
            let mut image = TestImage::rgb8(4, 3);
            image.version = version;
            let mut floating = TestLayer::rgba("Floating", 2, 1, vec![255, 0, 0, 128, 0, 0, 255, 255]);
            floating.x = -1;
            floating.y = 2;
            floating.props.push((5, if version >= 11 { 97u64.to_be_bytes().to_vec() } else { 97u32.to_be_bytes().to_vec() }));
            image.layers.push(floating);
            image.layers.push(TestLayer::rgba("Owner", 1, 1, vec![0, 255, 0, 255]));
            let result = import("floating.xcf", &image.write(), &Interrupt::NONE).unwrap();
            assert_eq!(result.document.layers.len(), 2);
            let layer = result.document.layers.iter().find(|layer| layer.name == "Floating").unwrap();
            let surface = layer.surface().unwrap();
            assert_eq!(surface.read_region(Rect::new(-1, 2, 1, 3)), vec![1.0, 0.0, 0.0, 128.0 / 255.0, 0.0, 0.0, 1.0, 1.0]);
            let Some(ImportConversion::Report(report)) = &result.document.metadata.import_conversion else { panic!("report") };
            let outcome = report.entries.iter().find(|entry| entry.feature_code == "xcf.floating_selection").unwrap();
            assert_eq!(outcome.targets[0].layer_id, Some(layer.id));
            assert!(outcome.source_description.contains("97"));
            assert!(report.attachments.iter().any(|attachment| attachment.kind == "xcf.property.5"));
            let native = photocraft_format::save_to_bytes(&result.document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&native).unwrap();
            assert_eq!(reopened.layers.len(), 2);
            assert_eq!(reopened.metadata.import_conversion, result.document.metadata.import_conversion);
            assert_eq!(reopened.layers.iter().find(|layer| layer.name == "Floating").unwrap().surface(), Some(surface));
        }
    }

    #[test]
    fn gray_floating_coverage_is_a_separate_rgb_layer_not_a_mode_change() {
        for alpha in [false, true] {
            let mut image = TestImage::rgb8(2, 1);
            let mut floating = TestLayer::rgba("Gray floating", 2, 1, if alpha { vec![32, 128, 224, 255] } else { vec![32, 224] });
            floating.kind = if alpha { 3 } else { 2 };
            floating.props.push((5, 128u64.to_be_bytes().to_vec()));
            image.layers.push(floating);
            image.layers.push(TestLayer::rgba("RGB owner", 1, 1, vec![255, 0, 0, 255]));
            image.channels.push(("Coverage owner", vec![80, 160], false));
            let result = import("gray-floating.xcf", &image.write(), &Interrupt::NONE).unwrap();
            assert_eq!(result.document.mode, photocraft_color::ColorMode::Rgb);
            let floating = result.document.layers.iter().find(|layer| layer.name == "Gray floating").unwrap();
            let expected =
                vec![32.0 / 255.0, 32.0 / 255.0, 32.0 / 255.0, if alpha { 128.0 / 255.0 } else { 1.0 }, 224.0 / 255.0, 224.0 / 255.0, 224.0 / 255.0, 1.0];
            assert_eq!(floating.surface().unwrap().read_region(Rect::new(0, 0, 2, 1)), expected);
            assert_eq!(result.document.channels[0].surface.read_region(Rect::new(0, 0, 2, 1)), vec![80.0 / 255.0, 160.0 / 255.0]);
            let owner = result.document.layers.iter().find(|layer| layer.name == "RGB owner").unwrap();
            assert_eq!(owner.surface().unwrap().read_region(Rect::new(0, 0, 1, 1)), vec![1.0, 0.0, 0.0, 1.0]);
        }
    }

    #[test]
    fn legacy_link_flags_use_one_native_group_and_leave_unlinked_siblings_independent() {
        let mut image = TestImage::rgb8(2, 1);
        for (name, linked) in [("A", true), ("B", true), ("Independent", false)] {
            let mut layer = TestLayer::rgba(name, 1, 1, vec![255, 0, 0, 255]);
            layer.props.push((9, u32::from(linked).to_be_bytes().to_vec()));
            image.layers.push(layer);
        }
        let result = import("links.xcf", &image.write(), &Interrupt::NONE).unwrap();
        for layer in &result.document.layers {
            assert_eq!(layer.link_group, (layer.name != "Independent").then_some(1));
        }
    }

    #[test]
    fn linked_stored_pixels_are_inert_raster_and_transform_is_not_applied_twice() {
        for flags in [0, 1, 2, 3, u32::MAX] {
            let mut image = TestImage::rgb8(3, 2);
            let mut layer = TestLayer::rgba("Source link", 1, 1, vec![10, 20, 30, 255]);
            layer.x = -2;
            layer.y = 1;
            layer.props = vec![(48, source_link("//untrusted.invalid/missing.png", flags)), (49, source_transform())];
            image.layers.push(layer);
            let result = import("resource.xcf", &image.write(), &Interrupt::NONE).unwrap();
            let layer = &result.document.layers[0];
            assert!(matches!(layer.content, LayerContent::Raster(_)));
            assert_eq!(layer.surface().unwrap().read_region(Rect::new(-2, 1, -1, 2)), vec![10.0 / 255.0, 20.0 / 255.0, 30.0 / 255.0, 1.0]);
            let Some(ImportConversion::Report(report)) = &result.document.metadata.import_conversion else { panic!("report") };
            assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.link_transform"));
            assert!(report.entries.iter().all(|entry| !entry.source_description.contains("untrusted.invalid")));
        }
    }

    #[test]
    fn missing_link_pixels_and_invalid_optional_placement_are_reported_without_external_reads() {
        let mut image = TestImage::rgb8(1, 1);
        let mut layer = TestLayer::rgba("Unavailable", 1, 1, Vec::new());
        layer.props = vec![(48, source_link("../../missing.png", 0)), (49, vec![0])];
        image.layers.push(layer);
        let result = import("unavailable.xcf", &image.write(), &Interrupt::NONE).unwrap();
        let Some(ImportConversion::Report(report)) = &result.document.metadata.import_conversion else { panic!("report") };
        assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.link_layer" && entry.reason == photocraft_doc::import_conversion::ConversionReason::ResourceUnavailable));
        assert!(report.entries.iter().any(
            |entry| entry.feature_code == "xcf.link_transform" && entry.reason == photocraft_doc::import_conversion::ConversionReason::InvalidOptionalData
        ));
        assert!(result.document.layers[0].surface().unwrap().content_bounds().is_empty());
    }

    #[test]
    fn floating_coverage_validation_and_gray_expansion_obey_native_allocation_budget() {
        let pixels = Pixels { width: 1, height: 1, channels: 2, data: vec![0.5, 1.0], underflow: false };
        assert!(gray_to_rgb(pixels, &mut 7, &Interrupt::NONE).is_err());
        let mut image = TestImage::rgb8(1, 1);
        image.precision = 650;
        let mut floating = TestLayer::rgba("Invalid floating coverage", 1, 1, [1.25f32, 1.0].into_iter().flat_map(f32::to_be_bytes).collect());
        floating.kind = 3;
        floating.props.push((5, 97u64.to_be_bytes().to_vec()));
        image.layers.push(floating);
        let error = super::super::import_with_sample_order("floating.xcf", &image.write(), Some(crate::HistoricalXcfSampleOrder::BigEndian), &Interrupt::NONE)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("coverage") && error.contains("outside the range"), "{error}");
    }
    #[test]
    fn gray_expansion_validates_complete_shapes_without_spending_budget() {
        for (width, height, channels, data) in [
            (1, 1, 2, vec![0.5]),
            (1, 1, 1, vec![0.5, 0.75]),
            (2, 2, 2, vec![0.; 7]),
            (0, 1, 1, vec![]),
            (u32::MAX, u32::MAX, 2, vec![]),
            (1, 1, 0, vec![]),
            (1, 1, 3, vec![0.; 3]),
        ] {
            let mut budget = usize::MAX;
            let result = gray_to_rgb(Pixels { width, height, channels, data, underflow: false }, &mut budget, &Interrupt::NONE);
            assert!(result.is_err());
            assert_eq!(budget, usize::MAX);
        }
    }
    #[test]
    fn gray_expansion_cancellation_is_transactional_before_during_and_at_end() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        for limit in [0, 2, 4] {
            let checks = AtomicUsize::new(0);
            let cancelled = || checks.fetch_add(1, Ordering::Relaxed) >= limit;
            let mut budget = 100_000;
            let result = gray_to_rgb(
                Pixels { width: 4097, height: 1, channels: 2, data: vec![0.5; 4097 * 2], underflow: false },
                &mut budget,
                &Interrupt::cancel_only(&cancelled),
            );
            assert!(matches!(result, Err(crate::IoError::Cancelled)));
            assert_eq!(budget, 100_000);
            assert_eq!(checks.load(Ordering::Relaxed), limit + 1);
        }
    }
    #[test]
    fn gray_expansion_preserves_native_hdr_alpha_dimensions_and_underflow() {
        for alpha in [false, true] {
            let mut budget = 24;
            let data = if alpha { vec![-0.25, 0.375, 3.5, 0.75] } else { vec![-0.25, 3.5] };
            let expanded =
                gray_to_rgb(Pixels { width: 2, height: 1, channels: if alpha { 2 } else { 1 }, data, underflow: true }, &mut budget, &Interrupt::NONE).unwrap();
            assert_eq!((expanded.width, expanded.height, expanded.channels, expanded.underflow), (2, 1, if alpha { 4 } else { 3 }, true));
            assert_eq!(expanded.data, if alpha { vec![-0.25, -0.25, -0.25, 0.375, 3.5, 3.5, 3.5, 0.75] } else { vec![-0.25, -0.25, -0.25, 3.5, 3.5, 3.5] });
            assert_eq!(budget, 8);
        }
        let mut budget = 15;
        assert!(gray_to_rgb(Pixels { width: 2, height: 1, channels: 1, data: vec![0.5; 2], underflow: false }, &mut budget, &Interrupt::NONE).is_err());
        assert_eq!(budget, 15);
    }
}
