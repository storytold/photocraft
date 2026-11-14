use super::*;
use crate::xcf::tests::{TestImage, TestLayer, W};

fn gray_lab_profile() -> std::sync::Arc<Vec<u8>> {
    let mut profile = Builtin::GrayGamma22.profile().clone();
    profile.pcs = craft_cms::Pcs::Lab;
    profile.with_encoded_bytes().to_bytes()
}

#[test]
fn gray_lab_literals_reject_luminance_rewrites_and_preserve_assigned_curve() {
    let bytes = gray_lab_profile();
    for encoding in ["Y u8", "YA u8", "Y~ u8", "Y~A u8"] {
        let pixels = if encoding.contains('A') { &[128, 64][..] } else { &[128][..] };
        assert!(matches!(literal_rgb(&ColorLiteral { encoding, pixels, profile: &bytes }), Err(LiteralColorError::GrayLabPcsTransfer)), "{encoding}");
    }
    let source = Profile::parse(&bytes).unwrap();
    let transform = Transform::new(&source, Builtin::Srgb.profile(), Intent::RelativeColorimetric, false).unwrap();
    let mut expected = [0.; 3];
    transform.convert_f32(&[128. / 255.], 1, &mut expected, 3, false).unwrap();
    for encoding in ["Y' u8", "Y'A u8"] {
        let alpha = encoding.contains('A');
        let pixels = if alpha { &[128, 64][..] } else { &[128][..] };
        let actual = literal_rgb(&ColorLiteral { encoding, pixels, profile: &bytes }).unwrap();
        for (actual, expected) in actual.iter().take(3).zip(expected) {
            assert!((actual - expected).abs() < 0.00001, "assigned L* curve: {encoding}");
        }
        assert_eq!(actual[3], if alpha { 64. / 255. } else { 1. });
    }
}

fn parasite(name: &str, data: &[u8]) -> Vec<u8> {
    let mut w = W { out: Vec::new(), wide: false };
    w.string(name);
    w.u32(0);
    w.u32(data.len() as u32);
    w.out.extend_from_slice(data);
    w.out
}

fn source(encoding: &str, profile: &[u8], vector: bool) -> Vec<u8> {
    let mut image = TestImage::rgb8(1, 1);
    image.version = 26;
    let mut layer = TestLayer::rgba("Gray Lab paint", 1, 1, vec![17, 31, 57, 255]);
    if vector {
        // A valid matching path prevents a missing-path fallback masking a bad color translation.
        let mut path = W { out: Vec::new(), wide: false };
        for value in [1, 0, 1] {
            path.u32(value);
        }
        path.string("Saved path");
        for value in [7, 0, 0, 0, 0] {
            path.u32(value);
        }
        image.image_props.push((25, path.out));
        let mut style = W { out: Vec::new(), wide: false };
        for value in [0, 7, 1, 0] {
            style.u32(value);
        }
        for _ in 0..2 {
            style.u32(0); // Solid paint.
            style.u32(1); // Antialias.
            style.string(encoding);
            style.u32(2);
            style.out.extend_from_slice(&[128, 64]);
            style.u32(profile.len() as u32);
            style.out.extend_from_slice(profile);
            style.string("");
        }
        for value in [1f32.to_bits(), 0, 0, 4f32.to_bits(), 0] {
            style.u32(value);
        }
        layer.props.push((47, style.out));
    } else {
        let escaped: String = profile.iter().map(|byte| format!("\\{byte:03o}")).collect();
        let config = format!("(text \"paint\") (color (color \"{encoding}\" 2 \"\\200\\100\" {} \"{escaped}\"))", profile.len());
        layer.props.push((26, 0u32.to_be_bytes().to_vec()));
        layer.props.push((21, parasite("gimp-text-layer", config.as_bytes())));
    }
    image.layers.push(layer);
    image.write()
}

#[test]
fn gray_lab_literal_transfer_fallback_preserves_complete_text_and_vector_imports() {
    use photocraft_doc::LayerContent;
    use photocraft_doc::import_conversion::{ConversionReason, ImportConversion};
    use photocraft_geom::Rect;
    let profile = gray_lab_profile();
    for vector in [false, true] {
        // Assigned-curve literals remain editable, proving the fixtures reach native translation.
        let assigned = crate::import("assigned.xcf", &source("Y'A u8", &profile, vector)).unwrap();
        assert!(if vector {
            matches!(assigned.document.layers[0].content, LayerContent::Shape(_))
        } else {
            matches!(assigned.document.layers[0].content, LayerContent::Text(_))
        });
        for encoding in ["YA u8", "Y~A u8"] {
            let bytes = source(encoding, &profile, vector);
            let imported = crate::import("gray-lab.xcf", &bytes).expect("optional paint must not discard the image");
            let LayerContent::Raster(pixels) = &imported.document.layers[0].content else { panic!("{encoding}/{vector}: stored raster") };
            assert_eq!(pixels.read_region(Rect::new(0, 0, 1, 1)), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
            assert!(imported.warnings.iter().any(|warning| warning.contains("opens as pixels")));
            let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("typed report") };
            let feature = if vector { "xcf.vector_layer" } else { "xcf.text" };
            assert!(report.entries.iter().any(|entry| entry.feature_code == feature
                && entry.reason == ConversionReason::MissingMapping
                && entry.source_description.contains("Gray Lab PCS")
                && entry.native_description.contains("Stored pixels preserved")));
            assert!(!report.attachments.is_empty(), "original configuration remains inert");
            let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            assert_eq!(reopened.layers, imported.document.layers);
            assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
        }
    }
}
