//! Photoshop's Video, Other, Average and matting steps (#10607): De-Interlace, NTSC Colors,
//! HSB/HSL, Custom, Average and Remove Black / White Matte, with the keys Photoshop records.

use super::descriptors::{desc, enm, get_enum, get_list, get_num, num, trimmed};
use super::events::pixel_target;
use super::fields::{Mapped, step};
use super::tables::{lookup, lookup_back};
use super::{D, Descriptor, Map, Value, json};

/// Whether every key of `d` is one of `keys` (codes compare without their space padding).
fn only_keys(d: &Descriptor, keys: &[&str]) -> bool {
    d.items.iter().all(|(k, _)| keys.contains(&trimmed(k).as_str()))
}

/// Whether `p` sets nothing but `allowed` (a `null` counts as unset).
fn sets_only(p: &Value, allowed: &[&str]) -> bool {
    p.as_object().is_none_or(|m| m.iter().all(|(k, v)| v.is_null() || allowed.contains(&k.as_str())))
}

/// NTSC Colors, Average and Remove Black / White Matte: Photoshop runs them with an empty
/// descriptor. A step that carries a key keeps the Photoshop step.
pub(super) fn bare_from(d: &Descriptor, id: &str) -> Mapped {
    if d.items.is_empty() { step(id, json!({})) } else { Mapped::Unsupported { harmless: false } }
}

/// The empty descriptor, for a step that edits the layer's pixels and sets nothing else.
pub(super) fn bare_to(_: &str, p: &Value) -> Option<Descriptor> {
    (pixel_target(p) && sets_only(p, &["target"])).then(|| desc("null"))
}

/// An enumeration key: (key, param, type, Photoshop ↔ PhotoCraft values, Photoshop's default).
type EnumKey = (&'static str, &'static str, &'static str, &'static [(&'static str, &'static str)], &'static str);

pub(super) const DE_INTERLACE: &[EnumKey] = &[
    ("IntE", "eliminate", "IntE", &[("ElmO", "oddFields"), ("ElmE", "evenFields")], "ElmO"),
    ("IntC", "createBy", "IntC", &[("CrtI", "interpolation"), ("CrtD", "duplication")], "CrtI"),
];

const HSB_MODELS: &[(&str, &str)] = &[("RGBC", "rgb"), ("HSBl", "hsb"), ("HSLC", "hsl")];

pub(super) const HSB_HSL: &[EnumKey] = &[("Inpt", "inputMode", "ClrS", HSB_MODELS, "RGBC"), ("Otpt", "rowOrder", "ClrS", HSB_MODELS, "HSBl")];

/// A step of enumerations only: each value looked up, Photoshop's default for a missing one.
/// An unknown value or key keeps the Photoshop step.
pub(super) fn enums_from(d: &Descriptor, id: &str, keys: &[EnumKey]) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    let names: Vec<&str> = keys.iter().map(|k| k.0).collect();
    if !only_keys(d, &names) {
        return unsupported;
    }
    let mut p = Map::new();
    for (key, param, _, table, default) in keys {
        let Some(v) = lookup(table, &get_enum(d, key).unwrap_or_else(|| (*default).to_string())) else { return unsupported };
        p.insert((*param).into(), v.into());
    }
    step(id, Value::Object(p))
}

/// Every key in recorded order; a value left out is the command's default, which is
/// Photoshop's for these two filters.
pub(super) fn enums_to(p: &Value, keys: &[EnumKey]) -> Option<Descriptor> {
    let params: Vec<&str> = keys.iter().map(|k| k.1).chain(["target"]).collect();
    if !pixel_target(p) || !sets_only(p, &params) {
        return None;
    }
    let mut d = desc("null");
    for (key, param, ty, table, default) in keys {
        let code = match p.get(*param) {
            None | Some(Value::Null) => *default,
            Some(v) => lookup_back(table, v.as_str()?)?,
        };
        d = d.with(key, enm(ty, code));
    }
    Some(d)
}

/// Custom (`Cstm`): `Mtrx` is the 5 × 5 kernel, row by row. Photoshop takes exactly 25 cells (a
/// 3 × 3 list makes it fail), so any other matrix keeps the Photoshop step.
pub(super) fn custom_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    if !only_keys(d, &["Scl", "Ofst", "Mtrx"]) {
        return unsupported;
    }
    let Some(cells) = get_list(d, "Mtrx") else { return unsupported };
    let kernel: Option<Vec<f64>> = cells.iter().map(num).collect();
    let Some(kernel) = kernel.filter(|k| k.len() == 25) else { return unsupported };
    let mut p = Map::new();
    p.insert("kernel".into(), json!(kernel));
    p.insert("scale".into(), get_num(d, "Scl ").unwrap_or(1.0).into());
    p.insert("offset".into(), get_num(d, "Ofst").unwrap_or(0.0).into());
    step("filter.other.custom", Value::Object(p))
}

/// `Scl `, `Ofst` and the 25 cells as integers; a kernel left out is the command's identity
/// (1 in the middle). A fraction anywhere refuses the step: it would replay rounded.
pub(super) fn custom_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !pixel_target(p) || !sets_only(p, &["target", "kernel", "scale", "offset"]) {
        return None;
    }
    let int = |v: &Value| {
        let n = v.as_f64()?;
        (n.fract() == 0.0 && (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&n)).then_some(n as i32)
    };
    let field = |k: &str, default: i32| match p.get(k) {
        None | Some(Value::Null) => Some(default),
        Some(v) => int(v),
    };
    let cells: Vec<i32> = match p.get("kernel") {
        None | Some(Value::Null) => (0..25).map(|i| i32::from(i == 12)).collect(),
        Some(v) => v.as_array()?.iter().map(int).collect::<Option<_>>()?,
    };
    if cells.len() != 25 {
        return None;
    }
    Some(
        desc("null")
            .with("Scl ", D::Integer(field("scale", 1)?))
            .with("Ofst", D::Integer(field("offset", 0)?))
            .with("Mtrx", D::List(cells.into_iter().map(D::Integer).collect())),
    )
}
