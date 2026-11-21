use super::tests::{TestImage, TestLayer};
use super::*;

fn parasite(name: &str, data: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&((name.len() + 1) as u32).to_be_bytes());
    bytes.extend_from_slice(name.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&0u32.to_be_bytes());
    bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
    bytes.extend_from_slice(data);
    bytes
}

#[test]
fn unsupported_vertical_column_order_keeps_exact_saved_pixels() {
    for direction in ["ttb-ltr", "ttb-ltr-upright"] {
        let config = format!("(text \"first\\nsecond\") (font-size 24) (base-direction {direction})");
        let mut source = TestImage::rgb8(1, 1);
        let mut layer = TestLayer::rgba("Vertical", 1, 1, vec![255, 0, 0, 255]);
        layer.props.push((21, parasite("gimp-text-layer", config.as_bytes())));
        source.layers.push(layer);
        let imported = import("vertical.xcf", &source.write(), &Interrupt::NONE).unwrap();
        let LayerContent::Raster(pixels) = &imported.document.layers[0].content else { panic!("saved raster for {direction}") };
        assert_eq!(pixels.read_region(Rect::new(0, 0, 1, 1)), vec![1., 0., 0., 1.]);
        assert!(imported.warnings.iter().any(|warning| warning.contains("opens as pixels")));
    }
}

#[test]
fn one_way_assigned_profile_keeps_stored_text_pixels_instead_of_aborting() {
    use craft_cms::pipeline::Stage;
    use craft_cms::profile::{Lut, LutKind};
    use craft_cms::{Builtin, Curve, Intent, Profile, Transform};
    let mut profile = Builtin::Srgb.profile().clone();
    profile.matrix = None;
    profile.trc = None;
    profile.b2a = [None, None, None];
    profile.a2b[1] = Some(Lut {
        kind: LutKind::Ab,
        inputs: 3,
        outputs: 3,
        stages: vec![
            Stage::Curves(vec![Curve::Identity; 3]),
            Stage::Clut(craft_cms::clut::Clut::sample(vec![2; 3], 3, |input, out| out.copy_from_slice(input)).unwrap()),
            Stage::Curves(vec![Curve::Identity; 3]),
        ],
    });
    let bytes = profile.with_encoded_bytes().to_bytes();
    let parsed = Profile::parse(&bytes).unwrap();
    assert!(Transform::new(&parsed, Builtin::Srgb.profile(), Intent::RelativeColorimetric, false).is_ok());
    assert!(Transform::new(Builtin::Srgb.profile(), &parsed, Intent::RelativeColorimetric, false).is_err());
    let mut source = TestImage::rgb8(1, 1);
    source.image_props.push((21, parasite("icc-profile", &bytes)));
    let mut layer = TestLayer::rgba("Text", 1, 1, vec![255, 0, 0, 255]);
    layer.props.push((26, 0u32.to_be_bytes().to_vec()));
    layer.props.push((21, parasite("gimp-text-layer", br#"(text "Ag") (font "Inter") (font-size 24)"#)));
    source.layers.push(layer);
    let imported = import("one-way.xcf", &source.write(), &Interrupt::NONE).expect("unavailable optional color conversion must preserve the image");
    let LayerContent::Raster(pixels) = &imported.document.layers[0].content else { panic!("stored text pixels") };
    assert_eq!(pixels.read_region(Rect::new(0, 0, 1, 1)), vec![1., 0., 0., 1.]);
    assert!(imported.warnings.iter().any(|warning| warning.contains("opens as pixels")));
}

fn image(bytes: &[u8]) -> Image<'_> {
    super::tests::decoder_image(bytes)
}

fn vector_layer(encoding: &str) -> Vec<u8> {
    fn word(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    fn string(bytes: &mut Vec<u8>, value: &str) {
        word(bytes, (value.len() + 1) as u32);
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
    }
    let mut style = Vec::new();
    for value in [0, 7, 1, 0] {
        word(&mut style, value);
    }
    for _ in 0..2 {
        word(&mut style, 0);
        word(&mut style, 0); // Inactive source antialiasing remains a bounded diagnostic.
        string(&mut style, encoding);
        word(&mut style, 4);
        style.extend_from_slice(&[255, 0, 0, 255]);
        word(&mut style, 0);
        string(&mut style, "");
    }
    for value in [1f32.to_bits(), 0, 0, 4f32.to_bits(), 0] {
        word(&mut style, value);
    }
    let mut bytes = vec![0];
    for value in [1, 1, 1] {
        word(&mut bytes, value);
    }
    string(&mut bytes, "Vector");
    word(&mut bytes, 47);
    word(&mut bytes, style.len() as u32);
    bytes.extend_from_slice(&style);
    bytes.extend_from_slice(&[0; 32]); // End properties, no hierarchy/mask/effects.
    bytes
}

#[test]
fn repeated_vector_offsets_charge_bounded_pending_diagnostics() {
    let bytes = vector_layer("R'G'B'A u8");
    let mut image = image(&bytes);
    let (layers, _) =
        image.import_layers(&vec![1; 1024], PixelFormat::RGBA8, PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false), &Interrupt::NONE).unwrap();
    assert_eq!(layers.len(), 1024);
    assert_eq!(image.pending_vectors.len(), 1024);
    let retained = image.pending_vectors.iter().flat_map(|(_, _, v)| &v.source_only).map(String::len).sum::<usize>();
    assert!((1024..=512 * 1024).contains(&retained), "repeated offsets retain {retained} bytes of bounded diagnostics");
    assert!(MAX_METADATA_BYTES - image.meta_budget >= retained, "owned pending diagnostics must debit the shared metadata allowance");
}

#[test]
fn repeated_text_offsets_charge_native_allocations_and_keep_rasters_when_full() {
    let config = format!("(text \"{}\") (font \"Inter\") (font-size 12)", "A".repeat(1024));
    let decoded = text::decode(config.as_bytes(), 72., (0, 0)).unwrap();
    let allowance = text::retained_allocation(&decoded.layer);
    assert!(allowance >= decoded.layer.text.capacity() + decoded.layer.font_family.capacity());
    let mut source = TestImage::rgb8(1, 1);
    source.version = 26;
    source.compression = Compression::None;
    let mut layer = TestLayer::rgba("", 1, 1, vec![255, 0, 0, 255]);
    layer.props.push((26, 0u32.to_be_bytes().to_vec()));
    layer.props.push((21, parasite("gimp-text-layer", config.as_bytes())));
    source.layers.push(layer);
    let bytes = source.write();
    let mut cursor = Cursor { b: &bytes, pos: 30, wide: true };
    props_version(&mut cursor, Some(26)).unwrap();
    let pointer = cursor.ptr().unwrap();
    let mut image = image(&bytes);
    image.meta_budget = allowance;
    let (layers, _) =
        image.import_layers(&[pointer; 16], PixelFormat::RGBA8, PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false), &Interrupt::NONE).unwrap();
    assert_eq!(layers.len(), 16);
    assert_eq!(layers.iter().filter(|layer| matches!(layer.content, LayerContent::Raster(_))).count(), 15);
    let retained = layers
        .iter()
        .filter_map(|layer| match &layer.content {
            LayerContent::Text(text) => Some(text::retained_allocation(text)),
            _ => None,
        })
        .sum::<usize>();
    assert_eq!(layers.iter().filter(|layer| matches!(layer.content, LayerContent::Text(_))).count(), 1);
    assert!(retained <= allowance);
    assert_eq!(image.meta_budget, allowance - retained);
    for layer in layers.iter().filter(|layer| matches!(layer.content, LayerContent::Raster(_))) {
        assert_eq!(layer.surface().unwrap().read_region(Rect::new(0, 0, 1, 1)), vec![1., 0., 0., 1.]);
    }
    assert!(image.warnings.finish().iter().any(|warning| warning.contains("metadata allowance") && warning.contains("opens as pixels")));
    let photocraft_doc::import_conversion::ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("typed report") };
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.text" && entry.reason == ConversionReason::RetentionLimit));
}

#[test]
fn editable_perceptual_text_matches_saved_raster_before_and_after_native_reopen() {
    let mut source = TestImage::rgb8(1, 1);
    source.version = 26;
    let mut text = TestLayer::rgba("Title", 1, 1, vec![0, 0, 0, 127]);
    text.props.push((26, 1u32.to_be_bytes().to_vec()));
    text.props.push((21, parasite("gimp-text-layer", b"(text \"Title\") (font \"Inter\") (font-size 12)")));
    source.layers.push(text);
    source.layers.push(TestLayer::rgba("White", 1, 1, vec![255, 255, 255, 255]));
    let doc = crate::import("editable.xcf", &source.write()).unwrap().document;
    source.layers[0].props[0].1 = 3u32.to_be_bytes().to_vec();
    let raster = crate::import("saved.xcf", &source.write()).unwrap().document;
    let expected = photocraft_compose::flatten(&raster);
    assert_eq!(expected.px.len(), 1);
    assert!(expected.px[0][..3].iter().all(|value| (value - 128. / 255.).abs() <= 1. / 255.));
    assert_eq!(expected.px[0][3], 1.);
    let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
    for document in [&doc, &reopened] {
        let title = document.layers.iter().find(|layer| layer.name == "Title").unwrap();
        assert!(matches!(&title.content, LayerContent::Text(text) if text.keep_layer_name && text.cache_uses_raster_blending));
        assert_eq!(photocraft_compose::flatten(document).px, expected.px);
    }
}

#[test]
fn unavailable_native_vector_colors_preserve_the_saved_raster() {
    let bytes = vector_layer("R'G'B'A u8");
    let mut image = image(&bytes);
    let (layers, _) = image.import_layers(&[1], PixelFormat::RGBA8, PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false), &Interrupt::NONE).unwrap();
    let mut document = Document::new("Vector", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    document.layers = layers.into();
    if let LayerContent::Raster(pixels) = &mut document.layers[0].content {
        pixels.write_region(Rect::new(0, 0, 1, 1), &[1., 0., 0., 1.]);
    }
    document.paths.push(photocraft_doc::vector::NamedPath { name: "Saved".into(), path: Default::default(), psd_raw: None });
    image.path_tattoos.insert(7, Some(PathBinding { index: 0, geometry_complete: true }));
    image.native_color_error = Some("profile has no native reverse transform".into());
    import_vectors(&mut document, &mut image, &Interrupt::NONE).unwrap();
    let LayerContent::Raster(pixels) = &document.layers[0].content else { panic!("saved raster fallback") };
    assert_eq!(pixels.read_region(Rect::new(0, 0, 1, 1)), vec![1., 0., 0., 1.]);
    assert!(image.warnings.finish().iter().any(|warning| warning.contains("opens as pixels")));
}

#[test]
fn deferred_effect_mask_has_an_explicit_omission_outcome() {
    fn string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&((value.len() + 1) as u32).to_be_bytes());
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
    }
    let mut bytes = vec![0];
    for value in ["Masked", "", "gegl:nop", "1"] {
        string(&mut bytes, value);
    }
    bytes.extend_from_slice(&[33u32.to_be_bytes(), 4u32.to_be_bytes(), f32::NAN.to_be_bytes()].concat());
    bytes.extend_from_slice(&[0; 8]); // End properties.
    let mask_offset = bytes.len() + 8;
    bytes.extend_from_slice(&(mask_offset as u64).to_be_bytes());
    bytes.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 1]);
    string(&mut bytes, "Mask");
    bytes.extend_from_slice(&[0; 24]);
    let mut document = Document::new("Masked", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let layer = Layer::new("Owner", LayerContent::Raster(Surface::new(PixelFormat::RGBA8)));
    let id = layer.id;
    document.layers.push(layer);
    let mut image = image(&bytes);
    let owner = image
        .outcomes
        .push(ConversionEntry {
            feature_code: "xcf.layer".into(),
            source: conversion::source("layer", 0, 0, "Owner", &[], None),
            targets: vec![NativeTargetRef { layer_id: Some(id), ..Default::default() }],
            disposition: ConversionDisposition::NativeEquivalent,
            reason: ConversionReason::NativePolicy,
            source_description: String::new(),
            native_description: String::new(),
            attachments: Vec::new(),
        })
        .unwrap();
    image.pending_effects.push(PendingLayerEffects { layer: id, owner, bounds: Rect::new(0, 0, 1, 1), pointers: vec![1] });
    import_native_effects(&mut document, &mut image, &Interrupt::NONE).unwrap();
    let photocraft_doc::import_conversion::ImportConversion::Report(report) = image.outcomes.finish().unwrap() else { panic!("typed report") };
    let effect = report.entries.iter().find(|e| e.feature_code == "xcf.layer_effect").unwrap();
    assert_eq!(effect.reason, ConversionReason::InvalidOptionalData);
    assert!(effect.native_description.contains("effect opacity"));
    let mask = report.entries.iter().find(|e| e.feature_code == "xcf.layer_effect.mask").expect("referenced mask was not retained and must be reported");
    assert_eq!(mask.disposition, ConversionDisposition::SkippedNonessential);
    assert!(mask.native_description.contains("not retained"));
    assert!(matches!(document.layers[0].content, LayerContent::Raster(_)));
}

#[test]
fn undecodable_text_and_vector_paint_keep_the_saved_nonblack_appearance() {
    let mut source = TestImage::rgb8(2, 1);
    let mut text = TestLayer::rgba("Saved white text", 2, 1, vec![255, 255, 255, 255, 0, 0, 0, 255]);
    let mut config = br#"(text "Ag") (font-size 24) (color (color "R'G'B'A float" 16 ""#.to_vec();
    // GIMP literals carry writer-native samples even when structural XCF fields are big-endian.
    for byte in [1f32.to_le_bytes(); 4].concat() {
        config.extend_from_slice(format!("\\{byte:03o}").as_bytes());
    }
    config.extend_from_slice(br#"" 0))"#);
    text.props.push((21, parasite("gimp-text-layer", &config)));
    let vector_wire = vector_layer("R'G'B'A float");
    let mut cursor = Cursor { b: &vector_wire, pos: 13, wide: true };
    cursor.string_bytes().unwrap();
    let properties = props(&mut cursor).unwrap();
    let mut vector = TestLayer::rgba("Saved blue vector", 1, 1, vec![0, 0, 255, 255]);
    vector.x = 1;
    vector.props.push((47, properties[0].data.to_vec()));
    source.layers.extend([vector, text]);
    let imported = import("unknown-paint.xcf", &source.write(), &Interrupt::NONE).unwrap();
    assert!(imported.document.layers.iter().all(|layer| matches!(layer.content, LayerContent::Raster(_))));
    let composite = photocraft_compose::flatten(&imported.document);
    assert_eq!(composite.px, [[1., 1., 1., 1.], [0., 0., 1., 1.]]);
    assert!(imported.warnings.iter().any(|w| w.contains("opens as pixels")));
    let photocraft_doc::import_conversion::ImportConversion::Report(report) = imported.document.metadata.import_conversion.as_deref().unwrap() else {
        panic!("typed report")
    };
    assert!(report.entries.iter().any(|e| e.feature_code == "xcf.vector_layer" && e.source_description.contains("could not be translated")));
}
