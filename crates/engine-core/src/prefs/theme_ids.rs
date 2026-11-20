//! Theme ids in the preferences: the dark and light slots and the favorites hold the ids of the
//! Craft suite's themes. The theme names PhotoCraft took before ids (`proMedium`, `studioLight`,
//! `Pro (Medium Gray)`, `gnome`, ...) read as their ids, in the slots, the favorites and `ui.set
//! {theme}`. Saved command ids go through [`command_id`].

use craft::theme::ThemeId;
use serde::{Deserialize, Deserializer};

/// How many favorite themes the appearance menu keeps.
pub const MAX_FAVORITES: usize = 64;

/// The suite theme a name PhotoCraft took before theme ids stands for, and whether it is the dark
/// variant. Case, spaces, `_`, `-` and parentheses don't matter: `proMedium`, `Pro (Medium Gray)`
/// and `pro_medium_gray` are one name.
pub fn alias(name: &str) -> Option<(&'static str, bool)> {
    Some(match name.to_ascii_lowercase().replace([' ', '_', '-', '(', ')'], "").as_str() {
        "pro" | "prodark" | "photoshop" | "dark" => ("craft.pro", true),
        "promedium" | "promediumgray" | "medium" | "mediumgray" => ("craft.artcraft", true),
        "prolightgray" | "lightgray" | "mediumlight" => ("craft.pro-light-gray", false),
        "prolight" | "prolightest" | "lightest" | "light" => ("craft.pro", false),
        "studio" | "studiodark" => ("craft.studio", true),
        "studiolight" => ("craft.studio", false),
        "classic" | "win2000" | "retro" => ("craft.classic", false),
        "adwaita" | "adwaitalight" | "gnome" | "gnomelight" => ("craft.adwaita", false),
        "adwaitadark" | "gnomedark" => ("craft.adwaita", true),
        "solarizeddark" | "solarized" => ("craft.solarized-dark", true),
        "breezelight" | "breeze" | "kde" | "kdelight" | "plasma" | "plasmalight" => ("craft.breeze", false),
        "breezedark" | "kdedark" | "plasmadark" => ("craft.breeze", true),
        "softdark" => ("craft.soft", true),
        "softlight" => ("craft.soft", false),
        _ => return None,
    })
}

/// The id a saved value names: an [`alias`]'s id, else the value itself.
pub fn id_of(value: &str) -> String {
    alias(value).map_or_else(|| value.to_owned(), |(id, _)| id.to_owned())
}

/// The theme `value` names, and its mode when an alias gave one: an [`alias`], else a theme id.
pub fn parse(value: &str) -> Option<(ThemeId, Option<bool>)> {
    match alias(value) {
        Some((id, dark)) => ThemeId::parse(id).ok().map(|id| (id, Some(dark))),
        None => ThemeId::parse(value).ok().map(|id| (id, None)),
    }
}

/// The command id a saved one names: the four Window › Workspace commands that moved to the
/// framework's workspace set read as their new ids, so a binding saved for one still matches a
/// command. The match is on the whole id: other `window.workspace.*` ids (`window.workspace.motion`,
/// a saved workspace's entry) read as saved. The old ids are never written.
pub fn command_id(id: &str) -> String {
    match id {
        "window.workspace.resetWorkspace" => "workspace.reset",
        "window.workspace.newWorkspace" => "workspace.new",
        "window.workspace.deleteWorkspace" => "workspace.delete",
        "window.workspace.lockWorkspace" => "workspace.lock",
        other => other,
    }
    .to_owned()
}

pub(super) fn theme<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    String::deserialize(d).map(|s| id_of(&s))
}

pub(super) fn themes<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Vec::<String>::deserialize(d).map(|v| v.iter().map(|s| id_of(s)).collect())
}

// PLAN: the sheet removes this adapter (L-27b), but `shortcut_sets.rs:32,35` (no set owns it) still
// reads it for a saved set's keys; it stays until that file remaps its keys in `parse_prefs_json`.
/// A map keyed by command id, with [`command_id`] applied to each key.
pub(crate) fn command_keys<'de, D: Deserializer<'de>, V: Deserialize<'de>>(d: D) -> Result<std::collections::BTreeMap<String, V>, D::Error> {
    std::collections::BTreeMap::<String, V>::deserialize(d).map(|map| map.into_iter().map(|(id, v)| (command_id(&id), v)).collect())
}
