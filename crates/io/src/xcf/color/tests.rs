//! App policy tests: source transfer interpretation must preserve assigned colorants.
use super::*;

fn precision(transfer: Transfer) -> SourcePrecision {
    SourcePrecision { storage: craft_xcf::Precision::GAMMA8, transfer }
}

fn expected_implicit(mode: ColorMode, transfer: Transfer) -> &'static Profile {
    match (mode, transfer) {
        (ColorMode::Grayscale, Transfer::Linear) => craft_cms::builtin::linear_gray(),
        (ColorMode::Grayscale, _) => Builtin::SGray.profile(),
        (_, Transfer::Linear) => Builtin::LinearSrgb.profile(),
        (_, _) => Builtin::Srgb.profile(),
    }
}

#[test]
fn implicit_profiles_preserve_encoding_and_missing_assignment_policy() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for transfer in [Transfer::Linear, Transfer::Nonlinear, Transfer::Perceptual] {
            let selected = select_profile(mode, precision(transfer), None).unwrap();
            assert_eq!(selected.bytes, expected_implicit(mode, transfer).to_bytes());
            assert_eq!(selected.tag_document, transfer == Transfer::Linear);
            assert!(selected.fallback_reason.is_none());
        }
    }
}

#[test]
fn assigned_rgb_primaries_survive_linear_and_perceptual_transfer_interpretation() {
    for builtin in [Builtin::DisplayP3, Builtin::AdobeRgbCompat, Builtin::ProPhotoCompat] {
        let original = Profile::parse(&builtin.profile().to_bytes()).unwrap();
        for transfer in [Transfer::Linear, Transfer::Perceptual] {
            let selected = select_profile(ColorMode::Rgb, precision(transfer), Some(original.to_bytes().as_ref().clone())).unwrap();
            let parsed = Profile::parse(&selected.bytes).unwrap();
            assert_eq!(parsed.matrix, original.matrix);
            assert_ne!(parsed.matrix, Builtin::Srgb.profile().matrix);
            assert_eq!(parsed.white_point, original.white_point);
            let expected = if transfer == Transfer::Linear { Curve::Identity } else { craft_cms::curve::srgb_trc() };
            for curve in parsed.trc.as_ref().unwrap() {
                for sample in [-0.25, 0., 0.25, 0.5, 1., 2.] {
                    assert!((curve.eval64(sample) - expected.eval64(sample)).abs() < 0.0001);
                }
            }
            assert!(selected.tag_document && selected.fallback_reason.is_none());
        }
    }
}

#[test]
fn nonlinear_assignments_remain_byte_identical() {
    for (mode, builtin) in [(ColorMode::Rgb, Builtin::AdobeRgbCompat), (ColorMode::Grayscale, Builtin::GrayGamma22)] {
        let bytes = builtin.profile().to_bytes();
        let selected = select_profile(mode, precision(Transfer::Nonlinear), Some(bytes.as_ref().clone())).unwrap();
        assert_eq!(selected.bytes, bytes);
        assert!(selected.tag_document && selected.fallback_reason.is_none());
    }
}

#[test]
fn gray_transfer_changes_preserve_assigned_white_and_use_luminance_not_lab_lightness() {
    let original = Profile::parse(&Builtin::GrayGamma22.profile().to_bytes()).unwrap();
    for (transfer, expected) in [(Transfer::Nonlinear, 0.5f64.powf(2.2)), (Transfer::Linear, 0.5), (Transfer::Perceptual, 0.21404114)] {
        let selected = select_profile(ColorMode::Grayscale, precision(transfer), Some(original.to_bytes().as_ref().clone())).unwrap();
        let parsed = Profile::parse(&selected.bytes).unwrap();
        assert_eq!(parsed.white_point, original.white_point);
        assert!((parsed.gray_trc.as_ref().unwrap().eval64(0.5) - expected).abs() < 0.0001);
        assert!(selected.fallback_reason.is_none());
    }
    let mut lab = Builtin::SGray.profile().clone();
    lab.pcs = craft_cms::Pcs::Lab;
    let bytes = lab.with_encoded_bytes().to_bytes();
    assert_eq!(profile(ColorMode::Grayscale, precision(Transfer::Nonlinear), Some(bytes.as_ref().clone())).unwrap(), bytes);
    for transfer in [Transfer::Linear, Transfer::Perceptual] {
        let selected = select_profile(ColorMode::Grayscale, precision(transfer), Some(bytes.as_ref().clone())).unwrap();
        assert!(selected.fallback_reason.as_ref().unwrap().contains("PCS-aware"));
        assert_eq!(selected.bytes, expected_implicit(ColorMode::Grayscale, transfer).to_bytes());
    }
}

#[test]
fn unusable_optional_profiles_fall_back_without_claiming_preserved_colorants() {
    let mut lut = Builtin::DisplayP3.profile().clone();
    lut.a2b = Builtin::LabD50.profile().a2b.clone();
    let mut unusable = Builtin::DisplayP3.profile().clone();
    unusable.matrix = None;
    unusable.trc = None;
    let cases = [
        (ColorMode::Rgb, Transfer::Nonlinear, vec![1, 2, 3], "assigned ICC"),
        (ColorMode::Grayscale, Transfer::Nonlinear, Vec::new(), "assigned ICC"),
        (ColorMode::Rgb, Transfer::Nonlinear, Builtin::SGray.profile().to_bytes().as_ref().clone(), "does not match"),
        (ColorMode::Grayscale, Transfer::Nonlinear, Builtin::DisplayP3.profile().to_bytes().as_ref().clone(), "does not match"),
        (ColorMode::Rgb, Transfer::Linear, lut.with_encoded_bytes().to_bytes().as_ref().clone(), "no separable native matrix/TRC"),
        (ColorMode::Rgb, Transfer::Nonlinear, unusable.with_encoded_bytes().to_bytes().as_ref().clone(), "not usable by native color management"),
    ];
    for (mode, transfer, bytes, reason) in cases {
        // Establish the intended failure, rather than accepting an unrelated malformed fixture.
        assert!(profile(mode, precision(transfer), Some(bytes.clone())).unwrap_err().to_string().contains(reason));
        let selected = select_profile(mode, precision(transfer), Some(bytes)).unwrap();
        assert_eq!(selected.bytes, expected_implicit(mode, transfer).to_bytes());
        assert!(selected.tag_document);
        assert!(selected.fallback_reason.as_ref().unwrap().contains(reason));
    }
}

#[test]
fn absolute_media_white_survives_v2_to_v4_curve_rewrite() {
    let mut definition = Builtin::DisplayP3.profile().clone();
    definition.white_point = [0.95047, 1., 1.08883];
    let mut bytes = definition.with_encoded_bytes().to_bytes().as_ref().clone();
    bytes[8] = 2;
    bytes[9] = 0x40;
    let original = Profile::parse(&bytes).unwrap();
    assert_ne!(original.media_white(), original.white_point);
    for transfer in [Transfer::Linear, Transfer::Perceptual] {
        let selected = select_profile(ColorMode::Rgb, precision(transfer), Some(bytes.clone())).unwrap();
        let parsed = Profile::parse(&selected.bytes).unwrap();
        for (actual, expected) in parsed.media_white().into_iter().zip(original.media_white()) {
            assert!((actual - expected).abs() < 0.00002);
        }
    }
}
