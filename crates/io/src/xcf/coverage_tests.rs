use super::tests::{TestImage, TestLayer};
use super::*;
use photocraft_doc::import_conversion::ImportConversion;

fn floating_coverage(precision: u32, alpha: [f64; 2], coverage: [f64; 2]) -> TestImage {
    let mut image = TestImage::rgb8(2, 1);
    image.version = 26;
    image.precision = precision;
    let encode = |values: &[f64]| -> Vec<u8> {
        match precision {
            550 => values.iter().flat_map(|value| f16::from_f64(*value).to_bits().to_be_bytes()).collect(),
            650 => values.iter().flat_map(|value| (*value as f32).to_be_bytes()).collect(),
            750 => values.iter().flat_map(|value| value.to_be_bytes()).collect(),
            _ => panic!("test precision"),
        }
    };
    let data = encode(&[-0.25, 1.5, 0.5, alpha[0], -0.125, 2.0, 0.75, alpha[1]]);
    let mut layer = TestLayer::rgba("HDR owner", 2, 1, data);
    let bytes = encode(&coverage);
    layer.mask = Some(bytes.clone());
    image.layers.push(layer);
    image.channels.push(("Coverage", bytes, false));
    image
}

#[test]
fn finite_float_coverage_overshoot_clamps_with_per_owner_durable_notices_and_hdr_color() {
    for precision in [550, 650, 750] {
        let [above, below] = if precision == 550 { [1.5, -0.25] } else { [1.0000001, -1e-7] };
        let image = floating_coverage(precision, [above, below], [below, above]);
        let imported = import("overshoot.xcf", &image.write(), &Interrupt::NONE).unwrap();
        let document = &imported.document;
        let surface = document.layers[0].surface().unwrap();
        assert_eq!(surface.pixel(0, 0), [-0.25, 1.5, 0.5, 1.0]);
        assert_eq!(surface.pixel(1, 0)[3], 0.0);
        assert_eq!(document.layers[0].mask.as_ref().unwrap().surface.read_region(document.bounds()), [0.0, 1.0]);
        assert_eq!(document.channels[0].surface.read_region(document.bounds()), [0.0, 1.0]);
        let Some(ImportConversion::Report(report)) = document.metadata.import_conversion.as_deref() else { panic!("report") };
        let notices: Vec<_> = report.entries.iter().filter(|entry| entry.feature_code == "xcf.coverage_clamped").collect();
        assert_eq!(notices.len(), 3, "one per layer/mask/channel, not per sample");
        assert!(notices.iter().all(|entry| entry.reason == ConversionReason::NumericalConversion));
        assert!(!report.entries.iter().any(|entry| entry.feature_code == "xcf.precision_underflow"));
        let saved = photocraft_format::save_to_bytes(document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened.layers[0].surface().unwrap().read_region(reopened.bounds()), surface.read_region(document.bounds()));
        assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
    }
}

#[test]
fn in_range_coverage_has_no_clamp_notice_and_nonfinite_coverage_still_fails() {
    for precision in [550, 650, 750] {
        let image = floating_coverage(precision, [0.5, 1.0], [0.25, 0.75]);
        let imported = import("ordinary.xcf", &image.write(), &Interrupt::NONE).unwrap();
        let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("report") };
        assert!(!report.entries.iter().any(|entry| entry.feature_code == "xcf.coverage_clamped"));
        assert_eq!(imported.document.layers[0].surface().unwrap().pixel(0, 0), [-0.25, 1.5, 0.5, 0.5]);
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for image in [floating_coverage(precision, [value, 1.0], [0.25, 0.75]), floating_coverage(precision, [0.5, 1.0], [value, 0.75])] {
                assert!(import("nonfinite.xcf", &image.write(), &Interrupt::NONE).is_err());
            }
        }
    }
}
