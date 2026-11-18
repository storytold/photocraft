use crate::xcf::tests::{TestImage, TestLayer, W};
use crate::xcf::*;
use photocraft_doc::LayerContent;
use photocraft_doc::import_conversion::{ConversionReason, ImportConversion};

fn native(text: &str) -> Vec<u8> {
    format!("(text \"{text}\") (font-size 12) (box-mode fixed) (box-width 16) (box-height 16)").into_bytes()
}
fn property(records: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = W { out: Vec::new(), wide: false };
    for (name, data) in records {
        writer.string(name);
        writer.u32(0);
        writer.u32(data.len() as u32);
        writer.out.extend_from_slice(data);
    }
    writer.out
}
fn source(properties: Vec<Vec<u8>>) -> Vec<u8> {
    let mut image = TestImage::rgb8(1, 1);
    image.version = 26;
    let mut layer = TestLayer::rgba("Text selection", 1, 1, vec![17, 31, 57, 255]);
    layer.props = properties.into_iter().map(|data| (21, data)).collect();
    image.layers.push(layer);
    image.write()
}
fn roundtrip(imported: crate::ImportResult, payloads: &[&[u8]]) {
    let ImportConversion::Report(report) = imported.document.metadata.import_conversion.as_deref().unwrap() else { panic!("typed report") };
    for payload in payloads {
        assert!(
            report.attachments.iter().any(|attachment| attachment.bytes.windows(payload.len()).any(|part| part == *payload)),
            "source configuration retained"
        );
    }
    let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(reopened.layers, imported.document.layers);
    assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
}

#[test]
fn last_native_text_wins_across_parasite_properties_and_keeps_native_priority() {
    let stale = native("stale");
    let fresh = native("fresh é");
    let dynamic = b"invalid dynamic text must not override the native configuration";
    for properties in [
        vec![property(&[("gimp-text-layer", &stale)]), property(&[("gimp-text-layer", &fresh)])],
        vec![property(&[("gimp-text-layer", &stale), ("gimp-text-layer", &fresh)])],
        vec![property(&[("gimp-text-layer", &stale)]), property(&[("plug_in_gdyntext/data", dynamic)]), property(&[("gimp-text-layer", &fresh)])],
        vec![property(&[("plug_in_gdyntext/data", dynamic)]), property(&[("gimp-text-layer", &fresh)]), property(&[("plug_in_gdyntext/data", dynamic)])],
    ] {
        let imported = crate::import("text-selection.xcf", &source(properties)).unwrap();
        let LayerContent::Text(layer) = &imported.document.layers[0].content else { panic!("editable native text") };
        assert_eq!(layer.text, "fresh é");
        assert!(!imported.warnings.iter().any(|warning| warning.contains("opens as pixels")));
        roundtrip(imported, &[&fresh]);
    }
}

#[test]
fn invalid_last_native_text_keeps_saved_pixels_instead_of_stale_editable_text() {
    let stale = native("stale");
    let invalid = b"(latest native text configuration is truncated";
    let bytes = source(vec![property(&[("gimp-text-layer", &stale)]), property(&[("gimp-text-layer", invalid)])]);
    let imported = crate::import("bad-latest-text.xcf", &bytes).unwrap();
    let layer = &imported.document.layers[0];
    assert!(matches!(layer.content, LayerContent::Raster(_)));
    assert_eq!(layer.surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    assert!(imported.warnings.iter().any(|warning| warning.contains("opens as pixels")));
    let ImportConversion::Report(report) = imported.document.metadata.import_conversion.as_deref().unwrap() else { panic!("typed report") };
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.text" && entry.reason == ConversionReason::InvalidOptionalData));
    roundtrip(imported, &[&stale, invalid]);
    let cancel = || true;
    assert!(matches!(crate::xcf::import("cancelled.xcf", &bytes, &Interrupt::cancel_only(&cancel)), Err(crate::IoError::Cancelled)));
}

#[test]
fn incompatible_pango_caps_keep_saved_pixels_and_the_source_markup() {
    let markup = br#"(markup "<markup><span variant='small-caps'><span text_transform='uppercase'>abc</span></span></markup>") (font-size 12)"#;
    let bytes = source(vec![property(&[("gimp-text-layer", markup)])]);
    let imported = crate::import("combined-caps.xcf", &bytes).unwrap();
    assert_eq!(imported.document.layers[0].surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    assert!(imported.warnings.iter().any(|warning| warning.contains("opens as pixels")));
    roundtrip(imported, &[markup]);
}
