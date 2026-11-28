//! Toolbar presets (`.tbr`, Edit › Toolbar › Save Preset… / Load Preset…): the toolbar's layout and
//! tool letters as a preset that the reference application reads and writes too (#14624). The file
//! format itself is `craft_psd::tbr`; this module maps its tool keys and shortcuts to PhotoCraft's.

use craft_psd::tbr::{self, Shortcut, Tool, ToolbarPreset};

use super::Preferences;

/// The largest preset read. A preset with every tool is about 3 KiB.
pub(super) const MAX_BYTES: u64 = tbr::MAX_BYTES as u64;

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
/// their defaults: shown, off. A layout the file can't hold is an error, not a file `read` refuses.
pub(super) fn write(prefs: &Preferences) -> Result<Vec<u8>, String> {
    let layout = prefs.toolbar.layout();
    let groups = |groups: &[Vec<String>]| -> Vec<Vec<Tool>> {
        groups
            .iter()
            .map(|group| {
                group
                    .iter()
                    .filter_map(|tool| {
                        let (_, key) = TOOL_KEYS.iter().find(|(name, _)| *name == tool)?;
                        let id = command_id(tool);
                        let shortcut = match prefs.shortcut(&id, default_letter(&id)).and_then(letter) {
                            Some(c) => Shortcut::Text(c.to_string()),
                            None => Shortcut::Absent,
                        };
                        Some(Tool { key: **key, shortcut })
                    })
                    .collect()
            })
            .collect()
    };
    tbr::write(&ToolbarPreset { toolbar: groups(&layout.toolbar), extras: groups(&layout.extras), ..ToolbarPreset::default() })
        .map_err(|e| format!("could not write the toolbar preset: {e}"))
}

/// Read a preset. A file `craft_psd::tbr` refuses is refused; inside the lists, tools that aren't
/// PhotoCraft's are skipped and a tool listed twice keeps its first place.
pub(super) fn read(bytes: &[u8]) -> Result<Preset, String> {
    let file = tbr::read(bytes).map_err(|e| format!("could not read the toolbar preset: {e}"))?;
    let mut preset = Preset::default();
    let mut seen: Vec<&str> = Vec::new();
    let mut names = |groups: &[Vec<Tool>]| -> Vec<Vec<String>> {
        groups
            .iter()
            .map(|group| {
                let mut names = Vec::new();
                for tool in group {
                    let Some((name, _)) = TOOL_KEYS.iter().find(|(_, key)| **key == tool.key) else { continue };
                    if seen.contains(name) {
                        continue;
                    }
                    seen.push(*name);
                    // No shortcut (or an empty one) is no letter; one that isn't a single letter
                    // leaves the tool's letter as it is.
                    let taken = match &tool.shortcut {
                        Shortcut::Absent => Some(None),
                        Shortcut::Text(s) if s.is_empty() => Some(None),
                        Shortcut::Text(s) => letter(s).map(Some),
                        Shortcut::NotText => None,
                    };
                    if let Some(taken) = taken {
                        preset.letters.push(((*name).to_string(), taken));
                    }
                    names.push((*name).to_string());
                }
                names
            })
            .collect()
    };
    let toolbar = names(&file.toolbar);
    let extras = names(&file.extras);
    preset.toolbar = toolbar;
    preset.extras = extras;
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
