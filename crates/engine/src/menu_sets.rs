//! Edit › Menus › Set: named menu sets, as in Photoshop. The customization in use stays in
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
//! The reference application keeps sets as `.mnu` files in its settings folder. Reading and
//! writing that format is the next step of #14450, so this module keeps sets in the preferences
//! and gives no file paths yet.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::prefs::{MenuCustomization, Preferences};

/// The built-in set's name: the menus as PhotoCraft ships them. The shortcut sets' built-in set
/// has the same name, and both dropdowns start there.
pub const DEFAULTS_NAME: &str = crate::shortcut_sets::DEFAULTS_NAME;
/// Saved sets are small; the cap keeps a hostile settings file or call bounded.
pub const MAX_SETS: usize = crate::shortcut_sets::MAX_SETS;
/// The colours Edit › Menus gives an item; `none` is no colour. The reference application's
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
