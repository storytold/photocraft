//! Scale Effects, Make-from-layer-effects and supported whole-style Set forms.
use super::descriptors::{PCT, class, class_ref, desc, get_obj, get_ref, jstr, pct, ref_class, trimmed};
use super::fields::{Mapped, step};
use super::{D, Descriptor, Id, ReferenceItem, Value, json};

const SCALE: &str = "layer.layerStyle.scaleEffects";
pub(super) const CREATE: &str = "layer.layerStyle.createLayer";
pub(super) const REPLACE: &str = "layer.layerStyle.replace";
const SOURCE: &str = "_atnLayerStyleDescriptor";

/// Only documented keys, once each: unknown or repeated settings cannot be discarded.
fn keys(d: &Descriptor, expected: &[&str]) -> bool {
    d.items.len() == expected.len() && expected.iter().all(|key| d.items.iter().filter(|(k, _)| k.is(key)).count() == 1)
}

/// The dialog records whole percentages; native fractional/clamped values are not guessed.
fn whole_percent(value: f64) -> bool {
    value.is_finite() && (1.0..=1000.0).contains(&value) && value.fract() == 0.0
}

/// The native Create Layers operation has only the measured implicit active-layer target.
fn create_descriptor() -> Descriptor {
    desc("null").with("null", class_ref("Lyr ")).with(
        "Usng",
        D::Reference(vec![
            ReferenceItem::Property { class: class("Prpr"), key: Id::new("Lefx") },
            ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
        ]),
    )
}

/// Parse the exact recorded values and reference chain, keeping unsupported forms complete.
fn parameters(id: &str, d: &Descriptor) -> Option<Value> {
    if id == REPLACE {
        return replacement_parameters(d);
    }
    if id == SCALE {
        if !keys(d, &["Scl "]) {
            return None;
        }
        let D::UnitFloat { unit, value } = d.get("Scl ")? else { return None };
        return (*unit == PCT && whole_percent(*value)).then(|| json!({"scale":value}));
    }
    if id != CREATE || !keys(d, &["null", "Usng"]) {
        return None;
    }
    let [ReferenceItem::Class(layer)] = get_ref(d, "null")? else { return None };
    let [ReferenceItem::Property { class: property, key }, ReferenceItem::Enumerated { class: target, type_id, value }] = get_ref(d, "Usng")? else {
        return None;
    };
    (layer.class_id.is("Lyr ") && property.class_id.is("Prpr") && key.is("Lefx") && target.class_id.is("Lyr ") && type_id.is("Ordn") && value.is("Trgt"))
        .then(|| json!({}))
}

/// Build supported native values explicitly; an absent scale uses the native default of 100.
fn descriptor(id: &str, p: &Value) -> Option<Descriptor> {
    if id == REPLACE {
        return replacement_descriptor(p);
    }
    let object = p.as_object()?;
    if object.keys().any(|key| key != SOURCE && !(id == SCALE && key == "scale")) {
        return None;
    }
    match id {
        SCALE => {
            let value = match p.get("scale") {
                None => 100.0,
                Some(v) => v.as_f64()?,
            };
            whole_percent(value).then(|| desc("null").with("Scl ", pct(value)))
        }
        CREATE => Some(create_descriptor()),
        _ => None,
    }
}

/// Preserve wire identity/order while unchanged, without making it a native option.
fn from(id: &str, d: &Descriptor) -> Mapped {
    let Some(mut p) = parameters(id, d) else { return Mapped::Unsupported { harmless: false } };
    let Some(canonical) = descriptor(id, &p) else { return Mapped::Unsupported { harmless: false } };
    if canonical != *d {
        p[SOURCE] = photocraft_paint::tile::b64_encode(&d.to_bytes()).into();
    }
    step(id, p)
}

/// Import Scale Effects only when the whole recorded percentage is represented.
pub(super) fn scale_from(d: &Descriptor) -> Mapped {
    from(SCALE, d)
}

/// A whole Lefx property is a replacement, not an update of one existing effect.
pub(super) fn replace_from(d: &Descriptor) -> Mapped {
    from(REPLACE, d)
}

fn replacement_reference() -> D {
    D::Reference(vec![
        ReferenceItem::Property { class: class("Prpr"), key: Id::new("Lefx") },
        ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
    ])
}

/// Accept one complete RGB Color Overlay, with no omitted effect or unrepresented switch.
fn replacement_parameters(d: &Descriptor) -> Option<Value> {
    if !keys(d, &["null", "T   "]) {
        return None;
    }
    let [ReferenceItem::Property { class: property, key }, ReferenceItem::Enumerated { class: layer, type_id, value }] = get_ref(d, "null")? else {
        return None;
    };
    if !property.class_id.is("Prpr") || !key.is("Lefx") || !layer.class_id.is("Lyr ") || !type_id.is("Ordn") || !value.is("Trgt") {
        return None;
    }
    let fx = get_obj(d, "T   ")?;
    if !fx.class_id.is("Lefx")
        || !keys(fx, &["Scl ", "SoFi"])
        || !matches!(fx.get("Scl "), Some(D::UnitFloat { unit, value }) if *unit == PCT && *value == 100.0)
    {
        return None;
    }
    let overlay = get_obj(fx, "SoFi")?;
    let allowed = ["enab", "present", "showInDialog", "Md  ", "Clr ", "Opct"];
    if !overlay.class_id.is("SoFi")
        || overlay
            .items
            .iter()
            .enumerate()
            .any(|(i, (key, _))| !allowed.iter().any(|known| key.is(known)) || overlay.items.iter().take(i).any(|(old, _)| old == key))
        || !matches!(overlay.get("enab"), Some(D::Boolean(_)))
        || ["present", "showInDialog"].iter().any(|key| !matches!(overlay.get(key), None | Some(D::Boolean(true))))
        || !matches!(overlay.get("Opct"), Some(D::UnitFloat { unit, value }) if *unit == PCT && value.is_finite() && (0.0..=100.0).contains(value))
    {
        return None;
    }
    let Some(D::Enumerated { type_id, value }) = overlay.get("Md  ") else { return None };
    if !type_id.is("BlnM") {
        return None;
    }
    let blend = crate::commands::blend_from_str(super::tables::blend_label(&trimmed(value))?)?;
    let color = get_obj(overlay, "Clr ")?;
    if !color.class_id.is("RGBC")
        || !keys(color, &["Rd  ", "Grn ", "Bl  "])
        || color.items.iter().any(|(_, value)| !match value {
            D::Double(value) => value.is_finite() && (0.0..=255.0).contains(value),
            D::Integer(value) => (0..=255).contains(value),
            _ => false,
        })
    {
        return None;
    }
    let (_, effects) = photocraft_io::effects_map::effects_from_descriptor(fx);
    let [effect @ photocraft_doc::Effect::ColorOverlay { common, .. }] = effects.as_slice() else { return None };
    // The PSD reader's fallback for an unknown blend is not a supported action mapping.
    if common.blend != blend {
        return None;
    }
    Some(json!({"effects":[{"kind":"colorOverlay","params":{},"fx":serde_json::to_value(effect).ok()?}]}))
}

fn replacement_descriptor(p: &Value) -> Option<Descriptor> {
    if p.as_object()?.iter().any(|(key, value)| !value.is_null() && !["effects", SOURCE].contains(&key.as_str())) {
        return None;
    }
    let [entry] = p.get("effects")?.as_array()?.as_slice() else { return None };
    if entry.as_object()?.keys().any(|key| !["kind", "params", "fx"].contains(&key.as_str())) || jstr(entry, "kind")? != "colorOverlay" {
        return None;
    }
    let empty = json!({});
    let params = entry.get("params").filter(|value| !value.is_null()).unwrap_or(&empty);
    if params.as_object()?.keys().any(|key| !["color", "opacity", "blend"].contains(&key.as_str())) {
        return None;
    }
    if let Some(value) = params.get("opacity")
        && !value.as_f64().is_some_and(|n| n.is_finite() && (0.0..=100.0).contains(&n))
    {
        return None;
    }
    if let Some(value) = params.get("color") {
        let color = value.as_str()?.strip_prefix('#').unwrap_or(value.as_str()?);
        if color.len() != 6 || !color.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
    }
    if let Some(value) = params.get("blend") {
        super::tables::blend_ps(value.as_str()?)?;
    }
    let mut effect = if let Some(snapshot) = entry.get("fx") {
        if snapshot.as_object()?.len() != 1 || snapshot.get("ColorOverlay").is_none() {
            return None;
        }
        serde_json::from_value::<photocraft_doc::Effect>(snapshot.clone()).ok()?
    } else {
        crate::layer_style::effect_from_params("colorOverlay", params)?
    };
    if !matches!(effect, photocraft_doc::Effect::ColorOverlay { .. }) {
        return None;
    }
    crate::layer_style::overlay_effect(&mut effect, params);
    let mut fx = photocraft_io::effects_map::effects_descriptor(true, std::slice::from_ref(&effect));
    fx.class_id = Id::new("Lefx");
    fx.items.retain(|(key, _)| key.is("Scl ") || key.is("SoFi"));
    let canonical = desc("null").with("null", replacement_reference()).with("T   ", D::Descriptor(fx));
    // Reject snapshots the constrained wire form would clamp, change colour space or omit.
    let back = replacement_parameters(&canonical)?;
    let back = back.get("effects")?.as_array()?.first()?.get("fx")?;
    (serde_json::from_value::<photocraft_doc::Effect>(back.clone()).ok()? == effect).then_some(canonical)
}

/// Find typed effects references in parsed values; opaque data and class names are not references.
fn has_style_reference(value: &D) -> bool {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            D::Reference(parts) if parts.iter().any(|part| matches!(part, ReferenceItem::Property { key, .. } if key.is("Lefx"))) => {
                return true;
            }
            D::Reference(_) => {}
            D::Descriptor(d) | D::GlobalObject(d) => pending.extend(d.items.iter().map(|(_, value)| value)),
            D::List(values) => pending.extend(values),
            D::ObjectArray(array) => pending.extend(array.body.items.iter().map(|(_, value)| value)),
            _ => {}
        }
    }
    false
}

/// Intercept layer-style Make intent before generic Make can turn it into a blank layer.
pub(super) fn create_from_descriptor(d: &Descriptor) -> Option<Mapped> {
    let layer_make = get_ref(d, "null").and_then(ref_class).as_deref() == Some("Lyr");
    let style_intent = d.items.iter().any(|(item_key, value)| {
        // Deep inspection belongs to Layer Make; other Make classes retain direct-reference handling.
        let style_reference = (layer_make || matches!(value, D::Reference(_))) && has_style_reference(value);
        if item_key.is("Usng") {
            // A layer Make may omit Usng, or use the ordinary layer-options object. Other
            // shapes must not be discarded as if they were an empty layer-options object.
            style_reference || (layer_make && !matches!(value, D::Descriptor(u) if u.class_id.is("Lyr ")))
        } else {
            // A miskeyed effects reference is still style intent, not an ordinary blank layer.
            layer_make && style_reference
        }
    });
    style_intent.then(|| from(CREATE, d))
}

/// Validate preserved data before fallback; supported edits export their current canonical values.
pub(super) fn style_to(id: &str, p: &Value) -> Option<Descriptor> {
    let canonical = descriptor(id, p)?;
    if p.get(SOURCE).is_none() {
        return Some(canonical);
    }
    let source = Descriptor::from_bytes(&photocraft_paint::tile::b64_decode(jstr(p, SOURCE)?)?).ok()?;
    let original = descriptor(id, &parameters(id, &source)?)?;
    Some(if original == canonical { source } else { canonical })
}
