//! Recorded Content-Aware Fill settings; selection and job/history behavior belong to the command.
use super::descriptors::{desc, enm, jstr};
use super::fields::{Mapped, step};
use super::tables::lookup_back;
use super::{D, Descriptor, Map, Value};
use crate::EngineError;

const CMD: &str = "edit.contentAwareFill";
const SOURCE: &str = "_atnCafDescriptor";
// Every value the workspace showed when such a step was played with dialogs on (#14254).
const SAMPLING: &[(&str, &str)] = &[("cafSamplingRegionAuto", "auto"), ("cafSamplingRegionRectangular", "rectangular"), ("cafSamplingRegionCustom", "custom")];
const COLOR: &[(&str, &str)] = &[
    ("cafColorAdaptationNone", "none"),
    ("cafColorAdaptationDefault", "default"),
    ("cafColorAdaptationHigh", "high"),
    ("cafColorAdaptationVeryHigh", "veryHigh"),
];
const ROTATION: &[(&str, &str)] = &[
    ("cafRotationAmountNone", "none"),
    ("cafRotationAmountLow", "low"),
    ("cafRotationAmountMedium", "medium"),
    ("cafRotationAmountHigh", "high"),
    ("cafRotationAmountFull", "full"),
];
const OUTPUT: &[(&str, &str)] = &[("cafOutputToCurrentLayer", "current"), ("cafOutputToNewLayer", "new"), ("cafOutputToDuplicateLayer", "duplicate")];
const KEYS: &[&str] = &["cafSamplingRegion", "cafSampleAllLayers", "cafColorAdaptationLevel", "cafRotationAmount", "cafScale", "cafMirror", "cafOutput"];
const PARAMS: &[&str] = &["sampling", "sampleAllLayers", "colorAdaptation", "rotationAdaptation", "scale", "mirror", "output"];
/// Marks a step recorded with an empty descriptor: it plays with the workspace's remembered
/// settings ([`caf_remembered`]) instead of fixed values.
const REMEMBERED: &str = "_atnCafRemembered";
/// The `prefs.dialogs` key under which the Content-Aware Fill workspace remembers its settings.
pub const CAF_WORKSPACE_PREFS: &str = "edit.contentAwareFill.workspace";

/// All seven settings are recorded, or none: an empty descriptor uses the remembered settings.
/// Other missing keys have no measured Photoshop default here.
fn recorded(d: &Descriptor) -> Option<Value> {
    if d.items.is_empty() {
        let mut p = Map::new();
        p.insert(REMEMBERED.into(), true.into());
        return Some(Value::Object(p));
    }
    if d.items.len() != KEYS.len() || KEYS.iter().any(|name| d.items.iter().filter(|(key, _)| key.is(name)).count() != 1) {
        return None;
    }
    let mut p = Map::new();
    for (key, param, table) in [(KEYS[0], PARAMS[0], SAMPLING), (KEYS[2], PARAMS[2], COLOR), (KEYS[3], PARAMS[3], ROTATION), (KEYS[6], PARAMS[6], OUTPUT)] {
        let D::Enumerated { type_id, value } = d.get(key)? else { return None };
        if !type_id.is(key) {
            return None;
        }
        p.insert(param.into(), table.iter().find(|(wire, _)| value.is(wire))?.1.into());
    }
    for (key, param) in [(KEYS[1], PARAMS[1]), (KEYS[4], PARAMS[4]), (KEYS[5], PARAMS[5])] {
        let D::Boolean(value) = d.get(key)? else { return None };
        p.insert(param.into(), (*value).into());
    }
    Some(Value::Object(p))
}

/// Write only represented native options, making their defaults explicit in all seven fields. A step without
/// `sampleAllLayers` is refused: the command then samples the composite when the layer is empty, which neither
/// recorded value says.
fn descriptor(p: &Value) -> Option<Descriptor> {
    let object = p.as_object()?;
    if !super::events::pixel_target(p)
        || object.keys().any(|key| !PARAMS.contains(&key.as_str()) && !matches!(key.as_str(), "target" | "seed" | SOURCE | REMEMBERED))
        || p.get("seed").is_some_and(|seed| seed.as_u64() != Some(1))
    {
        return None;
    }
    if let Some(remembered) = p.get(REMEMBERED) {
        // Values next to the mark would be dropped by an empty descriptor.
        return (remembered.as_bool() == Some(true) && !PARAMS.iter().any(|param| object.contains_key(*param))).then(|| desc("null"));
    }
    if !object.contains_key(PARAMS[1]) {
        return None;
    }
    let enumeration = |key: &str, param: &str, table: &[(&str, &str)], default: &str| {
        let value = match p.get(param) {
            None => default,
            Some(value) => value.as_str()?,
        };
        Some(enm(key, lookup_back(table, value)?))
    };
    let boolean = |param: &str| match p.get(param) {
        None => Some(D::Boolean(false)),
        Some(value) => value.as_bool().map(D::Boolean),
    };
    Some(
        desc("null")
            .with(KEYS[0], enumeration(KEYS[0], PARAMS[0], SAMPLING, "auto")?)
            .with(KEYS[1], boolean(PARAMS[1])?)
            .with(KEYS[2], enumeration(KEYS[2], PARAMS[2], COLOR, "default")?)
            .with(KEYS[3], enumeration(KEYS[3], PARAMS[3], ROTATION, "none")?)
            .with(KEYS[4], boolean(PARAMS[4])?)
            .with(KEYS[5], boolean(PARAMS[5])?)
            .with(KEYS[6], enumeration(KEYS[6], PARAMS[6], OUTPUT, "current")?),
    )
}

/// Why an empty recorded step refuses remembered Custom sampling ([`caf_remembered`]). The shell shows it translated.
pub const CAF_CUSTOM_AREA_NOT_REMEMBERED: &str = "This step plays with the remembered Content-Aware Fill settings, and their Custom sampling area isn't remembered. Fill once with Auto or Rectangular sampling in Content-Aware Fill, then play it again.";

/// Resolve an empty recorded step: the remembered settings fill what the step doesn't name, and
/// before any were remembered it writes to a new layer. Playing never changes what is remembered.
/// Remembered Custom sampling is refused: its area is painted in the workspace, which keeps none
/// (Custom reopens empty), and the step must not sample another area instead.
pub(crate) fn caf_remembered(p: &mut Value, prefs: &crate::prefs::Preferences) -> Result<(), EngineError> {
    let Some(object) = p.as_object_mut() else { return Ok(()) };
    if object.remove(REMEMBERED).and_then(|v| v.as_bool()) != Some(true) {
        return Ok(());
    }
    let saved = prefs.dialogs.get(CAF_WORKSPACE_PREFS);
    for (param, flag) in PARAMS.iter().map(|param| (*param, matches!(*param, "sampleAllLayers" | "scale" | "mirror"))) {
        // Only well-typed values: a damaged entry must not turn into another setting.
        let value = saved.and_then(|v| v.get(param)).filter(|v| if flag { v.is_boolean() } else { v.is_string() });
        if let Some(value) = value.filter(|_| !object.contains_key(param)) {
            object.insert(param.into(), value.clone());
        }
    }
    object.entry("output").or_insert_with(|| "new".into());
    if object.get("sampling").and_then(Value::as_str) == Some("custom") && !["area", "channel", "samplingStrokes"].iter().any(|key| object.contains_key(*key)) {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: CAF_CUSTOM_AREA_NOT_REMEMBERED.into() });
    }
    Ok(())
}

/// Import the verified seven-field form, retaining noncanonical wire data for an unchanged save.
pub(super) fn caf_from(d: &Descriptor) -> Mapped {
    let Some(mut p) = recorded(d) else { return Mapped::Unsupported { harmless: false } };
    if descriptor(&p).as_ref() != Some(d) {
        // Key order, descriptor identity and ID encoding are wire data, not native options.
        p[SOURCE] = photocraft_paint::tile::b64_encode(&d.to_bytes()).into();
    }
    step(CMD, p)
}

/// Export supported settings; preserve the recorded wire form only while its settings agree.
pub(super) fn caf_to(_: &str, p: &Value) -> Option<Descriptor> {
    let canonical = descriptor(p)?;
    if p.get(SOURCE).is_none() {
        return Some(canonical);
    }
    let source = Descriptor::from_bytes(&photocraft_paint::tile::b64_decode(jstr(p, SOURCE)?)?).ok()?;
    // Validate preserved data even after an edit; it cannot hide unsupported source options.
    let original = descriptor(&recorded(&source)?)?;
    // A supported edit has a verified canonical form; stale wire data cannot override it.
    Some(if original == canonical { source } else { canonical })
}
