//! Document-to-AVIF export: colour management, fidelity notes and the optional build feature.

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_io::{ExportOptions, export};
use photocraft_raster::Surface;

fn columns(mode: ColorMode, depth: SampleType, pixels: &[&[f32]]) -> Document {
    let mut doc = Document::new("AVIF export", Size::new(16 * pixels.len() as u32, 16), mode, depth);
    let mut surface = Surface::new(doc.pixel_format());
    for (i, pixel) in pixels.iter().enumerate() {
        let x = 16 * i as i32;
        surface.fill_rect(Rect::new(x, 0, x + 16, 16), pixel);
    }
    doc.layers.push(Layer::new("Pixels", LayerContent::Raster(surface)));
    doc
}

#[test]
fn avif_rejects_an_invalid_canvas_before_pixel_buffer_allocation() {
    // Sparse metadata only. Flattening this F32 canvas would overflow the buffer size;
    // AVIF must instead report its encoder/format restriction before entering that path.
    let doc = Document::new("invalid AVIF canvas", Size::new(u32::MAX, u32::MAX), ColorMode::Rgb, SampleType::F32);
    let error = export(&doc, "avif", &ExportOptions::default()).unwrap_err().to_string();
    if cfg!(feature = "avif") {
        assert!(error.contains("format maximum of 65535x65535"), "{error}");
    } else {
        assert!(error.contains("AVIF cannot be written in this build"), "{error}");
    }
}

#[cfg(not(feature = "avif"))]
#[test]
fn avif_without_the_feature_has_an_actionable_error() {
    let doc = columns(ColorMode::Rgb, SampleType::U8, &[&[0.2, 0.4, 0.6, 1.0]]);
    for name in ["avif", ".avif", "export.AVIF"] {
        let error = export(&doc, name, &ExportOptions::default()).unwrap_err().to_string();
        assert!(error.contains("AVIF") && error.contains("cannot be written in this build"), "{name}: {error}");
    }
}

#[cfg(feature = "avif")]
mod enabled {
    use super::*;
    use photocraft_cms::{Builtin, Intent, Transform};
    use photocraft_codecs::{CodecError, Format};
    use photocraft_io::{IoError, XmpEmbed, import};

    fn quality(q: u8) -> ExportOptions {
        let mut options = ExportOptions::default();
        options.encode.jpeg_quality = q;
        options
    }

    /// Traverse only container boxes. This checks the actual file's image dimensions and
    /// alpha/depth properties without relying on PhotoCraft's unsupported AVIF decoder.
    fn properties<'a>(bytes: &'a [u8], found: &mut Vec<([u8; 4], &'a [u8])>) {
        let mut remaining = bytes;
        while !remaining.is_empty() {
            assert!(remaining.len() >= 8, "truncated BMFF box");
            let size = u32::from_be_bytes(remaining[..4].try_into().unwrap()) as usize;
            assert!(size >= 8 && size <= remaining.len(), "invalid BMFF box size {size}");
            let kind: [u8; 4] = remaining[4..8].try_into().unwrap();
            let body = &remaining[8..size];
            found.push((kind, body));
            match &kind {
                b"meta" => properties(&body[4..], found), // FullBox version and flags.
                b"iprp" | b"ipco" => properties(body, found),
                _ => {}
            }
            remaining = &remaining[size..];
        }
    }

    fn assert_container(bytes: &[u8], size: Size, alpha: bool) {
        assert_eq!(photocraft_codecs::detect(bytes), Some(Format::Avif));
        let mut boxes = Vec::new();
        properties(bytes, &mut boxes);
        assert!(boxes.iter().any(|(kind, body)| kind == b"mdat" && !body.is_empty()));
        let dimensions: Vec<(u32, u32)> = boxes
            .iter()
            .filter(|(kind, _)| kind == b"ispe")
            .map(|(_, body)| (u32::from_be_bytes(body[4..8].try_into().unwrap()), u32::from_be_bytes(body[8..12].try_into().unwrap())))
            .collect();
        assert!(!dimensions.is_empty());
        assert!(dimensions.iter().all(|&s| s == (size.width, size.height)), "{dimensions:?}");
        let depths: Vec<u8> = boxes.iter().filter(|(kind, _)| kind == b"pixi").flat_map(|(_, body)| body[5..].iter().copied()).collect();
        assert!(!depths.is_empty() && depths.iter().all(|&d| d == 8), "{depths:?}");
        let has_alpha = boxes.iter().any(|(kind, body)| kind == b"auxC" && body.windows(5).any(|w| w == b"alpha"));
        assert_eq!(has_alpha, alpha, "alpha auxiliary image");
    }

    /// Optional artifacts for a libavif/browser pixel oracle. No production decoder is added:
    /// the PNG reference holds the sRGB pixels that an independent AVIF decoder should show.
    fn oracle_case(name: &str, avif: &[u8], expected: &Document) {
        if let Some(directory) = std::env::var_os("PHOTOCRAFT_AVIF_ORACLE_DIR") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(format!("io-{name}.avif")), avif).unwrap();
            let png = export(expected, "png", &ExportOptions::default()).unwrap();
            std::fs::write(directory.join(format!("io-{name}.png")), png.bytes).unwrap();
        }
    }

    #[test]
    fn document_exports_rgb_and_transparency_without_matting() {
        for alpha in [false, true] {
            let doc = columns(
                ColorMode::Rgb,
                SampleType::U8,
                &[&[0.8, 0.2, 0.1, 1.0], &[0.2, 0.6, 0.8, if alpha { 0.5 } else { 1.0 }], &[0.1, 0.2, 0.3, if alpha { 0.0 } else { 1.0 }]],
            );
            let result = export(&doc, "export.AVIF", &quality(100)).unwrap();
            assert_container(&result.bytes, doc.size, alpha);
            assert!(result.warnings.iter().any(|w| w == "lossy compression"), "{:?}", result.warnings);
            assert!(!result.warnings.iter().any(|w| w.contains("alpha") || w.contains("composited over white")), "{:?}", result.warnings);
            oracle_case(if alpha { "rgba" } else { "rgb" }, &result.bytes, &doc);
            // Export availability must not advertise an AVIF reader that does not exist.
            assert!(matches!(import("export.avif", &result.bytes), Err(IoError::Codec(CodecError::Unsupported { format: Format::Avif, .. }))));
        }
    }

    #[test]
    fn layered_export_flattens_opacity_and_alpha_before_colour_conversion() {
        let mut doc = columns(ColorMode::Rgb, SampleType::U8, &[&[0.2, 0.4, 0.6, 0.5], &[0.1, 0.3, 0.7, 1.0]]);
        doc.icc_profile = Some(Builtin::DisplayP3.profile().to_bytes());
        let mut top = Surface::new(doc.pixel_format());
        top.fill_rect(Rect::new(0, 0, 16, 16), &[0.8, 0.1, 0.2, 0.5]);
        let backdrop = doc.layers[0].surface().unwrap().pixel(8, 8);
        let source = top.pixel(8, 8);
        let right = doc.layers[0].surface().unwrap().pixel(24, 8);
        let mut layer = Layer::new("Half opacity", LayerContent::Raster(top));
        layer.opacity = 0.5;
        doc.layers.push(layer);
        let mut hidden = Surface::new(doc.pixel_format());
        hidden.fill_rect(doc.bounds(), &[0.0, 1.0, 0.0, 1.0]);
        let mut layer = Layer::new("Hidden green", LayerContent::Raster(hidden));
        layer.visible = false;
        doc.layers.push(layer);

        let result = export(&doc, "avif", &quality(100)).unwrap();
        assert_container(&result.bytes, doc.size, true);
        assert!(result.warnings.iter().any(|w| w.starts_with("3 layer(s) flattened")), "{:?}", result.warnings);
        assert!(result.warnings.iter().any(|w| w.contains("colours converted to sRGB")), "{:?}", result.warnings);

        // Independent straight-alpha Normal source-over, including layer opacity. This avoids
        // using the compositor itself as the expected result of the multi-layer export path.
        let source_alpha = source[3] * 0.5;
        let alpha = source_alpha + backdrop[3] * (1.0 - source_alpha);
        let mut left = vec![0.0; 4];
        for channel in 0..3 {
            left[channel] = (source[channel] * source_alpha + backdrop[channel] * backdrop[3] * (1.0 - source_alpha)) / alpha;
        }
        left[3] = alpha;
        // Composite quantization occurs at the document depth before the ICC transform.
        let mut reference = columns(ColorMode::Rgb, SampleType::U8, &[&left, &right]);
        let mut pixels = reference.layers[0].surface().unwrap().read_region(reference.bounds());
        Transform::new(Builtin::DisplayP3.profile(), Builtin::Srgb.profile(), Intent::Perceptual, true).unwrap().apply(&mut pixels, 4);
        let bounds = reference.bounds();
        reference.layers[0].surface_mut().unwrap().write_region(bounds, &pixels);
        assert_eq!(result.bytes, export(&reference, "avif", &quality(100)).unwrap().bytes, "multi-layer AVIF differs from explicit source-over/CMS reference");
        oracle_case("layers-p3-srgb", &result.bytes, &reference);
    }

    #[test]
    fn cmyk_export_uses_the_document_profile_and_preserves_alpha() {
        let mut doc = columns(ColorMode::Cmyk, SampleType::U8, &[&[0.2, 0.4, 0.6, 0.1, 1.0], &[0.8, 0.1, 0.0, 0.2, 0.5]]);
        doc.icc_profile = Some(Builtin::CoatedCmyk.profile().to_bytes());
        let result = export(&doc, "avif", &quality(100)).unwrap();
        assert_container(&result.bytes, doc.size, true);
        assert!(result.warnings.iter().any(|w| w.contains("CMYK converted to sRGB") && w.contains("document's colour profile")), "{:?}", result.warnings);
        assert!(!result.warnings.iter().any(|w| w.contains("not colour-managed")), "{:?}", result.warnings);

        // Build a separate RGB document from an explicit ICC transform, bypassing flat export's
        // native-CMYK conversion. Both documents must feed identical sRGB samples to the encoder.
        let source = doc.layers[0].surface().unwrap();
        let transform = Transform::new(Builtin::CoatedCmyk.profile(), Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).unwrap();
        let input = source.read_region(doc.bounds());
        let mut rgb = vec![0.0; (doc.size.area() * 4) as usize];
        transform.convert_f32(&input, 5, &mut rgb, 4, true);
        let mut reference = Document::new("sRGB", doc.size, ColorMode::Rgb, SampleType::U8);
        let mut surface = Surface::new(reference.pixel_format());
        surface.write_region(reference.bounds(), &rgb);
        reference.layers.push(Layer::new("Converted", LayerContent::Raster(surface)));
        let reference_bytes = export(&reference, "avif", &quality(100)).unwrap().bytes;
        assert_eq!(result.bytes, reference_bytes, "CMYK export differs from the explicit colour-managed reference");
        oracle_case("cmyk-srgb", &result.bytes, &reference);
    }

    #[test]
    fn non_srgb_export_converts_colours_before_dropping_the_profile() {
        let mut doc = columns(ColorMode::Rgb, SampleType::U8, &[&[0.2, 0.5, 0.8, 1.0], &[0.7, 0.3, 0.2, 0.5]]);
        doc.icc_profile = Some(Builtin::DisplayP3.profile().to_bytes());
        let result = export(&doc, "avif", &quality(100)).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("colours converted to sRGB")), "{:?}", result.warnings);
        let mut reference = doc.clone();
        let mut pixels = doc.layers[0].surface().unwrap().read_region(doc.bounds());
        Transform::new(Builtin::DisplayP3.profile(), Builtin::Srgb.profile(), Intent::Perceptual, true).unwrap().apply(&mut pixels, 4);
        reference.icc_profile = None;
        let bounds = reference.bounds();
        reference.layers[0].surface_mut().unwrap().write_region(bounds, &pixels);
        assert_eq!(result.bytes, export(&reference, "avif", &quality(100)).unwrap().bytes);
        oracle_case("p3-srgb", &result.bytes, &reference);
    }

    #[test]
    fn tagged_grayscale_uses_its_tone_curve_before_rgb_expansion() {
        let mut linear = Builtin::GrayGamma22.profile().clone();
        linear.description = "Synthetic linear gray".into();
        linear.gray_trc = Some(photocraft_cms::Curve::Gamma(1.0));
        let linear = linear.with_encoded_bytes();
        let profiles = [("gray-gamma22", Builtin::GrayGamma22.profile()), ("gray-linear", &linear)];
        for (name, profile) in profiles {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let mut doc = columns(ColorMode::Grayscale, depth, &[&[0.08, 1.0], &[0.25, 0.5], &[0.6, 1.0]]);
                doc.icc_profile = Some(profile.to_bytes());
                let result = export(&doc, "avif", &quality(100)).unwrap();
                assert_container(&result.bytes, doc.size, true);
                assert!(result.warnings.iter().any(|w| w.contains("colours converted to sRGB")), "{name} {depth:?}: {:?}", result.warnings);

                let mut values = doc.layers[0].surface().unwrap().read_region(doc.bounds());
                Transform::new(profile, Builtin::SGray.profile(), Intent::Perceptual, true).unwrap().apply(&mut values, 2);
                let mut reference = Document::new("sGray reference", doc.size, ColorMode::Grayscale, depth);
                let mut surface = Surface::new(reference.pixel_format());
                surface.write_region(reference.bounds(), &values);
                reference.layers.push(Layer::new("sGray", LayerContent::Raster(surface)));
                assert_eq!(result.bytes, export(&reference, "avif", &quality(100)).unwrap().bytes, "{name} {depth:?}: gray ICC conversion");

                // Independent viewers should see the neutral sRGB values, with the original alpha.
                let quantized = reference.layers[0].surface().unwrap().read_region(reference.bounds());
                let rgba: Vec<f32> = quantized.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect();
                let mut expected = Document::new("neutral sRGB", doc.size, ColorMode::Rgb, SampleType::U8);
                let mut surface = Surface::new(expected.pixel_format());
                surface.write_region(expected.bounds(), &rgba);
                expected.layers.push(Layer::new("Neutral", LayerContent::Raster(surface)));
                oracle_case(&format!("{name}-{depth:?}"), &result.bytes, &expected);
            }
        }
    }

    #[test]
    fn untagged_invalid_or_sgray_profiles_keep_grayscale_values() {
        let base = columns(ColorMode::Grayscale, SampleType::U8, &[&[0.08, 1.0], &[0.25, 0.5], &[0.6, 1.0]]);
        let expected = export(&base, "avif", &quality(100)).unwrap().bytes;
        for profile in [Some(std::sync::Arc::new(b"invalid ICC".to_vec())), Some(Builtin::SGray.profile().to_bytes()), Some(Builtin::Srgb.profile().to_bytes())]
        {
            let mut doc = base.clone();
            doc.icc_profile = profile;
            let result = export(&doc, "avif", &quality(100)).unwrap();
            assert_eq!(result.bytes, expected);
            assert!(!result.warnings.iter().any(|w| w.contains("converted to sRGB")), "{:?}", result.warnings);
        }
        // The AVIF-specific fix must leave native grayscale/profile exports unchanged.
        let mut tagged = base;
        tagged.icc_profile = Some(Builtin::GrayGamma22.profile().to_bytes());
        let png = export(&tagged, "png", &ExportOptions::default()).unwrap();
        let back = import("gray.png", &png.bytes).unwrap().document;
        assert_eq!(back.icc_profile, tagged.icc_profile);
        assert_eq!(back.layers[0].surface().unwrap().read_region(back.bounds()), tagged.layers[0].surface().unwrap().read_region(tagged.bounds()));
    }

    #[test]
    fn higher_depth_export_reports_quantization_and_hdr_clipping() {
        for depth in [SampleType::U16, SampleType::F32] {
            let doc = columns(ColorMode::Rgb, depth, &[&[0.2, 0.4, 0.6, 0.5], &[if depth == SampleType::F32 { 1.5 } else { 1.0 }, 0.0, 0.5, 1.0]]);
            let result = export(&doc, "avif", &quality(100)).unwrap();
            assert_container(&result.bytes, doc.size, true);
            let from = if depth == SampleType::U16 { "16-bit" } else { "32-bit float" };
            assert!(result.warnings.iter().any(|w| w == &format!("{from} will be reduced to 8-bit")), "{:?}", result.warnings);
            assert_eq!(result.warnings.iter().any(|w| w.contains("HDR values outside 0..1 will be clipped")), depth == SampleType::F32);
            // The independent decoder checks actual 8-bit quantization, alpha and HDR clipping.
            let reference = columns(ColorMode::Rgb, SampleType::U8, &[&[0.2, 0.4, 0.6, 0.5], &[1.0, 0.0, 0.5, 1.0]]);
            oracle_case(if depth == SampleType::U16 { "rgba16" } else { "rgba32f-hdr" }, &result.bytes, &reference);
        }
    }

    #[test]
    fn dropped_metadata_is_reported_and_export_as_none_omits_xmp_warning() {
        let mut doc = columns(ColorMode::Rgb, SampleType::U8, &[&[0.2, 0.4, 0.6, 1.0]]);
        doc.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
        doc.metadata.exif = Some(std::sync::Arc::new(b"Exif test marker".to_vec()));
        doc.metadata.xmp = Some("<x:xmpmeta xmlns:x='adobe:ns:meta/'>AVIF test marker</x:xmpmeta>".into());
        doc.resolution_dpi = 300.0;
        let result = export(&doc, "avif", &quality(90)).unwrap();
        for what in ["ICC profile", "EXIF", "XMP", "resolution (DPI)"] {
            assert!(result.warnings.iter().any(|w| w.starts_with(what) && w.contains("not supported")), "{what}: {:?}", result.warnings);
        }
        let none = ExportOptions { xmp: XmpEmbed::None, ..quality(90) };
        let result = export(&doc, "avif", &none).unwrap();
        assert!(!result.warnings.iter().any(|w| w.starts_with("XMP")), "{:?}", result.warnings);
    }

    #[test]
    fn export_quality_changes_the_encoded_image() {
        let doc = columns(ColorMode::Rgb, SampleType::U8, &[&[0.13, 0.27, 0.73, 1.0], &[0.76, 0.42, 0.15, 0.5]]);
        let low = export(&doc, "avif", &quality(10)).unwrap();
        let high = export(&doc, "avif", &quality(95)).unwrap();
        assert_container(&low.bytes, doc.size, true);
        assert_container(&high.bytes, doc.size, true);
        assert_ne!(low.bytes, high.bytes, "quality option did not reach the AVIF encoder");
    }
}
