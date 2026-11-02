use super::descriptors::{
    PX, Target, ang, boolean, class, code, desc, enm, enumerated, get_bool, get_enum, get_num, get_obj, get_ref, jbool, jnum, jstr, layer_ref, layer_target,
    long, pct, px, ref_class, target, trimmed,
};
use super::events::pixel_target;
use super::fields::{F, Kind, Mapped, fields_from, fields_to, step};
use super::tables::{CHANNELS, INTERPOLATION, RADIAL_METHOD, RADIAL_QUALITY, channel_of, lookup, lookup_back};
use super::{D, Descriptor, Map, ReferenceItem, UnicodeString, Value, json};

/// `transform`: Photoshop records a Free Transform relative to the frame (offset of its centre,
/// width/height in percent, angle). A layer moved only becomes `layer.translate`; anything
/// else is `edit.transform`'s relative form; the selection's frame is `select.transformSelection`.
pub(super) fn transform_from(d: &Descriptor) -> Mapped {
    // No reference (a smart object replaced in place, `replaceLayer`) means the target layer.
    let target_layer = [ReferenceItem::Enumerated { class: class("Lyr "), type_id: code("Ordn"), value: code("Trgt") }];
    let r: &[ReferenceItem] = get_ref(d, "null").unwrap_or(&target_layer);
    let unsupported = Mapped::Unsupported { harmless: false };
    let mut p = Map::new();
    if let Some(o) = get_obj(d, "Ofst") {
        let in_px = |k: &str| matches!(o.get(k), Some(D::UnitFloat { unit: PX, .. }));
        if !in_px("Hrzn") || !in_px("Vrtc") {
            return unsupported;
        }
        if let Some(x) = get_num(o, "Hrzn") {
            p.insert("dx".into(), x.into());
        }
        if let Some(y) = get_num(o, "Vrtc") {
            p.insert("dy".into(), y.into());
        }
    }
    for (key, param) in [("Wdth", "scaleX"), ("Hght", "scaleY"), ("Angl", "rotate")] {
        if let Some(v) = get_num(d, key) {
            p.insert(param.into(), v.into());
        }
    }
    if d.get("Skew").is_some() || d.get("Usng").is_some() {
        return unsupported;
    }
    // A whole-pixel move is a plain `layer.translate`; a fractional one needs `edit.transform`
    // to keep its sub-pixel placement and interpolation.
    let whole_pixels = ["dx", "dy"].iter().all(|k| p.get(*k).and_then(Value::as_f64).is_none_or(|v| v.fract() == 0.0));
    let geometry_only = p.keys().all(|k| k == "dx" || k == "dy") && whole_pixels;
    // The resampling applies to a selection as much as to a layer (#7515).
    // Photoshop's bicubic variants are kept (the transforms render them as Bicubic, and they export
    // back unchanged); Automatic and Preserve Details become the nearest bicubic choice. Dropping
    // them fell back to the command's default, bilinear for a selection (#8697).
    let interpolation = get_enum(d, "Intr").and_then(|i| lookup(INTERPOLATION, &i)).and_then(|i| match i {
        "bicubic" | "bilinear" | "nearest" | "bicubicSharper" | "bicubicSmoother" | "bicubicAutomatic" => Some(i),
        "automatic" => Some("bicubicAutomatic"),
        "preserveDetails" => Some("bicubic"),
        _ => None,
    });
    match (ref_class(r).as_deref(), target(r)) {
        (Some("Chnl"), Target::Property(prop)) if prop == "fsel" => {
            if let Some(i) = interpolation {
                p.insert("interpolation".into(), i.into());
            }
            step("select.transformSelection", Value::Object(p))
        }
        (Some("Lyr"), _) => {
            let Some(mut lp) = layer_target(r) else { return unsupported };
            if geometry_only {
                lp.insert("dx".into(), p.get("dx").and_then(Value::as_f64).unwrap_or(0.0).into());
                lp.insert("dy".into(), p.get("dy").and_then(Value::as_f64).unwrap_or(0.0).into());
                return step("layer.translate", Value::Object(lp));
            }
            lp.extend(p);
            if let Some(i) = interpolation {
                lp.insert("interpolation".into(), i.into());
            }
            step("edit.transform", Value::Object(lp))
        }
        _ => unsupported,
    }
}

pub(super) fn transform_to(id: &str, p: &Value) -> Option<Descriptor> {
    let given = |k: &str| p.get(k).is_some_and(|v| !v.is_null());
    if ["quad", "corners", "matrix", "rect"].iter().any(|k| given(k)) {
        return None;
    }
    // The descriptor scales and rotates about the centre (`FTcs` average) without skew or warp:
    // another reference point, a skew or a warp would replay as a different transform (#7516,
    // #7517, #7518, #7519).
    let nonzero = |k: &str| p.get(k).is_some_and(|v| !v.is_null() && v.as_f64() != Some(0.0));
    let off_centre = p.get("reference").is_some_and(|v| !v.is_null() && *v != json!("center"));
    if nonzero("skewX") || nonzero("skewY") || off_centre || ["warp", "style", "mesh", "grid"].iter().any(|k| given(k)) {
        return None;
    }
    let target = match id {
        // The `Trnf` descriptor has no field for a mask, Quick Mask or alpha channel target; a
        // layer reference would transform the layer's pixels instead.
        "edit.transform" if !pixel_target(p) => return None,
        "edit.transform" => layer_ref(p)?,
        "select.transformSelection" => D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: code("fsel") }]),
        _ => return None,
    };
    let mut d = desc("null").with("null", target).with("FTcs", enm("QCSt", "Qcsa"));
    d = d.with("Ofst", D::Descriptor(desc("Ofst").with("Hrzn", px(jnum(p, "dx").unwrap_or(0.0))).with("Vrtc", px(jnum(p, "dy").unwrap_or(0.0)))));
    for (key, param) in [("Wdth", "scaleX"), ("Hght", "scaleY")] {
        if let Some(v) = jnum(p, param) {
            d = d.with(key, pct(v));
        }
    }
    if let Some(a) = jnum(p, "rotate") {
        d = d.with("Angl", ang(a));
    }
    if let Some(i) = jstr(p, "interpolation").and_then(|i| lookup_back(INTERPOLATION, i)) {
        d = d.with("Intr", enm("Intp", i));
    }
    Some(d)
}

/// `Clcn` (Apply Image / Calculations blending) ↔ `image.applyImage`'s `blending`.
const CALC_BLEND: &[(&str, &str)] = &[
    ("Nrml", "normal"),
    ("Mltp", "multiply"),
    ("Scrn", "screen"),
    ("Ovrl", "overlay"),
    ("SftL", "softLight"),
    ("HrdL", "hardLight"),
    ("CDdg", "colorDodge"),
    ("CBrn", "colorBurn"),
    ("Drkn", "darken"),
    ("Lghn", "lighten"),
    ("Dfrn", "difference"),
    ("Xclu", "exclusion"),
    ("linearBurn", "linearBurn"),
    ("linearDodge", "linearDodge"),
    ("Add ", "add"),
    ("Sbtr", "subtract"),
];

const SOURCE_CHANNELS: &[(&str, &str)] =
    &[("RGB", "composite"), ("CMYK", "composite"), ("Lab", "composite"), ("Trsp", "transparency"), ("Msk", "mask"), ("fsel", "selection")];

pub(super) fn apply_image_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    let Some(with) = get_obj(d, "With") else { return unsupported };
    let Some(r) = get_ref(with, "T   ") else { return unsupported };
    let mut source = Map::new();
    for item in r {
        match item {
            ReferenceItem::Enumerated { class, value, .. } if class.class_id.is("Chnl") => {
                let v = trimmed(value);
                let Some(channel) = lookup(SOURCE_CHANNELS, &v).or_else(|| channel_of(&v)) else { return unsupported };
                source.insert("channel".into(), channel.into());
            }
            // A named (alpha) channel.
            ReferenceItem::Name { class, name } if class.class_id.is("Chnl") => {
                source.insert("channel".into(), name.to_string_lossy().into());
            }
            ReferenceItem::Name { class, name } if class.class_id.is("Lyr ") => {
                source.insert("layer".into(), name.to_string_lossy().into());
            }
            ReferenceItem::Enumerated { class, value, .. } if class.class_id.is("Lyr ") && trimmed(value) == "Mrgd" => {
                source.insert("layer".into(), "merged".into());
            }
            _ => return unsupported,
        }
    }
    if let Some(i) = get_bool(with, "Invr") {
        source.insert("invert".into(), i.into());
    }
    let mut p = json!({"source": source});
    if let Some(c) = get_enum(with, "Clcl") {
        match lookup(CALC_BLEND, &c) {
            Some(b) => p["blending"] = json!(b),
            None => return unsupported,
        }
    }
    for (key, param) in [("Scl ", "scale"), ("Ofst", "offset"), ("Opct", "opacity")] {
        if let Some(v) = get_num(with, key) {
            p[param] = json!(v);
        }
    }
    if let Some(t) = get_bool(with, "PrsT") {
        p["preserveTransparency"] = json!(t);
    }
    step("image.applyImage", p)
}

/// Apply Image's source channel as a `Chnl` reference item. The dialog sends Channels-panel names
/// ("RGB", "Red", "Alpha 1"), which the command reads case-insensitively: the composite names map to
/// the composite, colour names to their enumeration, and any other name to a named (alpha) channel,
/// as Photoshop records it.
fn apply_image_channel(channel: &str) -> Option<ReferenceItem> {
    let l = channel.to_ascii_lowercase();
    let key = match l.as_str() {
        "rgb" | "cmyk" | "lab" | "merged" => "composite",
        "layermask" => "mask",
        // Sources the command knows by keyword but Photoshop has no channel for.
        "quickmask" | "quick mask" | "vectormask" | "vector mask" => return None,
        other => other,
    };
    Some(match lookup_back(SOURCE_CHANNELS, key).or_else(|| lookup_back(CHANNELS, key)) {
        Some(chan) => ReferenceItem::Enumerated { class: class("Chnl"), type_id: code("Chnl"), value: code(chan) },
        None if !channel.trim().is_empty() => ReferenceItem::Name { class: class("Chnl"), name: UnicodeString::new_nul(channel) },
        None => return None,
    })
}

pub(super) fn apply_image_to(_: &str, p: &Value) -> Option<Descriptor> {
    // The optional blending mask has no descriptor mapping: refuse it rather than drop it,
    // which would widen the edit to the whole layer.
    let mask_channel = match p.get("mask") {
        Some(m) => m.get("channel"),
        None => p.get("maskChannel"),
    };
    if mask_channel.is_some_and(|c| !(c.is_null() || c == "none" || c == "")) {
        return None;
    }
    // The Apply Image dialog sends flat `sourceLayer`/`sourceChannel`/`sourceInvert` fields; read
    // them as the command does, or the export would quietly reset them to the merged composite.
    let source = crate::channel_cmds::source_spec(p, "source", "source").unwrap_or(json!({}));
    // A source from another open document has no event form here: skip the step, don't rewrite it.
    if source.get("document").is_some_and(|d| !d.is_null()) {
        return None;
    }
    // A layer id or channel index has no name to record: refuse it rather than write the composite.
    let not_name = |k: &str| source.get(k).is_some_and(|v| !(v.is_null() || v.is_string()));
    if !source.is_object() || not_name("layer") || not_name("channel") {
        return None;
    }
    let mut items = vec![apply_image_channel(jstr(&source, "channel").unwrap_or("composite"))?];
    match jstr(&source, "layer") {
        Some("merged") | None => items.push(ReferenceItem::Enumerated { class: class("Lyr "), type_id: code("Ordn"), value: code("Mrgd") }),
        Some(name) => items.push(ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul(name) }),
    }
    let mut with = desc("null").with("T   ", D::Reference(items));
    if let Some(i) = jbool(&source, "invert") {
        with = with.with("Invr", D::Boolean(i));
    }
    with = with.with("Clcl", enm("Clcn", lookup_back(CALC_BLEND, jstr(p, "blending").unwrap_or("multiply"))?));
    if let Some(v) = jnum(p, "opacity") {
        with = with.with("Opct", pct(v));
    }
    if let Some(v) = jnum(p, "scale") {
        with = with.with("Scl ", D::Double(v));
    }
    if let Some(v) = jnum(p, "offset") {
        with = with.with("Ofst", D::Double(v));
    }
    if let Some(t) = jbool(p, "preserveTransparency") {
        with = with.with("PrsT", D::Boolean(t));
    }
    Some(desc("null").with("With", D::Descriptor(with)))
}

pub(super) const RIPPLE_SIZE: &[(&str, &str)] = &[("Sml", "small"), ("Mdm", "medium"), ("Lrg", "large")];

/// Whether a Select and Mask step carries Refine Edge Brush strokes or the workspace's tool
/// edits (Quick Selection, Brush, Lasso…), which no descriptor holds.
pub(super) fn has_refine_strokes(p: &Value) -> bool {
    p.get("refineStrokes").is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty())) || crate::select_mask_edits::has_edits(p)
}

/// Select and Mask to Photoshop. The descriptor has no field for the Refine Edge Brush or the
/// workspace's tools, so a step with `refineStrokes` or `edits` can't be exported faithfully. One with `sampleAllLayers` is not
/// exported here either: it exports as `smartBrushWorkspace`.
pub(super) fn refine_edge_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !pixel_target(p) || p.get("sampleAllLayers").is_some() || has_refine_strokes(p) {
        return None;
    }
    fields_to(p, REFINE_EDGE, desc("null"))
}

/// Select and Mask (`refineSelectionEdge`). Smart Radius, decontamination and the output follow
/// Photoshop's; the edge itself is PhotoCraft's own matting, so results differ in detail.
pub(super) const REFINE_EDGE: &[F] = &[
    F("refineEdgeBorderRadius", "radius", Kind::Px),
    F("refineEdgeAutoRadius", "smartRadius", Kind::Bool),
    F("refineEdgeSmooth", "smooth", Kind::Long),
    F("refineEdgeFeatherRadius", "feather", Kind::Px),
    F("refineEdgeBorderContrast", "contrast", Kind::Pct),
    F("refineEdgeChoke", "shiftEdge", Kind::Pct),
    F("refineEdgeDecontaminate", "decontaminate", Kind::Bool),
    F("refineEdgeDeconAmount", "amount", Kind::Pct),
    F("refineEdgeOutput", "output", Kind::Enum("refineEdgeOutput", REFINE_OUTPUT)),
];

const REFINE_OUTPUT: &[(&str, &str)] = &[
    ("refineEdgeOutputSelection", "selection"),
    ("refineEdgeOutputUserMask", "layerMask"),
    ("refineEdgeOutputNewLayer", "newLayer"),
    ("refineEdgeOutputNewLayerWithMask", "newLayerWithMask"),
    ("refineEdgeOutputNewDocument", "newDocument"),
    ("refineEdgeOutputNewDocumentWithMask", "newDocumentWithMask"),
    // Newer Photoshop's name for Output To: Selection.
    ("selectionOutputToSelection", "selection"),
];

/// Filter Gallery effects (`GEfc` with the effect in `GEfk`): Photoshop's effect code, the
/// command, and its fields. A stack of several effects stays unsupported.
const GALLERY: &[(&str, &str, &[F])] = &[("Spt", "filter.gallery.spatter", &[F("SprR", "sprayRadius", Kind::Long), F("Smth", "smoothness", Kind::Long)])];

pub(super) const GALLERY_IDS: &[&str] = &["filter.gallery.spatter"];

pub(super) fn gallery_from(d: &Descriptor) -> Mapped {
    let Some(effect) = get_enum(d, "GEfk") else { return Mapped::Unsupported { harmless: false } };
    match GALLERY.iter().find(|(code, _, _)| *code == effect).and_then(|(_, id, fields)| Some((*id, fields_from(d, fields)?))) {
        Some((id, p)) => step(id, Value::Object(p)),
        None => Mapped::Unsupported { harmless: false },
    }
}

pub(super) fn gallery_to(id: &str, p: &Value) -> Option<Descriptor> {
    let (code, _, fields) = GALLERY.iter().find(|(_, cmd, _)| *cmd == id)?;
    fields_to(p, fields, desc("null").with("GEfk", enm("GEft", code)))
}

/// Select and Mask as Photoshop CC 2015.5 and later record it (`smartBrushWorkspace`); the
/// older Refine Edge event is [`REFINE_EDGE`], which is also what Select and Mask exports as.
pub(super) const SMART_BRUSH: &[F] = &[
    F("smartBrushRadius", "radius", Kind::Px),
    F("smartBrushUseSmartRadius", "smartRadius", Kind::Bool),
    F("smartBrushSmooth", "smooth", Kind::Long),
    F("smartBrushFeather", "feather", Kind::Px),
    F("smartBrushContrast", "contrast", Kind::Pct),
    F("smartBrushShiftEdge", "shiftEdge", Kind::Pct),
    F("smartBrushDecontaminate", "decontaminate", Kind::Bool),
    F("smartBrushDeconAmount", "amount", Kind::Pct),
    F("refineEdgeOutput", "output", Kind::Enum("refineEdgeOutput", REFINE_OUTPUT)),
    F("sampleAllLayers", "sampleAllLayers", Kind::Bool),
];

/// Select › Color Range's `Clrs` presets ↔ `select.colorRange`'s `select`.
const COLOR_RANGE: &[(&str, &str)] = &[
    ("Rds", "reds"),
    ("Ylws", "yellows"),
    ("Grns", "greens"),
    ("Cyns", "cyans"),
    ("Bls", "blues"),
    ("Mgnt", "magentas"),
    ("Hghl", "highlights"),
    ("Mdtn", "midtones"),
    ("Shdw", "shadows"),
    ("OtOf", "outOfGamut"),
    ("skinTone", "skinTones"),
];

/// The Color Range keys the converter reads (or that don't change a preset's result:
/// `colorModel` is the model of sampled colours).
const COLOR_RANGE_KEYS: &[&str] = &[
    "Clrs",
    "Fzns",
    "Invr",
    "colorModel",
    "shadowsFuzziness",
    "shadowsUpperLimit",
    "midtonesFuzziness",
    "midtonesLowerLimit",
    "midtonesUpperLimit",
    "highlightsFuzziness",
    "highlightsLowerLimit",
];

/// Color Range (`ClrR`) with a preset. Sampled Colors records Lab minimum and maximum colours,
/// which `select.colorRange` (eyedropper points on the image) can't reproduce, and an unknown
/// key (Detect Faces, Localized Color Clusters…) could change the result: both stay
/// unsupported.
pub(super) fn color_range_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    if d.items.iter().any(|(k, _)| !COLOR_RANGE_KEYS.contains(&trimmed(k).as_str())) {
        return unsupported;
    }
    let Some(preset) = get_enum(d, "Clrs").and_then(|c| lookup(COLOR_RANGE, &c)) else { return unsupported };
    let mut p = Map::new();
    p.insert("select".into(), preset.into());
    let key = |suffix: &str| get_num(d, &format!("{preset}{suffix}"));
    match preset {
        "skinTones" => {
            if let Some(f) = get_num(d, "Fzns") {
                p.insert("fuzziness".into(), f.into());
            }
        }
        "shadows" | "midtones" | "highlights" => {
            if let Some(f) = key("Fuzziness") {
                p.insert("fuzziness".into(), f.into());
            }
            let range = match (preset, key("LowerLimit"), key("UpperLimit")) {
                ("shadows", _, hi) => hi.map(Value::from),
                ("highlights", lo, _) => lo.map(Value::from),
                (_, Some(lo), Some(hi)) => Some(json!([lo, hi])),
                (_, None, None) => None,
                _ => return unsupported,
            };
            if let Some(r) = range {
                p.insert("tonalRange".into(), r);
            }
        }
        _ => {}
    }
    if get_bool(d, "Invr") == Some(true) {
        p.insert("invert".into(), true.into());
    }
    step("select.colorRange", Value::Object(p))
}

pub(super) fn color_range_to(_: &str, p: &Value) -> Option<Descriptor> {
    let preset = jstr(p, "select")?;
    if p.as_object()?.keys().any(|k| !matches!(k.as_str(), "select" | "fuzziness" | "tonalRange" | "invert")) {
        return None;
    }
    let mut d = desc("null").with("Clrs", enm("Clrs", lookup_back(COLOR_RANGE, preset)?));
    let tonal = matches!(preset, "shadows" | "midtones" | "highlights");
    if let Some(f) = jnum(p, "fuzziness") {
        match preset {
            "skinTones" => d = d.with("Fzns", long(f)),
            _ if tonal => d = d.with(&format!("{preset}Fuzziness"), long(f)),
            _ => {}
        }
    }
    if let Some(r) = p.get("tonalRange") {
        match preset {
            "shadows" => d = d.with("shadowsUpperLimit", long(r.as_f64()?)),
            "highlights" => d = d.with("highlightsLowerLimit", long(r.as_f64()?)),
            "midtones" => {
                let a = r.as_array()?;
                d = d.with("midtonesLowerLimit", long(a.first()?.as_f64()?)).with("midtonesUpperLimit", long(a.get(1)?.as_f64()?));
            }
            _ => return None,
        }
    }
    if jbool(p, "invert") == Some(true) {
        d = d.with("Invr", D::Boolean(true));
    }
    Some(d)
}

const RADIAL_BLUR_FIELDS: &[F] =
    &[F("Amnt", "amount", Kind::Long), F("BlrM", "method", Kind::Enum("BlrM", RADIAL_METHOD)), F("BlrQ", "quality", Kind::Enum("BlrQ", RADIAL_QUALITY))];

pub(super) fn radial_blur_from(d: &Descriptor) -> Mapped {
    match fields_from(d, RADIAL_BLUR_FIELDS) {
        Some(m) => step("filter.blur.radialBlur", Value::Object(m)),
        None => Mapped::Unsupported { harmless: false },
    }
}

pub(super) fn radial_blur_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !pixel_target(p) {
        return None;
    }
    // Photoshop's Radial Blur has no center parameter and always blurs from the center (0.5, 0.5).
    // An explicit center must match 0.5 exactly; any non-default center cannot be faithfully exported.
    let is_default_center = |k: &str| match p.get(k) {
        None | Some(Value::Null) => true,
        Some(v) => v.as_f64() == Some(0.5),
    };
    if !is_default_center("centerX") || !is_default_center("centerY") {
        return None;
    }
    fields_to(p, RADIAL_BLUR_FIELDS, desc("null"))
}

// ------------------------------------------------------------------ newer features (#10610)

/// Whether every key of `d` is one of `keys` (codes compare without their space padding).
fn only_keys(d: &Descriptor, keys: &[&str]) -> bool {
    d.items.iter().all(|(k, _)| keys.contains(&trimmed(k).as_str()))
}

/// The step's `key` read by `read` (its space padding ignored, as `only_keys` admits it),
/// `default` when the step leaves it out, `None` when it holds a value `read` can't take: that
/// is still the step's setting, so the step is kept rather than played with the default. A key
/// recorded more than once is `None` too: the step saves one value, so it would drop the other.
fn setting<T>(d: &Descriptor, key: &str, read: impl Fn(&D) -> Option<T>, default: T) -> Option<T> {
    let mut found = d.items.iter().filter(|(k, _)| trimmed(k) == key.trim_end());
    match (found.next(), found.next()) {
        (Some((_, v)), None) => read(v),
        (None, _) => Some(default),
        (Some(_), Some(_)) => None,
    }
}

/// Whether `p` sets nothing but `allowed` (a `null` counts as unset).
fn sets_only(p: &Value, allowed: &[&str]) -> bool {
    p.as_object().is_none_or(|m| m.iter().all(|(k, v)| v.is_null() || allowed.contains(&k.as_str())))
}

/// Focus Area (`focusMask`) and Remove Background (`removeBackground`): Photoshop runs both with
/// an empty descriptor. A step that carries settings keeps them as a Photoshop step.
pub(super) fn bare_from(d: &Descriptor, id: &str) -> Mapped {
    if d.items.is_empty() { step(id, json!({})) } else { Mapped::Unsupported { harmless: false } }
}

/// Only a step with no settings writes the empty descriptor; any setting would be lost.
pub(super) fn bare_to(_: &str, p: &Value) -> Option<Descriptor> {
    sets_only(p, &[]).then(|| desc("null"))
}

/// Select Sky (`selectSky`): `sampleAllLayers` only, false when the step leaves it out
/// (the command's own default is true).
pub(super) fn select_sky_from(d: &Descriptor) -> Mapped {
    match setting(d, "sampleAllLayers", boolean, false) {
        Some(all) if only_keys(d, &["sampleAllLayers"]) => step("select.sky", json!({"sampleAllLayers": all})),
        _ => Mapped::Unsupported { harmless: false },
    }
}

/// The event has no mode, threshold or softness: only their replay values survive.
pub(super) fn select_sky_to(_: &str, p: &Value) -> Option<Descriptor> {
    let at = |k: &str, v: Value| p.get(k).is_none_or(|x| x.is_null() || *x == v || (x.as_f64().is_some() && x.as_f64() == v.as_f64()));
    if !sets_only(p, &["sampleAllLayers", "mode", "threshold", "softness"])
        || !at("mode", json!("replace"))
        || !at("threshold", json!(50))
        || !at("softness", json!(50))
    {
        return None;
    }
    let all = match p.get("sampleAllLayers") {
        None | Some(Value::Null) => true,
        Some(v) => v.as_bool()?,
    };
    Some(desc("null").with("sampleAllLayers", D::Boolean(all)))
}

/// A plain double, as Photoshop records Oil Paint's settings and the step saves them back: a
/// value with a unit, or an integer, would save back as another value.
fn double(v: &D) -> Option<f64> {
    match v {
        D::Double(n) if n.is_finite() => Some(*n),
        _ => None,
    }
}

/// An integer, as Photoshop records Oil Paint's light direction and the step saves it back.
fn integer(v: &D) -> Option<i32> {
    match v {
        D::Integer(n) => Some(*n),
        _ => None,
    }
}

/// Oil Paint's keys in recorded order: (key, param, Photoshop's default, the command's default).
const OIL_PAINT: &[(&str, &str, f64, f64)] = &[
    ("stylization", "stylization", 4.0, 4.0),
    ("cleanliness", "cleanliness", 2.299_999_952_316_28, 5.0),
    ("brushScale", "scale", 0.800_000_011_920_93, 1.0),
    ("microBrush", "bristleDetail", 10.0, 5.0),
];

/// `filter.stylize.oilPaint`'s ranges. The command clamps a value outside them when it plays,
/// so such a step keeps the Photoshop step on import and is refused on export, rather than
/// mapping a step that plays clamped and saves back unchanged.
const OIL_PAINT_LIMITS: &[(&str, f64, f64)] =
    &[("stylization", 0.1, 10.0), ("cleanliness", 0.0, 10.0), ("scale", 0.1, 10.0), ("bristleDetail", 0.0, 10.0), ("shine", 0.0, 10.0)];

/// Whether `n` is within `filter.stylize.oilPaint`'s range for `param` (a param without one is).
fn in_oil_paint_limits(param: &str, n: f64) -> bool {
    OIL_PAINT_LIMITS.iter().all(|(p, min, max)| *p != param || (*min..=*max).contains(&n))
}

/// Oil Paint (`oilPaint`). Missing keys take Photoshop's defaults, which differ from the
/// command's for cleanliness, scale, bristle detail and shine, so they are written out. A
/// setting of another number type, with a unit, or outside the command's ranges keeps the
/// Photoshop step.
pub(super) fn oil_paint_from(d: &Descriptor) -> Mapped {
    match oil_paint_params(d) {
        Some(p) => step("filter.stylize.oilPaint", Value::Object(p)),
        None => Mapped::Unsupported { harmless: false },
    }
}

fn oil_paint_params(d: &Descriptor) -> Option<Map<String, Value>> {
    let keys = ["lightingOn", "stylization", "cleanliness", "brushScale", "microBrush", "LghD", "specularity"];
    if !only_keys(d, &keys) {
        return None;
    }
    let mut p = Map::new();
    p.insert("lighting".into(), setting(d, "lightingOn", boolean, true)?.into());
    for (key, param, ps, _) in OIL_PAINT {
        p.insert((*param).into(), setting(d, key, double, *ps).filter(|n| in_oil_paint_limits(param, *n))?.into());
    }
    p.insert("angle".into(), setting(d, "LghD", |v| integer(v).map(f64::from), -60.0)?.into());
    p.insert("shine".into(), setting(d, "specularity", double, 1.299_999_952_316_28).filter(|n| in_oil_paint_limits("shine", *n))?.into());
    Some(p)
}

/// Every key in recorded order, the command's defaults for what the step leaves out. The light
/// direction is an integer: a fractional angle would replay rounded, so the step is refused, as
/// is a setting outside the command's ranges.
pub(super) fn oil_paint_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !pixel_target(p) || !sets_only(p, &["target", "lighting", "stylization", "cleanliness", "scale", "bristleDetail", "angle", "shine"]) {
        return None;
    }
    let num = |k: &str, default: f64| match p.get(k) {
        None | Some(Value::Null) => Some(default),
        Some(v) => v.as_f64().filter(|n| n.is_finite() && in_oil_paint_limits(k, *n)),
    };
    let lighting = match p.get("lighting") {
        None | Some(Value::Null) => true,
        Some(v) => v.as_bool()?,
    };
    let angle = num("angle", -60.0)?;
    if angle.fract() != 0.0 || !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&angle) {
        return None;
    }
    let mut d = desc("null").with("lightingOn", D::Boolean(lighting));
    for (key, param, _, ours) in OIL_PAINT {
        d = d.with(key, D::Double(num(param, *ours)?));
    }
    Some(d.with("LghD", D::Integer(angle as i32)).with("specularity", D::Double(num("shine", 1.0)?)))
}

/// A unit double in `unit`: the step saves `Amnt` in `#Prc` and the radius in `#Pxl`, so a
/// value in another unit, or without one, would save back as another value.
fn in_unit(unit: [u8; 4]) -> impl Fn(&D) -> Option<f64> {
    move |v| match v {
        D::UnitFloat { unit: u, value } if *u == unit && value.is_finite() => Some(*value),
        _ => None,
    }
}

/// `filter.sharpen.smartSharpen`'s amount and radius limits, in `smart_sharpen_params`' key order.
/// The command rejects a value outside them, so such a step keeps the Photoshop step on import
/// and is refused on export, rather than mapping a step that fails when it plays.
const SMART_SHARPEN_LIMITS: &[(&str, f64, f64)] = &[("amount", 1.0, 500.0), ("radius", 0.1, 64.0)];

/// Smart Sharpen (`smartSharpen`), as Photoshop records it without its dialog: the legacy
/// filter, removing Lens Blur, with no noise reduction (so `reduceNoise` is 0). A step that
/// asks for what the command does not do (another blur, More Accurate, the newer filter, a
/// preset), or an amount or radius in another unit or outside the command's limits, keeps it
/// as a Photoshop step.
pub(super) fn smart_sharpen_from(d: &Descriptor) -> Mapped {
    match smart_sharpen_params(d) {
        Some(p) => step("filter.sharpen.smartSharpen", Value::Object(p)),
        None => Mapped::Unsupported { harmless: false },
    }
}

fn smart_sharpen_params(d: &Descriptor) -> Option<Map<String, Value>> {
    let keys = ["presetKind", "useLegacy", "Amnt", "Rds", "moreAccurate", "blur"];
    let as_recorded = only_keys(d, &keys)
        && setting(d, "presetKind", enumerated, "presetKindCustom".into())? == "presetKindCustom"
        && setting(d, "useLegacy", boolean, true)?
        && !setting(d, "moreAccurate", boolean, false)?
        && setting(d, "blur", enumerated, "lensBlur".into())? == "lensBlur";
    if !as_recorded {
        return None;
    }
    let mut p = Map::new();
    for ((key, unit, default), (param, min, max)) in [("Amnt", *b"#Prc", 200.0), ("Rds", PX, 1.0)].into_iter().zip(SMART_SHARPEN_LIMITS) {
        let n = setting(d, key, in_unit(unit), default).filter(|n| (*min..=*max).contains(n))?;
        p.insert((*param).into(), n.into());
    }
    p.insert("reduceNoise".into(), 0.into());
    Some(p)
}

/// Every key in recorded order. The legacy filter has no noise reduction, so only a step that
/// sets `reduceNoise` to 0 is written (the command reduces noise by 10 when it is left out).
pub(super) fn smart_sharpen_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !pixel_target(p) || !sets_only(p, &["target", "amount", "radius", "reduceNoise"]) || p.get("reduceNoise").and_then(Value::as_f64) != Some(0.0) {
        return None;
    }
    let num = |k: &str, default: f64| match p.get(k) {
        None | Some(Value::Null) => Some(default),
        Some(v) => v.as_f64().filter(|n| SMART_SHARPEN_LIMITS.iter().any(|(param, min, max)| *param == k && (*min..=*max).contains(n))),
    };
    Some(
        desc("null")
            .with("presetKind", enm("presetKindType", "presetKindCustom"))
            .with("useLegacy", D::Boolean(true))
            .with("Amnt", pct(num("amount", 100.0)?))
            .with("Rds ", px(num("radius", 1.0)?))
            .with("moreAccurate", D::Boolean(false))
            .with("blur", enm("blurType", "lensBlur")),
    )
}
