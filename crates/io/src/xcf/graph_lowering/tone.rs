//! Typed graph numbers reuse the same verified native controls as binary effects.
use super::{Failure, Result};
use crate::xcf::effects;
use craft_xcf::effects::graph::{Node, Value};
use photocraft_doc::SmartFilter;
use photocraft_raster::Interrupt;

pub(super) fn is_operation(operation: &str) -> bool {
    matches!(operation, "brightness-contrast" | "gegl:brightness-contrast" | "exposure" | "gegl:exposure" | "levels" | "gegl:levels")
}

fn number(node: &Node, name: &str, default: f64, low: f64, high: f64) -> Result<f64> {
    let value = match node.properties.iter().find(|property| property.name == name) {
        None => default,
        Some(property) => match property.value {
            Value::Number(value) => value,
            _ => return Err(Failure::Invalid("graph tone property is not a typed number")),
        },
    };
    if !value.is_finite() {
        return Err(Failure::Invalid("graph tone number is not finite"));
    }
    // Graph numbers are actual F64 literals, unlike FILTER_PROP_FLOAT storage.
    // Validate their exact domain before the shared native helper accepts the
    // slightly wider representable F32 endpoints needed by binary source data.
    if !(low..=high).contains(&value) {
        return Err(Failure::Unsupported("graph tone number exceeds the editable native domain"));
    }
    Ok(value)
}

pub(super) fn map(node: &Node, ctl: &Interrupt<'_>) -> Result<Vec<SmartFilter>> {
    let names: &[&str] = match node.operation.as_str() {
        "brightness-contrast" | "gegl:brightness-contrast" => &["brightness", "contrast"],
        "exposure" | "gegl:exposure" => &["black-level", "exposure"],
        "levels" | "gegl:levels" => &["in-low", "in-high", "out-low", "out-high"],
        _ => return Err(Failure::Unsupported("graph tone operation")),
    };
    if node.properties.len() > names.len() {
        return Err(Failure::Unsupported("graph tone property count exceeds its bounded schema"));
    }
    for (index, property) in node.properties.iter().enumerate() {
        ctl.check().map_err(|_| Failure::Cancelled)?;
        if !names.contains(&property.name.as_str()) {
            return Err(Failure::Unsupported("unmapped graph tone property"));
        }
        if node.properties.iter().take(index).any(|earlier| earlier.name == property.name) {
            return Err(Failure::Invalid("duplicate graph tone property"));
        }
    }
    let mapped = match node.operation.as_str() {
        "brightness-contrast" | "gegl:brightness-contrast" => {
            effects::tone::brightness_contrast_values(number(node, "brightness", 0., -3., 3.)?, number(node, "contrast", 1., -5., 5.)?, ctl)
        }
        "exposure" | "gegl:exposure" => {
            effects::tone::exposure_values(number(node, "black-level", 0., -0.1, 0.1)?, number(node, "exposure", 0., -20., 20.)?, ctl)
        }
        "levels" | "gegl:levels" => effects::tone::levels_values(
            number(node, "in-low", 0., 0., 1.)?,
            number(node, "in-high", 1., 0., 1.)?,
            number(node, "out-low", 0., 0., 1.)?,
            number(node, "out-high", 1., 0., 1.)?,
            ctl,
        ),
        _ => return Err(Failure::Unsupported("graph tone operation")),
    };
    mapped.map(|(filters, _note)| filters).map_err(|error| match error {
        effects::Failure::Cancelled => Failure::Cancelled,
        effects::Failure::Invalid(_) => Failure::Invalid("invalid native graph tone control"),
        effects::Failure::Range(_) | effects::Failure::Unsupported(_) => Failure::Unsupported("graph tone control is not natively representable"),
        effects::Failure::Limit => Failure::Storage,
    })
}
