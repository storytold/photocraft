//! Pure native-data preparation for primary XCF tone effects.
//! Shared craft-xcf owns all framing/configuration decoding. This adapter does
//! not read masks, render pixels, mutate owners or publish a native stage.

pub(super) mod coverage;
pub(super) mod installer;
#[cfg(test)]
mod levels_tests;
#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod masked_exposure_tests;
mod point_color;
#[cfg(test)]
mod point_color_tests;
#[cfg(test)]
mod tests;
pub(in crate::xcf) mod tone;

use craft_xcf::effects::{ArgumentValue, OwnedEffect, Owner, OwnerKind, StoredEffect};
use photocraft_color::BlendMode;
use photocraft_doc::SmartFilter;
use photocraft_raster::Interrupt;
use serde_json::Value;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(super) enum Failure {
    #[error("cancelled")]
    Cancelled,
    #[error("invalid stored effect: {0}")]
    Invalid(&'static str),
    #[error("unsupported stored effect: {0}")]
    Unsupported(&'static str),
    #[error("stored effect exceeds native control range: {0}")]
    Range(&'static str),
}

type Result<T> = std::result::Result<T, Failure>;

/// A reference is not an empty mask: installation must resolve or explicitly
/// reject it, retaining original owner pixels if usable coverage is unavailable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MaskRequirement {
    None,
    Required(u64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Envelope {
    pub visible: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    pub mask: MaskRequirement,
}

#[derive(Debug, PartialEq)]
pub(super) struct Mapping {
    pub owner: Owner,
    /// Filters stay full-strength. Apply the source envelope exactly once around
    /// the compound stage, after complete private preparation/mask admission.
    pub filters: Vec<SmartFilter>,
    pub envelope: Envelope,
    pub notes: Vec<&'static str>,
}

/// None means another adapter may own this identity. A known but rejected stage
/// must remain unapplied with disclosure; cancellation must propagate as fatal.
pub(super) fn map(source: &OwnedEffect<'_>, ctl: &Interrupt<'_>) -> Result<Option<Mapping>> {
    ctl.check().map_err(|_| Failure::Cancelled)?;
    let effect = &source.record;
    let supported = matches!(effect.operation, "gegl:brightness-contrast" | "gegl:exposure" | "gegl:levels")
        || point_color::supports(effect.operation);
    if !supported {
        return Ok(None);
    }
    if !matches!(source.owner.kind, OwnerKind::Layer) {
        return Err(Failure::Unsupported("coverage owners require their own scalar/mask adapter"));
    }
    if !effect.errors.is_empty() || effect.arguments.iter().any(|argument| matches!(argument.value, ArgumentValue::Invalid { .. })) {
        return Err(Failure::Invalid("invalid optional fields or arguments must not acquire native defaults"));
    }
    if !effect.unknown_properties.is_empty() {
        return Err(Failure::Unsupported("unknown source envelope properties require fallback"));
    }
    let version = if effect.operation == "gegl:exposure" { "1:0" } else { "0:0" };
    if effect.version.is_some_and(|stored| stored != version) {
        return Err(Failure::Unsupported("operation version has no native mapping"));
    }
    if !effect.opacity.is_finite() || !(0.0..=1.0).contains(&effect.opacity) {
        return Err(Failure::Invalid("opacity is not finite normalized coverage"));
    }
    if effect.blend_space != 0 || effect.composite_space != 0 || effect.composite_mode != 0 || effect.clip || effect.region != 0 {
        return Err(Failure::Unsupported("explicit source blend/composite/region policy is not installed by this adapter"));
    }
    let blend = match effect.mode {
        0 | 28 | 62 => BlendMode::Normal,
        _ => return Err(Failure::Unsupported("source effect blend mode is outside the initial native adapter")),
    };
    let (filters, note) = if point_color::supports(effect.operation) {
        point_color::map(effect, ctl)?
    } else {
        tone::map(effect, ctl)?
    };
    ctl.check().map_err(|_| Failure::Cancelled)?;
    Ok(Some(Mapping {
        owner: source.owner,
        filters,
        envelope: Envelope {
            visible: effect.visible,
            opacity: effect.opacity,
            blend,
            mask: if effect.mask_offset == 0 { MaskRequirement::None } else { MaskRequirement::Required(effect.mask_offset) },
        },
        notes: vec![note],
    }))
}

fn allowed(effect: &StoredEffect<'_>, names: &[&str], ctl: &Interrupt<'_>) -> Result<()> {
    for (index, argument) in effect.arguments.iter().enumerate() {
        ctl.check().map_err(|_| Failure::Cancelled)?;
        if !names.contains(&argument.name) {
            return Err(Failure::Unsupported("unmapped scalar argument"));
        }
        if effect.arguments.get(..index).is_some_and(|earlier| earlier.iter().any(|previous| previous.name == argument.name)) {
            return Err(Failure::Invalid("duplicate stored argument"));
        }
    }
    Ok(())
}

fn number(effect: &StoredEffect<'_>, name: &str, default: f64, low: f64, high: f64) -> Result<f64> {
    let value = match effect.arguments.iter().find(|argument| argument.name == name).map(|argument| &argument.value) {
        None => default,
        Some(ArgumentValue::Int(value)) => f64::from(*value),
        Some(ArgumentValue::UInt(value)) => f64::from(*value),
        Some(ArgumentValue::Float(value)) => f64::from(*value),
        _ => return Err(Failure::Invalid("scalar argument has a nonnumeric storage type")),
    };
    if !value.is_finite() {
        return Err(Failure::Invalid("scalar argument is not finite"));
    }
    // Stored declared doubles use FILTER_PROP_FLOAT. Admit the representable
    // F32 endpoints (not an epsilon), including the source black-level +/-0.1.
    if !(f64::from(low as f32)..=f64::from(high as f32)).contains(&value) {
        return Err(Failure::Range("source scalar"));
    }
    Ok(value)
}

fn filter(command: &'static str, params: Value) -> SmartFilter {
    SmartFilter { command: command.into(), params, blend: BlendMode::Normal, opacity: 1.0, visible: true }
}
