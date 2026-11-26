//! Interpret the source sample encoding as an ordinary native ICC profile.
//!
//! XCF precision describes the transfer curve separately from the assigned primaries.
//! The public GIMP format-profile API distinguishes linear, assigned-curve and sRGB-curve
//! encodings. Changing the transfer curve therefore must preserve the assigned colorants.

use std::sync::Arc;

use craft_cms::{Builtin, ColorSpace, Curve, Intent, Profile, TransformOptions};
use photocraft_color::ColorMode;

use craft_xcf::samples::{SourcePrecision, Transfer};

use super::{Result, invalid};

#[cfg(test)]
mod tests;

pub(super) struct ProfileSelection {
    pub bytes: Arc<Vec<u8>>,
    /// Linear samples need an explicit encoding; an absent ordinary nonlinear assignment
    /// remains absent so the host's missing-profile policy can still ask or assign a space.
    pub tag_document: bool,
    /// Assigned colorants were unusable; callers must disclose the implicit interpretation
    /// and retain the original optional configuration within the report's existing bounds.
    pub fallback_reason: Option<String>,
}

/// Optional assigned-profile failures must not discard independently usable pixels.
/// The fallback labels the original samples with the encoding-correct implicit space;
/// it does not transform pixels or claim to preserve the unavailable assigned colorants.
pub(super) fn select_profile(mode: ColorMode, precision: SourcePrecision, assigned: Option<Vec<u8>>) -> Result<ProfileSelection> {
    let has_assigned = assigned.is_some();
    let tag_document = has_assigned || precision.transfer == Transfer::Linear;
    match profile(mode, precision, assigned) {
        Ok(bytes) => Ok(ProfileSelection { bytes, tag_document, fallback_reason: None }),
        Err(error) if has_assigned => Ok(ProfileSelection { bytes: profile(mode, precision, None)?, tag_document, fallback_reason: Some(error.to_string()) }),
        Err(error) => Err(error),
    }
}

/// Combines source sample encoding with assigned primaries into a usable native ICC profile, rejecting
/// incompatible profiles or inseparable transfer-curve rewrites.
pub(super) fn profile(mode: ColorMode, precision: SourcePrecision, assigned: Option<Vec<u8>>) -> Result<Arc<Vec<u8>>> {
    let expected = if mode == ColorMode::Grayscale { ColorSpace::Gray } else { ColorSpace::Rgb };
    let implicit = assigned.is_none();
    let mut profile = match assigned {
        Some(bytes) => Profile::parse(&bytes).map_err(|error| invalid(format!("assigned ICC profile: {error}")))?,
        // With no assigned colorants, XCF's implicit spaces already have exact
        // ordinary native profiles. Keep their canonical bytes so native color
        // policy need not resample an equivalent profile to recognize them.
        None => match (expected, precision.transfer == Transfer::Linear) {
            (ColorSpace::Gray, true) => craft_cms::builtin::linear_gray().clone(),
            (ColorSpace::Gray, false) => Builtin::SGray.profile().clone(),
            (_, true) => Builtin::LinearSrgb.profile().clone(),
            (_, false) => Builtin::Srgb.profile().clone(),
        },
    };
    if profile.color_space != expected {
        return Err(invalid(format!("assigned ICC profile model {:?} does not match the XCF {:?} samples", profile.color_space, expected)));
    }
    if !implicit && matches!(precision.transfer, Transfer::Linear | Transfer::Perceptual) {
        if !profile.is_matrix_shaper() || profile.b2a.iter().any(Option::is_some) {
            return Err(invalid("assigned ICC profile has no separable native matrix/TRC representation for the XCF sample encoding"));
        }
        let curve = if precision.transfer == Transfer::Linear { Curve::Identity } else { craft_cms::curve::srgb_trc() };
        if expected == ColorSpace::Gray {
            // A Lab-PCS Gray TRC encodes L*, not luminance. Until a PCS-aware
            // rewrite is available, refuse to persist a silently mislabelled curve.
            if profile.pcs == craft_cms::Pcs::Lab {
                return Err(invalid("assigned Gray Lab PCS requires a PCS-aware sample transfer-curve conversion"));
            }
            profile.gray_trc = Some(curve);
        } else {
            profile.trc = Some([curve.clone(), curve.clone(), curve]);
        }
        profile.description = format!("{} ({} samples)", profile.description, if precision.transfer == Transfer::Linear { "linear" } else { "sRGB-curve" });
        // The writer emits v4, where wtpt is the absolute media white.
        profile.white_point = profile.media_white();
        profile = profile.with_encoded_bytes();
    }
    // Establish usability through the same native CMS used by document color policy. This
    // validates essentials before publishing a document, without transforming or clipping HDR.
    let destination = if expected == ColorSpace::Gray { Builtin::SGray } else { Builtin::Srgb };
    craft_cms::transform::cached(&profile, destination.profile(), TransformOptions { intent: Intent::RelativeColorimetric, bpc: false, ..Default::default() })
        .map_err(|error| invalid(format!("assigned ICC profile is not usable by native color management: {error}")))?;
    Ok(profile.to_bytes())
}
