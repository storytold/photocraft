//! Unusable optional assignments must preserve pixels with an explicit implicit-space interpretation.
use super::*;
use crate::xcf::tests::{TestImage, TestLayer};
use photocraft_doc::LayerContent;
use photocraft_doc::import_conversion::{ConversionDisposition, ConversionReason, ImportConversion};
use photocraft_geom::Rect;

type AssignmentCase = (ColorMode, u32, Vec<u8>, &'static str);

fn unusable_assignments() -> Vec<AssignmentCase> {
    let mut lut = Builtin::DisplayP3.profile().clone();
    lut.a2b = Builtin::LabD50.profile().a2b.clone();
    let lut = lut.with_encoded_bytes().to_bytes();
    let mut gray_lab = Builtin::SGray.profile().clone();
    gray_lab.pcs = photocraft_cms::Pcs::Lab;
    let gray_lab = gray_lab.with_encoded_bytes().to_bytes();
    let mut no_transform = Builtin::DisplayP3.profile().clone();
    no_transform.matrix = None;
    no_transform.trc = None;
    let no_transform = no_transform.with_encoded_bytes().to_bytes();
    vec![
        (ColorMode::Rgb, 150, vec![1, 2, 3], "assigned ICC"),
        (ColorMode::Grayscale, 150, vec![1, 2, 3], "assigned ICC"),
        (ColorMode::Rgb, 650, Builtin::SGray.profile().to_bytes().as_ref().clone(), "does not match"),
        (ColorMode::Grayscale, 650, Builtin::DisplayP3.profile().to_bytes().as_ref().clone(), "does not match"),
        (ColorMode::Rgb, 600, lut.as_ref().clone(), "no separable native matrix/TRC"),
        (ColorMode::Rgb, 675, lut.as_ref().clone(), "no separable native matrix/TRC"),
        (ColorMode::Grayscale, 600, gray_lab.as_ref().clone(), "PCS-aware"),
        (ColorMode::Grayscale, 675, gray_lab.as_ref().clone(), "PCS-aware"),
        (ColorMode::Rgb, 650, no_transform.as_ref().clone(), "not usable by native color management"),
    ]
}

#[test]
fn unusable_assigned_profiles_choose_encoding_correct_implicit_profiles_without_relabeling() {
    for (mode, code, assigned, expected_error) in unusable_assignments() {
        let precision = Precision::from_header(26, code).unwrap();
        // Establish that the fixture exercises the named rejection, rather than
        // succeeding via a different CMS path or being accidentally malformed.
        let error = profile(mode, precision, Some(assigned.clone())).unwrap_err().to_string();
        assert!(error.contains(expected_error), "{mode:?}/{code}: {error}");
        let selected = select_profile(mode, precision, Some(assigned)).unwrap();
        assert_eq!(selected.bytes, profile(mode, precision, None).unwrap());
        assert!(selected.fallback_reason.as_ref().unwrap().contains(expected_error));
    }
}

fn parasite(icc: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&12u32.to_be_bytes());
    out.extend_from_slice(b"icc-profile\0");
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&(icc.len() as u32).to_be_bytes());
    out.extend_from_slice(icc);
    out
}

fn fixture(mode: ColorMode, code: u32, assigned: Option<&[u8]>) -> (Vec<u8>, Vec<f32>) {
    let mut image = TestImage::rgb8(1, 1);
    image.version = 26;
    image.precision = code;
    image.base_type = u32::from(mode == ColorMode::Grayscale);
    if let Some(assigned) = assigned {
        image.image_props.push((21, parasite(assigned)));
    }
    let values = if code == 150 {
        if mode == ColorMode::Grayscale { vec![31. / 255., 1.] } else { vec![17. / 255., 31. / 255., 57. / 255., 1.] }
    } else if mode == ColorMode::Grayscale {
        vec![-0.25, 1.]
    } else {
        vec![-0.25, 0.5, 2., 1.]
    };
    let data = if code == 150 {
        if mode == ColorMode::Grayscale { vec![31, 255] } else { vec![17, 31, 57, 255] }
    } else {
        values.iter().flat_map(|value: &f32| value.to_be_bytes()).collect()
    };
    let mut layer = TestLayer::rgba("Usable pixels", 1, 1, data);
    if mode == ColorMode::Grayscale {
        layer.kind = 3;
    }
    image.layers.push(layer);
    (image.write(), values)
}

#[test]
fn public_import_falls_back_for_unusable_optional_icc_with_warning_report_raw_retention_and_native_persistence() {
    for (mode, code, assigned, _) in unusable_assignments() {
        let (bytes, expected_pixels) = fixture(mode, code, Some(&assigned));
        let imported = crate::import("optional-icc.xcf", &bytes).unwrap();
        let expected_profile = profile(mode, Precision::from_header(26, code).unwrap(), None).unwrap();
        assert_eq!(imported.document.icc_profile.as_ref(), Some(&expected_profile));
        let LayerContent::Raster(pixels) = &imported.document.layers[0].content else { panic!("usable native raster") };
        assert_eq!(pixels.read_region(Rect::new(0, 0, 1, 1)), expected_pixels);
        assert!(imported.source_read_only && !imported.preview_only);
        assert!(imported.warnings.iter().any(|warning| warning.contains("assigned ICC") && warning.contains("implicit")));
        let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("typed conversion report") };
        assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.color_encoding"
            && entry.reason == ConversionReason::NativeLimitation
            && entry.disposition == ConversionDisposition::NativeApproximation));
        assert!(!report.entries.iter().any(|entry| entry.feature_code == "xcf.color_encoding" && entry.disposition == ConversionDisposition::NativeEquivalent));
        let raw = parasite(&assigned);
        assert!(report.attachments.iter().any(|attachment| attachment.bytes.as_slice() == raw));
        let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened.icc_profile, imported.document.icc_profile);
        assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
        let LayerContent::Raster(pixels) = &reopened.layers[0].content else { panic!("native raster") };
        assert_eq!(pixels.read_region(Rect::new(0, 0, 1, 1)), expected_pixels);
    }
}

#[test]
fn usable_assignments_and_unassigned_encodings_do_not_report_a_fallback() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        let assigned = if mode == ColorMode::Rgb { Builtin::DisplayP3 } else { Builtin::GrayGamma22 }.profile().to_bytes();
        for code in [600, 650, 675] {
            let precision = Precision::from_header(26, code).unwrap();
            for source in [None, Some(assigned.as_slice())] {
                let selected = select_profile(mode, precision, source.map(<[u8]>::to_vec)).unwrap();
                assert!(selected.fallback_reason.is_none());
                assert_eq!(selected.bytes, profile(mode, precision, source.map(<[u8]>::to_vec)).unwrap());
                let (bytes, _) = fixture(mode, code, source);
                let imported = crate::import("usable-icc.xcf", &bytes).unwrap();
                assert_eq!(imported.document.icc_profile.as_ref(), Some(&selected.bytes));
                assert!(!imported.warnings.iter().any(|warning| warning.contains("assigned ICC") && warning.contains("implicit")));
                let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("typed report") };
                assert!(
                    report
                        .entries
                        .iter()
                        .any(|entry| entry.feature_code == "xcf.color_encoding" && entry.disposition == ConversionDisposition::NativeEquivalent)
                );
            }
        }
    }
}

#[test]
fn assigned_icc_fallback_warning_survives_report_saturation_without_discarding_pixels() {
    let mut source = TestImage::rgb8(1, 1);
    source.version = 26;
    source.image_props.push((21, parasite(&[1, 2, 3])));
    for index in 0..20 {
        let pixel = if index == 0 { vec![17, 31, 57, 255] } else { vec![0; 4] };
        let mut layer = TestLayer::rgba("Saved pixels", 1, 1, pixel);
        // Stay within the source property bound per layer while exhausting the
        // independent report allowance before color-encoding reporting occurs.
        layer.props = vec![(777, Vec::new()); 4000];
        source.layers.push(layer);
    }
    let imported = crate::import("optional-icc-report-cap.xcf", &source.write()).unwrap();
    assert_eq!(imported.document.layers.len(), 20);
    let warnings: Vec<_> = imported.warnings.iter().filter(|warning| warning.contains("assigned ICC") && warning.contains("implicit")).collect();
    assert_eq!(warnings.len(), 1);
    let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("bounded report") };
    report.validate().unwrap();
    assert_eq!(report.entries.iter().filter(|entry| entry.feature_code == "xcf.report_truncated").count(), 1);
    assert!(!report.entries.iter().any(|entry| entry.feature_code == "xcf.color_encoding"), "exercise fallback disclosure after the report stops recording");
    assert_eq!(imported.document.icc_profile, Some(Builtin::Srgb.profile().to_bytes()));
    let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
    assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
    assert_eq!(reopened.icc_profile, imported.document.icc_profile);
    assert_eq!(reopened.layers.len(), 20);
    let mut visible = 0;
    for layer in &reopened.layers {
        let LayerContent::Raster(pixels) = &layer.content else { panic!("saved raster") };
        if pixels.read_region(Rect::new(0, 0, 1, 1)) != vec![0.; 4] {
            assert_eq!(pixels.read_region(Rect::new(0, 0, 1, 1)), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
            visible += 1;
        }
    }
    assert_eq!(visible, 1);
}
