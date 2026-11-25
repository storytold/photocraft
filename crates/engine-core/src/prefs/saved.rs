//! The settings file's document: the preferences together with Edit › Color Settings and the
//! preset libraries ([`SavedPrefs`]), how it is read, and the session methods that move it in and
//! out.

use crate::color_cmds::ColorExt;
use crate::presets::PresetsExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    InvalidPref, PREFS_VERSION, Preferences, SECTIONS, SMALL_ADDRESS_SPACE, accepted_effect_cache_mb, migrate_integrations, read_lenient, set_path,
    strip_nulls, theme_ids,
};
use crate::Session;

/// Everything PhotoCraft keeps in its settings file: the preferences at the top level (their
/// sections as `[general]`, `[interface]`, ...), Edit › Color Settings as `colorSettings` and the
/// preset libraries as `presets`. The type the settings store holds
/// (`craft::settings::SettingsStore<SavedPrefs>`); [`SavedPrefsExt::saved_prefs`] and
/// [`SavedPrefsExt::set_saved_prefs`] move it in and out of the session.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SavedPrefs {
    /// The preferences.
    #[serde(flatten)]
    pub prefs: Preferences,
    /// Edit › Color Settings.
    pub color_settings: crate::color_cmds::ColorSettings,
    /// The preset libraries, as [`PresetState::to_json`](crate::presets::PresetState::to_json) writes them.
    pub presets: Value,
}

/// Written through [`strip_nulls`]: TOML has no null, and the store refuses a document holding one
/// at any depth. A null in these files always means "not set", which a missing key reads as too.
impl Serialize for SavedPrefs {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut v = self.prefs.to_json();
        if let Value::Object(m) = &mut v {
            // Match inks by RGB when SettingsStore merges concurrent saves.
            m.insert("toolColors".into(), super::tool_colors::encode(&self.prefs.tool_colors));
            m.insert("colorSettings".into(), serde_json::to_value(&self.color_settings).map_err(serde::ser::Error::custom)?);
            m.insert("presets".into(), self.presets.clone());
        }
        strip_nulls(&mut v);
        v.serialize(serializer)
    }
}

impl craft::settings::Settings for SavedPrefs {
    const APP_ID: &'static str = "photocraft";
    const SCHEMA_VERSION: u32 = 1;
}

/// Reads a `preferences.json` written by [`SavedPrefsExt::prefs_to_json`] (the file before
/// `config.toml`), with its migrations: the old shift-key default, the single-theme preference,
/// the `sr` catalog and the renamed command ids. Missing keys keep their defaults and unknown keys
/// are ignored, so files from older and newer versions load. A value of the wrong type (a
/// hand-edited `"recentFileCount": "ten"`) keeps its default and everything else still loads
/// (#6611); the returned list names those values. Only text that isn't a JSON object fails as a
/// whole.
pub fn parse_prefs_json(s: &str) -> std::result::Result<(SavedPrefs, Vec<InvalidPref>), String> {
    let mut v: Value = serde_json::from_str(s).map_err(|e| format!("preferences: {e}"))?;
    if !v.is_object() {
        return Err("preferences: expected a JSON object".into());
    }
    // Files before version 2 hold Use Shift Key for Tool Switch off because that was the default
    // when they were written: every key is saved, chosen or not. Like Photoshop it is on by
    // default now (#3528), so those files take the new default (#5647: a tool letter pressed again
    // kept cycling its group).
    if v.get("version").and_then(Value::as_u64).unwrap_or(0) < 2
        && let Some(tools) = v.get_mut("tools").and_then(Value::as_object_mut)
        && tools.get("useShiftKeyForToolSwitch") == Some(&json!(false))
    {
        tools.remove("useShiftKeyForToolSwitch");
    }
    // Saved preferences before appearance modes had one concrete theme. Preserve its appearance
    // instead of silently switching established users to Auto.
    if let Some(interface) = v.get_mut("interface").and_then(Value::as_object_mut)
        && !interface.contains_key("appearanceMode")
        && let Some((id, dark)) = interface.get("theme").and_then(Value::as_str).and_then(theme_ids::alias)
    {
        let (mode, slot) = if dark { ("dark", "darkTheme") } else { ("light", "lightTheme") };
        interface.insert("appearanceMode".into(), json!(mode));
        // A partially migrated file may already contain a newer theme selection (#5139).
        interface.entry(slot).or_insert_with(|| json!(id));
    }
    // The Serbian interface catalog was `sr`; since the catalogs moved to craft-i18n it is
    // `sr-latn`, the same Latin text.
    if let Some(language) = v.get_mut("interface").and_then(|interface| interface.get_mut("language"))
        && language.as_str().is_some_and(|code| code.eq_ignore_ascii_case("sr"))
    {
        *language = json!("sr-latn");
    }
    if let Some(integrations) = v.get_mut("integrations") {
        migrate_integrations(integrations);
    }
    remap_command_ids(&mut v);
    let color = v.as_object_mut().and_then(|m| m.remove("colorSettings"));
    let presets = v.as_object_mut().and_then(|m| m.remove("presets")).unwrap_or(Value::Null);
    let mut invalid = Vec::new();
    let prefs: Preferences = read_lenient(v, &[], &mut invalid);
    let color_settings = color.map(|c| read_lenient(c, &["colorSettings"], &mut invalid)).unwrap_or_default();
    Ok((SavedPrefs { prefs, color_settings, presets }, invalid))
}

/// [`parse_prefs_json`] as the settings store's legacy import calls it
/// (`craft::settings::LegacyFile::parse`): the file's bytes in, the values reset as the store's
/// section errors out.
pub fn parse_legacy_prefs(bytes: &[u8]) -> std::result::Result<(SavedPrefs, Vec<craft::settings::SectionError>), String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("preferences: {e}"))?;
    let (saved, invalid) = parse_prefs_json(text)?;
    Ok((saved, invalid.iter().map(reset_value).collect()))
}

/// The file the preferences lived in before `config.toml`, in the same folder. The settings store imports it while
/// there is no `config.toml` and never writes or removes it, so an older build still finds it.
pub const LEGACY_PREFS_FILE: &str = "preferences.json";

/// The settings store's options for the folder `dir` (in memory without one): `config.toml` there, importing
/// [`LEGACY_PREFS_FILE`] through [`parse_legacy_prefs`] while there is none.
#[cfg(not(target_arch = "wasm32"))]
pub fn store_options(dir: Option<&std::path::Path>) -> craft::settings::OpenOptions<SavedPrefs> {
    let location = dir.map_or(craft::settings::Location::Memory, |dir| craft::settings::Location::Dir(dir.to_path_buf()));
    let legacy = dir.map(|dir| craft::settings::LegacyFile { path: dir.join(LEGACY_PREFS_FILE), parse: parse_legacy_prefs }).into_iter().collect();
    craft::settings::OpenOptions::new(location).legacy(legacy)
}

/// The preferences saved in `dir` as the app's settings store loads them, read without opening the store
/// (`craft::settings::SettingsStore::read`, which writes nothing): the launch reads before the window opens and
/// `photocraft-cli mcp`. Defaults without a folder.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_saved(dir: Option<&std::path::Path>) -> SavedPrefs {
    craft::settings::SettingsStore::read(&store_options(dir)).0
}

/// A value the load reset, in the form the settings store reports it: the dotted key path as the
/// section, the value the file held as the message.
pub fn reset_value(bad: &InvalidPref) -> craft::settings::SectionError {
    craft::settings::SectionError { section: bad.path(), message: format!("unreadable value {}", bad.value) }
}

/// Applies [`theme_ids::command_id`] once to the saved command ids: the keys of `shortcuts`,
/// `alternateShortcuts` and `menus.colors`, and the items of `menus.hidden`.
fn remap_command_ids(v: &mut Value) {
    fn remap_keys(map: Option<&mut Value>) {
        if let Some(Value::Object(m)) = map {
            *m = std::mem::take(m).into_iter().map(|(id, item)| (theme_ids::command_id(&id), item)).collect();
        }
    }
    remap_keys(v.get_mut("shortcuts"));
    remap_keys(v.get_mut("alternateShortcuts"));
    if let Some(menus) = v.get_mut("menus") {
        remap_keys(menus.get_mut("colors"));
        if let Some(Value::Array(hidden)) = menus.get_mut("hidden") {
            for id in hidden {
                if let Value::String(s) = id {
                    *s = theme_ids::command_id(s);
                }
            }
        }
    }
}

/// The settings file on a session: preferences, colour settings and presets together.
pub trait SavedPrefsExt {
    /// Everything persisted as one JSON document: the preferences plus `colorSettings`.
    fn prefs_to_json(&self) -> String;
    /// [`Self::prefs_to_json`] as a JSON tree (frontends merge it with what storage holds).
    fn prefs_value(&self) -> Value;
    /// Restore preferences saved by [`Self::prefs_to_json`]: [`parse_prefs_json`], then
    /// [`Self::set_saved_prefs`]. The returned list names the values both left at their
    /// defaults. Only text that isn't a JSON object fails as a whole.
    fn load_prefs_json(&mut self, s: &str) -> std::result::Result<Vec<InvalidPref>, String>;
    /// Everything the settings file holds, as the session has it now.
    fn saved_prefs(&self) -> SavedPrefs;
    /// Takes `saved` as the session's preferences, colour settings and presets, from either source
    /// (the settings file, or an older `preferences.json` through [`parse_prefs_json`]), and
    /// applies the engine-side ones. Values that read but that this build no longer accepts are
    /// put back to their defaults (or the nearest accepted value) and returned.
    fn set_saved_prefs(&mut self, saved: SavedPrefs) -> Vec<InvalidPref>;
}

impl SavedPrefsExt for Session {
    fn prefs_to_json(&self) -> String {
        serde_json::to_string_pretty(&self.prefs_value()).unwrap_or_default()
    }

    fn prefs_value(&self) -> Value {
        let mut v = self.prefs().to_json();
        if let Value::Object(m) = &mut v {
            m.insert("colorSettings".into(), serde_json::to_value(&self.color().settings).unwrap_or(Value::Null));
            m.insert("version".into(), json!(PREFS_VERSION));
            m.insert("presets".into(), self.presets().to_json(self));
        }
        v
    }

    fn load_prefs_json(&mut self, s: &str) -> std::result::Result<Vec<InvalidPref>, String> {
        let (saved, mut invalid) = parse_prefs_json(s)?;
        invalid.extend(self.set_saved_prefs(saved));
        Ok(invalid)
    }

    fn saved_prefs(&self) -> SavedPrefs {
        SavedPrefs { prefs: self.prefs().clone(), color_settings: self.color().settings.clone(), presets: self.presets().to_json(self) }
    }

    fn set_saved_prefs(&mut self, saved: SavedPrefs) -> Vec<InvalidPref> {
        let SavedPrefs { mut prefs, color_settings, presets } = saved;
        let mut invalid = Vec::new();
        // An Effect Cache the field no longer accepts (above the 32-bit cap, saved before the cap
        // or by a 64-bit build; a hand-edited 0) loads as the nearest value it does. Otherwise
        // Apply from another Preferences tab, which validates every value, would reject it.
        prefs.performance.effect_cache_mb = accepted_effect_cache_mb(prefs.performance.effect_cache_mb, SMALL_ADDRESS_SPACE);
        self.color_mut().settings = color_settings;
        // A number outside 0..MAX reads, but `validate_settings` would then refuse every later
        // Color Settings edit: reset it like an unreadable value.
        let gain = self.color().settings.spot_dot_gain;
        if !(gain.is_finite() && (0.0..=photocraft_compose::multichannel::MAX_SPOT_DOT_GAIN).contains(&gain)) {
            invalid.push(InvalidPref { keys: vec!["colorSettings".into(), "spotDotGain".into()], value: json!(gain) });
            self.color_mut().settings.spot_dot_gain = photocraft_compose::multichannel::SPOT_DOT_GAIN;
        }
        photocraft_compose::psblend::set_text_gamma(self.color().settings.blend_text_gamma);
        photocraft_compose::multichannel::set_spot_dot_gain(self.color().settings.spot_dot_gain);
        self.prefs.edit(|p| *p = prefs);
        self.load_presets_json(presets);
        self.apply_prefs();
        invalid
    }
}

/// Full JSON view used by `prefs.get`: preferences plus `colorSettings`.
pub(super) fn prefs_view(s: &Session) -> Value {
    let mut v = s.prefs().to_json();
    if let Value::Object(m) = &mut v {
        m.insert("colorSettings".into(), serde_json::to_value(&s.color().settings).unwrap_or(Value::Null));
    }
    // Old clients read the single-theme preference: the theme the appearance mode shows.
    if let Some(interface) = v.get_mut("interface").and_then(Value::as_object_mut) {
        interface.insert("theme".into(), json!(s.prefs().interface.shown_theme()));
    }
    v
}

/// Set one dotted path, routing `colorSettings.*` to the colour state.
/// Batches defer colour validation until their complete candidate is staged. Returns
/// whether colour settings were touched, including whole-section/root-object updates.
pub(super) fn set_pref(s: &mut Session, path: &str, value: Value, validate_color: bool) -> std::result::Result<bool, String> {
    // The section itself or a path inside it: `colorSettingsworkingRgb`, `colorSettings.` and
    // `colorSettings..workingRgb` are none, and the authorization policy doesn't treat them as one.
    if let Some(rest) =
        path.strip_prefix("colorSettings").filter(|r| r.is_empty() || r.strip_prefix('.').is_some_and(|c| c.split('.').all(|part| !part.is_empty())))
    {
        let mut cur = serde_json::to_value(&s.color().settings).map_err(|e| e.to_string())?;
        let rest = rest.trim_start_matches('.');
        if rest.is_empty() {
            cur = value;
        } else {
            set_path(&mut cur, rest, value)?;
        }
        let next = serde_json::from_value(cur).map_err(|e| format!("invalid value for `{path}`: {e}"))?;
        if validate_color {
            crate::color_cmds::validate_settings(&next)?;
        }
        s.color_mut().settings = next;
        s.prefs.edit(|_| ());
        return Ok(true);
    }
    let mut next = s.prefs().clone();
    let mut color_changed = false;
    if path.is_empty() || SECTIONS.iter().any(|(id, _)| *id == path) || matches!(path, "shortcuts" | "alternateShortcuts" | "menus" | "toolbar") {
        // Whole section (or everything): merge the object key by key so each value is checked.
        let obj = value.as_object().ok_or_else(|| format!("`{path}` needs an object"))?;
        let current = s.prefs().interface.clone();
        let applies = |p: &str, v: &Value| p != "interface.theme" || current.section_theme_changes(v);
        for (k, v) in obj {
            let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
            if path.is_empty() && k == "colorSettings" {
                color_changed |= set_pref(s, "colorSettings", v.clone(), validate_color)?;
                continue;
            }
            if v.is_object()
                && !p.starts_with("shortcuts")
                && !p.starts_with("alternateShortcuts")
                && !p.ends_with(".colors")
                && next.get(&p).is_some_and(|c| c.is_object())
            {
                for (k2, v2) in v.as_object().into_iter().flatten() {
                    let p2 = format!("{p}.{k2}");
                    if applies(&p2, v2) {
                        next.set(&p2, v2.clone())?;
                    }
                }
            } else if applies(&p, v) {
                next.set(&p, v.clone())?;
            }
        }
    } else {
        next.set(path, value)?;
    }
    if next.actions.create_single_history_state != s.prefs().actions.create_single_history_state && crate::action_history::history_policy_in_use(s) {
        return Err("cannot change action history settings during playback".into());
    }
    s.prefs.edit(|p| *p = next);
    Ok(color_changed)
}
