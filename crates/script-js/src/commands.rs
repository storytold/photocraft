//! The session commands scripts can reach, with the parameters each may take.
//!
//! Deny by default: a global that needs another command adds it here. The same check runs on
//! every command a run sends, in process and in the script host's supervisor, so a host that
//! forges a request can't reach anything a script couldn't.
//!
//! Paths are only checked for shape here. Which paths a run may write is the app's decision:
//! see [`crate::grants`].

use serde_json::Value;

/// How a command parameter must look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Param {
    /// A tab or History panel index.
    Index,
    /// A layer id.
    Id,
    /// A whole number in `min..=max`.
    Int(i64, i64),
    /// A finite number in `min..=max`.
    Number(f64, f64),
    Bool,
    /// Text of at most this many bytes.
    Text(usize),
    /// One of these strings.
    Enum(&'static [&'static str]),
    /// A file path: text of at most [`MAX_PATH_BYTES`], no NUL.
    Path,
    /// `type.edit` runs: one `{start, end, size?, font?, color?}`.
    Runs,
    /// A JSON object of at most [`MAX_JSON_BYTES`] serialized, nested at most [`MAX_JSON_DEPTH`].
    Json,
}

/// Required, or may be left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    Required,
    Optional,
}

use Need::{Optional, Required};
use Param::{Bool, Enum, Id, Index, Int, Json, Number, Path, Runs, Text};

/// Longest path a script may name.
pub const MAX_PATH_BYTES: usize = 4096;
/// Largest custom options value.
pub const MAX_JSON_BYTES: usize = 64 << 10;
/// Deepest custom options value.
pub const MAX_JSON_DEPTH: usize = 8;
/// Longest layer or document name.
const NAME: usize = 1024;
/// Longest text for a type layer.
const TEXT: usize = 1 << 20;
/// Largest canvas side, as `file.new` takes it.
pub(crate) const SIDE: f64 = 300_000.0;
/// Type sizes the engine takes, in points.
const TYPE_SIZE: (f64, f64) = (photocraft_engine::type_cmds::TYPE_SIZE_MIN_PT, photocraft_engine::type_cmds::TYPE_SIZE_MAX_PT);
/// Largest index or id.
const MAX_INDEX: u64 = 1 << 53;
/// Largest translation or position, in pixels.
const FAR: i64 = 1 << 30;

/// Blend modes a script may set, as `layer.setProps` names them.
pub const BLENDS: &[&str] = &[
    "Normal",
    "Dissolve",
    "Darken",
    "Multiply",
    "Color Burn",
    "Linear Burn",
    "Darker Color",
    "Lighten",
    "Screen",
    "Color Dodge",
    "Linear Dodge (Add)",
    "Lighter Color",
    "Overlay",
    "Soft Light",
    "Hard Light",
    "Vivid Light",
    "Linear Light",
    "Pin Light",
    "Hard Mix",
    "Difference",
    "Exclusion",
    "Subtract",
    "Divide",
    "Hue",
    "Saturation",
    "Color",
    "Luminosity",
    "Pass Through",
];

/// The preferences a script may read and set: unit choices only, never anything that names a
/// file or folder.
pub const PREFS: &[&str] = &["unitsAndRulers.rulers", "unitsAndRulers.typeUnits"];
/// The values those preferences take.
pub const UNIT_NAMES: &[&str] = &["pixels", "inches", "cm", "mm", "points", "picas", "percent"];

const ANCHORS: &[&str] = &["topLeft", "top", "topRight", "left", "center", "right", "bottomLeft", "bottom", "bottomRight"];
const RESAMPLE: &[&str] = &["automatic", "preserveDetails", "bicubicSmoother", "bicubicSharper", "bicubic", "bilinear", "nearest"];

type Spec = (&'static str, &'static [(&'static str, Param, Need)]);

/// The commands, with every parameter each may take.
pub const COMMANDS: &[Spec] = &[
    ("session.inspect", &[]),
    (
        "document.inspect",
        &[
            ("document", Index, Optional),
            ("serial", Index, Optional),
            ("history", Bool, Optional),
            ("layers", Bool, Optional),
            ("unlessRevision", Json, Optional),
        ],
    ),
    ("document.activate", &[("document", Index, Optional), ("serial", Index, Optional)]),
    (
        "file.new",
        &[
            ("width", Number(1.0, SIDE), Required),
            ("height", Number(1.0, SIDE), Required),
            ("resolution", Number(1.0, 30_000.0), Required),
            ("name", Text(NAME), Optional),
            ("mode", Enum(&["bitmap", "rgb", "grayscale", "cmyk", "lab"]), Optional),
            ("background", Enum(&["white", "black", "backgroundColor", "transparent"]), Optional),
        ],
    ),
    ("file.close", &[("save", Enum(&["discard", "ask"]), Required)]),
    ("image.duplicate", &[("name", Text(NAME), Required), ("mergedOnly", Bool, Required)]),
    (
        "image.imageSize",
        &[
            ("width", Number(1.0, SIDE), Required),
            ("height", Number(1.0, SIDE), Required),
            ("resolution", Number(1.0, 30_000.0), Optional),
            ("resample", Enum(RESAMPLE), Optional),
        ],
    ),
    ("image.canvasSize", &[("width", Number(1.0, SIDE), Required), ("height", Number(1.0, SIDE), Required), ("anchor", Enum(ANCHORS), Required)]),
    (
        "image.crop",
        &[
            ("x", Number(-SIDE, SIDE), Required),
            ("y", Number(-SIDE, SIDE), Required),
            ("width", Number(1.0, SIDE), Required),
            ("height", Number(1.0, SIDE), Required),
        ],
    ),
    (
        "image.trim",
        &[
            ("basedOn", Enum(&["transparent", "topLeft", "bottomRight"]), Required),
            ("top", Bool, Required),
            ("bottom", Bool, Required),
            ("left", Bool, Required),
            ("right", Bool, Required),
        ],
    ),
    ("layer.flattenImage", &[]),
    ("layer.mergeVisible", &[]),
    ("layer.select", &[("layer", Id, Required)]),
    ("layer.new.layer", &[("name", Text(NAME), Optional), ("group", Id, Optional)]),
    ("layer.new.group", &[("name", Text(NAME), Optional), ("group", Id, Optional)]),
    ("layer.delete", &[("layer", Id, Required)]),
    ("layer.duplicate", &[("layer", Id, Required), ("name", Text(NAME), Optional)]),
    ("layer.renameLayer", &[("layer", Id, Required), ("name", Text(NAME), Required)]),
    (
        "layer.setProps",
        &[
            ("layer", Id, Required),
            ("visible", Bool, Optional),
            ("opacity", Number(0.0, 1.0), Optional),
            ("fill", Number(0.0, 1.0), Optional),
            ("blend", Enum(BLENDS), Optional),
            ("locks", Json, Optional),
        ],
    ),
    ("layer.translate", &[("layer", Id, Required), ("dx", Int(-FAR, FAR), Required), ("dy", Int(-FAR, FAR), Required)]),
    ("layer.moveTo", &[("layer", Id, Required), ("target", Id, Required), ("position", Enum(&["above", "below", "into"]), Required), ("copy", Bool, Optional)]),
    ("layer.mergeDown", &[("layer", Id, Required)]),
    ("type.info", &[("layer", Id, Required)]),
    (
        "type.create",
        &[
            ("x", Number(-SIDE, SIDE), Required),
            ("y", Number(-SIDE, SIDE), Required),
            ("text", Text(TEXT), Required),
            ("size", Number(TYPE_SIZE.0, TYPE_SIZE.1), Optional),
            ("font", Text(NAME), Optional),
            ("layer", Id, Optional),
        ],
    ),
    // A move goes from one supported position to another: up to twice the range's half.
    (
        "type.edit",
        &[("layer", Id, Required), ("text", Text(TEXT), Optional), ("runs", Runs, Optional), ("move", Param::Number(-2.0 * SIDE, 2.0 * SIDE), Optional)],
    ),
    ("history.selectState", &[("state", Index, Required)]),
    ("select.all", &[]),
    ("select.deselect", &[]),
    ("select.rect", &[("x", Int(-FAR, FAR), Required), ("y", Int(-FAR, FAR), Required), ("width", Int(1, FAR), Required), ("height", Int(1, FAR), Required)]),
    (
        "file.saveACopy",
        &[
            ("path", Path, Required),
            ("quality", Number(0.0, 12.0), Optional),
            ("layers", Bool, Optional),
            ("tiffLayers", Bool, Optional),
            ("tiffCompression", Enum(&["none", "lzw", "zip"]), Optional),
        ],
    ),
    (
        "file.export.saveForWebLegacy",
        &[
            ("path", Path, Required),
            ("format", Enum(&["gif", "png8", "png24", "jpeg", "wbmp", "webp"]), Required),
            ("quality", Number(0.0, 100.0), Optional),
            ("colors", Number(2.0, 256.0), Optional),
            ("transparency", Bool, Optional),
        ],
    ),
    ("prefs.get", &[("path", Enum(PREFS), Required)]),
    ("prefs.set", &[("path", Enum(PREFS), Required), ("value", Enum(UNIT_NAMES), Required)]),
    ("scripts.customOptions.get", &[("key", Text(NAME), Required)]),
    ("scripts.customOptions.put", &[("key", Text(NAME), Required), ("value", Json, Required), ("persistent", Bool, Required)]),
    ("scripts.customOptions.erase", &[("key", Text(NAME), Required)]),
];

/// The parameter every editing command may carry: the serial of the document it is for. The
/// host makes that document active and runs the command in the same call (see
/// [`take_document`]), so a tab closed or switched while the script runs can't make the command
/// act on another document.
pub const ON_DOCUMENT: &str = "onDocument";

/// Commands that name their document themselves, or none.
const NO_ON_DOCUMENT: &[&str] = &[
    "session.inspect",
    "document.inspect",
    "document.activate",
    "file.new",
    "prefs.get",
    "prefs.set",
    "scripts.customOptions.get",
    "scripts.customOptions.put",
    "scripts.customOptions.erase",
];

/// Take [`ON_DOCUMENT`] out of a command's parameters: the serial of the document the host must
/// make active before running it.
pub fn take_document(params: &mut Value) -> Option<u64> {
    params.as_object_mut()?.remove(ON_DOCUMENT)?.as_u64()
}

/// The command ids, in [`COMMANDS`] order.
pub fn allowed() -> impl Iterator<Item = &'static str> {
    COMMANDS.iter().map(|(id, _)| *id)
}

/// The start of `s`, at most `max` bytes, cut at a character boundary.
fn cut(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("")
}

/// How deep `v` nests (a scalar is 0), stopping once past `limit`.
fn depth(v: &Value, limit: usize) -> usize {
    let mut deepest = 0;
    let mut stack = vec![(v, 0usize)];
    while let Some((v, d)) = stack.pop() {
        deepest = deepest.max(d);
        if deepest > limit {
            break;
        }
        match v {
            Value::Array(a) => stack.extend(a.iter().map(|c| (c, d + 1))),
            Value::Object(o) => stack.extend(o.values().map(|c| (c, d + 1))),
            _ => {}
        }
    }
    deepest
}

fn color(v: &Value) -> bool {
    v.as_str().is_some_and(|s| s.len() == 7 && s.starts_with('#') && s.bytes().skip(1).all(|b| b.is_ascii_hexdigit()))
}

fn param_ok(rule: Param, v: &Value) -> bool {
    match rule {
        Index | Id => v.as_u64().is_some_and(|i| i <= MAX_INDEX),
        Int(lo, hi) => v.as_i64().is_some_and(|i| (lo..=hi).contains(&i)),
        Number(lo, hi) => v.as_f64().is_some_and(|f| f.is_finite() && (lo..=hi).contains(&f)),
        Bool => v.is_boolean(),
        Text(max) => v.as_str().is_some_and(|s| s.len() <= max),
        Enum(names) => v.as_str().is_some_and(|s| names.contains(&s)),
        Path => v.as_str().is_some_and(|s| !s.is_empty() && s.len() <= MAX_PATH_BYTES && !s.contains('\0')),
        Runs => {
            let Some([run]) = v.as_array().map(Vec::as_slice) else { return false };
            let Some(run) = run.as_object() else { return false };
            let known = run.keys().all(|k| matches!(k.as_str(), "start" | "end" | "size" | "font" | "color"));
            let span = ["start", "end"].iter().all(|k| run.get(*k).is_some_and(|v| param_ok(Int(0, i64::from(u32::MAX)), v)));
            let size = run.get("size").is_none_or(|s| param_ok(Number(TYPE_SIZE.0, TYPE_SIZE.1), s));
            let font = run.get("font").is_none_or(|f| param_ok(Text(NAME), f));
            let col = run.get("color").is_none_or(color);
            known && span && size && font && col
        }
        Json => v.is_object() && depth(v, MAX_JSON_DEPTH) <= MAX_JSON_DEPTH && serde_json::to_vec(v).is_ok_and(|b| b.len() <= MAX_JSON_BYTES),
    }
}

/// Check a command request against [`COMMANDS`]: a known command, an object of parameters,
/// every required one there, none other, each the shape its rule says. A `"move"` for
/// `type.edit` is the exception to scalar rules: a `[dx, dy]` pair of those numbers.
///
/// # Errors
/// What is wrong with it, for the failure message.
pub fn check_command(id: &str, params: &Value) -> Result<(), String> {
    let Some((_, allowed)) = COMMANDS.iter().find(|(name, _)| *name == id) else {
        return Err(format!("the command {:?} is not available to scripts", cut(id, 80)));
    };
    let Value::Object(map) = params else { return Err(format!("{id}: the parameters are not an object")) };
    for key in map.keys() {
        if key == ON_DOCUMENT && !NO_ON_DOCUMENT.contains(&id) {
            if !param_ok(Index, &map[key]) {
                return Err(format!("{id}: the parameter {key:?} is missing or invalid"));
            }
            continue;
        }
        if !allowed.iter().any(|(k, _, _)| k == key) {
            return Err(format!("{id}: the parameter {:?} is not one it takes", cut(key, 80)));
        }
    }
    for (key, rule, need) in *allowed {
        let ok = match (map.get(*key), need) {
            (None, Required) => false,
            (None, Optional) => true,
            (Some(v), _) if id == "type.edit" && *key == "move" => v.as_array().is_some_and(|p| p.len() == 2 && p.iter().all(|c| param_ok(*rule, c))),
            (Some(v), _) => param_ok(*rule, v),
        };
        if !ok {
            return Err(format!("{id}: the parameter {key:?} is missing or invalid"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn known_commands_with_their_parameters_pass() {
        for (id, p) in [
            ("session.inspect", json!({})),
            ("document.activate", json!({"document": 0})),
            ("document.activate", json!({"serial": 7})),
            ("layer.delete", json!({"layer": 3, "onDocument": 7})),
            ("file.close", json!({"save": "ask", "onDocument": 7})),
            ("layer.setProps", json!({"layer": 3, "opacity": 0.5, "blend": "Multiply"})),
            ("type.edit", json!({"layer": 3, "runs": [{"start": 0, "end": 4, "size": 24, "color": "#ff0000"}]})),
            ("type.edit", json!({"layer": 3, "move": [10.5, -20]})),
            // From one supported position to the other end of the range.
            ("type.edit", json!({"layer": 3, "move": [-600_000, 600_000]})),
            ("type.edit", json!({"layer": 3, "runs": [{"start": 0, "end": u32::MAX}]})),
            ("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": "cm"})),
            ("scripts.customOptions.put", json!({"key": "k", "value": {"a": [1, 2]}, "persistent": true})),
            ("file.saveACopy", json!({"path": "/tmp/a.png"})),
        ] {
            assert_eq!(check_command(id, &p), Ok(()), "{id} {p}");
        }
    }

    #[test]
    fn anything_else_is_refused() {
        for (id, p) in [
            ("prefs.set", json!({"path": "scratchDisks.paths", "value": "pixels"})),
            ("prefs.get", json!({"path": "fileHandling.recentFiles"})),
            ("file.open", json!({"path": "/etc/passwd"})),
            ("layer.setProps", json!({"layer": 3, "locked": true})),
            ("layer.setProps", json!({"opacity": 0.5})),
            ("layer.setProps", json!({"layer": 3, "opacity": 2})),
            ("file.close", json!({"document": 0, "save": "ask"})),
            ("type.edit", json!({"layer": 3, "runs": [{"start": 0, "end": 1}, {"start": 1, "end": 2}]})),
            ("type.edit", json!({"layer": 3, "runs": [{"start": 0, "end": 1, "tracking": 5}]})),
            ("type.edit", json!({"layer": 3, "runs": [{}]})),
            ("type.edit", json!({"layer": 3, "runs": [{"start": 0}]})),
            ("type.edit", json!({"layer": 3, "move": [1, 2, 3]})),
            ("type.edit", json!({"layer": 3, "move": [600_001, 0]})),
            // Past what a 32-bit engine can index: it would wrap to an empty run.
            ("type.edit", json!({"layer": 3, "runs": [{"start": 0, "end": 1_u64 << 32}]})),
            ("file.saveACopy", json!({"path": "a\u{0}b"})),
            ("file.saveACopy", json!({"path": "/a.png", "overwrite": true})),
            ("scripts.customOptions.put", json!({"key": "k", "value": [1], "persistent": true})),
            ("scripts.customOptions.put", json!({"key": "k", "value": {"a": "x".repeat(MAX_JSON_BYTES)}, "persistent": true})),
            ("session.inspect", json!(null)),
            ("session.inspect", json!({"coalesce": "x"})),
            ("session.inspect", json!({"onDocument": 1})),
            ("file.new", json!({"width": 1, "height": 1, "resolution": 72, "onDocument": 1})),
            ("layer.delete", json!({"layer": 3, "onDocument": "7"})),
        ] {
            assert!(check_command(id, &p).is_err(), "{id} {p}");
        }
    }

    #[test]
    fn deep_custom_options_are_refused_without_walking_them_all() {
        let mut v = json!(1);
        for _ in 0..100_000 {
            let mut m = serde_json::Map::new();
            m.insert("a".into(), v);
            v = Value::Object(m);
        }
        assert!(!param_ok(Json, &v));
        // serde_json's own drop recursion is not ours to fix: leak the test value.
        std::mem::forget(v);
    }
}
