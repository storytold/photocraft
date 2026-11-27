//! The commands PhotoCraft binds keys to, as craft-keymap's catalog, and the conversion between a
//! keymap's overrides and the preferences' `shortcuts` / `alternateShortcuts` maps.
//!
//! It lives in the engine because the engine's headless `edit.keyboardShortcuts {"importKys": …}`
//! plans a `.kys` import over the same catalog the Keyboard Shortcuts dialog and the runtime use,
//! and the engine cannot depend on the UI. The UI adds the menu paths and the live tool list on
//! top of [`command_defs`] (`shortcut_dispatch::command_rows`).
//!
//! The preferences keep a command's main key in `shortcuts` (`""` for none) and its further keys
//! in `alternateShortcuts`; a keymap's [`Overrides`] keep the whole list of keys per command. An
//! entry equal to the command's defaults is not an override on either side.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use craft::keymap::{Binding, CommandDef, CommandKind, CommandRef, Keymap, Overrides, Platform, Scope, Scopes, reserved};

use crate::prefs::{BindKind, Preferences, bindable_items, default_alternates, normalize_shortcut};
use crate::shortcut_sets::ShortcutSet;

/// A command's default keys in our notation: the main key, then the further keys (the main key is
/// never repeated among them).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Defaults {
    main: Option<String>,
    alternates: Vec<String>,
}

impl Defaults {
    /// The keys in the order an override list holds them.
    fn keys(&self) -> Vec<String> {
        self.main.iter().chain(&self.alternates).cloned().collect()
    }
}

/// The default keys of every bindable id.
fn defaults() -> HashMap<&'static str, Defaults> {
    bindable_items()
        .map(|(id, _, _, default, _)| {
            let main = default.and_then(normalize_shortcut);
            let alternates = alternates_without(&main, default_alternates(id).filter_map(normalize_shortcut));
            (id, Defaults { main, alternates })
        })
        .collect()
}

/// `keys` without repeats and without `main`, which the command already answers.
fn alternates_without(main: &Option<String>, keys: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for key in keys {
        if main.as_ref() != Some(&key) && !out.contains(&key) {
            out.push(key);
        }
    }
    out
}

fn parse_binding(text: &str) -> Option<Binding> {
    Binding::parse(&normalize_shortcut(text)?).ok()
}

/// Every bindable engine and shell command, tool and held temporary tool, with its label, menu
/// path and default keys. All are in the app scope: the engine builds no panel scopes. A tool's
/// group is its default key, so the tools that share a letter cycle on it as Photoshop's tool
/// groups do.
pub fn command_defs() -> Vec<CommandDef> {
    let defaults = defaults();
    let mut seen = HashSet::new();
    bindable_items()
        .filter(|item| seen.insert(item.0))
        .map(|(id, label, menu, default, kind)| {
            let mut def = CommandDef::new(CommandRef::new(id), label, Scope::app());
            let path = || menu.iter().map(|s| (*s).to_owned()).chain(std::iter::once(label.to_owned())).collect::<Vec<_>>();
            match kind {
                BindKind::Command if !menu.is_empty() => def = def.with_path(path()),
                BindKind::Command => {}
                // Foreign sets file a tool under "Tools" although PhotoCraft's menus do not list it.
                BindKind::Tool => {
                    def = def.with_kind(CommandKind::Tool { group: default.map(str::to_owned) }).with_import_path(path());
                }
                BindKind::Hold => def = def.with_kind(CommandKind::Momentary).with_import_path(path()),
            }
            if let Some(keys) = defaults.get(id) {
                for key in keys.keys().iter().filter_map(|k| parse_binding(k)) {
                    def = def.with_default(key);
                }
            }
            def
        })
        .collect()
}

/// The keymap of `prefs` on `platform`: [`command_defs`] with the system's reserved chords and the
/// preferences' bindings as overrides. The headless import passes `Platform::current()`; the UI
/// passes the platform of its runtime.
pub fn keymap(prefs: &Preferences, platform: Platform) -> Keymap {
    Keymap::new(command_defs(), Scopes::new(), platform).with_reserved(reserved::system(platform)).with_overrides(prefs_overrides(prefs))
}

/// The preferences' bindings as keymap overrides: per command the main key and the further keys in
/// effect. Entries for ids outside the catalog (an action's function key) are kept as they are.
pub fn prefs_overrides(prefs: &Preferences) -> Overrides {
    overrides_of(&prefs.shortcuts, &prefs.alternate_shortcuts)
}

/// A saved shortcut set as keymap overrides, as [`prefs_overrides`] reads the preferences.
pub fn set_overrides(set: &ShortcutSet) -> Overrides {
    overrides_of(&set.shortcuts, &set.alternate_shortcuts)
}

fn overrides_of(shortcuts: &BTreeMap<String, String>, alternates: &BTreeMap<String, Vec<String>>) -> Overrides {
    let defaults = defaults();
    let mut out = Overrides::new();
    let ids: BTreeSet<&String> = shortcuts.keys().chain(alternates.keys()).collect();
    for id in ids {
        let default = defaults.get(id.as_str()).cloned().unwrap_or_default();
        let main = match shortcuts.get(id) {
            Some(key) => normalize_shortcut(key),
            None => default.main.clone(),
        };
        let further = match alternates.get(id) {
            Some(list) => alternates_without(&main, list.iter().filter_map(|k| normalize_shortcut(k))),
            None => alternates_without(&main, default.alternates.iter().cloned()),
        };
        let keys: Vec<String> = main.into_iter().chain(further).collect();
        if keys != default.keys() {
            out.insert(id.clone(), keys);
        }
    }
    out
}

/// Keymap overrides as the preferences keep them: (`shortcuts`, `alternateShortcuts`). A command
/// whose main key is its default has no `shortcuts` entry, and one whose further keys are the
/// defaults has no `alternateShortcuts` entry. A key text the preferences cannot spell (a double
/// press, a scoped binding) is left out.
pub fn overrides_to_prefs(overrides: &Overrides) -> (BTreeMap<String, String>, BTreeMap<String, Vec<String>>) {
    let defaults = defaults();
    let (mut set, mut alternates) = (BTreeMap::new(), BTreeMap::new());
    for (id, texts) in overrides.iter() {
        let default = defaults.get(id).cloned().unwrap_or_default();
        let keys: Vec<String> = texts.iter().filter_map(|t| normalize_shortcut(t)).collect();
        let main = keys.first().cloned();
        if main != default.main {
            set.insert(id.to_owned(), main.clone().unwrap_or_default());
        }
        let further = alternates_without(&main, keys.iter().skip(1).cloned());
        if further != alternates_without(&main, default.alternates.iter().cloned()) {
            alternates.insert(id.to_owned(), further);
        }
    }
    (set, alternates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_and_overrides_convert_both_ways() {
        crate::install().unwrap();
        let mut prefs = Preferences::default();
        prefs.shortcuts.insert("edit.copy".into(), "F9".into());
        prefs.shortcuts.insert("edit.cut".into(), String::new());
        prefs.alternate_shortcuts.insert("edit.paste".into(), vec!["Cmd+Shift+F4".into()]);
        prefs.shortcuts.insert("actions.play:Sepia".into(), "F5".into());
        let overrides = prefs_overrides(&prefs);
        // Copy keeps its default alternate F3 beside the new main key; Cut has none; Paste keeps
        // its main key and replaces its alternates.
        assert_eq!(overrides.get("edit.copy"), Some(&["F9".to_owned(), "F3".to_owned()][..]));
        assert_eq!(overrides.get("edit.cut"), Some(&["F2".to_owned()][..]));
        assert_eq!(overrides.get("actions.play:Sepia"), Some(&["F5".to_owned()][..]));
        let (set, alternates) = overrides_to_prefs(&overrides);
        assert_eq!(set.get("edit.copy").map(String::as_str), Some("F9"));
        assert_eq!(set.get("edit.cut").map(String::as_str), Some("F2"));
        assert!(!alternates.contains_key("edit.copy"), "F3 is Copy's default alternate");
        assert_eq!(alternates.get("edit.paste"), Some(&vec!["Cmd+Shift+F4".to_owned()]));
        assert!(!set.contains_key("edit.paste"), "Paste keeps its default main key");
    }

    #[test]
    fn a_binding_equal_to_the_defaults_is_no_override() {
        crate::install().unwrap();
        let mut prefs = Preferences::default();
        prefs.shortcuts.insert("edit.copy".into(), "Cmd+C".into());
        assert!(prefs_overrides(&prefs).is_empty());
    }

    #[test]
    fn the_catalog_holds_commands_tools_and_held_keys_once() {
        crate::install().unwrap();
        let defs = command_defs();
        let mut ids: Vec<String> = defs.iter().map(|d| d.command.key()).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "an id twice");
        let tool = defs.iter().find(|d| d.command.id == "tools.select.move").expect("the Move tool");
        assert!(matches!(tool.kind, CommandKind::Tool { .. }));
        assert!(defs.iter().find(|d| d.command.id == "tools.temporary.hand").is_some_and(|d| d.kind == CommandKind::Momentary));
    }
}
