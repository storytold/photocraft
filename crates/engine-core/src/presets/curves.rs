//! Curve presets: the Curves dialog's gear menu (Save Preset…, Load Preset…, separator, Delete
//! Current Preset — photocraft #14379).
//!
//! A preset is a name plus the curves in the form [`image.adjustments.curves`] already takes
//! (`points`, the channel keys `red`/`green`/`blue`, `cyan`/`magenta`/`yellow`/`black` or
//! `lightness`/`a`/`b`, and `pencil` for a drawn curve), so the file holds no second curve model
//! that could drift from the adjustment itself. The file ([`EXT`], `Curves.pcrv`) is UTF-8 JSON,
//! written and read through the engine's own parser ([`crate::adjust_params::from_params`]), which
//! is what rejects a malformed, truncated or nonsensical curve:
//!
//! ```text
//! { "photoCraftCurvePreset": 1, "name": "My Curve", "curves": { "points": [[0,0],[100,150],[255,255]], … } }
//! ```
//!
//! The presets the user saved or loaded are the [`PresetState::curves_presets`] list and the one the
//! dialog has selected is [`PresetState::curves_preset`] (`None` is the Default curve); like the
//! other user preset sets they persist with the preferences. Deleting a preset drops it from that
//! list — the file on disk is the user's and stays.
//!
//! [`image.adjustments.curves`]: crate::adjust_params

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{PresetsExt, always, bad, presets_changed};
use crate::adjust_params;
use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};
use photocraft_doc::adjust::ToneSpace;

/// The extension of PhotoCraft's curve preset file.
pub const EXT: &str = "pcrv";
/// The file format this reads and writes.
const VERSION: u32 = 1;
/// Largest accepted preset file: the biggest curve a document can hold is a pencil map per channel.
pub const MAX_FILE_BYTES: u64 = 1 << 20;
/// Most curve presets the user list holds.
pub const MAX_PRESETS: usize = 1000;
/// Longest preset name kept, in characters.
const MAX_NAME: usize = 255;

/// The parameter keys that name a curve, as opposed to the keys that only adjust it.
const CURVE_KEYS: [&str; 12] = ["points", "gray", "red", "green", "blue", "cyan", "magenta", "yellow", "black", "lightness", "a", "b"];

/// One curve preset: a name and the curves as `image.adjustments.curves` takes them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, craft_ui_macros::PlainData)]
pub struct CurvePreset {
    pub name: String,
    /// The curves: `points`, the channel keys, and `pencil` for a drawn curve.
    #[plain_data(assume = "JSON values: plain data")]
    pub curves: Value,
}

/// The file's JSON shape.
#[derive(Serialize, Deserialize)]
struct File {
    #[serde(rename = "photoCraftCurvePreset")]
    version: u32,
    #[serde(default)]
    name: Option<String>,
    curves: Value,
}

/// `curves`, checked the way the Curves command checks its parameters, in the form the command
/// writes them back. An object naming no curve is refused: a preset of nothing to apply.
fn normalized(curves: &Value) -> Result<Value> {
    if curves.get("photoCraftCurvePreset").is_some() {
        return Err(EngineError::BadParams { cmd: "curve preset".into(), msg: "`curves` holds the curves, not a whole preset file".into() });
    }
    let object = curves.as_object().ok_or_else(|| EngineError::BadParams { cmd: "curve preset".into(), msg: "`curves` must be an object".into() })?;
    if !CURVE_KEYS.iter().any(|key| object.contains_key(*key)) {
        return Err(EngineError::BadParams { cmd: "curve preset".into(), msg: "`curves` must name at least one curve".into() });
    }
    let parsed = adjust_params::from_params("curves", curves, None, photocraft_doc::ColorMode::Rgb)?;
    Ok(adjust_params::to_params(&parsed))
}

/// The preset as file bytes.
pub fn to_bytes(preset: &CurvePreset) -> Result<Vec<u8>> {
    let file = File { version: VERSION, name: Some(preset.name.clone()), curves: preset.curves.clone() };
    serde_json::to_vec_pretty(&file).map_err(|e| EngineError::Other(format!("cannot write the curve preset: {e}")))
}

/// The colour mode whose Curves dialog works in `space`.
fn space_mode(space: ToneSpace) -> photocraft_doc::ColorMode {
    match space {
        ToneSpace::Rgb => photocraft_doc::ColorMode::Rgb,
        ToneSpace::Cmyk => photocraft_doc::ColorMode::Cmyk,
        ToneSpace::Lab => photocraft_doc::ColorMode::Lab,
    }
}

/// The channels of `space` that `curves` actually moves: a channel equal to the straight line the
/// neutral Curves adjustment holds is no curve at all, and a preset of only the straight lines of
/// one colour mode can be applied in any other.
fn moved_channels(space: ToneSpace, curves: &Value) -> Vec<&'static str> {
    let Ok(plain) = adjust_params::default_for("curves", space_mode(space)) else {
        return Vec::new();
    };
    let neutral = adjust_params::to_params(&plain);
    adjust_params::channel_keys(space)
        .iter()
        .filter(|key| match (curves.get(*key), neutral.get(*key)) {
            (Some(here), Some(plain)) => here != plain,
            (Some(_), None) => true,
            _ => false,
        })
        .copied()
        .collect()
}

/// The tone space `curves` names a real channel of, from the parameter keys the Curves command
/// itself uses ([`adjust_params::channel_keys`]): `None` when it only holds the composite or
/// straight lines, which every colour mode has.
fn curves_space(curves: &Value) -> Option<ToneSpace> {
    curves.as_object()?;
    [ToneSpace::Cmyk, ToneSpace::Lab, ToneSpace::Rgb].into_iter().find(|space| !moved_channels(*space, curves).is_empty())
}

/// The lower-case name of a tone space, as the `space` parameter spells it.
fn space_name(space: ToneSpace) -> &'static str {
    match space {
        ToneSpace::Rgb => "rgb",
        ToneSpace::Cmyk => "cmyk",
        ToneSpace::Lab => "lab",
    }
}

/// A `space` parameter: absent, or one of `rgb`, `cmyk`, `lab`.
fn wanted_space(cmd: &str, p: &Value) -> Result<Option<ToneSpace>> {
    let want = match p.get("space") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) => s.trim().to_ascii_lowercase(),
        Some(_) => return Err(bad(cmd, "`space` must be \"rgb\", \"cmyk\" or \"lab\"")),
    };
    let space = match want.as_str() {
        "rgb" => ToneSpace::Rgb,
        "cmyk" => ToneSpace::Cmyk,
        "lab" => ToneSpace::Lab,
        other => return Err(bad(cmd, format!("`space` must be \"rgb\", \"cmyk\" or \"lab\", not \"{other}\""))),
    };
    Ok(Some(space))
}

/// Why a preset whose channels belong to another colour mode cannot be applied in `want`, naming
/// both channel sets. Reading a file into the user's list is one thing; writing curves the open
/// dialog has no place for into it is a silent half-apply, so it is refused.
fn check_space(cmd: &str, curves: &Value, want: ToneSpace) -> Result<()> {
    let Some(found) = curves_space(curves) else { return Ok(()) };
    if found == want {
        return Ok(());
    }
    let named = moved_channels(found, curves);
    let keys = |space: ToneSpace| adjust_params::channel_keys(space).join(", ");
    Err(bad(
        cmd,
        format!("the preset curves {} for {} do not fit this {} dialog, which has {}", named.join(", "), space_name(found), space_name(want), keys(want)),
    ))
}

/// The preset in `bytes`, or why the file is not one. A file naming no curve of its own comes back
/// with an empty name, which the caller fills in from the file's name.
pub fn from_bytes(bytes: &[u8]) -> Result<CurvePreset> {
    let cmd = "curve preset file";
    if bytes.is_empty() {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: "the file is empty".into() });
    }
    let file: File = serde_json::from_slice(bytes).map_err(|e| EngineError::BadParams { cmd: cmd.into(), msg: format!("not a curve preset file: {e}") })?;
    if file.version != VERSION {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("curve preset format version {} is not supported", file.version) });
    }
    Ok(CurvePreset { name: file.name.unwrap_or_default().trim().to_string(), curves: normalized(&file.curves)? })
}

/// The list as saved with the preferences.
pub fn to_json(list: &[CurvePreset]) -> Value {
    json!(list)
}

/// A saved list, entry by entry: an unusable entry is skipped, the rest survive (#5602), and the
/// list is cut to [`MAX_PRESETS`].
pub fn from_json(saved: &Value) -> Vec<CurvePreset> {
    let mut list: Vec<CurvePreset> = Vec::new();
    for entry in saved.as_array().into_iter().flatten().take(MAX_PRESETS) {
        let Ok(preset) = serde_json::from_value::<CurvePreset>(entry.clone()) else { continue };
        let Ok(curves) = normalized(&preset.curves) else { continue };
        let name = preset.name.trim();
        if name.is_empty() {
            continue;
        }
        list.push(CurvePreset { name: unique(&list, &clip(name)), curves });
    }
    list
}

/// A preset name unique in `list`: `base`, `base 2`, `base 3`… (the dialog and agents address
/// presets by name).
fn unique(list: &[CurvePreset], base: &str) -> String {
    super::unique_name_by(base, |n| list.iter().any(|p| p.name == n))
}

/// `name` cut to [`MAX_NAME`] characters.
fn clip(name: &str) -> String {
    name.chars().take(MAX_NAME).collect()
}

/// The preset list, with `preset` appended under a unique name and made the current one.
fn select(s: &mut Session, preset: CurvePreset) {
    s.presets_mut().curves_presets.push(preset.clone());
    s.presets_mut().curves_preset = Some(preset.name);
}

/// The path's file name without its extension, if it has one.
fn stem(path: &str) -> Option<String> {
    std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().trim().to_string()).filter(|s| !s.is_empty())
}

/// A `name` parameter: absent, blank or of another type.
fn name_param(cmd: &str, p: &Value) -> Result<Option<String>> {
    match p.get("name") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(n)) if !n.trim().is_empty() => Ok(Some(n.trim().to_string())),
        Some(_) => Err(bad(cmd, "`name` must be a non-empty string")),
    }
}

/// A `path` parameter: absent, blank or of another type.
fn path_param(cmd: &str, p: &Value) -> Result<Option<String>> {
    match p.get("path") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.trim().to_string())),
        Some(_) => Err(bad(cmd, "`path` must be a non-empty file name")),
    }
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let presets = &s.presets().curves_presets;
    Ok(json!({
        "count": presets.len(),
        "current": s.presets().curves_preset,
        "presets": presets.iter().map(|p| json!({"name": p.name, "curves": p.curves})).collect::<Vec<_>>(),
    }))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "curves.presets.save";
    let raw = p.get("curves").ok_or_else(|| bad(CMD, "missing `curves` (the curves as image.adjustments.curves takes them)"))?;
    let curves = normalized(raw)?;
    let path = path_param(CMD, p)?;
    let name = name_param(CMD, p)?.or_else(|| path.as_deref().and_then(stem));
    // `record: false` only answers with the bytes: a dialog that writes the file itself asks
    // first, so a failed or cancelled write leaves the user's preset list exactly as it was.
    let record = match p.get("record") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad(CMD, "`record` must be true or false")),
    };
    // The cap is checked for a preview too: a dialog asking for bytes of a preset it could not
    // record should be told before it writes the file.
    if s.presets().curves_presets.len() >= MAX_PRESETS {
        return Err(bad(CMD, format!("the curve preset list is full ({MAX_PRESETS} presets)")));
    }
    let base = name.unwrap_or_else(|| format!("Curve {}", s.presets().curves_presets.len() + 1));
    let name = unique(&s.presets().curves_presets, &clip(base.trim()));
    let preset = CurvePreset { name: name.clone(), curves };
    let bytes = to_bytes(&preset)?;
    // The file first: a path that cannot be written leaves the user list alone.
    if let Some(path) = &path {
        crate::file_cmds::write_file(path, &bytes)?;
    }
    let mut out = json!({"name": name, "count": s.presets().curves_presets.len()});
    match path {
        Some(path) => out["path"] = json!(path),
        None => out["data"] = json!(photocraft_paint::tile::b64_encode(&bytes)),
    }
    if record {
        select(s, preset);
        presets_changed(s);
        out["count"] = json!(s.presets().curves_presets.len());
        out["current"] = json!(s.presets().curves_preset);
    }
    Ok(out)
}

fn load(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "curves.presets.load";
    let want = wanted_space(CMD, p)?;
    let (bytes, file_stem) = crate::preset_import_cmds::file_bytes_limited(p, CMD, &format!(".{EXT} file"), MAX_FILE_BYTES)?;
    let mut preset = from_bytes(&bytes)?;
    // A caller that names the colour mode it is applying in (`space`) gets an error instead of a
    // half-apply when the file's channels belong to another mode; the list and the selection stay
    // as they were.
    if let Some(want) = want {
        check_space(CMD, &preset.curves, want)?;
    }
    if preset.name.is_empty() {
        preset.name = file_stem;
    }
    if preset.name.trim().is_empty() {
        preset.name = format!("Curve {}", s.presets().curves_presets.len() + 1);
    }
    if s.presets().curves_presets.len() >= MAX_PRESETS {
        return Err(bad(CMD, format!("the curve preset list is full ({MAX_PRESETS} presets)")));
    }
    preset.name = unique(&s.presets().curves_presets, &clip(preset.name.trim()));
    let curves = preset.curves.clone();
    let name = preset.name.clone();
    select(s, preset);
    presets_changed(s);
    Ok(json!({"name": name, "curves": curves, "count": s.presets().curves_presets.len(), "current": s.presets().curves_preset}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "curves.presets.delete";
    let name = match p.get("name") {
        None | Some(Value::Null) => s.presets().curves_preset.clone().ok_or_else(|| bad(CMD, "no curve preset is current (see curves.presets.list)"))?,
        Some(Value::String(n)) if !n.trim().is_empty() => n.trim().to_string(),
        Some(_) => return Err(bad(CMD, "`name` must be a preset name")),
    };
    let at = s
        .presets()
        .curves_presets
        .iter()
        .position(|preset| preset.name == name)
        .ok_or_else(|| bad(CMD, format!("no user curve preset \"{name}\" (see curves.presets.list)")))?;
    s.presets_mut().curves_presets.remove(at);
    if s.presets().curves_preset.as_deref() == Some(name.as_str()) {
        s.presets_mut().curves_preset = None;
    }
    presets_changed(s);
    Ok(json!({"deleted": name, "count": s.presets().curves_presets.len(), "current": s.presets().curves_preset}))
}

/// The gear menu's commands (#14379). They change the user's preset list, which persists with the
/// preferences, so the three that change it are recorded in the session journal like the other
/// preset commands (`layer.layerStyle.contours.*`); the list is a query and is not.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "curves.presets.list",
            label: "List Curve Presets",
            menu: &[],
            shortcut: None,
            params: "{} → {count, current: name|null (null = the Default curve), presets:[{name, curves: params for image.adjustments.curves}]}",
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "curves.presets.save",
            label: "Save Curve Preset…",
            menu: &[],
            shortcut: None,
            params: r##"{"curves":{"points":[[in 0..255,out 0..255],…],"red":…,"pencil":bool?} (all channels),"name":str? (default: `path`'s file name),"path":".pcrv file"?,"record":bool (default true; false only answers with the bytes, for a host that writes the file itself)} → {name (made unique), count, path | data (base64 when no path)} (Curves dialog › gear › Save Preset…)"##,
            enabled: always,
            run: save,
            journal: true,
        },
        CommandSpec {
            id: "curves.presets.load",
            label: "Load Curve Preset…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".pcrv file"?,"data":base64 bytes?,"space":"rgb"|"cmyk"|"lab"? (the colour mode the curves are applied in: a preset whose channels belong to another one is refused, naming them)} → {name, curves (params for image.adjustments.curves), count, current} (Curves dialog › gear › Load Preset…)"##,
            enabled: always,
            run: load,
            journal: true,
        },
        CommandSpec {
            id: "curves.presets.delete",
            label: "Delete Current Curve Preset",
            menu: &[],
            shortcut: None,
            params: r##"{"name":str? (default: the current preset)} → {deleted, count, current|null} (Curves dialog › gear › Delete Current Preset; the preset's file on disk is left alone)"##,
            enabled: always,
            run: delete,
            journal: true,
        },
    ]
}
