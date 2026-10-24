//! Interpret the source sample encoding as an ordinary native ICC profile.
//!
//! XCF precision describes the transfer curve separately from the assigned primaries.
//! The public GIMP format-profile API distinguishes linear, assigned-curve and sRGB-curve
//! encodings. Changing the transfer curve therefore must preserve the assigned colorants.

use std::sync::Arc;

use photocraft_cms::{Builtin, ColorSpace, Curve, Intent, Profile, TransformOptions};
use photocraft_color::ColorMode;

use super::{Precision, Result, invalid};

pub(super) fn profile(mode: ColorMode, precision: Precision, assigned: Option<Vec<u8>>) -> Result<Arc<Vec<u8>>> {
    let expected = if mode == ColorMode::Grayscale { ColorSpace::Gray } else { ColorSpace::Rgb };
    let implicit = assigned.is_none();
    let mut profile = match assigned {
        Some(bytes) => Profile::parse(&bytes).map_err(|error| invalid(format!("assigned ICC profile: {error}")))?,
        // With no assigned colorants, XCF's implicit spaces already have exact
        // ordinary native profiles. Keep their canonical bytes so native color
        // policy need not resample an equivalent profile to recognize them.
        None => match (expected, precision.linear) {
            (ColorSpace::Gray, true) => photocraft_cms::builtin::linear_gray().clone(),
            (ColorSpace::Gray, false) => Builtin::SGray.profile().clone(),
            (_, true) => Builtin::LinearSrgb.profile().clone(),
            (_, false) => Builtin::Srgb.profile().clone(),
        },
    };
    if profile.color_space != expected {
        return Err(invalid(format!("assigned ICC profile model {:?} does not match the XCF {:?} samples", profile.color_space, expected)));
    }
    if !implicit && (precision.linear || precision.perceptual) {
        if !profile.is_matrix_shaper() || profile.b2a.iter().any(Option::is_some) {
            return Err(invalid("assigned ICC profile has no separable native matrix/TRC representation for the XCF sample encoding"));
        }
        let curve = if precision.linear { Curve::Identity } else { photocraft_cms::curve::srgb_trc() };
        if expected == ColorSpace::Gray {
            profile.gray_trc = Some(curve);
        } else {
            profile.trc = Some([curve.clone(), curve.clone(), curve]);
        }
        profile.description = format!("{} ({} samples)", profile.description, if precision.linear { "linear" } else { "sRGB-curve" });
        profile = profile.with_encoded_bytes();
    }
    // Establish usability through the same native CMS used by document color policy. This
    // validates essentials before publishing a document, without transforming or clipping HDR.
    let destination = if expected == ColorSpace::Gray { Builtin::SGray } else { Builtin::Srgb };
    photocraft_cms::transform::cached(
        &profile,
        destination.profile(),
        TransformOptions { intent: Intent::RelativeColorimetric, bpc: false, ..Default::default() },
    )
    .map_err(|error| invalid(format!("assigned ICC profile is not usable by native color management: {error}")))?;
    Ok(profile.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perceptual_precision_codes_preserve_storage_width_and_native_depth() {
        use photocraft_color::SampleType;
        for (code, bytes, float, depth) in [
            (175, 1, false, SampleType::U8),
            (275, 2, false, SampleType::U16),
            (375, 4, false, SampleType::F32),
            (575, 2, true, SampleType::F32),
            (675, 4, true, SampleType::F32),
            (775, 8, true, SampleType::F32),
        ] {
            let precision = Precision::from_header(26, code).unwrap();
            assert_eq!((precision.bytes, precision.float, precision.depth()), (bytes, float, depth));
            assert!(precision.perceptual);
            assert!(!precision.linear);
            assert!(Precision::from_header(6, code).is_err());
        }
        assert!(Precision::from_header(26, 475).is_err());
        assert!(Precision::from_header(26, 875).is_err());
    }

    #[test]
    fn implicit_sample_encodings_use_exact_canonical_native_profiles() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for code in [100, 150, 175, 600, 650, 675] {
                let precision = Precision::from_header(26, code).unwrap();
                let bytes = profile(mode, precision, None).unwrap();
                let expected = match (mode, precision.linear) {
                    (ColorMode::Grayscale, true) => photocraft_cms::builtin::linear_gray(),
                    (ColorMode::Grayscale, false) => Builtin::SGray.profile(),
                    (_, true) => Builtin::LinearSrgb.profile(),
                    (_, false) => Builtin::Srgb.profile(),
                };
                assert_eq!(bytes, expected.to_bytes(), "canonical {mode:?}/precision{code}");
                let parsed = Profile::parse(&bytes).unwrap();
                assert_eq!(parsed.color_space, if mode == ColorMode::Rgb { ColorSpace::Rgb } else { ColorSpace::Gray });
                assert!(parsed.same_colors(expected));
                let curve = if mode == ColorMode::Rgb { &parsed.trc.as_ref().unwrap()[0] } else { parsed.gray_trc.as_ref().unwrap() };
                for sample in [0., 0.25, 0.5, 1.] {
                    let expected_value = if precision.linear {
                        sample
                    } else if sample <= 0.04045 {
                        sample / 12.92
                    } else {
                        ((sample + 0.055) / 1.055_f64).powf(2.4)
                    };
                    assert!((curve.eval64(sample) - expected_value).abs() < 0.0001, "native curve {mode:?}/precision{code}/sample{sample}");
                }
            }
        }
    }

    #[test]
    fn assigned_primaries_survive_linear_and_perceptual_encoding() {
        for source in [Builtin::DisplayP3, Builtin::AdobeRgbCompat, Builtin::ProPhotoCompat] {
            for (code, expected) in [(600, Curve::Identity), (675, photocraft_cms::curve::srgb_trc())] {
                let original = Profile::parse(&source.profile().to_bytes()).unwrap();
                let converted = profile(ColorMode::Rgb, Precision::from_header(26, code).unwrap(), Some(original.to_bytes().as_ref().clone())).unwrap();
                let parsed = Profile::parse(&converted).unwrap();
                assert_eq!(parsed.matrix, original.matrix);
                assert_eq!(parsed.white_point, original.white_point);
                for curve in parsed.trc.as_ref().unwrap() {
                    for value in [-0.25, 0.0, 0.25, 0.5, 1.0, 2.0] {
                        assert!((curve.eval64(value) - expected.eval64(value)).abs() < 0.0001);
                    }
                }
                assert_ne!(parsed.matrix, Builtin::Srgb.profile().matrix);
            }
        }
    }

    #[test]
    fn nonlinear_preserves_assigned_bytes_and_implicit_profiles_are_explicit() {
        let original = Builtin::AdobeRgbCompat.profile().to_bytes();
        assert_eq!(profile(ColorMode::Rgb, Precision::GAMMA8, Some(original.as_ref().clone())).unwrap(), original);
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for code in [150, 100, 175] {
                let bytes = profile(mode, Precision::from_header(26, code).unwrap(), None).unwrap();
                let parsed = Profile::parse(&bytes).unwrap();
                assert_eq!(parsed.color_space, if mode == ColorMode::Rgb { ColorSpace::Rgb } else { ColorSpace::Gray });
                let curve = if mode == ColorMode::Rgb { &parsed.trc.as_ref().unwrap()[0] } else { parsed.gray_trc.as_ref().unwrap() };
                assert!((curve.eval64(0.5) - if code == 100 { 0.5 } else { 0.21404114 }).abs() < 0.0001);
            }
        }
    }

    #[test]
    fn gray_assigned_curve_and_white_point_are_interpreted_separately() {
        let original = Profile::parse(&Builtin::GrayGamma22.profile().to_bytes()).unwrap();
        for (code, expected) in [(650, 0.5f64.powf(2.2)), (600, 0.5), (675, 0.21404114)] {
            let converted = profile(ColorMode::Grayscale, Precision::from_header(26, code).unwrap(), Some(original.to_bytes().as_ref().clone())).unwrap();
            let parsed = Profile::parse(&converted).unwrap();
            assert_eq!(parsed.white_point, original.white_point);
            assert!((parsed.gray_trc.as_ref().unwrap().eval64(0.5) - expected).abs() < 0.0001);
        }
    }

    #[test]
    fn invalid_and_mismatched_assigned_profiles_fail_without_relabeling() {
        assert!(profile(ColorMode::Rgb, Precision::GAMMA8, Some(vec![1, 2, 3])).unwrap_err().to_string().contains("assigned ICC"));
        for (mode, source) in [(ColorMode::Rgb, Builtin::SGray), (ColorMode::Grayscale, Builtin::DisplayP3)] {
            assert!(profile(mode, Precision::GAMMA8, Some(source.profile().to_bytes().as_ref().clone())).unwrap_err().to_string().contains("does not match"));
        }
        let mut nonseparable = Builtin::DisplayP3.profile().clone();
        nonseparable.a2b = Builtin::LabD50.profile().a2b.clone();
        let bytes = nonseparable.with_encoded_bytes().to_bytes();
        for code in [600, 675] {
            assert!(
                profile(ColorMode::Rgb, Precision::from_header(26, code).unwrap(), Some(bytes.as_ref().clone()))
                    .unwrap_err()
                    .to_string()
                    .contains("no separable native matrix/TRC")
            );
        }
    }

    #[test]
    fn repeated_profile_validation_preserves_profiles_and_rejects_invalid_inputs_after_cache_warmup() {
        let original = Builtin::DisplayP3.profile().to_bytes();
        for code in [600, 650, 675] {
            let precision = Precision::from_header(26, code).unwrap();
            let first = profile(ColorMode::Rgb, precision, Some(original.as_ref().clone())).unwrap();
            for _ in 0..3 {
                assert_eq!(profile(ColorMode::Rgb, precision, Some(original.as_ref().clone())).unwrap(), first);
            }
            // A warmed transform must not turn later malformed, mismatched, or
            // unsupported source profiles into successful imports.
            assert!(profile(ColorMode::Rgb, precision, Some(original[..100].to_vec())).unwrap_err().to_string().contains("assigned ICC"));
            assert!(
                profile(ColorMode::Rgb, precision, Some(Builtin::SGray.profile().to_bytes().as_ref().clone()))
                    .unwrap_err()
                    .to_string()
                    .contains("does not match")
            );
            let mut unusable = Builtin::DisplayP3.profile().clone();
            unusable.matrix = None;
            unusable.trc = None;
            let unusable = unusable.with_encoded_bytes().to_bytes();
            let error = profile(ColorMode::Rgb, precision, Some(unusable.as_ref().clone())).unwrap_err().to_string();
            assert!(error.contains(if code == 650 { "not usable by native color management" } else { "no separable native matrix/TRC" }), "{error}");
            assert_eq!(profile(ColorMode::Rgb, precision, Some(original.as_ref().clone())).unwrap(), first);
        }
    }

    #[test]
    fn import_preserves_hdr_samples_with_assigned_colorants_and_native_persistence() {
        use crate::xcf::tests::{TestImage, TestLayer};
        use photocraft_doc::LayerContent;
        use photocraft_geom::Rect;
        for code in [600, 650, 675] {
            let mut source = TestImage::rgb8(1, 1);
            source.precision = code;
            let icc = Builtin::AdobeRgbCompat.profile().to_bytes();
            let mut parasite = Vec::new();
            parasite.extend_from_slice(&12u32.to_be_bytes());
            parasite.extend_from_slice(b"icc-profile\0");
            parasite.extend_from_slice(&0u32.to_be_bytes());
            parasite.extend_from_slice(&(icc.len() as u32).to_be_bytes());
            parasite.extend_from_slice(&icc);
            source.image_props.push((21, parasite));
            let values = [-0.25f32, 0.5, 2.0, 1.0];
            source.layers.push(TestLayer::rgba("HDR", 1, 1, values.iter().flat_map(|value| value.to_be_bytes()).collect()));
            let bytes = source.write();
            let mut previous = None;
            for _ in 0..3 {
                let result = crate::xcf::import_with_sample_order(
                    "color.xcf",
                    &bytes,
                    Some(crate::HistoricalXcfSampleOrder::BigEndian),
                    &photocraft_raster::Interrupt::NONE,
                )
                .unwrap();
                let LayerContent::Raster(surface) = &result.document.layers[0].content else { panic!("raster") };
                assert_eq!(surface.read_region(Rect::new(0, 0, 1, 1)), values);
                if let Some(previous) = &previous {
                    assert_eq!(&result.document.icc_profile, previous);
                }
                previous = Some(result.document.icc_profile.clone());
                let saved = photocraft_format::save_to_bytes(&result.document, &photocraft_format::SaveOptions::default()).unwrap();
                let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
                assert_eq!(reopened.icc_profile, result.document.icc_profile);
                let assigned = Profile::parse(reopened.icc_profile.as_ref().unwrap()).unwrap();
                assert_eq!(assigned.matrix, Profile::parse(&icc).unwrap().matrix);
            }
        }
    }
}
