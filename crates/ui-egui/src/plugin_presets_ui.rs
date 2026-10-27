//! The Presets row of a plug-in dialog: named sets of the plug-in's parameter values.
//!
//! Every plug-in gets this automatically, keyed by its id. The presets live in one store per
//! plug-in that the shell reads and writes ([`crate::Services::load_plugin_presets`] /
//! [`crate::Services::save_plugin_presets`]); the plug-in never sees them. Stored values are checked
//! against the plug-in's current manifest ([`photocraft_plugins::Manifest::preset_values`]) before
//! they're listed or applied, so a preset saved by an older version can't feed it bad values.
//!
//! Dialog fields: `__presets` (name → checked values, for the list), `__preset` (the chosen name,
//! `""` for none), `__presetNaming` (the name being typed after "Save As…") and `__presetError`.

use photocraft_plugins::presets::{MAX_PRESETS, Presets, valid_name};
use photocraft_plugins::{Manifest, ParamSpec};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

const PRESETS: &str = "__presets";
const BUILTINS: &str = "__builtin_presets";
const SOURCE: &str = "__preset_source";
const CHOSEN: &str = "__preset";
const NAMING: &str = "__presetNaming";
const ERROR: &str = "__presetError";

/// Adds preset fields when a user store or manifest built-ins are available.
pub fn open_fields(app: &PhotocraftApp, m: &Manifest, fields: &mut Map<String, Value>) {
    let has_store = app.services.load_plugin_presets.is_some() && app.services.save_plugin_presets.is_some();
    if !has_store && m.presets.is_empty() {
        return;
    }
    let supported: Presets =
        app.services.load_plugin_presets.map(|load| load(&m.id)).unwrap_or_default().into_iter().filter(|(_, values)| known_effect(m, values)).collect();
    fields.insert(PRESETS.into(), to_value(&m.valid_presets(&supported)));
    fields.insert(BUILTINS.into(), to_value(&m.presets));
    fields.insert(SOURCE.into(), json!(""));
    fields.insert(CHOSEN.into(), json!(""));
}

/// Whether the dialog has a Presets row.
pub fn owns(f: &Map<String, Value>) -> bool {
    f.contains_key(PRESETS)
}

fn to_value(p: &Presets) -> Value {
    Value::Object(p.iter().map(|(k, v)| (k.clone(), Value::Object(v.clone()))).collect())
}

fn listed(f: &Map<String, Value>) -> Presets {
    listed_from(f, "user")
}

fn listed_from(f: &Map<String, Value>, source: &str) -> Presets {
    f.get(if source == "builtin" { BUILTINS } else { PRESETS })
        .and_then(Value::as_object)
        .map(|o| o.iter().filter_map(|(k, v)| v.as_object().map(|m| (k.clone(), m.clone()))).collect())
        .unwrap_or_default()
}

fn selector_default(m: &Manifest) -> Option<&str> {
    let key = m.preset_selector.as_ref()?;
    let (_, ParamSpec::Choice { options, default }) = m.params.iter().find(|(name, _)| name == key)? else { return None };
    default.as_deref().or_else(|| options.first().map(String::as_str))
}

/// Presets for removed effects stay in storage, but must not become legacy default-effect presets.
fn known_effect(m: &Manifest, values: &Map<String, Value>) -> bool {
    let Some(key) = m.preset_selector.as_ref() else { return true };
    let Some(value) = values.get(key) else { return true };
    let Some((_, ParamSpec::Choice { options, .. })) = m.params.iter().find(|(name, _)| name == key) else { return false };
    value.as_str().is_some_and(|name| options.iter().any(|option| option == name))
}

fn selector_value<'a>(m: &'a Manifest, f: &'a Map<String, Value>) -> Option<(&'a str, &'a str)> {
    let key = m.preset_selector.as_deref()?;
    let value = f.get(key).and_then(Value::as_str).or_else(|| selector_default(m))?;
    Some((key, value))
}

/// Older presets without a selector belong to its default effect (Lightning in FreeCandy).
fn matches_effect(m: &Manifest, f: &Map<String, Value>, values: &Map<String, Value>) -> bool {
    if !known_effect(m, values) {
        return false;
    }
    let Some((key, current)) = selector_value(m, f) else { return true };
    values.get(key).and_then(Value::as_str).or_else(|| selector_default(m)) == Some(current)
}

fn for_effect(m: &Manifest, f: &Map<String, Value>) -> Presets {
    listed(f).into_iter().filter(|(_, values)| matches_effect(m, f, values)).collect()
}

/// The scope ignores subordinate conditions: presets also retain hidden substyle controls.
fn in_effect(m: &Manifest, f: &Map<String, Value>, key: &str) -> bool {
    let Some((selector, current)) = selector_value(m, f) else { return true };
    m.visible_when(key).and_then(|conditions| conditions.get(selector)).is_none_or(|expected| expected.as_str() == Some(current))
}

/// Factory recipes can leave material choices independent of their style settings.
/// Explicit recipe values always apply, and saved user presets remain complete snapshots.
fn retains_current(m: &Manifest, source: &str, values: &Map<String, Value>, key: &str) -> bool {
    source == "builtin" && !values.contains_key(key) && m.preset_preserve.iter().any(|preserved| preserved == key)
}

fn matches_values(m: &Manifest, f: &Map<String, Value>, source: &str, values: &Map<String, Value>) -> bool {
    if !matches_effect(m, f, values) {
        return false;
    }
    let Ok(resolved) = m.resolve_params(&Value::Object(values.clone())) else { return false };
    resolved
        .iter()
        .filter(|(key, _)| in_effect(m, f, key) && !retains_current(m, source, values, key))
        .all(|(key, value)| f.get(key).is_some_and(|current| same_control_value(current, value)))
}

/// Controls round-trip numbers through f32 while manifest defaults are parsed as f64.
/// Compare at the control's precision so merely displaying a preset does not mark it Custom.
fn same_control_value(current: &Value, expected: &Value) -> bool {
    match (current, expected) {
        (Value::Number(a), Value::Number(b)) => a.as_f64().map(|v| v as f32) == b.as_f64().map(|v| v as f32),
        (Value::Array(a), Value::Array(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_control_value(a, b)),
        _ => current == expected,
    }
}

/// Puts the preset `name`'s values into the dialog; parameters it doesn't set go back to their
/// defaults within the selected effect, except factory recipe `presetPreserve` controls.
/// Other effects keep their values.
pub fn apply(m: &Manifest, f: &mut Map<String, Value>, name: &str) -> bool {
    let source = if listed(f).contains_key(name) { "user" } else { "builtin" };
    apply_from(m, f, source, name)
}

/// Applies a named preset from an explicit source (`builtin` or `user`).
pub fn apply_from(m: &Manifest, f: &mut Map<String, Value>, source: &str, name: &str) -> bool {
    if !matches!(source, "builtin" | "user") {
        return false;
    }
    let Some(values) = listed_from(f, source).remove(name) else { return false };
    if !matches_effect(m, f, &values) {
        return false;
    }
    let Ok(resolved) = m.resolve_params(&Value::Object(values.clone())) else { return false };
    let scoped: Map<String, Value> = resolved.into_iter().filter(|(key, _)| in_effect(m, f, key) && !retains_current(m, source, &values, key)).collect();
    f.extend(scoped);
    f.insert(CHOSEN.into(), json!(name));
    f.insert(SOURCE.into(), json!(source));
    true
}

/// Saves the dialog's current values as `name` (replacing a preset of that name).
pub fn save_as(app: &mut PhotocraftApp, m: &Manifest, f: &mut Map<String, Value>, name: &str) -> Result<(), String> {
    if !valid_name(name) {
        return Err(format!("{name:?} can't be used as a preset name"));
    }
    if listed(f).get(name).is_some_and(|existing| !matches_effect(m, f, existing)) {
        return Err("That name is used by another effect. Choose a different name.".into());
    }
    let current = crate::filter_dialog::params_of(f);
    let mut values = current.as_object().and_then(|c| m.preset_values(c)).ok_or("this plug-in has no values to save")?;
    values.retain(|key, _| in_effect(m, f, key));
    store(app, &m.id, |all| {
        if all.get(name).is_some_and(|existing| !matches_effect(m, f, existing)) {
            return Err("That name is used by another effect. Choose a different name.".into());
        }
        if all.len() >= MAX_PRESETS && !all.contains_key(name) {
            return Err(format!("a plug-in keeps at most {MAX_PRESETS} presets"));
        }
        all.insert(name.to_string(), values.clone());
        Ok(())
    })?;
    let mut list = listed(f);
    list.insert(name.to_string(), values);
    f.insert(PRESETS.into(), to_value(&list));
    f.insert(CHOSEN.into(), json!(name));
    f.insert(SOURCE.into(), json!("user"));
    Ok(())
}

/// Deletes the preset `name`.
pub fn delete(app: &mut PhotocraftApp, m: &Manifest, f: &mut Map<String, Value>, name: &str) -> Result<(), String> {
    if f.get(SOURCE).and_then(Value::as_str) == Some("builtin") && f.get(CHOSEN).and_then(Value::as_str) == Some(name) {
        return Err("Built-in presets cannot be deleted.".into());
    }
    if !for_effect(m, f).contains_key(name) {
        return Err("That preset does not belong to the selected effect.".into());
    }
    store(app, &m.id, |all| {
        if all.get(name).is_some_and(|existing| !matches_effect(m, f, existing)) {
            return Err("That preset does not belong to the selected effect.".into());
        }
        all.remove(name);
        Ok(())
    })?;
    let mut list = listed(f);
    list.remove(name);
    f.insert(PRESETS.into(), to_value(&list));
    f.insert(CHOSEN.into(), json!(""));
    f.insert(SOURCE.into(), json!(""));
    Ok(())
}

/// Re-reads the stored presets (so presets this version of the plug-in can't use are kept, and
/// changes from another window aren't lost), changes them and writes them back.
fn store(app: &mut PhotocraftApp, id: &str, change: impl FnOnce(&mut Presets) -> Result<(), String>) -> Result<(), String> {
    let (Some(load), Some(save)) = (app.services.load_plugin_presets, app.services.save_plugin_presets) else {
        return Err("User preset storage is unavailable.".into());
    };
    let mut all = load(id);
    change(&mut all)?;
    save(id, &all)
}

/// The Presets row: a list of saved names (choosing one applies it), "Save As…" and "Delete".
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let Some(plugin) = f.get("id").and_then(Value::as_str).and_then(photocraft_plugins::registry::get) else { return };
    let m = plugin.manifest();
    let t = Tokens::get(ui.ctx());
    let user_presets = for_effect(m, f);
    let builtins: Presets = listed_from(f, "builtin").into_iter().filter(|(_, values)| matches_effect(m, f, values)).collect();
    let mut chosen = f.get(CHOSEN).and_then(Value::as_str).unwrap_or_default().to_string();
    let mut source = f.get(SOURCE).and_then(Value::as_str).unwrap_or("user").to_string();
    let selected = if source == "builtin" { &builtins } else { &user_presets };
    if !selected.get(&chosen).is_some_and(|values| matches_values(m, f, &source, values)) {
        chosen.clear();
        source.clear();
        f.insert(CHOSEN.into(), json!(""));
        f.insert(SOURCE.into(), json!(""));
    }
    let can_save = app.services.load_plugin_presets.is_some() && app.services.save_plugin_presets.is_some();
    let (mut picked, mut save_as_clicked, mut delete_clicked) = (false, false, false);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Presets")).color(t.text_dim));
        let mut selection = (source.clone(), chosen.clone());
        let mut options = vec![(Some((String::new(), String::new())), tl!("Custom")), (None, "Built-in presets")];
        options.extend(builtins.keys().map(|name| (Some(("builtin".to_string(), name.clone())), name.as_str())));
        options.push((None, "Your presets"));
        options.extend(user_presets.keys().map(|name| (Some(("user".to_string(), name.clone())), name.as_str())));
        picked = crate::widgets::dropdown_sections(ui, &format!("plugin-presets-{}", m.id), tl!("Presets"), &mut selection, &options, 150.0);
        (source, chosen) = selection;
        save_as_clicked = ui.add_enabled_ui(can_save, |ui| crate::widgets::secondary_button(ui, tl!("Save As…"), 0.0).clicked()).inner;
        delete_clicked = ui
            .add_enabled_ui(can_save && source == "user" && !chosen.is_empty(), |ui| crate::widgets::secondary_button(ui, tl!("Delete"), 0.0).clicked())
            .inner;
    });
    let mut result = Ok(());
    // Opening the name field isn't an action: an earlier error stays until something succeeds.
    // A pick counts only once it applies.
    let mut acted = delete_clicked;
    if picked {
        if chosen.is_empty() {
            f.insert(CHOSEN.into(), json!(""));
            f.insert(SOURCE.into(), json!(""));
            acted = true;
        } else {
            acted = apply_from(m, f, &source, &chosen);
        }
    }
    if save_as_clicked {
        f.insert(NAMING.into(), json!(if chosen.is_empty() { String::new() } else { chosen.clone() }));
    }
    if delete_clicked && !chosen.is_empty() {
        result = delete(app, m, f, &chosen);
    }
    if let Some(mut name) = f.get(NAMING).and_then(Value::as_str).map(str::to_string) {
        let (mut save, mut cancel) = (false, false);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Name:")).color(t.text_dim));
            let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(150.0).char_limit(photocraft_plugins::presets::MAX_NAME_CHARS));
            if save_as_clicked {
                r.request_focus();
            }
            let ok = valid_name(name.trim());
            // Enter here belongs to the name field: keep it from also confirming the dialog.
            let entered = r.lost_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            save = ui.add_enabled_ui(ok, |ui| crate::widgets::secondary_button(ui, tl!("Save"), 0.0).clicked()).inner || (ok && entered);
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 0.0).clicked();
        });
        if save {
            f.remove(NAMING);
            acted = true;
            result = save_as(app, m, f, name.trim());
        } else if cancel {
            f.remove(NAMING);
        } else {
            f.insert(NAMING.into(), json!(name));
        }
    }
    match result {
        Ok(()) if acted => {
            f.remove(ERROR);
        }
        Ok(()) => {}
        Err(e) => {
            log::warn!("plug-in presets for {}: {e}", m.id);
            f.insert(ERROR.into(), json!(e));
        }
    }
    if let Some(e) = f.get(ERROR).and_then(Value::as_str) {
        ui.add(egui::Label::new(egui::RichText::new(e).color(t.text_dim)).wrap());
    }
    ui.add_space(6.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The in-memory store the tests' services read and write (one test at a time touches it).
    static STORE: Mutex<Option<Presets>> = Mutex::new(None);
    static LOCK: Mutex<()> = Mutex::new(());

    fn load(_id: &str) -> Presets {
        STORE.lock().unwrap().clone().unwrap_or_default()
    }
    fn save(_id: &str, p: &Presets) -> Result<(), String> {
        *STORE.lock().unwrap() = Some(p.clone());
        Ok(())
    }

    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../plugins/tests/fixtures/invert.wasm");

    fn app_with_store() -> PhotocraftApp {
        let services = crate::Services { load_plugin_presets: Some(load), save_plugin_presets: Some(save), ..Default::default() };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.run("plugin.install", json!({"path": FIXTURE})).unwrap();
        app
    }

    /// A manifest with every parameter kind, standing in for an installed plug-in's.
    fn manifest() -> Manifest {
        Manifest::parse(
            br#"{"id":"org.example.presets","name":"P","kind":"filter","params":{
                "amount":{"type":"number","min":0,"max":100,"default":50},
                "mode":{"type":"choice","options":["a","b"]},
                "at":{"type":"point","default":[50,50]}}}"#,
        )
        .unwrap()
    }

    #[test]
    fn presets_save_apply_and_delete_through_the_store() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A stale preset (from an older plug-in version) and one that needs clamping.
        let mut stored = Presets::new();
        stored.insert("Old".into(), json!({"gone": 1}).as_object().unwrap().clone());
        stored.insert("Wide".into(), json!({"amount": 400, "mode": "zzz", "at": [10, 20]}).as_object().unwrap().clone());
        *STORE.lock().unwrap() = Some(stored);
        let mut app = app_with_store();
        let m = manifest();
        let mut f = Map::new();
        f.insert("id".into(), json!(m.id));
        f.insert("amount".into(), json!(50.0));
        f.insert("mode".into(), json!("a"));
        f.insert("at".into(), json!([50.0, 50.0]));
        open_fields(&app, &m, &mut f);
        assert_eq!(listed(&f).keys().collect::<Vec<_>>(), ["Wide"], "the stale preset isn't listed");
        assert!(apply(&m, &mut f, "Wide"));
        assert_eq!(
            (&f["amount"], &f["mode"], &f["at"]),
            (&json!(100.0), &json!("a"), &json!([10.0, 20.0])),
            "clamped; unknown choice falls back to the default"
        );
        f.insert("amount".into(), json!(12.5));
        save_as(&mut app, &m, &mut f, "Mine").unwrap();
        let s = STORE.lock().unwrap().clone().unwrap();
        assert_eq!(s.keys().collect::<Vec<_>>(), ["Mine", "Old", "Wide"], "the stale preset stays in the file");
        assert_eq!(Value::Object(s["Mine"].clone()), json!({"amount": 12.5, "mode": "a", "at": [10.0, 20.0]}), "no id or dialog keys");
        assert!(save_as(&mut app, &m, &mut f, "../evil").is_err());
        assert!(save_as(&mut app, &m, &mut f, "a/b").is_err());
        delete(&mut app, &m, &mut f, "Mine").unwrap();
        assert!(!STORE.lock().unwrap().as_ref().unwrap().contains_key("Mine"));
        assert_eq!(f[CHOSEN], json!(""));
    }

    #[test]
    fn plugin_dialogs_get_a_presets_row_only_with_a_store() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *STORE.lock().unwrap() = None;
        let id = "plugin.filter.org.photocraft.example.invert";
        // The invert fixture has no parameters and runs directly; check the field plumbing instead.
        let app = app_with_store();
        let m = photocraft_plugins::registry::get("org.photocraft.example.invert").unwrap();
        let mut f = Map::new();
        open_fields(&app, m.manifest(), &mut f);
        assert!(owns(&f));
        let bare = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut f = Map::new();
        open_fields(&bare, m.manifest(), &mut f);
        assert!(!owns(&f), "no store, no Presets row ({id})");
    }

    #[test]
    fn effect_presets_keep_other_effects_and_hidden_substyles_and_protect_names() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let m = Manifest::parse(
            br#"{"id":"org.example.suite","name":"Suite","kind":"filter","presetSelector":"effect","params":{
            "effect":{"type":"choice","options":["lightning","fire","shadow"]},
            "bolt_width":{"type":"number","min":0,"max":100,"default":5,"visibleWhen":{"effect":"lightning"}},
            "glow":{"type":"number","min":0,"max":100,"default":10,"visibleWhen":{"effect":"lightning"}},
            "fire_style":{"type":"choice","options":["soft","hard"],"visibleWhen":{"effect":"fire"}},
            "fire_amount":{"type":"number","min":0,"max":100,"default":30,"visibleWhen":{"effect":"fire"}},
            "fire_detail":{"type":"number","min":0,"max":100,"default":40,"visibleWhen":{"effect":"fire","fire_style":"hard"}},
            "shadow_softness":{"type":"number","min":0,"max":100,"default":20,"visibleWhen":{"effect":"shadow"}}}}"#,
        )
        .unwrap();
        let stored = [
            ("Legacy Bolt".to_string(), json!({"bolt_width": 11.0})),
            ("Removed Effect".to_string(), json!({"effect": "removed", "bolt_width": 55.0})),
            ("Malformed Effect".to_string(), json!({"effect": null, "bolt_width": 66.0})),
            ("Fire".to_string(), json!({"effect": "fire", "fire_style": "soft", "fire_amount": 32.0, "fire_detail": 47.0})),
        ]
        .into_iter()
        .map(|(name, values)| (name, values.as_object().unwrap().clone()))
        .collect();
        *STORE.lock().unwrap() = Some(stored);
        let mut app = app_with_store();
        let mut f = m.resolve_params(&Value::Null).unwrap();
        f.insert("bolt_width".into(), json!(17.0));
        f.insert("glow".into(), json!(19.0));
        f.insert("shadow_softness".into(), json!(29.0));
        open_fields(&app, &m, &mut f);
        assert_eq!(for_effect(&m, &f).keys().collect::<Vec<_>>(), ["Legacy Bolt"]);
        assert!(!listed(&f).contains_key("Removed Effect"), "a removed selector cannot become a legacy Lightning preset");
        assert!(!listed(&f).contains_key("Malformed Effect"));
        assert!(save_as(&mut app, &m, &mut f, "Malformed Effect").is_err());
        assert!(!apply(&m, &mut f, "Removed Effect"));
        assert!(delete(&mut app, &m, &mut f, "Removed Effect").is_err());
        assert!(save_as(&mut app, &m, &mut f, "Removed Effect").is_err());
        assert_eq!(STORE.lock().unwrap().as_ref().unwrap()["Removed Effect"]["effect"], json!("removed"), "orphan preset stays untouched in storage");
        assert!(!apply(&m, &mut f, "Fire"), "out-of-effect calls cannot switch effects");
        assert!(apply(&m, &mut f, "Legacy Bolt"));
        assert_eq!(f["glow"], json!(10.0), "missing current-effect values reset to their defaults");
        assert_eq!(f["shadow_softness"], json!(29.0));
        f.insert("effect".into(), json!("fire"));
        assert_eq!(for_effect(&m, &f).keys().collect::<Vec<_>>(), ["Fire"]);
        let before = f.clone();
        assert!(!apply(&m, &mut f, "Legacy Bolt"));
        assert!(delete(&mut app, &m, &mut f, "Legacy Bolt").is_err());
        assert!(save_as(&mut app, &m, &mut f, "Legacy Bolt").unwrap_err().contains("another effect"));
        assert_eq!(f, before, "rejected actions leave the dialog intact");
        assert!(STORE.lock().unwrap().as_ref().unwrap().contains_key("Legacy Bolt"));
        assert!(apply(&m, &mut f, "Fire"));
        assert_eq!(f["bolt_width"], json!(11.0));
        assert_eq!(f["shadow_softness"], json!(29.0));
        assert_eq!(f["fire_detail"], json!(47.0), "hidden substyle values apply with the preset");
        assert!(matches_values(&m, &f, "user", &listed(&f)["Fire"]));
        f.insert("shadow_softness".into(), json!(51.0));
        assert!(matches_values(&m, &f, "user", &listed(&f)["Fire"]), "another effect's values do not mark this preset Custom");
        save_as(&mut app, &m, &mut f, "My Fire").unwrap();
        let saved = STORE.lock().unwrap().as_ref().unwrap()["My Fire"].clone();
        assert_eq!(Value::Object(saved), json!({"effect": "fire", "fire_style": "soft", "fire_amount": 32.0, "fire_detail": 47.0}));
        f.insert("fire_amount".into(), json!(44.0));
        assert!(!matches_values(&m, &f, "user", &listed(&f)["My Fire"]), "editing current controls returns the row to Custom");
        save_as(&mut app, &m, &mut f, "My Fire").unwrap();
        assert_eq!(STORE.lock().unwrap().as_ref().unwrap()["My Fire"]["fire_amount"], json!(44.0), "same-effect names can be replaced");
        delete(&mut app, &m, &mut f, "My Fire").unwrap();
        assert!(!STORE.lock().unwrap().as_ref().unwrap().contains_key("My Fire"));
    }

    #[test]
    fn builtin_presets_work_without_storage_and_user_copies_survive_reopen() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *STORE.lock().unwrap() = None;
        let m = Manifest::parse(
            br#"{"id":"org.example.factory","name":"Factory","kind":"filter",
            "presetSelector":"effect","params":{
                "effect":{"type":"choice","options":["bolt","fire"]},
                "width":{"type":"number","min":0,"max":100,"default":10,"visibleWhen":{"effect":"bolt"}},
                "heat":{"type":"number","min":0,"max":100,"default":20,"visibleWhen":{"effect":"fire"}}},
            "presets":{"Soft":{"effect":"bolt","width":12},"Flame":{"effect":"fire","heat":40}}}"#,
        )
        .unwrap();
        let bare = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut f = m.resolve_params(&Value::Null).unwrap();
        f.insert("heat".into(), json!(77.0));
        open_fields(&bare, &m, &mut f);
        assert!(owns(&f));
        assert!(apply_from(&m, &mut f, "builtin", "Soft"));
        assert_eq!(f["width"], json!(12.0));
        assert_eq!(f["heat"], json!(77.0));
        assert_eq!(f[SOURCE], json!("builtin"));
        assert!(!apply_from(&m, &mut f, "builtin", "Flame"));
        let mut app = app_with_store();
        assert!(delete(&mut app, &m, &mut f, "Soft").is_err());
        f.insert("width".into(), json!(31.0));
        f.insert("__meta".into(), json!("private"));
        save_as(&mut app, &m, &mut f, "Soft").unwrap();
        assert_eq!(f[SOURCE], json!("user"));
        assert_eq!(m.presets["Soft"]["width"], json!(12.0));
        let stored = load(&m.id);
        assert_eq!(stored.len(), 1);
        assert!(!stored["Soft"].contains_key("__meta"));
        let mut reopened = m.resolve_params(&Value::Null).unwrap();
        open_fields(&app, &m, &mut reopened);
        assert!(apply(&m, &mut reopened, "Soft"));
        assert_eq!(reopened["width"], json!(31.0), "legacy entry point prefers user name collisions");
        assert!(apply_from(&m, &mut reopened, "builtin", "Soft"));
        assert_eq!(reopened["width"], json!(12.0));
        assert!(delete(&mut app, &m, &mut reopened, "Soft").is_err());
        assert!(apply_from(&m, &mut reopened, "user", "Soft"));
        delete(&mut app, &m, &mut reopened, "Soft").unwrap();
        assert!(listed(&reopened).is_empty());
        assert!(apply_from(&m, &mut reopened, "builtin", "Soft"));
        assert_eq!(m.presets["Soft"]["width"], json!(12.0));
    }

    #[test]
    fn builtin_styles_keep_material_choices_and_saved_presets_restore_them() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *STORE.lock().unwrap() = None;
        let m = Manifest::parse(
            br##"{"id":"org.example.fur","name":"Fur","kind":"filter",
            "presetSelector":"effect","presetPreserve":["coat","colour"],"params":{
                "effect":{"type":"choice","options":["fur","fire"]},
                "coat":{"type":"choice","options":["plain","zebra","tiger"],"default":"plain","visibleWhen":{"effect":"fur"}},
                "colour":{"type":"color","default":"#cc9955","visibleWhen":{"effect":"fur"}},
                "length":{"type":"number","min":1,"max":60,"default":12,"visibleWhen":{"effect":"fur"}},
                "density":{"type":"number","min":10,"max":100,"default":65,"visibleWhen":{"effect":"fur"}},
                "heat":{"type":"number","min":0,"max":100,"default":20,"visibleWhen":{"effect":"fire"}}},
            "presets":{"Short":{"effect":"fur","length":4},"Long":{"effect":"fur","length":45},
                "White tiger":{"effect":"fur","coat":"tiger","colour":"#eeeeee","length":20}}}"##,
        )
        .unwrap();
        let mut app = app_with_store();
        let mut f = m.resolve_params(&Value::Null).unwrap();
        open_fields(&app, &m, &mut f);
        f.insert("coat".into(), json!("zebra"));
        f.insert("colour".into(), json!("#777777"));
        f.insert("density".into(), json!(90.0));
        f.insert("heat".into(), json!(77.0));
        assert!(apply_from(&m, &mut f, "builtin", "Short"));
        assert_eq!(f["coat"], json!("zebra"), "grooming does not replace the chosen coat");
        assert_eq!(f["colour"], json!("#777777"));
        assert_eq!(f["length"], json!(4.0));
        assert_eq!(f["density"], json!(65.0), "other omitted controls still use their defaults");
        assert_eq!(f["heat"], json!(77.0), "another effect's controls are untouched");
        assert!(apply_from(&m, &mut f, "builtin", "Long"));
        assert_eq!(f["coat"], json!("zebra"));
        assert_eq!(f["length"], json!(45.0));
        f.insert("coat".into(), json!("tiger"));
        assert!(matches_values(&m, &f, "builtin", &m.presets["Long"]), "changing coat keeps the selected hair style");
        save_as(&mut app, &m, &mut f, "My tiger").unwrap();
        f.insert("coat".into(), json!("zebra"));
        f.insert("colour".into(), json!("#ff8800"));
        assert!(apply_from(&m, &mut f, "user", "My tiger"));
        assert_eq!(f["coat"], json!("tiger"), "saved presets remain complete snapshots");
        assert_eq!(f["colour"], json!("#777777"));
        assert!(apply_from(&m, &mut f, "builtin", "White tiger"));
        assert_eq!(f["coat"], json!("tiger"), "an explicit factory material value takes precedence");
        assert_eq!(f["colour"], json!("#eeeeee"));
        f.insert("coat".into(), json!("zebra"));
        assert!(!matches_values(&m, &f, "builtin", &m.presets["White tiger"]));
    }

    #[test]
    fn plugin_preset_applies_params_on_every_tab() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let m = Manifest::parse(
            br##"{"id":"org.example.tabs","name":"T","kind":"filter","params":{
                "effect":{"type":"choice","options":["a","b"]},
                "amount":{"type":"number","min":0,"max":100,"default":50,"group":"Bolt"},
                "glow":{"type":"number","min":0,"max":1,"default":0.5,"group":"Glow"},
                "inner":{"type":"color","default":"#ffffff","group":"Colour"}}}"##,
        )
        .unwrap();
        let mut stored = Presets::new();
        stored.insert("All".into(), json!({"effect": "b", "amount": 10, "glow": 0.25, "inner": "#ff8800"}).as_object().unwrap().clone());
        *STORE.lock().unwrap() = Some(stored);
        let mut app = app_with_store();
        let mut f = Map::new();
        f.insert("id".into(), json!(m.id));
        f.insert("__tab".into(), json!("Glow"));
        open_fields(&app, &m, &mut f);
        assert!(apply(&m, &mut f, "All"));
        assert_eq!((&f["effect"], &f["amount"], &f["glow"], &f["inner"]), (&json!("b"), &json!(10.0), &json!(0.25), &json!("#ff8800")));
        assert_eq!(f["__tab"], json!("Glow"), "applying a preset keeps the tab");
        // Saving stores every param, whichever tab it is on.
        f.insert("amount".into(), json!(20.0));
        save_as(&mut app, &m, &mut f, "Mine").unwrap();
        let s = STORE.lock().unwrap().clone().unwrap();
        assert_eq!(Value::Object(s["Mine"].clone()), json!({"effect": "b", "amount": 20.0, "glow": 0.25, "inner": "#ff8800"}));
    }

    #[test]
    fn rendering_decimal_preset_controls_does_not_mark_them_custom() {
        let m = Manifest::parse(
            br#"{"id":"org.example.decimals","name":"Decimals","kind":"filter","params":{
                "amount":{"type":"number","min":0,"max":1,"default":0.1},
                "point":{"type":"point","default":[10.1,90.9]}},
                "presets":{"Decimal":{"amount":0.1,"point":[10.1,90.9]}}}"#,
        )
        .unwrap();
        let mut fields = m.resolve_params(&Value::Null).unwrap();
        fields.insert("__command".into(), json!("plugin.run"));
        fields.insert("__spec".into(), json!(m.params_notation()));
        let mut h = egui_kittest::Harness::builder().build_ui_state(crate::filter_dialog::body, fields);
        h.run_steps(4);
        assert!(matches_values(&m, h.state(), "builtin", &m.presets["Decimal"]), "displaying unchanged decimal values preserves the selected preset");
        h.state_mut().insert("amount".into(), json!(0.11));
        assert!(!matches_values(&m, h.state(), "builtin", &m.presets["Decimal"]), "a real edit still marks the preset Custom");
    }
}
