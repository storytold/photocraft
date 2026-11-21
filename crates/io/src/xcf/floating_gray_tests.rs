use super::tests::{TestImage, TestLayer};
use super::*;
use photocraft_doc::import_conversion::ImportConversion;

fn floating_image(precision: u32, alpha: bool, values: &[f64]) -> TestImage {
    let mut source = TestImage::rgb8(2, 1);
    source.version = 26;
    source.precision = precision;
    let bytes = if precision == 650 {
        values.iter().flat_map(|value| (*value as f32).to_be_bytes()).collect()
    } else {
        values.iter().flat_map(|value| value.to_be_bytes()).collect()
    };
    let mut layer = TestLayer::rgba("Floating gray color", 2, 1, bytes);
    layer.kind = if alpha { 3 } else { 2 };
    layer.props.push((5, 97u64.to_be_bytes().to_vec()));
    source.layers.push(layer);
    source
}

#[test]
fn floating_gray_hdr_and_signed_color_survive_rgb_import_and_native_save() {
    for precision in [650, 750] {
        for alpha in [false, true] {
            let values = if alpha { vec![1.5, 0.25, -0.25, 1.0] } else { vec![1.5, -0.25] };
            let source = floating_image(precision, alpha, &values);
            let imported = import("floating-gray.xcf", &source.write(), &Interrupt::NONE).unwrap();
            let expected = vec![1.5, 1.5, 1.5, if alpha { 0.25 } else { 1.0 }, -0.25, -0.25, -0.25, 1.0];
            let document = &imported.document;
            assert_eq!(document.mode, ColorMode::Rgb);
            assert_eq!(document.depth, SampleType::F32);
            assert_eq!(document.layers.len(), 1);
            assert_eq!(document.layers[0].surface().unwrap().read_region(document.bounds()), expected);
            assert_eq!(document.layers[0].locks.transparency, !alpha);
            let bytes = photocraft_format::save_to_bytes(document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(reopened.layers[0].surface().unwrap().read_region(reopened.bounds()), expected);
            assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
        }
    }
}

#[test]
fn floating_gray_alpha_clamps_before_native_narrowing_with_durable_notice() {
    for alpha in [-1e-50, 1.0 + 2.0f64.powi(-24)] {
        let source = floating_image(750, true, &[1.5, alpha, -0.25, 1.0]);
        let imported = import("floating-gray-alpha.xcf", &source.write(), &Interrupt::NONE).unwrap();
        let expected = [1.5, 1.5, 1.5, alpha.clamp(0.0, 1.0) as f32, -0.25, -0.25, -0.25, 1.0];
        assert_eq!(imported.document.layers[0].surface().unwrap().read_region(imported.document.bounds()), expected);
        let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("report") };
        let notices: Vec<_> = report.entries.iter().filter(|entry| entry.feature_code == "xcf.coverage_clamped").collect();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].reason, ConversionReason::NumericalConversion);
        let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened.layers[0].surface().unwrap().read_region(reopened.bounds()), expected);
        assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
    }
    for alpha in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let source = floating_image(750, true, &[1.5, alpha, -0.25, 1.0]);
        assert!(import("nonfinite-gray-alpha.xcf", &source.write(), &Interrupt::NONE).is_err());
    }
}
