//! Scalar preparation only. Native command domains are defined by
//! engine-core::adjust_params; no GEGL operation is executed here.
use super::{Failure, Result, allowed, filter, number};
use craft_xcf::effects::StoredEffect;
use photocraft_doc::SmartFilter;
use photocraft_raster::Interrupt;
use serde_json::json;

pub(super) fn map(effect: &StoredEffect<'_>, ctl: &Interrupt<'_>) -> Result<(Vec<SmartFilter>, &'static str)> {
    match effect.operation {
        "gegl:brightness-contrast" => brightness_contrast(effect, ctl),
        "gegl:exposure" => exposure(effect, ctl),
        "gegl:levels" => levels(effect, ctl),
        _ => Err(Failure::Unsupported("tone operation")),
    }
}

fn brightness_contrast(effect: &StoredEffect<'_>, ctl: &Interrupt<'_>) -> Result<(Vec<SmartFilter>, &'static str)> {
    allowed(effect, &["brightness", "contrast"], ctl)?;
    let brightness = number(effect, "brightness", 0., -3., 3.)?;
    let gain = number(effect, "contrast", 1., -5., 5.)?;
    brightness_contrast_values(brightness, gain, ctl)
}

pub(in crate::xcf) fn brightness_contrast_values(brightness: f64, gain: f64, ctl: &Interrupt<'_>) -> Result<(Vec<SmartFilter>, &'static str)> {
    ctl.check().map_err(|_| Failure::Cancelled)?;
    value(brightness, -3., 3.)?;
    value(gain, -5., 5.)?;
    let mut filters = Vec::new();
    if gain < 0. {
        filters.push(filter("image.adjustments.invert", json!({})));
    }
    let magnitude = gain.abs();
    let contrast = if magnitude >= 1. { (1. - magnitude.recip()) * 100. } else { (magnitude - 1.) * 100. };
    filters.push(filter("image.adjustments.brightnessContrast", json!({"brightness":0.,"contrast":contrast,"legacy":true})));
    let mut offset = brightness * 255.;
    // Six ordinary +/-150 brightness controls cover source +/-3 without
    // clamping a source endpoint or inventing a larger native command range.
    for _ in 0..6 {
        ctl.check().map_err(|_| Failure::Cancelled)?;
        if offset == 0. {
            break;
        }
        let step = offset.clamp(-150., 150.);
        filters.push(filter("image.adjustments.brightnessContrast", json!({"brightness":step,"contrast":0.,"legacy":true})));
        offset -= step;
    }
    if offset.abs() > f64::EPSILON {
        return Err(Failure::Range("bounded brightness decomposition"));
    }
    Ok((
        filters,
        "Source gain/brightness becomes native legacy contrast, optional Invert and bounded additive controls. Native pivots, integer intermediate clipping/quantization and document encoding differ; apply source opacity and mask once around the compound stage.",
    ))
}

fn exposure(effect: &StoredEffect<'_>, ctl: &Interrupt<'_>) -> Result<(Vec<SmartFilter>, &'static str)> {
    allowed(effect, &["black-level", "exposure"], ctl)?;
    let exposure = number(effect, "exposure", 0., -20., 20.)?;
    let black = number(effect, "black-level", 0., -0.1, 0.1)?;
    exposure_values(black, exposure, ctl)
}

pub(in crate::xcf) fn exposure_values(black: f64, exposure: f64, ctl: &Interrupt<'_>) -> Result<(Vec<SmartFilter>, &'static str)> {
    ctl.check().map_err(|_| Failure::Cancelled)?;
    value(black, -0.1, 0.1)?;
    value(exposure, -20., 20.)?;
    // Move (sample - black) * gain into the native post-gain offset. If that
    // offset cannot fit +/-0.5, subtract black in one neutral exposure first.
    let offset = -black * exposure.exp2();
    let filters = if (-0.5..=0.5).contains(&offset) {
        vec![filter("image.adjustments.exposure", json!({"exposure":exposure,"offset":offset,"gamma":1.}))]
    } else {
        vec![
            filter("image.adjustments.exposure", json!({"exposure":0.,"offset":-black,"gamma":1.})),
            filter("image.adjustments.exposure", json!({"exposure":exposure,"offset":0.,"gamma":1.})),
        ]
    };
    Ok((
        filters,
        "Source black subtraction/exposure becomes bounded native Exposure data (two controls when needed for offset range). Native document transfer/HDR and integer intermediate clipping/quantization remain authoritative; apply source opacity and mask once around the compound stage.",
    ))
}

/// The small editable native Levels domain only. Source GEGL endpoints are
/// unbounded, so signed/HDR, collapsed/reversed and native-unrepresentable
/// intervals remain unapplied rather than choosing another effect or moving
/// endpoints. Public property definitions: https://gegl.org/operations/gegl-levels.html.
fn levels(effect: &StoredEffect<'_>, ctl: &Interrupt<'_>) -> Result<(Vec<SmartFilter>, &'static str)> {
    allowed(effect, &["in-low", "in-high", "out-low", "out-high"], ctl)?;
    let low = number(effect, "in-low", 0., 0., 1.)?;
    let high = number(effect, "in-high", 1., 0., 1.)?;
    let out_low = number(effect, "out-low", 0., 0., 1.)?;
    let out_high = number(effect, "out-high", 1., 0., 1.)?;
    levels_values(low, high, out_low, out_high, ctl)
}

pub(in crate::xcf) fn levels_values(low: f64, high: f64, out_low: f64, out_high: f64, ctl: &Interrupt<'_>) -> Result<(Vec<SmartFilter>, &'static str)> {
    ctl.check().map_err(|_| Failure::Cancelled)?;
    for v in [low, high, out_low, out_high] {
        value(v, 0., 1.)?;
    }
    let black = low * 255.;
    let white = high * 255.;
    // Validate both the serialized native control domain and the F32 command
    // parser's minimum span; its max(inWhite,inBlack+2) must not move a value.
    if black > 253. || white < 2. || white - black < 2. || (white as f32) < (black as f32) + 2. || out_high <= out_low {
        return Err(Failure::Range("ascending Levels endpoints do not fit the native editable domain"));
    }
    ctl.check().map_err(|_| Failure::Cancelled)?;
    Ok((
        vec![filter("image.adjustments.levels", json!({"inBlack":black,"inWhite":white,"gamma":1.,"outBlack":out_low*255.,"outWhite":out_high*255.}))],
        "Normalized ascending source endpoints become editable native Levels with neutral gamma. Native document-channel transfer, input/output clipping and per-depth quantization take precedence; this is a native approximation, not source GEGL color/clipping equivalence.",
    ))
}

// Shared binary/graph preparation uses the same representable stored-F32
// endpoints. Graph syntax validates its own exact declared domain first.
fn value(v: f64, low: f64, high: f64) -> Result<()> {
    if !v.is_finite() {
        return Err(Failure::Invalid("nonfinite native tone scalar"));
    }
    if !(f64::from(low as f32)..=f64::from(high as f32)).contains(&v) {
        return Err(Failure::Range("native tone scalar"));
    }
    Ok(())
}

#[cfg(test)]
mod values_tests {
    use super::*;
    #[test]
    fn numeric_helpers_share_binary_mapping_without_record_fabrication() {
        use crate::xcf::effects::tests::{scalar, source};
        let ctl = &Interrupt::NONE;
        for (binary, numeric) in [
            (source("gegl:brightness-contrast", vec![scalar("brightness", 0.5), scalar("contrast", -2.)]), brightness_contrast_values(0.5, -2., ctl).unwrap()),
            (source("gegl:exposure", vec![scalar("black-level", 0.1), scalar("exposure", 4.)]), exposure_values(f64::from(0.1f32), 4., ctl).unwrap()),
            (
                source("gegl:levels", vec![scalar("in-low", 0.125), scalar("in-high", 0.875), scalar("out-low", 0.25), scalar("out-high", 0.75)]),
                levels_values(0.125, 0.875, 0.25, 0.75, ctl).unwrap(),
            ),
        ] {
            let mapped = map(&binary, ctl).unwrap();
            assert_eq!(mapped, numeric);
        }
    }
    #[test]
    fn numeric_helpers_reject_nonfinite_out_of_domain_and_cancelled_requests() {
        assert!(brightness_contrast_values(f64::NAN, 1., &Interrupt::NONE).is_err());
        assert!(brightness_contrast_values(0., 5.1, &Interrupt::NONE).is_err());
        assert!(exposure_values(0.2, 0., &Interrupt::NONE).is_err());
        assert!(levels_values(-1., 1., 0., 1., &Interrupt::NONE).is_err());
        let cancel = Interrupt::cancel_only(&|| true);
        assert!(matches!(brightness_contrast_values(0., 1., &cancel), Err(Failure::Cancelled)));
        assert!(matches!(exposure_values(0., 0., &cancel), Err(Failure::Cancelled)));
        assert!(matches!(levels_values(0., 1., 0., 1., &cancel), Err(Failure::Cancelled)));
    }
}
