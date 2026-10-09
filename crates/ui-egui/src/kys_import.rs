//! Photoshop keyboard shortcut sets (`.kys`, Edit › Keyboard Shortcuts › Save Set) imported
//! into Edit › Keyboard Shortcuts (#1163): the dialog's Import Photoshop Shortcuts… button,
//! File › Open and a drop on the window.
//!
//! A `.kys` is XML: `<command kind="static|dynamic" name="Gaussian Blur...">` rows, each with
//! one or more `<shortcut>Control+Opt+Cmd+G</shortcut>`, then the `<tool>` keys. Photoshop
//! names the command by its menu label alone, so rows are matched to our commands by label.
//! Where one label sits in several menus (Levels… under Image › Adjustments and Layer › New
//! Adjustment Layer; All under Select, Edit › Purge, View › Show), the command whose default is
//! one of the imported keys wins, then the one that has a default at all, then the first in
//! Photoshop's menu order. Ours are one per command, so a row's first shortcut is the one
//! kept, unless one of its keys is already the command's default (Zoom In lists `Cmd++`
//! before `Cmd+=`). Tool keys are not imported: the tool letters are fixed here.
//!
//! The import fills the open dialog's overrides, so OK applies them like hand-made changes
//! (a shortcut moved to a command is taken from its old owner) and Cancel discards them.

use std::collections::{BTreeMap, HashMap};

use serde_json::{Value, json};

use crate::PhotocraftApp;

/// One `<command>` row: Photoshop's label and its shortcuts, first one primary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KysCommand {
    pub name: String,
    pub shortcuts: Vec<String>,
}

/// The parsed set: its name, its command rows and how many tools carry a key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KysSet {
    /// The set's name as Photoshop shows it (`texcuts`), from the root's `filename` attribute,
    /// which a saved set spells `$$$/FileName/Presets/KeyboardCustomization/texcuts=texcuts`.
    pub name: Option<String>,
    pub commands: Vec<KysCommand>,
    pub tool_keys: usize,
}

/// Parse a `.kys` file (or Photoshop's live `Keyboard Shortcuts.psp`, the same XML). Rows
/// without a name or a shortcut are skipped.
pub fn parse(text: &str) -> Result<KysSet, String> {
    let text = text.trim_start_matches('\u{feff}');
    let doc = roxmltree::Document::parse(text).map_err(|e| format!("not a keyboard shortcut set (.kys): {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "photoshop-keyboard-shortcuts" {
        return Err("not a keyboard shortcut set (.kys): no <photoshop-keyboard-shortcuts> root".into());
    }
    let name = root.attribute("filename").map(|f| f.rsplit(['=', '/']).next().unwrap_or(f).trim()).filter(|n| !n.is_empty()).map(str::to_string);
    let mut set = KysSet { name, ..Default::default() };
    for node in root.children().filter(|n| n.is_element()) {
        match node.tag_name().name() {
            "command" => {
                let Some(name) = node.attribute("name").map(str::trim).filter(|n| !n.is_empty()) else { continue };
                let shortcuts: Vec<String> = node
                    .children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "shortcut")
                    .filter_map(|n| n.text().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string))
                    .collect();
                if !shortcuts.is_empty() {
                    set.commands.push(KysCommand { name: name.to_string(), shortcuts });
                }
            }
            "tool" if node.text().is_some_and(|t| !t.trim().is_empty()) => set.tool_keys += 1,
            _ => {}
        }
    }
    Ok(set)
}

/// What an import would change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Command id → the normalised shortcut from the file, for every matched row.
    pub set: BTreeMap<String, String>,
    /// Photoshop names with no command here (scripts, Bridge, the macOS application menu…).
    pub unknown: Vec<String>,
    /// Rows whose shortcut could not be read (`unknown` does not list them).
    pub unreadable: Vec<String>,
    /// Second and later shortcuts of a row, which have no place here.
    pub alternates: usize,
}

/// Photoshop's label and ours compared loosely: case, a trailing ellipsis (`...` or `…`) and
/// surrounding space do not count.
fn key(label: &str) -> String {
    let l = label.trim().replace("...", "…");
    l.trim_end_matches('…').trim().to_lowercase()
}

/// Match the rows to our commands (see the module notes for the tie-breaks).
pub fn plan(app: &PhotocraftApp, set: &KysSet) -> Plan {
    // Candidates by label, in Photoshop's menu order, each id once; then engine commands the
    // menus list under another label (Preferences › General… is the Settings… item).
    let mut by_label: HashMap<String, Vec<(String, Option<String>)>> = HashMap::new();
    let items = crate::menus::menu_items(app);
    let menu = items.iter().filter(|i| i.label != "---" && crate::menus::is_live(&i.id)).map(|i| (i.id.clone(), i.label.clone()));
    let specs = photocraft_engine::command_specs().iter().filter(|c| !c.menu.is_empty()).map(|c| (c.id.to_string(), c.label.to_string()));
    for (id, label) in menu.chain(specs) {
        let entry = by_label.entry(key(&label)).or_default();
        if !entry.iter().any(|(i, _)| *i == id) {
            let default = crate::shortcuts::default_shortcut(&id);
            entry.push((id, default));
        }
    }
    let mut plan = Plan::default();
    for row in &set.commands {
        let Some(candidates) = by_label.get(&key(&row.name)) else {
            plan.unknown.push(row.name.clone());
            continue;
        };
        let Some(sc) = row.shortcuts.first().and_then(|s| photocraft_engine::prefs::normalize_shortcut(s)) else {
            plan.unreadable.push(row.name.clone());
            continue;
        };
        plan.alternates += row.shortcuts.len().saturating_sub(1);
        let imported: Vec<String> = row.shortcuts.iter().filter_map(|s| photocraft_engine::prefs::normalize_shortcut(s)).collect();
        let pick = candidates
            .iter()
            .find(|(_, d)| d.as_deref().and_then(photocraft_engine::prefs::normalize_shortcut).is_some_and(|d| imported.contains(&d)))
            .or_else(|| candidates.iter().find(|(_, d)| d.is_some()))
            .or_else(|| candidates.first());
        if let Some((id, default)) = pick {
            // A row that lists the command's own default among its keys keeps the default
            // (Zoom In is "Cmd++" then "Cmd+=" in Photoshop; ours is Cmd+=).
            let default = default.as_deref().and_then(photocraft_engine::prefs::normalize_shortcut);
            let sc = default.filter(|d| imported.contains(d)).unwrap_or(sc);
            plan.set.insert(id.clone(), sc);
        }
    }
    plan
}

/// Put the plan into the open Keyboard Shortcuts dialog (opening one when none is), as
/// overrides the OK button applies: a key equal to the command's default clears its override.
/// Returns the dialog id and the plan as JSON.
pub fn import_text(app: &mut PhotocraftApp, file_name: &str, text: &str) -> Result<Value, String> {
    let set = parse(text).map_err(|e| format!("{file_name}: {e}"))?;
    let plan = plan(app, &set);
    let id = match app.ui.dialogs.iter().find(|d| d.fields.get("__prefsui").and_then(Value::as_str) == Some("shortcuts")) {
        Some(d) => d.id,
        None => crate::prefs_ui::open_shortcuts(app, 0),
    };
    let f = &mut app.ui.dialog_mut(id).ok_or("the Keyboard Shortcuts dialog is gone")?.fields;
    let mut overrides: BTreeMap<String, String> = f.get("overrides").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    for (cmd, sc) in &plan.set {
        let default = crate::shortcuts::default_shortcut(cmd).and_then(|d| photocraft_engine::prefs::normalize_shortcut(&d));
        if default.as_deref() == Some(sc.as_str()) {
            overrides.remove(cmd);
        } else {
            overrides.insert(cmd.clone(), sc.clone());
        }
    }
    let shown = std::path::Path::new(file_name).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| file_name.to_string());
    let n = plan.set.len().to_string();
    let mut message = crate::i18n::fmt(tl!("Imported {n} shortcuts from {file}. Click OK to keep them."), &[("n", &n), ("file", &shown)]);
    if !plan.unknown.is_empty() {
        let u = plan.unknown.len().to_string();
        message.push(' ');
        message.push_str(&crate::i18n::fmt(tl!("{n} commands are not in PhotoCraft."), &[("n", &u)]));
    }
    f.insert("tab".into(), json!(0));
    f.insert("overrides".into(), json!(overrides));
    f.insert("selected".into(), json!(""));
    f.insert("capture".into(), json!(false));
    f.insert("message".into(), json!(message));
    Ok(json!({
        "dialog": id,
        "name": set.name,
        "imported": plan.set.len(),
        "set": plan.set,
        "unknown": plan.unknown,
        "unreadable": plan.unreadable,
        "alternates": plan.alternates,
        "toolKeys": set.tool_keys,
    }))
}

/// A `.kys` opened like a document (File › Open, a drop): its bytes are text.
pub fn open_bytes(app: &mut PhotocraftApp, name: &str, bytes: &[u8]) -> Result<Value, String> {
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| format!("{name}: not a keyboard shortcut set (.kys): not UTF-8"))?;
    import_text(app, name, &text)
}

/// The dialog's Import Photoshop Shortcuts… button: ask for the file, then fill the dialog.
pub fn import_dialog(app: &mut PhotocraftApp) -> Result<Value, String> {
    app.pick_file_bytes(|app, name, bytes| open_bytes(app, &name, &bytes))
}

/// Preferences › `dialogs` key that records the one-time import (its value names the source).
pub const IMPORTED_PREF: &str = "photoshopShortcuts";

/// First launch on a machine with Photoshop: its live shortcut set ([`crate::Services::photoshop_shortcuts`])
/// is applied once, so Photoshop's hands work here from the start. Someone who already changed
/// a shortcut keeps their own set. Either way the preference remembers it, so a later launch
/// never asks again; Edit › Keyboard Shortcuts › Reset All to Defaults undoes the import.
pub fn auto_import(app: &mut PhotocraftApp) {
    if app.session.prefs().dialogs.contains_key(IMPORTED_PREF) {
        return;
    }
    let Some((source, text)) = app.services.photoshop_shortcuts.as_mut().and_then(|f| f()) else { return };
    let outcome = if app.session.prefs().shortcuts.is_empty() {
        import_text(app, &source, &text).and_then(|r| {
            let id = r["dialog"].as_u64().ok_or("no dialog")?;
            crate::dialogs::confirm(app, id)?;
            Ok(Some(r))
        })
    } else {
        Ok(None)
    };
    app.session.prefs.edit(|p| {
        p.dialogs.insert(IMPORTED_PREF.into(), json!({"source": source}));
    });
    match outcome {
        Ok(Some(r)) => {
            let changed = app.session.prefs().shortcuts.len();
            if changed == 0 {
                return; // The set is Photoshop's defaults, which are ours too.
            }
            let set = r["name"]
                .as_str()
                .map_or_else(|| std::path::Path::new(&source).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(), str::to_string);
            let id = crate::notices::post(
                app,
                "Keyboard shortcuts imported",
                vec!["Your keyboard shortcut set {set} is in use here: {n} shortcuts differ from the defaults. Edit › Keyboard Shortcuts shows them; Reset All to Defaults undoes this.".to_owned()],
                false,
                None,
            );
            if let Some(n) = app.ui.notices.iter_mut().find(|n| n.id == id) {
                n.args = vec![("set".to_owned(), set), ("n".to_owned(), changed.to_string())];
            }
        }
        Ok(None) => {}
        Err(e) => {
            app.ui.status = format!("The keyboard shortcut set was not imported: {e}");
            app.ui.status_error = true;
        }
    }
}

#[cfg(test)]
#[path = "kys_import_tests.rs"]
mod tests;
