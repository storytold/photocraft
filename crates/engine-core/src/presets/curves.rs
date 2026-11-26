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

/// Everything a preset file owns outright: the curves and their channels, plus how they are drawn
/// (`pencil` for a drawn map, `smooth` for the passes the command applies on the way in). A file
/// names `pencil` only for a drawn curve, so the keys it does not name still replace what the host
/// had: a preset of control points loaded into a Pencil-mode dialog leaves it in point mode.
const OWNED_KEYS: [&str; 14] = ["points", "gray", "red", "green", "blue", "cyan", "magenta", "yellow", "black", "lightness", "a", "b", "pencil", "smooth"];

/// `dialog`'s parameters with the loaded preset's `curves` written into them: everything the preset
/// owns comes from the preset, and everything else (the addressing keys the dialog opened with)
/// stays as the dialog has it. `None` when either side is not an object.
pub fn over(dialog: &Value, curves: &Value) -> Option<Value> {
    let mut object = dialog.as_object()?.clone();
    for key in OWNED_KEYS {
        object.remove(key);
    }
    object.extend(curves.as_object()?.clone());
    Some(Value::Object(object))
}

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

/// The colour mode whose channel keys `curves` names, `None` when it names none of them. A preset
/// holds the curves of one colour mode, because one colour mode is what one dialog writes. Naming
/// the channels of two is a hand-edited file: parsing it would keep the family its mode happens to
/// use and lose the other without a word, so it is refused here, on the input as it stands.
fn named_space(curves: &Value) -> Result<Option<ToneSpace>> {
    let Some(object) = curves.as_object() else { return Ok(None) };
    let named: Vec<ToneSpace> = [ToneSpace::Cmyk, ToneSpace::Lab, ToneSpace::Rgb]
        .into_iter()
        .filter(|space| adjust_params::channel_keys(*space).iter().any(|key| object.contains_key(*key)))
        .collect();
    match named.as_slice() {
        [] => Ok(None),
        [only] => Ok(Some(*only)),
        many => {
            let keys = |space: ToneSpace| -> String {
                let mut named: Vec<&str> = Vec::new();
                for key in adjust_params::channel_keys(space) {
                    if object.contains_key(*key) {
                        named.push(*key);
                    }
                }
                named.join(", ")
            };
            let spelled = many.iter().map(|space| format!("{} ({})", space_name(*space), keys(*space))).collect::<Vec<_>>().join(" and ");
            Err(bad("curve preset", format!("the curves name channels of {spelled}; a preset holds the curves of one colour mode")))
        }
    }
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
    named_space(curves)?;
    let parsed = adjust_params::from_params("curves", curves, None, photocraft_doc::ColorMode::Rgb)?;
    Ok(adjust_params::to_params(&parsed))
}

/// The preset as file bytes.
pub fn to_bytes(preset: &CurvePreset) -> Result<Vec<u8>> {
    let file = File { version: VERSION, name: Some(preset.name.clone()), curves: preset.curves.clone() };
    serde_json::to_vec_pretty(&file).map_err(|e| EngineError::Other(format!("cannot write the curve preset: {e}")))
}

/// The two numbers of the point at `at` in `points`: `None` when that entry is not a pair of numbers.
fn point_at(points: &[Value], at: usize) -> Option<(f64, f64)> {
    let pair = points.get(at)?.as_array()?;
    Some((pair.first()?.as_f64()?, pair.get(1)?.as_f64()?))
}

/// Whether `here` is the straight line `plain` describes, whatever points along it the file spells
/// out: every one of its points answers with the line's value at that input. The neutral adjustment
/// holds the line as two points; a hand-written curve that adds a point on the same line is that
/// same line, and comparing point lists would read it as a curve of its own.
fn on_the_line(here: &Value, plain: &Value) -> bool {
    let (Some(points), Some(line)) = (here.as_array(), plain.as_array()) else {
        return here == plain;
    };
    if line.is_empty() {
        return points.is_empty();
    }
    let (Some((x0, y0)), Some((x1, y1))) = (point_at(line, 0), point_at(line, line.len() - 1)) else {
        return here == plain;
    };
    points.iter().all(|point| match point.as_array().map(|pair| (pair.first().and_then(Value::as_f64), pair.get(1).and_then(Value::as_f64))) {
        Some((Some(input), Some(output))) => {
            let expected = if (x1 - x0).abs() < 1e-9 { y0 } else { y0 + (y1 - y0) * (input - x0) / (x1 - x0) };
            (output - expected).abs() < 1e-6
        }
        _ => false,
    })
}

/// Whether `curves` moves the composite curve: its `points` differ from the straight line the
/// neutral Curves adjustment holds. A drawn (pencil) curve is one map - the curve of the channel the
/// dialog edits - so a pencil preset moves no composite curve of its own.
fn moves_composite(curves: &Value) -> bool {
    if curves.get("pencil").and_then(Value::as_bool).unwrap_or(false) {
        return false;
    }
    let Ok(plain) = adjust_params::default_for("curves", photocraft_doc::ColorMode::Rgb) else {
        return false;
    };
    match (curves.get("points"), adjust_params::to_params(&plain).get("points")) {
        (Some(here), Some(straight)) => !on_the_line(here, straight),
        (Some(_), None) => true,
        _ => false,
    }
}

/// The colour mode whose Curves dialog works in `space`.
fn space_mode(space: ToneSpace) -> photocraft_doc::ColorMode {
    match space {
        ToneSpace::Rgb => photocraft_doc::ColorMode::Rgb,
        ToneSpace::Cmyk => photocraft_doc::ColorMode::Cmyk,
        ToneSpace::Lab => photocraft_doc::ColorMode::Lab,
    }
}

/// The channels of `space` that `curves` actually moves: a channel that is the straight line the
/// neutral Curves adjustment holds - however many points along it the file spells out - is no curve
/// at all, and a preset of only the straight lines of one colour mode can be applied in any other.
fn moved_channels(space: ToneSpace, curves: &Value) -> Vec<&'static str> {
    let Ok(plain) = adjust_params::default_for("curves", space_mode(space)) else {
        return Vec::new();
    };
    let neutral = adjust_params::to_params(&plain);
    adjust_params::channel_keys(space)
        .iter()
        .filter(|key| match (curves.get(*key), neutral.get(*key)) {
            (Some(here), Some(plain)) => !on_the_line(here, plain),
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

/// What a dialog can take from a preset: the channels of one colour mode; the part of a mode a
/// dialog aimed at the Channels panel's selection edits - those components, and the combined curve
/// when its panel lists one (a colour set does, an isolated single component does not); or the
/// composite curve alone. A gray document - and a mask or saved channel, which holds gray levels -
/// edits its tone in the composite curve only, so a preset with channel curves has no place there
/// even when its channels are the ones the document's colour mode would have.
#[derive(Clone, PartialEq)]
enum Applies {
    Space(ToneSpace),
    Parts(ToneSpace, Vec<&'static str>, bool),
    Composite,
}

/// The `space` value on its own: a colour mode, or the composite curve alone for `gray`.
fn space_applies(cmd: &str, want: &str) -> Result<Applies> {
    Ok(match want {
        "rgb" => Applies::Space(ToneSpace::Rgb),
        "cmyk" => Applies::Space(ToneSpace::Cmyk),
        "lab" => Applies::Space(ToneSpace::Lab),
        "gray" => Applies::Composite,
        other => return Err(bad(cmd, format!("`space` must be \"rgb\", \"cmyk\", \"lab\" or \"gray\", not \"{other}\""))),
    })
}

/// The colour mode a channel key belongs to, and the key as the Curves command spells it: `None` for
/// a name that is not one of its channels.
fn channel_owner(name: &str) -> Option<(ToneSpace, &'static str)> {
    for space in [ToneSpace::Cmyk, ToneSpace::Lab, ToneSpace::Rgb] {
        for key in adjust_params::channel_keys(space) {
            if *key == name {
                return Some((space, *key));
            }
        }
    }
    None
}

/// The `channel` parameter as given: one name or a list of them.
fn named_channels(cmd: &str, p: &Value) -> Result<Vec<String>> {
    Ok(match p.get("channel") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(one)) => vec![one.trim().to_ascii_lowercase()],
        Some(Value::Array(list)) => {
            let mut names = Vec::new();
            for entry in list {
                match entry.as_str() {
                    Some(one) => names.push(one.trim().to_ascii_lowercase()),
                    None => return Err(bad(cmd, format!("`channel` must name channels, not {entry}"))),
                }
            }
            names
        }
        Some(other) => return Err(bad(cmd, format!("`channel` must be a channel name or a list of them, not {other}"))),
    })
}

/// A `space` parameter, the `channel` or channels beside it, and whether that panel also lists the
/// combined curve: what the caller's dialog can edit. A channel knows the colour mode it belongs to,
/// so a host that names only channels is checked the same way; naming a mode that disagrees, or
/// channels of more than one mode, is an error rather than a silent choice of which half to believe.
fn wanted_space(cmd: &str, p: &Value) -> Result<Option<Applies>> {
    let want = match p.get("space") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.trim().to_ascii_lowercase()),
        Some(_) => return Err(bad(cmd, "`space` must be \"rgb\", \"cmyk\", \"lab\" or \"gray\"")),
    };
    let named = named_channels(cmd, p)?;
    let Some(first) = named.first() else {
        return Ok(match want {
            None => None,
            Some(want) => Some(space_applies(cmd, &want)?),
        });
    };
    let mut keys: Vec<&'static str> = Vec::new();
    let Some((owner, key)) = channel_owner(first) else {
        return Err(bad(cmd, format!("`channel` must name a channel the Curves command knows, not \"{first}\"")));
    };
    keys.push(key);
    for name in named.iter().skip(1) {
        let Some((space, key)) = channel_owner(name) else {
            return Err(bad(cmd, format!("`channel` must name a channel the Curves command knows, not \"{name}\"")));
        };
        if space != owner {
            return Err(bad(cmd, format!("`channel` names channels of {} and {}, which is more than one colour mode", space_name(owner), space_name(space))));
        }
        keys.push(key);
    }
    let combined = match p.get("combined") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(on)) => *on,
        Some(_) => return Err(bad(cmd, "`combined` must be true or false")),
    };
    if let Some(want) = &want
        && space_applies(cmd, want)? != Applies::Space(owner)
    {
        return Err(bad(cmd, format!("`channel` \"{first}\" belongs to {}, not to \"{want}\"", space_name(owner))));
    }
    Ok(Some(Applies::Parts(owner, keys, combined)))
}

/// Why a preset whose curves the open dialog cannot show is refused instead of applied in pieces,
/// naming what has no place. Reading a file into the user's list is one thing; writing curves into a
/// dialog that will drop them on the way to OK is a silent half-apply. A dialog aimed at part of its
/// mode holds the curves its panel lists: the selected components, and the combined curve only where
/// the panel lists one - a single isolated component lists none, so its composite stays the straight
/// line the neutral adjustment starts from, as the hidden curves of a gray plane do (#5335).
fn check_space(cmd: &str, curves: &Value, want: Applies) -> Result<()> {
    if let Applies::Parts(space, keys, combined) = &want {
        let here = aimed_here(*space, keys, *combined);
        let found = curves_space(curves);
        let named: Vec<&'static str> = found.map(|of| moved_channels(of, curves)).unwrap_or_default().into_iter().filter(|key| !keys.contains(key)).collect();
        if !named.is_empty() {
            return Err(bad(cmd, format!("the preset curves {} for {} do not fit {here}", named.join(", "), space_name(found.unwrap_or(*space)))));
        }
        if !*combined && moves_composite(curves) {
            return Err(bad(cmd, format!("the preset's composite curve does not fit {here}")));
        }
        return Ok(());
    }
    let Some(found) = curves_space(curves) else { return Ok(()) };
    if let Applies::Space(space) = &want
        && found == *space
    {
        return Ok(());
    }
    let here = match &want {
        Applies::Space(space) => format!("this {} dialog, which has {}", space_name(*space), adjust_params::channel_keys(*space).join(", ")),
        _ => "this gray dialog, which edits the composite curve only".to_string(),
    };
    let named = moved_channels(found, curves);
    Err(bad(cmd, format!("the preset curves {} for {} do not fit {here}", named.join(", "), space_name(found))))
}

/// How a dialog aimed at part of its colour mode is named in a refusal: the components its panel
/// lists, and the combined curve when the panel lists one.
fn aimed_here(space: ToneSpace, keys: &[&'static str], combined: bool) -> String {
    let listed = if combined { format!("{} and its combined curve", keys.join(", ")) } else { keys.join(", ") };
    format!("this {} dialog aimed at {listed}, which edits those curves alone", space_name(space))
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
            params: r##"{"curves":{"points":[[in 0..255,out 0..255],…],"red":…,"pencil":bool?} (all channels of one colour mode: a preset naming channels of two is refused, naming them),"name":str? (default: `path`'s file name),"path":".pcrv file"?,"record":bool (default true; false only answers with the bytes, for a host that writes the file itself)} → {name (made unique), count, path | data (base64 when no path)} (Curves dialog › gear › Save Preset…)"##,
            enabled: always,
            run: save,
            journal: true,
        },
        CommandSpec {
            id: "curves.presets.load",
            label: "Load Curve Preset…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".pcrv file"?,"data":base64 bytes?,"space":"rgb"|"cmyk"|"lab"|"gray"? (the colour mode the curves are applied in: a preset whose channels belong to another one is refused, naming them, and `gray` - a dialog that edits the composite curve only - refuses any channel curve),"channel":"red"|"green"|"blue"|"cyan"|"magenta"|"yellow"|"black"|"lightness"|"a"|"b"?, or a list of them (the components a dialog aimed at part of its colour mode lists; a name that is not a channel, channels of two modes together, or a mode beside them that disagrees is an error),"combined":bool (the panel also lists the set's combined entry, which is its composite curve; a single component lists none)} → {name, curves (params for image.adjustments.curves), count, current} (Curves dialog › gear › Load Preset…)"##,
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
