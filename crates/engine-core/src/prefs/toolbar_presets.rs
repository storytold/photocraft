//! Toolbar presets (`.tbr`, Edit › Toolbar › Save Preset… / Load Preset…): one action descriptor
//! with no file header, in the layout the reference application reads and writes (#14624).

use craft_psd::descriptor::{Descriptor, Id, ParseLimits, UnicodeString, Value as D};

use super::Preferences;

/// The largest preset read. A preset with every tool is about 3 KiB.
pub(super) const MAX_BYTES: u64 = 64 * 1024;

/// Per-container limits for a preset's descriptors and lists (hostile input).
const LIMITS: ParseLimits = ParseLimits { max_descriptor_items: 64, max_list_items: 256 };

/// Each tool's four-char key in a preset, by its canonical name ([`ToolbarCustomization::tool_name`]).
const TOOL_KEYS: &[(&str, &[u8; 4])] = &[
    ("Move", b"arwT"),
    ("Artboard", b"ArtT"),
    ("RectMarquee", b"rgmt"),
    ("EllipseMarquee", b"elmt"),
    ("SingleRowMarquee", b"srmt"),
    ("SingleColumnMarquee", b"scmt"),
    ("Lasso", b"laso"),
    ("PolygonLasso", b"pgon"),
    ("MagneticLasso", b"mlas"),
    ("ObjectSelection", b"mgla"),
    ("QuickSelection", b"qksl"),
    ("MagicWand", b"wand"),
    ("Crop", b"crop"),
    ("PerspectiveCrop", b"pcrp"),
    ("Slice", b"SlcT"),
    ("SliceSelect", b"SlST"),
    ("Frame", b"FpoT"),
    ("Eyedropper", b"eyed"),
    ("ColorSampler", b"cols"),
    ("Ruler", b"meaT"),
    ("Note", b"TxtA"),
    ("Count", b"coun"),
    ("SpotHealing", b"sphB"),
    ("Remove", b"caft"),
    ("Healing", b"stmm"),
    ("Patch", b"ptch"),
    ("ContentAwareMove", b"rcmp"),
    ("RedEye", b"rdey"),
    ("Brush", b"pntb"),
    ("Pencil", b"penc"),
    ("ColorReplacement", b"crbt"),
    ("MixerBrush", b"wetb"),
    ("CloneStamp", b"stam"),
    ("PatternStamp", b"stmp"),
    ("HistoryBrush", b"hstB"),
    ("ArtHistoryBrush", b"ahbt"),
    ("Eraser", b"eras"),
    ("BackgroundEraser", b"sera"),
    ("MagicEraser", b"mera"),
    ("Gradient", b"bndT"),
    ("PaintBucket", b"buck"),
    ("Blur", b"blur"),
    ("Sharpen", b"shar"),
    ("Smudge", b"smud"),
    ("AdjustmentBrush", b"adjb"),
    ("Dodge", b"dodg"),
    ("Burn", b"burn"),
    ("Sponge", b"dsat"),
    ("Pen", b"penT"),
    ("FreeformPen", b"mpen"),
    ("CurvaturePen", b"cpen"),
    ("AddAnchorPoint", b"aknt"),
    ("DeleteAnchorPoint", b"dknt"),
    ("ConvertPoint", b"cknt"),
    ("Type", b"txBx"),
    ("VerticalType", b"txBV"),
    ("VerticalTypeMask", b"vtyS"),
    ("HorizontalTypeMask", b"typS"),
    ("PathSelection", b"pcsT"),
    ("DirectSelection", b"ptha"),
    ("Rectangle", b"Rect"),
    ("EllipseShape", b"Elps"),
    ("Triangle", b"TrSh"),
    ("Polygon", b"Poly"),
    ("Star", b"Star"),
    ("Line", b"linT"),
    ("CustomShape", b"CuSh"),
    ("Hand", b"hand"),
    ("RotateView", b"rott"),
    ("Zoom", b"zoom"),
];

/// A tool's `tools.select.<tool>` command id from its canonical name.
pub(super) fn command_id(tool: &str) -> String {
    let mut chars = tool.chars();
    let first = chars.next().map(|c| c.to_ascii_lowercase().to_string()).unwrap_or_default();
    format!("tools.select.{first}{}", chars.as_str())
}

/// The tool's default letter (`shell_commands::TOOLS`).
fn default_letter(id: &str) -> Option<&'static str> {
    crate::shell_commands::TOOLS.iter().find(|(t, _, _)| *t == id).and_then(|(_, _, key)| *key)
}

/// A shortcut as a tool letter: one ASCII letter, in uppercase. Anything else is not a letter a
/// preset holds.
fn letter(s: &str) -> Option<char> {
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => Some(c.to_ascii_uppercase()),
        _ => None,
    }
}

/// A preset as read: the two lists of groups (canonical tool names), and each listed tool's letter
/// (`None`: no letter) in first-listed order.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Preset {
    pub toolbar: Vec<Vec<String>>,
    pub extras: Vec<Vec<String>>,
    pub letters: Vec<(String, Option<char>)>,
}

/// The preset of `prefs`' toolbar: its layout and each tool's letter in effect. Settings PhotoCraft
/// has none of yet (the Show toggles, Disable Shortcuts for Hidden Toolbar Extras) are written with
/// their defaults: shown, off.
pub(super) fn write(prefs: &Preferences) -> Vec<u8> {
    let layout = prefs.toolbar.layout();
    let groups = |groups: &[Vec<String>]| -> D {
        D::List(
            groups
                .iter()
                .map(|group| {
                    D::List(
                        group
                            .iter()
                            .filter_map(|tool| {
                                let (_, key) = TOOL_KEYS.iter().find(|(name, _)| *name == tool)?;
                                let id = command_id(tool);
                                let mut d = object().with("toolKey", D::Integer(i32::from_be_bytes(**key)));
                                if let Some(c) = prefs.shortcut(&id, default_letter(&id)).and_then(letter) {
                                    d = d.with("keyboardShortcut", D::Text(UnicodeString::new_nul(&c.to_string())));
                                }
                                Some(D::Descriptor(d))
                            })
                            .collect(),
                    )
                })
                .collect(),
        )
    };
    object()
        .with("tver", D::Integer(1))
        .with("tpad", D::Integer(0))
        .with("fgbg", D::Boolean(true))
        .with("qkmm", D::Boolean(true))
        .with("sImI", D::Boolean(true))
        .with("scmd", D::Boolean(true))
        .with("shov", D::Boolean(true))
        .with("nana", D::Boolean(false))
        .with("ddkb", D::Boolean(false))
        .with("tlst", groups(&layout.toolbar))
        .with("oflt", groups(&layout.extras))
        .to_bytes()
}

/// A descriptor as the reference application writes them: the name is a lone NUL, the class `null`.
fn object() -> Descriptor {
    Descriptor { name: UnicodeString::new_nul(""), class_id: Id::new("null"), items: Vec::new() }
}

/// Read a preset. A file that isn't one whole descriptor with both lists is refused; inside the
/// lists, entries that aren't a known tool are skipped and a tool listed twice keeps its first place.
pub(super) fn read(bytes: &[u8]) -> Result<Preset, String> {
    const NOT_A_PRESET: &str = "could not read the toolbar preset";
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("{NOT_A_PRESET}: the file is too large"));
    }
    let (d, used) = Descriptor::parse_prefix_with_limits(bytes, LIMITS).map_err(|e| format!("{NOT_A_PRESET}: {e}"))?;
    if used != bytes.len() {
        return Err(format!("{NOT_A_PRESET}: trailing bytes"));
    }
    let mut preset = Preset::default();
    let mut seen: Vec<&str> = Vec::new();
    let mut lists = Vec::new();
    for key in ["tlst", "oflt"] {
        let Some(D::List(groups)) = d.get(key) else { return Err(format!("{NOT_A_PRESET}: `{key}` is missing")) };
        let mut out = Vec::new();
        for group in groups {
            let D::List(tools) = group else { return Err(format!("{NOT_A_PRESET}: `{key}` holds something other than a tool group")) };
            let mut names = Vec::new();
            for tool in tools {
                let D::Descriptor(t) = tool else { continue };
                let Some(D::Integer(code)) = t.get("toolKey") else { continue };
                let code = code.to_be_bytes();
                let Some((name, _)) = TOOL_KEYS.iter().find(|(_, key)| **key == code) else { continue };
                if seen.contains(name) {
                    continue;
                }
                seen.push(*name);
                // No shortcut (or an empty one) is no letter; one that isn't a single letter
                // leaves the tool's letter as it is.
                let taken = match t.get("keyboardShortcut") {
                    None => Some(None),
                    Some(D::Text(s)) if s.to_string_lossy().is_empty() => Some(None),
                    Some(D::Text(s)) => letter(&s.to_string_lossy()).map(Some),
                    Some(_) => None,
                };
                if let Some(taken) = taken {
                    preset.letters.push(((*name).to_string(), taken));
                }
                names.push((*name).to_string());
            }
            out.push(names);
        }
        lists.push(out);
    }
    preset.extras = lists.pop().unwrap_or_default();
    preset.toolbar = lists.pop().unwrap_or_default();
    Ok(preset)
}

/// Take a preset's letters as they are, with no conflict check: a letter equal to the tool's default
/// drops the override, any other is stored, and no letter removes the tool's shortcut.
pub(super) fn apply_letters(prefs: &mut Preferences, letters: &[(String, Option<char>)]) {
    for (tool, letter) in letters {
        let id = command_id(tool);
        let letter = letter.map(|c| c.to_string());
        if letter.as_deref() == default_letter(&id) {
            prefs.shortcuts.remove(&id);
        } else {
            prefs.shortcuts.insert(id, letter.unwrap_or_default());
        }
    }
}

/// Write a preset to `path`, replacing any file there atomically (a failed write keeps it).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn write_file(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    photocraft_format::atomic_write(std::path::Path::new(path), bytes)
}

#[cfg(target_arch = "wasm32")]
pub(super) fn write_file(_: &str, _: &[u8]) -> std::io::Result<()> {
    Err(std::io::Error::other("paths can't be written on the web; omit `path` to get the preset back"))
}

#[cfg(test)]
#[path = "toolbar_presets_tests.rs"]
mod tests;
