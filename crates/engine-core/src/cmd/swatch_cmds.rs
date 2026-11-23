//! Swatches panel commands (`swatches.*`): add, delete, rename, reorder and group swatches, set
//! the foreground or background colour from one, restore the defaults, and import or export
//! swatch files (`.aco`, `.act`, `.ase`, `.gpl`; export also `.css`).
//!
//! The library is [`PresetState::swatches`](crate::presets::PresetState) (model and colour
//! conversions in [`crate::presets::swatches`]); the file formats are craft-color's
//! (`craft::color_ui::palette`). Swatches are addressed by name (names are kept unique across the
//! library, as for the other presets), optionally narrowed by `group`, or by `index` (position
//! across all groups, in panel order). Like the other preset commands they change the
//! preferences document, not a document's history.

use crate::presets::PresetsExt;
use crate::presets::presets_changed;
use craft::color_ui::palette::{SwatchFormat, SwatchNote, default_swatches, read_swatches, write_swatches};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::presets::swatches::{SwatchColor, SwatchConv, SwatchEntry, SwatchGroup, SwatchKind, color_from_param, color_json, hex, kind_id, parse_kind};
use crate::presets::{bad, str_param};
use crate::{Result, Session};

/// Most swatches the library holds (imports beyond it are refused).
pub const MAX_SWATCHES: usize = 100_000;
/// Most groups the library holds.
pub const MAX_GROUPS: usize = 10_000;
/// Longest swatch or group name kept (characters).
pub const MAX_NAME: usize = 255;

/// Trim a hand-edited or imported library to the limits (never refuse to start over it).
pub fn cap_library(groups: &mut Vec<SwatchGroup>) {
    groups.truncate(MAX_GROUPS);
    let mut left = MAX_SWATCHES;
    for g in groups.iter_mut() {
        g.entries.truncate(left);
        left -= g.entries.len();
        g.name = clip(&g.name);
        for s in &mut g.entries {
            s.name = clip(&s.name);
        }
    }
}

fn clip(s: &str) -> String {
    s.chars().take(MAX_NAME).collect()
}

fn total(groups: &[SwatchGroup]) -> usize {
    groups.iter().map(|g| g.entries.len()).sum()
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

fn has_swatches(s: &Session) -> std::result::Result<(), String> {
    if total(&s.presets().swatches) > 0 { Ok(()) } else { Err("the Swatches panel is empty".into()) }
}

fn has_groups(s: &Session) -> std::result::Result<(), String> {
    if s.presets().swatches.is_empty() { Err("there are no swatch groups".into()) } else { Ok(()) }
}

fn name_param(p: &Value, k: &str, cmd: &str) -> Result<Option<String>> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(clip(s.trim()))),
        Some(_) => Err(bad(cmd, format!("`{k}` must be a non-empty string"))),
    }
}

/// Position (group, swatch) of the swatch called `name`, optionally only in `group`.
fn locate_swatch(groups: &[SwatchGroup], name: &str, group: Option<&str>) -> Option<(usize, usize)> {
    crate::presets::find_in(groups.iter().map(|g| (g.name.as_str(), g.entries.iter().map(|e| e.name.as_str()))), name, group)
}

/// `base`, or `base 2`, `base 3`… so swatch names stay unique across all groups.
fn unique_swatch_name(groups: &[SwatchGroup], base: &str) -> String {
    crate::presets::unique_name_by(base, |n| groups.iter().any(|g| g.entries.iter().any(|e| e.name == n)))
}

/// `base`, or `base 1`, `base 2`… so group names stay unique.
fn unique_group(groups: &[SwatchGroup], base: &str) -> String {
    crate::presets::unique_group(base, |n| groups.iter().any(|g| g.name == n))
}

/// The swatch a command names: `"swatch"` (name, narrowed by `"group"`) or `"index"`.
fn locate(s: &Session, p: &Value, cmd: &str) -> Result<(usize, usize)> {
    let groups = &s.presets().swatches;
    if let Some(name) = str_param(p, "swatch") {
        return locate_swatch(groups, name, str_param(p, "group")).ok_or_else(|| bad(cmd, format!("no swatch \"{name}\" (see swatches.list)")));
    }
    if let Some(i) = p.get("index") {
        let i = i.as_u64().ok_or_else(|| bad(cmd, "`index` must be a non-negative integer"))?;
        let mut left = usize::try_from(i).unwrap_or(usize::MAX);
        for (gi, g) in groups.iter().enumerate() {
            if left < g.entries.len() {
                return Ok((gi, left));
            }
            left -= g.entries.len();
        }
        return Err(bad(cmd, format!("no swatch at index {i} (the panel has {})", total(groups))));
    }
    Err(bad(cmd, "give `swatch` (a name) or `index`"))
}

/// Name → position of the first swatch with that name (optionally only in `group`), so commands
/// taking lists of names stay linear on big libraries.
fn index_by_name<'a>(groups: &'a [SwatchGroup], group: Option<&str>) -> std::collections::HashMap<&'a str, (usize, usize)> {
    let mut m = std::collections::HashMap::new();
    for (gi, g) in groups.iter().enumerate().filter(|(_, g)| group.is_none_or(|n| g.name == n)) {
        for (ii, sw) in g.entries.iter().enumerate() {
            m.entry(sw.name.as_str()).or_insert((gi, ii));
        }
    }
    m
}

fn group_of(s: &Session, name: &str, cmd: &str) -> Result<usize> {
    s.presets().swatches.iter().position(|g| g.name == name).ok_or_else(|| bad(cmd, format!("no swatch group \"{name}\"")))
}

fn swatch_json(conv: &SwatchConv, sw: &SwatchEntry) -> Value {
    let mut v = color_json(&sw.color);
    v["name"] = json!(sw.name);
    v["hex"] = json!(hex(conv.rgb(&sw.color)));
    if sw.kind != SwatchKind::Process {
        v["kind"] = json!(kind_id(sw.kind));
    }
    v
}

// ------------------------------------------------------------------ commands

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let conv = SwatchConv::new(s);
    let groups: Vec<Value> = s
        .presets()
        .swatches
        .iter()
        .map(|g| json!({"name": g.name, "swatches": g.entries.iter().map(|sw| swatch_json(&conv, sw)).collect::<Vec<_>>()}))
        .collect();
    Ok(json!({"groups": groups, "count": total(&s.presets().swatches)}))
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "swatches.add";
    let color = match p.get("color") {
        None | Some(Value::Null) => {
            let f = s.tools.foreground;
            SwatchColor::Rgb([f[0], f[1], f[2]].map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }))
        }
        Some(v) => color_from_param(v).map_err(|e| bad(CMD, e))?,
    };
    let kind = match str_param(p, "kind") {
        None => SwatchKind::Process,
        Some(k) => parse_kind(k).ok_or_else(|| bad(CMD, format!("unknown kind `{k}` (process|global|spot)")))?,
    };
    if total(&s.presets().swatches) >= MAX_SWATCHES {
        return Err(bad(CMD, format!("the Swatches panel is full ({MAX_SWATCHES} swatches)")));
    }
    let group = name_param(p, "group", CMD)?;
    let explicit = name_param(p, "name", CMD)?;
    let groups = &mut s.presets_mut().swatches;
    let name = match explicit {
        Some(n) => unique_swatch_name(groups, &n),
        // Photoshop's default: "Color Swatch 1", "Color Swatch 2"…
        None => {
            let taken = |n: &str| groups.iter().any(|g| g.entries.iter().any(|i| i.name == n));
            (1..).map(|i| format!("Color Swatch {i}")).find(|n| !taken(n)).unwrap_or_default()
        }
    };
    let gi = match group {
        Some(g) => match groups.iter().position(|x| x.name == g) {
            Some(i) => i,
            None => {
                if groups.len() >= MAX_GROUPS {
                    return Err(bad(CMD, "too many swatch groups"));
                }
                groups.push(SwatchGroup { name: g, entries: Vec::new() });
                groups.len() - 1
            }
        },
        None => {
            if groups.is_empty() {
                groups.push(SwatchGroup { name: "Swatches".into(), entries: Vec::new() });
            }
            0
        }
    };
    let entries = &mut groups[gi].entries;
    let at = match p.get("index") {
        None | Some(Value::Null) => entries.len(),
        Some(v) => usize::try_from(v.as_u64().ok_or_else(|| bad(CMD, "`index` must be a non-negative integer"))?).unwrap_or(usize::MAX).min(entries.len()),
    };
    entries.insert(at, SwatchEntry { name: name.clone(), color, kind });
    let gname = groups[gi].name.clone();
    presets_changed(s);
    Ok(json!({"name": name, "group": gname, "index": at}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "swatches.delete";
    let mut targets: Vec<(usize, usize)> = match p.get("swatch") {
        Some(Value::Array(a)) => {
            let ix = index_by_name(&s.presets().swatches, str_param(p, "group"));
            let mut v = Vec::with_capacity(a.len().min(MAX_SWATCHES));
            for n in a {
                let name = n.as_str().ok_or_else(|| bad(CMD, "`swatch` names must be strings"))?;
                v.push(*ix.get(name).ok_or_else(|| bad(CMD, format!("no swatch \"{name}\" (see swatches.list)")))?);
            }
            v
        }
        _ => vec![locate(s, p, CMD)?],
    };
    // Remove from the back so earlier positions stay valid; duplicates count once.
    targets.sort_unstable();
    targets.dedup();
    let groups = &mut s.presets_mut().swatches;
    let mut names = Vec::with_capacity(targets.len());
    for (gi, ii) in targets.into_iter().rev() {
        if let Some(g) = groups.get_mut(gi)
            && ii < g.entries.len()
        {
            names.push(g.entries.remove(ii).name);
        }
    }
    names.reverse();
    presets_changed(s);
    Ok(json!({"deleted": names}))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "swatches.rename";
    let (gi, ii) = locate(s, p, CMD)?;
    let to = name_param(p, "name", CMD)?.ok_or_else(|| bad(CMD, "missing `name`"))?;
    let groups = &mut s.presets_mut().swatches;
    if groups[gi].entries[ii].name != to && locate_swatch(groups, &to, None).is_some() {
        return Err(bad(CMD, format!("a swatch named \"{to}\" already exists")));
    }
    groups[gi].entries[ii].name = to.clone();
    presets_changed(s);
    Ok(json!({"name": to}))
}

fn move_swatch(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "swatches.move";
    let (gi, ii) = locate(s, p, CMD)?;
    let ti = match name_param(p, "to", CMD)? {
        Some(t) => group_of(s, &t, CMD)?,
        None => gi,
    };
    let at = match p.get("position") {
        None | Some(Value::Null) => None,
        Some(v) => Some(usize::try_from(v.as_u64().ok_or_else(|| bad(CMD, "`position` must be a non-negative integer"))?).unwrap_or(usize::MAX)),
    };
    let groups = &mut s.presets_mut().swatches;
    let item = groups[gi].entries.remove(ii);
    let entries = &mut groups[ti].entries;
    let at = at.unwrap_or(entries.len()).min(entries.len());
    entries.insert(at, item);
    let gname = groups[ti].name.clone();
    presets_changed(s);
    Ok(json!({"group": gname, "position": at}))
}

/// `swatches.moveGroup`: the group to index `to` among the groups (clamped to the list).
fn move_group(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "swatches.moveGroup";
    let g = name_param(p, "group", CMD)?.ok_or_else(|| bad(CMD, "missing `group`"))?;
    let to = p.get("to").and_then(Value::as_u64).ok_or_else(|| bad(CMD, "`to` must be a non-negative integer"))?;
    let from = group_of(s, &g, CMD)?;
    let groups = &mut s.presets_mut().swatches;
    let at = usize::try_from(to).unwrap_or(usize::MAX).min(groups.len() - 1);
    let group = groups.remove(from);
    groups.insert(at, group);
    presets_changed(s);
    Ok(json!({"group": g, "position": at}))
}

fn new_group(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "swatches.newGroup";
    if s.presets().swatches.len() >= MAX_GROUPS {
        return Err(bad(CMD, "too many swatch groups"));
    }
    let base = name_param(p, "name", CMD)?.unwrap_or_else(|| "Group".into());
    // Swatches to move into the new group (Photoshop: New Group from the selected swatches).
    let mut moving: Vec<(usize, usize)> = Vec::new();
    if let Some(a) = p.get("swatches") {
        let a = a.as_array().ok_or_else(|| bad(CMD, "`swatches` must be a list of names"))?;
        let ix = index_by_name(&s.presets().swatches, None);
        for n in a {
            let name = n.as_str().ok_or_else(|| bad(CMD, "`swatches` names must be strings"))?;
            moving.push(*ix.get(name).ok_or_else(|| bad(CMD, format!("no swatch \"{name}\"")))?);
        }
    }
    moving.sort_unstable();
    moving.dedup();
    let groups = &mut s.presets_mut().swatches;
    let name = unique_group(groups, &base);
    let mut entries: Vec<SwatchEntry> = Vec::with_capacity(moving.len());
    for (gi, ii) in moving.into_iter().rev() {
        entries.push(groups[gi].entries.remove(ii));
    }
    entries.reverse();
    let n = entries.len();
    groups.push(SwatchGroup { name: name.clone(), entries });
    presets_changed(s);
    Ok(json!({"group": name, "moved": n}))
}

fn rename_group(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "swatches.renameGroup";
    let g = name_param(p, "group", CMD)?.ok_or_else(|| bad(CMD, "missing `group`"))?;
    let to = name_param(p, "name", CMD)?.ok_or_else(|| bad(CMD, "missing `name`"))?;
    let i = group_of(s, &g, CMD)?;
    if g != to && s.presets().swatches.iter().any(|x| x.name == to) {
        return Err(bad(CMD, format!("a group named \"{to}\" already exists")));
    }
    s.presets_mut().swatches[i].name = to.clone();
    presets_changed(s);
    Ok(json!({"group": to}))
}

fn delete_group(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "swatches.deleteGroup";
    let g = name_param(p, "group", CMD)?.ok_or_else(|| bad(CMD, "missing `group`"))?;
    let keep = match p.get("keepSwatches") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool().ok_or_else(|| bad(CMD, "`keepSwatches` must be true or false"))?,
    };
    let i = group_of(s, &g, CMD)?;
    let groups = &mut s.presets_mut().swatches;
    if keep && groups.len() < 2 {
        return Err(bad(CMD, "there is no other group to keep the swatches in"));
    }
    let removed = groups.remove(i);
    let n = removed.entries.len();
    if keep {
        // Into the group above (or the new first group), like Photoshop's "Delete Group Only".
        let into = i.saturating_sub(1).min(groups.len().saturating_sub(1));
        if let Some(t) = groups.get_mut(into) {
            t.entries.extend(removed.entries);
        }
    }
    presets_changed(s);
    Ok(json!({"group": g, if keep { "kept" } else { "deleted" }: n}))
}

fn use_swatch(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "swatches.use";
    let (gi, ii) = locate(s, p, CMD)?;
    let target = str_param(p, "target").unwrap_or("foreground");
    let sw = s.presets().swatches[gi].entries[ii].clone();
    let rgb = SwatchConv::new(s).rgb(&sw.color);
    let c = [rgb[0], rgb[1], rgb[2], 1.0];
    match target {
        "foreground" => s.tools.foreground = c,
        "background" => s.tools.background = c,
        t => return Err(bad(CMD, format!("unknown target `{t}` (foreground|background)"))),
    }
    // A CMYK swatch keeps its inks (as a colour picked in CMYK does, #4196): the Color Picker
    // shows them and a CMYK document is painted with them, not with the sRGB round trip (#7474).
    if let SwatchColor::Cmyk(inks) = sw.color
        && inks.iter().all(|v| v.is_finite())
    {
        s.tools.remember_cmyk(rgb, inks.map(|v| v.clamp(0.0, 1.0)));
    }
    Ok(json!({"swatch": sw.name, "target": target, "hex": hex(rgb)}))
}

fn reset(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "swatches.reset";
    let append = match p.get("append") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool().ok_or_else(|| bad(CMD, "`append` must be true or false"))?,
    };
    let mut b = default_swatches();
    if append {
        if total(&s.presets().swatches) + total(&b) > MAX_SWATCHES || s.presets().swatches.len() + b.len() > MAX_GROUPS {
            return Err(bad(CMD, "the Swatches panel is full"));
        }
        for g in &mut b {
            g.name = unique_group(&s.presets().swatches, &g.name);
            for sw in &mut g.entries {
                sw.name = unique_swatch_name(&s.presets().swatches, &sw.name);
            }
            s.presets_mut().swatches.push(g.clone());
        }
    } else {
        s.presets_mut().swatches = b;
    }
    presets_changed(s);
    Ok(json!({"groups": s.presets().swatches.len(), "count": total(&s.presets().swatches)}))
}

// ------------------------------------------------------------------ files

/// Colors in an `.act` color table (what the format holds).
pub const ACT_COLORS: usize = craft_psd::act::COLOR_COUNT;

/// `.act` bytes as `#rrggbb` colors plus the transparent index, for the Color Table dialog.
/// A trailer count of 0 gives one white entry, as Photoshop loads it, never an empty table
/// that `image.mode.colorTable` would refuse.
pub fn act_table(bytes: &[u8]) -> std::result::Result<(Vec<String>, Option<u64>), String> {
    let f = craft_psd::act::parse(bytes).map_err(|e| e.to_string())?;
    let hex = |c: &[u8; 3]| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
    let mut colors: Vec<String> = f.used().iter().map(hex).collect();
    if colors.is_empty() {
        colors.push(hex(&[255, 255, 255]));
    }
    Ok((colors, f.transparent_index().map(|i| i as u64)))
}

/// Encode colors as `.act` bytes (at most [`ACT_COLORS`]); the transparent index is kept.
pub fn act_bytes(colors: &[[u8; 3]], transparent: Option<u16>) -> std::result::Result<Vec<u8>, String> {
    craft_psd::act::write(colors, Some(colors.len().min(ACT_COLORS) as u16), transparent).map_err(|e| e.to_string())
}

/// The format an id or extension names (`aco`, `.ACT`, …).
fn parse_format(s: &str) -> Option<SwatchFormat> {
    SwatchFormat::from_name(&format!("x.{}", s.trim_start_matches('.')))
}

/// One sentence for a note of a read (`writing` false) or a write of `format`.
fn word_note(note: &SwatchNote, format: SwatchFormat, writing: bool) -> String {
    let ext = format.extension();
    match *note {
        SwatchNote::BookColors(n) if writing => {
            format!("{n} colour-book swatches can't be stored in a .{ext} file and were left out (save as .aco to keep them)")
        }
        SwatchNote::BookColors(n) => {
            format!("{n} swatches use a colour book or colour space PhotoCraft can't show; they are kept (shown gray) and saved back unchanged")
        }
        SwatchNote::Skipped(n) => format!("{n} swatches in an unknown colour model were skipped"),
        SwatchNote::GroupsFlattened => format!(".{ext} files have no groups; the swatches were saved as one list"),
        SwatchNote::ConvertedToRgb(n) => format!("{n} non-RGB swatches were converted to RGB to fit a .{ext} file"),
        SwatchNote::KindsDropped(n) => {
            format!("{n} spot or global swatches were saved as process colours; .{ext} files don't mark them (save as .ase to keep them)")
        }
        SwatchNote::Truncated(n) => format!("only the first {ACT_COLORS} swatches fit in an .act file; {n} were left out"),
        SwatchNote::NamesCut(n) => format!("{n} names were cut to the length the .{ext} format holds"),
        SwatchNote::TransparentKept(n) => format!("color {n} is the table's transparent color; swatches have no transparency so it was kept opaque"),
        // The note list may grow (the enum is non-exhaustive).
        _ => "some data could not be kept".into(),
    }
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "swatches.import";
    let (bytes, stem) = crate::preset_import_cmds::file_bytes(p, CMD, ".aco, .act, .ase or .gpl file")?;
    let mode = str_param(p, "mode").unwrap_or("append");
    let replace = match mode {
        "append" => false,
        "replace" => true,
        m => return Err(bad(CMD, format!("unknown mode `{m}` (append|replace)"))),
    };
    let group = match name_param(p, "group", CMD)? {
        Some(g) => g,
        None if !stem.trim().is_empty() => clip(stem.trim()),
        None => "Imported Swatches".into(),
    };
    // The extension is a hint for the sniffing: an explicit `format`, else the path's extension,
    // else none (`bin`). An `.act` has no signature: without a hint it is told by its length.
    let hint = match str_param(p, "format") {
        Some(f) => {
            let format = parse_format(f).ok_or_else(|| bad(CMD, format!("unknown format `{f}` (aco|act|ase|gpl)")))?;
            if !format.can_read() {
                return Err(bad(CMD, format!(".{} files can be written, not read", format.extension())));
            }
            format.extension().to_string()
        }
        None => str_param(p, "path").and_then(|path| std::path::Path::new(path).extension()).and_then(|e| e.to_str()).unwrap_or("bin").to_string(),
    };
    let format = SwatchFormat::sniff(&bytes, &format!("{group}.{hint}")).ok_or_else(|| bad(CMD, "not an .aco, .act, .ase or .gpl swatch file"))?;
    // The group name is the file stem: craft-color names ungrouped colours after it.
    let (groups, report) = read_swatches(&bytes, &format!("{group}.{}", format.extension())).map_err(|e| bad(CMD, e.to_string()))?;
    let warnings: Vec<String> = report.notes.iter().map(|n| word_note(n, format, false)).collect();
    let base: Vec<SwatchGroup> = if replace { Vec::new() } else { s.presets().swatches.clone() };
    if total(&base) + total(&groups) > MAX_SWATCHES || base.len() + groups.len() > MAX_GROUPS {
        return Err(bad(CMD, format!("too many swatches: the panel holds at most {MAX_SWATCHES} in {MAX_GROUPS} groups")));
    }
    let mut lib = base;
    let count = total(&groups);
    let mut names = Vec::with_capacity(groups.len());
    // Names stay unique across the library (agents address swatches by name).
    let mut next: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut taken: std::collections::HashSet<String> = lib.iter().flat_map(|g| g.entries.iter().map(|i| i.name.clone())).collect();
    // Group names are numbered from 1 as in `unique_group` ("Grays 1"), and the numbering
    // resumes per base name.
    let mut group_next: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut group_taken: std::collections::HashSet<String> = lib.iter().map(|g| g.name.clone()).collect();
    for mut g in groups {
        g.name = clip(&g.name);
        if group_taken.contains(&g.name) {
            let from = group_next.get(&g.name).copied().unwrap_or(1);
            let (i, n) = (from..).map(|i| (i, format!("{} {i}", g.name))).find(|(_, n)| !group_taken.contains(n)).unwrap_or((from, g.name.clone()));
            group_next.insert(g.name.clone(), i + 1);
            g.name = n;
        }
        group_taken.insert(g.name.clone());
        for sw in &mut g.entries {
            sw.name = clip(&sw.name);
            if taken.contains(&sw.name) {
                // Resume numbering per base name, so a file of identical names stays linear.
                let from = next.get(&sw.name).copied().unwrap_or(2);
                let (i, n) = (from..).map(|i| (i, format!("{} {i}", sw.name))).find(|(_, n)| !taken.contains(n)).unwrap_or((from, sw.name.clone()));
                next.insert(sw.name.clone(), i + 1);
                sw.name = n;
            }
            taken.insert(sw.name.clone());
        }
        names.push(g.name.clone());
        lib.push(g);
    }
    s.presets_mut().swatches = lib;
    presets_changed(s);
    Ok(json!({"format": format.extension(), "mode": mode, "groups": names, "count": count, "warnings": warnings}))
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "swatches.export";
    let path = str_param(p, "path");
    let format = match str_param(p, "format") {
        Some(f) => parse_format(f).ok_or_else(|| bad(CMD, format!("unknown format `{f}` (aco|act|ase|gpl|css)")))?,
        None => path.and_then(SwatchFormat::from_name).unwrap_or(SwatchFormat::Aco),
    };
    let mut groups: Vec<SwatchGroup> = match name_param(p, "group", CMD)? {
        Some(g) => vec![s.presets().swatches[group_of(s, &g, CMD)?].clone()],
        None => s.presets().swatches.clone(),
    };
    // Only some swatches (Photoshop: Export Selected Swatches).
    if let Some(a) = p.get("swatches") {
        let a = a.as_array().ok_or_else(|| bad(CMD, "`swatches` must be a list of names"))?;
        let mut want = std::collections::HashSet::with_capacity(a.len().min(MAX_SWATCHES));
        {
            let ix = index_by_name(&groups, None);
            for n in a {
                let name = n.as_str().ok_or_else(|| bad(CMD, "`swatches` names must be strings"))?;
                if !ix.contains_key(name) {
                    return Err(bad(CMD, format!("no swatch \"{name}\"")));
                }
                want.insert(name.to_string());
            }
        }
        for g in &mut groups {
            g.entries.retain(|sw| want.contains(&sw.name));
        }
        groups.retain(|g| !g.entries.is_empty());
    }
    if total(&groups) == 0 {
        return Err(bad(CMD, "no swatches to export"));
    }
    let (bytes, report) = write_swatches(&groups, format).map_err(|e| bad(CMD, e.to_string()))?;
    // What the report says was left out is not in the file.
    let left_out: usize = report
        .notes
        .iter()
        .map(|n| match n {
            SwatchNote::BookColors(c) | SwatchNote::Truncated(c) => *c,
            _ => 0,
        })
        .sum();
    let count = total(&groups).saturating_sub(left_out);
    let warnings: Vec<String> = report.notes.iter().map(|n| word_note(n, format, true)).collect();
    let mut out = json!({"format": format.extension(), "count": count, "bytes": bytes.len(), "warnings": warnings});
    match path {
        Some(path) => {
            crate::file_cmds::write_file(path, &bytes)?;
            out["path"] = json!(path);
        }
        None => out["data"] = json!(photocraft_paint::tile::b64_encode(&bytes)),
    }
    Ok(out)
}

/// Swatches panel command specs.
pub fn specs() -> Vec<CommandSpec> {
    const COLOR: &str = r##""#rrggbb"|{"model":"rgb|hsb|cmyk|lab|gray","values":[rgb 0..255 | hue°,s%,b% | c%,m%,y%,k% | L,a,b | k%]}"##;
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let target = r##""swatch":name (or "index":n across all groups),"group":name? (narrows the name lookup)"##;
    vec![
        CommandSpec {
            id: "swatches.list",
            label: "Swatches",
            menu: &[],
            shortcut: None,
            params: r##"{} → {groups:[{name,swatches:[{name,model,values,hex,kind?}]}],count}. values are in Color Picker units; hex is the colour in the RGB working space."##,
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "swatches.add",
            label: "New Swatch…",
            menu: &[],
            shortcut: None,
            params: leak(format!(
                r##"{{"color":{COLOR}?=foreground,"name":str?="Color Swatch N","group":name?=first (created when missing),"index":n?=end,"kind":"process|global|spot"="process"}} → {{name,group,index}}"##
            )),
            enabled: always,
            run: add,
            journal: true,
        },
        CommandSpec {
            id: "swatches.delete",
            label: "Delete Swatch",
            menu: &[],
            shortcut: None,
            params: leak(format!(r##"{{{target}; "swatch" may be a list of names}} → {{deleted:[names]}}"##)),
            enabled: has_swatches,
            run: delete,
            journal: true,
        },
        CommandSpec {
            id: "swatches.rename",
            label: "Rename Swatch…",
            menu: &[],
            shortcut: None,
            params: leak(format!(r##"{{{target},"name":str}} → {{name}}"##)),
            enabled: has_swatches,
            run: rename,
            journal: true,
        },
        CommandSpec {
            id: "swatches.move",
            label: "Move Swatch",
            menu: &[],
            shortcut: None,
            params: leak(format!(r##"{{{target},"to":group?=its own group,"position":n?=end}} → {{group,position}}"##)),
            enabled: has_swatches,
            run: move_swatch,
            journal: true,
        },
        CommandSpec {
            id: "swatches.moveGroup",
            label: "Move Swatch Group",
            menu: &[],
            shortcut: None,
            params: r##"{"group":name,"to":n (index among the groups, clamped to the list)} → {group,position}"##,
            enabled: has_groups,
            run: move_group,
            journal: true,
        },
        CommandSpec {
            id: "swatches.newGroup",
            label: "New Swatch Group…",
            menu: &[],
            shortcut: None,
            params: r##"{"name":str="Group" (made unique),"swatches":[names]? (moved into the new group)} → {group,moved}"##,
            enabled: always,
            run: new_group,
            journal: true,
        },
        CommandSpec {
            id: "swatches.renameGroup",
            label: "Rename Swatch Group…",
            menu: &[],
            shortcut: None,
            params: r##"{"group":name,"name":str} → {group}"##,
            enabled: has_groups,
            run: rename_group,
            journal: true,
        },
        CommandSpec {
            id: "swatches.deleteGroup",
            label: "Delete Swatch Group",
            menu: &[],
            shortcut: None,
            params: r##"{"group":name,"keepSwatches":bool=false (move its swatches into the group above)} → {group,deleted|kept}"##,
            enabled: has_groups,
            run: delete_group,
            journal: true,
        },
        CommandSpec {
            id: "swatches.use",
            label: "Use Swatch",
            menu: &[],
            shortcut: None,
            params: leak(format!(
                r##"{{{target},"target":"foreground|background"="foreground"}} → {{swatch,target,hex}}. CMYK, Lab and gray swatches are converted to the RGB working space with colour management."##
            )),
            enabled: has_swatches,
            run: use_swatch,
            journal: true,
        },
        CommandSpec {
            id: "swatches.reset",
            label: "Reset Swatches",
            menu: &[],
            shortcut: None,
            params: r##"{"append":bool=false (add the defaults instead of replacing the library)} → {groups,count}"##,
            enabled: always,
            run: reset,
            journal: true,
        },
        CommandSpec {
            id: "swatches.import",
            label: "Import Swatches…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".aco, .act, .ase or .gpl file"?,"data":base64 bytes?,"format":"aco|act|ase|gpl"? (detected; an .act by its .act path or its 768/772-byte length),"mode":"append|replace"="append","group":str?=file name (the group for ungrouped colours)} → {format,mode,groups,count,warnings}"##,
            enabled: always,
            run: import,
            journal: true,
        },
        CommandSpec {
            id: "swatches.export",
            label: "Export Swatches…",
            menu: &[],
            shortcut: None,
            params: r##"{"format":"aco|act|ase|gpl|css"?=from path, else aco,"path":file? (else the bytes come back as base64 `data`),"group":name? (only that group),"swatches":[names]? (only these)} → {format,count,bytes,warnings,path|data}"##,
            enabled: has_swatches,
            run: export,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod act_zero_count_tests;
