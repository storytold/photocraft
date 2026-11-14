use super::*;
use crate::xcf::tests::{TestImage, TestLayer};
use photocraft_doc::import_conversion::ImportConversion;

fn source(mode: ColorMode, code: u32, assigned: Option<&[u8]>) -> (Vec<u8>, Vec<f32>) {
    let precision = Precision::from_header(26, code).unwrap();
    let (encoded, values): (Vec<u8>, Vec<f32>) = match (precision.float, precision.bytes) {
        (false, 1) => {
            let samples = if mode == ColorMode::Grayscale { vec![31u8, 255] } else { vec![17u8, 31, 57, 255] };
            (samples.clone(), samples.iter().map(|sample| f32::from(*sample) / 255.).collect())
        }
        (false, 2) => {
            let samples = if mode == ColorMode::Grayscale { vec![0x5678u16, 0xffff] } else { vec![0x1234u16, 0x5678, 0x9abc, 0xffff] };
            (samples.iter().flat_map(|sample| sample.to_be_bytes()).collect(), samples.iter().map(|sample| f32::from(*sample) / 65535.).collect())
        }
        (false, 4) => {
            let samples = if mode == ColorMode::Grayscale { vec![u32::MAX, u32::MAX] } else { vec![0, u32::MAX, 0, u32::MAX] };
            (samples.iter().flat_map(|sample| sample.to_be_bytes()).collect(), samples.iter().map(|sample| if *sample == 0 { 0. } else { 1. }).collect())
        }
        (true, bytes) => {
            let values: Vec<f32> = if mode == ColorMode::Grayscale { vec![0.5, 1.] } else { vec![0.25, 0.5, 0.75, 1.] };
            let encoded = values
                .iter()
                .flat_map(|sample| match bytes {
                    2 => crate::xcf::f16::from_f32(*sample).to_bits().to_be_bytes().to_vec(),
                    4 => sample.to_be_bytes().to_vec(),
                    8 => f64::from(*sample).to_be_bytes().to_vec(),
                    _ => panic!("fixture sample width"),
                })
                .collect();
            (encoded, values)
        }
        _ => panic!("fixture precision"),
    };
    let mut image = TestImage::rgb8(1, 1);
    image.version = 26;
    image.precision = code;
    image.base_type = u32::from(mode == ColorMode::Grayscale);
    if let Some(profile) = assigned {
        let mut parasite = Vec::new();
        parasite.extend_from_slice(&12u32.to_be_bytes());
        parasite.extend_from_slice(b"icc-profile\0");
        parasite.extend_from_slice(&0u32.to_be_bytes());
        parasite.extend_from_slice(&(profile.len() as u32).to_be_bytes());
        parasite.extend_from_slice(profile);
        image.image_props.push((21, parasite));
    }
    let mut layer = TestLayer::rgba("Native samples", 1, 1, encoded);
    if mode == ColorMode::Grayscale {
        layer.kind = 3;
    }
    image.layers.push(layer);
    (image.write(), values)
}

#[test]
fn unassigned_nonlinear_rgb_and_gray_stay_untagged_at_all_native_depths() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for code in [150, 250, 350, 550, 650, 750, 175, 275, 375, 575, 675, 775] {
            let precision = Precision::from_header(26, code).unwrap();
            let selected = select_profile(mode, precision, None).unwrap();
            assert!(!selected.tag_document, "no source assignment: {mode:?}/{code}");
            assert!(selected.fallback_reason.is_none());
            // Internal conversion still has an encoding-correct profile; absence in the document
            // is deliberate input to the host's existing missing-profile policy.
            assert_eq!(selected.bytes, profile(mode, precision, None).unwrap());
            let (bytes, expected) = source(mode, code, None);
            let imported = crate::import("untagged.xcf", &bytes).unwrap();
            assert_eq!(imported.document.depth, precision.depth());
            assert!(imported.document.icc_profile.is_none(), "{mode:?}/{code} must not invent an embedded assignment");
            assert_eq!(imported.document.layers[0].surface().unwrap().pixel(0, 0), expected);
            let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("report") };
            let encoding = report.entries.iter().find(|entry| entry.feature_code == "xcf.color_encoding").unwrap();
            assert!(encoding.native_description.to_ascii_lowercase().contains("untagged"));
            let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            assert!(reopened.icc_profile.is_none());
            assert_eq!(reopened.layers[0].surface().unwrap().pixel(0, 0), expected);
        }
    }
}

#[test]
fn legacy_unassigned_rgb_and_gray_remain_untagged() {
    for version in [0, 3, 11] {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            let mut image = TestImage::rgb8(1, 1);
            image.version = version;
            image.base_type = u32::from(mode == ColorMode::Grayscale);
            let mut layer = TestLayer::rgba("Legacy untagged", 1, 1, if mode == ColorMode::Grayscale { vec![31, 255] } else { vec![17, 31, 57, 255] });
            if mode == ColorMode::Grayscale {
                layer.kind = 3;
            }
            image.layers.push(layer);
            let imported = crate::import("legacy-untagged.xcf", &image.write()).unwrap();
            assert!(imported.document.icc_profile.is_none(), "XCF{version}/{mode:?}");
        }
    }
}

#[test]
fn linear_encoding_and_explicit_assignments_keep_their_native_profiles() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        let assigned = if mode == ColorMode::Rgb { Builtin::DisplayP3 } else { Builtin::GrayGamma22 }.profile().to_bytes();
        for code in [100, 200, 300, 500, 600, 700] {
            let (bytes, expected) = source(mode, code, None);
            let imported = crate::import("linear.xcf", &bytes).unwrap();
            let selected = select_profile(mode, Precision::from_header(26, code).unwrap(), None).unwrap();
            assert!(selected.tag_document);
            assert_eq!(imported.document.icc_profile, Some(selected.bytes));
            assert_eq!(imported.document.layers[0].surface().unwrap().pixel(0, 0), expected);
        }
        for code in [150, 250, 650, 175, 275, 675] {
            let (bytes, expected) = source(mode, code, Some(&assigned));
            let imported = crate::import("assigned.xcf", &bytes).unwrap();
            let selected = select_profile(mode, Precision::from_header(26, code).unwrap(), Some(assigned.as_ref().clone())).unwrap();
            assert!(selected.tag_document);
            assert!(selected.fallback_reason.is_none());
            assert_eq!(imported.document.icc_profile, Some(selected.bytes));
            assert_eq!(imported.document.layers[0].surface().unwrap().pixel(0, 0), expected);
            let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap().icc_profile, imported.document.icc_profile);
        }
    }
}
