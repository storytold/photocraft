//! Edit â€º Menus â€º Set: named menu sets, as in Photoshop. The customization in use stays in
//! [`Preferences::menus`] (what the menus read); a set is a saved copy of it,
//! `Preferences::menu_sets` keyed by name, and `Preferences::menu_set` names the one in use
//! ("" for the built-in set, [`DEFAULTS_NAME`]). The menus differ from the set in use after an
//! edit until that edit is saved, which is what marks the Set row "(modified)". Keyboard
//! shortcut sets work the same way ([`crate::shortcut_sets`]), and the two lists are separate:
//! the Menus tab's Set dropdown is not the Keyboard Shortcuts tab's (#14450).
//!
//! `edit.menus` takes the set parameters: `useSet` (load a set's menus, before the call's other
//! changes), then `saveSet` (save the menus as they end up under a name and use it, which is
//! also how New makes a set from the menus in use), `deleteSet`, `sets` (replace every saved
//! set: the dialog's OK) and `activeSet`. The reply describes them: `sets`, `activeSet`,
//! `defaultsName` and `modified`.
//!
//! The reference application keeps each set as a `.mnu` file in a menu-sets folder of its settings
//! directory ([`SETS_DIR`], [`folder`]), the set name being the file name without the extension.
//! [`to_mnu`] and [`from_mnu`] write and read that layout - UTF-8 XML, one `<command>` element per
//! changed item, the measured numeric ids ([`REF_ITEMS`]) and colour numbers (0 to 7) - and keep
//! every entry they could not name ([`Mnu::kept`]). A malformed file is an error: bad XML, a colour
//! number outside the measured list, or a file over [`MAX_MNU_BYTES`]. The preferences stay the
//! store of what the menus in use are; the files are where the named sets live (#14450).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::prefs::{MenuCustomization, Preferences};

/// The built-in set's name: the menus as PhotoCraft ships them. The shortcut sets' built-in set
/// has the same name, and both dropdowns start there.
pub const DEFAULTS_NAME: &str = crate::shortcut_sets::DEFAULTS_NAME;
/// Saved sets are small; the cap keeps a hostile settings file or call bounded.
pub const MAX_SETS: usize = crate::shortcut_sets::MAX_SETS;
/// The colours Edit â€º Menus gives an item; `none` is no colour. The reference application's
/// twelve-entry list is #14336, not #14450.
pub const COLORS: [&str; 8] = ["none", "red", "orange", "yellow", "green", "blue", "violet", "gray"];

/// A saved menu set: the same thing the preferences keep as the customization in use, so a set
/// and the live menus share one shape instead of two that have to be kept in step.
pub type MenuSet = MenuCustomization;

/// `menus` with its hidden items deduplicated and in a stable order, and without the `none` colour
/// sentinel. The order items were hidden in is not part of a set, and a set written by a call or a
/// hand-edited file is stored like this so comparing the two sides means anything. `none` is the
/// picker's no-colour choice, not a colour: `prefs::customize::menus` removes the entry for it, so
/// a set that kept one would differ from the menus it just loaded and the row would read
/// "(modified)" after a call that changed nothing.
fn normalized(menus: &MenuSet) -> MenuSet {
    let colors = menus.colors.iter().filter(|(_, c)| c.as_str() != "none").map(|(id, c)| (id.clone(), c.clone())).collect();
    MenuSet { hidden: menus.hidden.iter().cloned().collect::<BTreeSet<_>>().into_iter().collect(), colors }
}

/// The menus `p` uses now, as a set: what `saveSet` stores.
pub fn of(p: &Preferences) -> MenuSet {
    normalized(&p.menus)
}

/// Check a set as a call sent it: only the colours `edit.menus` accepts, and only items that can
/// be hidden (#14335).
fn validate(set: &MenuSet) -> Result<(), String> {
    if let Some(c) = set.colors.values().find(|c| !COLORS.contains(&c.as_str())) {
        return Err(format!("unknown colour `{c}` ({})", COLORS.join("|")));
    }
    if let Some(id) = set.hidden.iter().find(|id| !MenuCustomization::hideable(id)) {
        return Err(format!("`{id}` can't be hidden"));
    }
    Ok(())
}

/// The saved copy of the set in use: `None` for the built-in set (or one that no longer exists).
fn saved(p: &Preferences) -> Option<&MenuSet> {
    p.menu_sets.get(&p.menu_set)
}

/// Do these menus differ from the set `active` of `sets` ("" or a missing name: the built-in
/// set)? Hidden items compare as a set, so re-hiding the same items in another order is not a
/// change. The Menus tab's "(modified)" and the Save button's enablement read this.
pub fn differs(sets: &BTreeMap<String, MenuSet>, active: &str, menus: &MenuSet) -> bool {
    let now = normalized(menus);
    match sets.get(active) {
        Some(s) => normalized(s) != now,
        None => now != MenuSet::default(),
    }
}

/// Do the menus in use differ from the set in use?
pub fn modified(p: &Preferences) -> bool {
    differs(&p.menu_sets, &p.menu_set, &p.menus)
}

/// Before the call's menu changes: `useSet` loads a set ("" or [`DEFAULTS_NAME`]: the built-in
/// set, whose menus are empty) and makes it the one in use. Returns whether it changed anything.
pub fn load(next: &mut Preferences, p: &Value) -> Result<bool, String> {
    let Some(v) = p.get("useSet") else { return Ok(false) };
    let name = v.as_str().ok_or("`useSet` is a set name")?.trim();
    if name.is_empty() || name.eq_ignore_ascii_case(DEFAULTS_NAME) {
        next.menus = MenuSet::default();
        next.menu_set = String::new();
    } else {
        // A set from an older or hand-edited settings file may list items that can't be hidden;
        // `is_hidden` leaves those shown, as #14335 measures.
        let set = next.menu_sets.get(name).cloned().ok_or_else(|| format!("no menu set named `{name}`"))?;
        next.menus = set;
        next.menu_set = name.to_string();
    }
    Ok(true)
}

/// After the call's menu changes: `sets` (replace every saved set), `deleteSet`, `saveSet` (save
/// the menus as they now stand under a name and use it) and `activeSet`. Returns whether it
/// changed anything. The set name rules are the shortcut sets' (`crate::shortcut_sets::set_name`):
/// the built-in name is reserved in both lists.
pub fn store(next: &mut Preferences, p: &Value) -> Result<bool, String> {
    let mut changed = false;
    if let Some(v) = p.get("sets") {
        let obj = v.as_object().ok_or("`sets` maps set names to {hidden, colors}")?;
        if obj.len() > MAX_SETS {
            return Err(format!("at most {MAX_SETS} menu sets"));
        }
        let mut sets = BTreeMap::new();
        for (name, s) in obj {
            let name = crate::shortcut_sets::set_name(name)?;
            let set: MenuSet = serde_json::from_value(s.clone()).map_err(|e| format!("set `{name}`: {e}"))?;
            validate(&set).map_err(|e| format!("set `{name}`: {e}"))?;
            sets.insert(name, normalized(&set));
        }
        next.menu_sets = sets;
        changed = true;
    }
    if let Some(v) = p.get("deleteSet") {
        // Delete is disabled for the built-in set in the dialog; here its reserved name is an
        // error, the same way the name rules reject it for `saveSet`.
        let name = crate::shortcut_sets::set_name(v.as_str().ok_or("`deleteSet` is a set name")?)?;
        if next.menu_sets.remove(&name).is_none() {
            return Err(format!("no menu set named `{name}`"));
        }
        if next.menu_set == name {
            next.menu_set = String::new();
        }
        changed = true;
    }
    if let Some(v) = p.get("saveSet") {
        let name = crate::shortcut_sets::set_name(v.as_str().ok_or("`saveSet` is a set name")?)?;
        if !next.menu_sets.contains_key(&name) && next.menu_sets.len() >= MAX_SETS {
            return Err(format!("at most {MAX_SETS} menu sets"));
        }
        next.menu_sets.insert(name.clone(), of(next));
        next.menu_set = name;
        changed = true;
    }
    if let Some(v) = p.get("activeSet") {
        let name = v.as_str().ok_or("`activeSet` is a set name")?.trim();
        if name.is_empty() || name.eq_ignore_ascii_case(DEFAULTS_NAME) {
            next.menu_set = String::new();
        } else if next.menu_sets.contains_key(name) {
            next.menu_set = name.to_string();
        } else {
            return Err(format!("no menu set named `{name}`"));
        }
        changed = true;
    }
    Ok(changed)
}

/// The sets, the one in use and whether the menus differ from it, for the command's reply.
pub fn describe(p: &Preferences) -> Value {
    json!({
        "sets": p.menu_sets.keys().collect::<Vec<_>>(),
        "activeSet": if saved(p).is_some() { p.menu_set.as_str() } else { "" },
        "defaultsName": DEFAULTS_NAME,
        "modified": modified(p),
    })
}

// ---- the `.mnu` files: the sets as files in the settings directory (#14450) ----

use std::path::{Path, PathBuf};

/// The folder of the settings directory the sets live in. The issue measures a menu-sets folder in
/// the settings directory; the reference application's own folder name is part of its
/// `$$$/FileName/Presets/MenuCustomization` path token, and its file is what this folder holds.
pub const SETS_DIR: &str = "menu-sets";
/// A set file's extension, as the reference's Save dialog names it ("Menu Customization (*.MNU)").
pub const SETS_EXT: &str = "MNU";
/// A set file bigger than this is refused instead of read: a menu set holds one row per changed
/// item of a menu tree, so this is far above what the widest set needs.
pub const MAX_MNU_BYTES: usize = 1 << 20;
/// The most set files one folder read takes, as [`MAX_SETS`] bounds the saved sets.
pub const MAX_SET_FILES: usize = MAX_SETS;
/// PhotoCraft's name in the root element, lower case, as the reference application spells its own:
/// `<photocraft-menu-settings â€¦>`.
const APP_NAME: &str = "photocraft";
/// What the root element's name ends with, in both applications.
const ROOT_SUFFIX: &str = "-menu-settings";
/// The only file version measured (#14450).
const FILE_VERSION: &str = "2";
/// The path the root's `filename` attribute carries, as measured, then the file's own name and the
/// set name it names.
const FILE_KEY: &str = "$$$/FileName/Presets/MenuCustomization";
/// What the file helpers answer where there is no settings folder to write into. Only the web stubs
/// use it: the desktop build has a real file system.
#[cfg(target_arch = "wasm32")]
const NO_SETTINGS: &str = "menu set files need a settings folder, which this build does not have";

/// One measured menu item: our command id, the number the reference application's file gives it,
/// the menu it was seen in (`parentMenuID`: 2 File, 3 Edit, 40 Info panel, 41 Layers panel) and
/// whether that was one of its panel menus (`menuType` 1) or an application menu (0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RefItem {
    /// The command id the menus and a set use.
    pub command: &'static str,
    /// The number the reference application's file gives this item.
    pub number: u32,
    /// The menu the row names as its parent.
    pub parent: u32,
    /// True for a panel menu.
    pub panel: bool,
}

/// The items #14450 measured numbers for, in that file's numeric order. Nothing else is here: a
/// number we have not measured keeps its row unchanged ([`Mnu::kept`]) rather than becoming a guess,
/// and the reference's file browser rows (1520, 1530), its Paste Special submenu (1298) and its Info
/// panel rows (3127, 7959) name items our menu tree has no command for yet.
pub const REF_ITEMS: &[RefItem] = &[
    RefItem { command: "file.new", number: 10, parent: 2, panel: false },
    RefItem { command: "file.open", number: 20, parent: 2, panel: false },
    RefItem { command: "file.openAs", number: 21, parent: 2, panel: false },
    RefItem { command: "file.openAsSmartObject", number: 22, parent: 2, panel: false },
    RefItem { command: "file.close", number: 31, parent: 2, panel: false },
    RefItem { command: "file.closeAll", number: 37, parent: 2, panel: false },
    RefItem { command: "file.closeOthers", number: 46, parent: 2, panel: false },
    RefItem { command: "edit.toggleLastState", number: 133, parent: 3, panel: false },
    RefItem { command: "layer.new.layer", number: 1099, parent: 41, panel: true },
    RefItem { command: "layer.mergeVisible", number: 1139, parent: 41, panel: true },
    RefItem { command: "edit.fade", number: 1154, parent: 3, panel: false },
];

/// The measured row of one of our commands, if it has one.
pub fn ref_item(command: &str) -> Option<&'static RefItem> {
    REF_ITEMS.iter().find(|item| item.command == command)
}

/// Our command the reference application's number `number` names, if we have one for it.
pub fn ref_command(number: u32) -> Option<&'static str> {
    REF_ITEMS.iter().find(|item| item.number == number).map(|item| item.command)
}

/// The file's colour number of a colour name: [`COLORS`] in order, so `none` is 0 and `gray` is 7.
pub fn color_number(color: &str) -> Option<u32> {
    COLORS.iter().position(|c| *c == color).and_then(|at| u32::try_from(at).ok())
}

/// The colour name of a file's colour number. `None` for a number outside the measured list: 8 to 11
/// belong to the twelve-entry colour list of #14336, which this issue does not read.
pub fn color_name(number: u32) -> Option<&'static str> {
    usize::try_from(number).ok().and_then(|at| COLORS.get(at)).copied()
}

/// What a `.mnu` file holds: the set the rows we know describe, and the entries we cannot name â€” a
/// row with a number of theirs, a `<command>` that is not a static one, and any other element. A
/// rewrite writes those back as they were read, which is what the issue asks for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mnu {
    /// The set the known rows describe.
    pub set: MenuSet,
    /// The entries we could not name, in the order they were read, as XML text.
    pub kept: Vec<String>,
}

/// Read the `.mnu` text of a set: the measured layout of #14450, and nothing else. Bad XML, a root
/// that is not a menu set's, a version other than the measured one, a `hidden` that is not 0 or 1 and
/// a colour number outside [`color_name`]'s list are all errors, never a crash; so is text longer
/// than [`MAX_MNU_BYTES`].
pub fn from_mnu(text: &str) -> Result<Mnu, String> {
    // The reference application's own writer puts a byte-order mark before the declaration.
    let body = text.strip_prefix('\u{feff}').unwrap_or(text);
    if body.len() > MAX_MNU_BYTES {
        return Err(format!("menu set file: larger than {} KiB", MAX_MNU_BYTES / 1024));
    }
    // A set file is one root and one flat list of rows, so the depth bound is not what stops a
    // hostile file: the input bound is, and it is set twice on purpose (here and in the parser).
    let options = craft_xml::Options { doctype: craft_xml::Doctype::Reject, ..craft_xml::Options::local() }.with_limits(craft_xml::Limits {
        max_input: MAX_MNU_BYTES,
        max_depth: 8,
        ..craft_xml::Limits::DEFAULT
    });
    let root = options.parse(body.as_bytes()).map_err(|e| format!("menu set file: {e}"))?;
    if !root.local().ends_with(ROOT_SUFFIX) {
        return Err(format!("not a menu set file: its root element is <{}>", root.local()));
    }
    if root.attr("version") != Some(FILE_VERSION) {
        return Err(format!("menu set file version `{}` is not supported", root.attr("version").unwrap_or("nothing")));
    }
    let mut out = Mnu::default();
    for child in root.elements() {
        // An element we cannot take as a static command row of a known number is kept as it was.
        let unknown = child.local() != "command" || child.attr("kind") != Some("static");
        let number = child.attr("id").and_then(|v| v.parse::<u32>().ok());
        if unknown || number.is_none() {
            out.kept.push(child.to_xml());
            continue;
        }
        let hidden = match child.attr("hidden").unwrap_or("0") {
            "0" => false,
            "1" => true,
            other => return Err(format!("menu set file: hidden=\"{other}\" is not 0 or 1")),
        };
        let color = match child.attr("color").unwrap_or("0") {
            "0" => None,
            other => Some(
                color_name(other.parse::<u32>().map_err(|_| format!("menu set file: colour \"{other}\" is not a number"))?)
                    .ok_or_else(|| format!("menu set file: unknown colour number \"{other}\""))?,
            ),
        };
        match number.and_then(ref_command) {
            None => out.kept.push(child.to_xml()),
            Some(command) => {
                if hidden {
                    out.set.hidden.push(command.to_owned());
                }
                if let Some(name) = color {
                    out.set.colors.insert(command.to_owned(), name.to_owned());
                }
            }
        }
    }
    out.set.hidden.sort();
    out.set.hidden.dedup();
    Ok(out)
}

/// The `.mnu` text of the set `name`: the measured root element, one row per changed item that has a
/// measured number ([`REF_ITEMS`]) â€” hidden items and coloured items, in the file's own numeric order
/// â€” and then `kept`, the entries a file held that PhotoCraft cannot name. An item of ours without a
/// measured number has no row to write, so it lives in the preferences until it gets one.
pub fn to_mnu(name: &str, set: &MenuSet, kept: &[String]) -> String {
    let root = format!("{APP_NAME}{ROOT_SUFFIX}");
    let key = craft_xml::esc_attr(&format!("{FILE_KEY}/{name}={name}"));
    let mut out = String::from(craft_xml::DECL);
    out.push_str(&format!("<{root} version=\"{FILE_VERSION}\" filename=\"{key}\" modified=\"0\">\n"));
    for item in REF_ITEMS {
        let hidden = set.hidden.iter().any(|id| id == item.command);
        // `none` is 0, and 0 is "no colour", so a `none` entry writes no colour of its own.
        let color = set.colors.get(item.command).and_then(|c| color_number(c)).unwrap_or(0);
        if !hidden && color == 0 {
            continue;
        }
        out.push_str(&format!(
            "  <command kind=\"static\" id=\"{}\" parentMenuID=\"{}\" menuType=\"{}\" hidden=\"{}\" color=\"{color}\"/>\n",
            item.number,
            item.parent,
            u8::from(item.panel),
            u8::from(hidden),
        ));
    }
    for entry in kept {
        out.push_str("  ");
        out.push_str(entry);
        out.push('\n');
    }
    out.push_str(&format!("</{root}>\n"));
    out
}

/// The folder `settings` â€” the settings directory â€” keeps its menu sets in.
pub fn folder(settings: &Path) -> PathBuf {
    settings.join(SETS_DIR)
}

/// The file name the set `name` has in that folder.
pub fn file_name(name: &str) -> String {
    format!("{name}.{SETS_EXT}")
}

/// The set name a set file's name gives: the name without its extension, and only for a `.MNU` file.
pub fn set_name_of(file: &Path) -> Option<String> {
    if !file.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(SETS_EXT)) {
        return None;
    }
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    (!stem.is_empty()).then(|| stem.to_owned())
}

/// The file of the set `name` inside the folder `dir`. A set name can come from a Save dialog or a
/// hand-edited preferences file, so a name that cannot be one file's name is an error here rather
/// than a path that leaves the folder.
pub fn path_of(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a set needs a name".into());
    }
    if name.chars().any(|c| matches!(c, '/' | '\\' | ':') || char::is_control(c)) || name == "." || name == ".." {
        return Err(format!("`{name}` cannot name a set file"));
    }
    Ok(dir.join(file_name(name)))
}

/// The text of the file `path`, refused before it is read when it is bigger than [`MAX_MNU_BYTES`].
#[cfg(not(target_arch = "wasm32"))]
fn read_text(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{}: not a file", path.display()));
    }
    if meta.len() > MAX_MNU_BYTES as u64 {
        return Err(format!("{}: larger than {} KiB", path.display(), MAX_MNU_BYTES / 1024));
    }
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The sets in the menu-sets folder `dir`, keyed by set name, with one message per file that could
/// not be read. A folder that is not there holds nothing and says nothing: the first Save makes it.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_sets(dir: &Path) -> (BTreeMap<String, MenuSet>, Vec<String>) {
    let mut sets = BTreeMap::new();
    let mut errors = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (sets, errors),
        Err(e) => return (sets, vec![format!("{}: {e}", dir.display())]),
    };
    let mut files: Vec<(String, PathBuf)> = entries.flatten().filter_map(|e| set_name_of(&e.path()).map(|name| (name, e.path()))).collect();
    files.sort();
    if files.len() > MAX_SET_FILES {
        errors.push(format!("{}: more than {MAX_SET_FILES} set files, the rest were left out", dir.display()));
        files.truncate(MAX_SET_FILES);
    }
    for (name, path) in files {
        match read_text(&path).and_then(|text| from_mnu(&text).map_err(|e| format!("{}: {e}", path.display()))) {
            Ok(file) => {
                sets.insert(name, file.set);
            }
            Err(e) => errors.push(e),
        }
    }
    (sets, errors)
}

/// The sets in the menu-sets folder `dir`; nothing, where there is no file system.
#[cfg(target_arch = "wasm32")]
pub fn read_sets(_: &Path) -> (BTreeMap<String, MenuSet>, Vec<String>) {
    (BTreeMap::new(), Vec::new())
}

/// Write the set `name` to its file in the folder `dir`.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_set(dir: &Path, name: &str, set: &MenuSet) -> Result<(), String> {
    write_set_at(&path_of(dir, name)?, name, set)
}

/// Write `set` to the file `path`, making its folder when it is not there. A file already there is
/// rewritten with its unknown entries kept, which is what "keeps unknown entries when it rewrites a
/// file" means for a Save over an existing set; a file there that does not read is left alone rather
/// than overwritten blind.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_set_at(path: &Path, name: &str, set: &MenuSet) -> Result<(), String> {
    let kept = if path.is_file() { from_mnu(&read_text(path)?)?.kept } else { Vec::new() };
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(path, to_mnu(name, set, &kept)).map_err(|e| format!("{}: {e}", path.display()))
}

/// Write the set `name` to its file in the folder `dir`; no file system, no file.
#[cfg(target_arch = "wasm32")]
pub fn write_set(_: &Path, _: &str, _: &MenuSet) -> Result<(), String> {
    Err(NO_SETTINGS.into())
}

/// Write `set` to the file `path`; no file system, no file.
#[cfg(target_arch = "wasm32")]
pub fn write_set_at(_: &Path, _: &str, _: &MenuSet) -> Result<(), String> {
    Err(NO_SETTINGS.into())
}

/// Take the file of the set `name` in the folder `dir` away.
#[cfg(not(target_arch = "wasm32"))]
pub fn delete_set(dir: &Path, name: &str) -> Result<(), String> {
    delete_set_at(&path_of(dir, name)?)
}

/// Take the set file `path` away; a file that is not there is deleted too.
#[cfg(not(target_arch = "wasm32"))]
pub fn delete_set_at(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Take the file of the set `name` in the folder `dir` away; no file system, nothing to take.
#[cfg(target_arch = "wasm32")]
pub fn delete_set(_: &Path, _: &str) -> Result<(), String> {
    Err(NO_SETTINGS.into())
}

/// Take the set file `path` away; no file system, nothing to take.
#[cfg(target_arch = "wasm32")]
pub fn delete_set_at(_: &Path) -> Result<(), String> {
    Err(NO_SETTINGS.into())
}
