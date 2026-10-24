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

use photocraft_plugins::Manifest;
use photocraft_plugins::presets::{MAX_PRESETS, Presets, valid_name};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

const PRESETS: &str = "__presets";
const CHOSEN: &str = "__preset";
const NAMING: &str = "__presetNaming";
const ERROR: &str = "__presetError";

/// Adds the Presets row's fields to a new plug-in dialog for `m` (nothing without a presets store).
pub fn open_fields(app: &PhotocraftApp, m: &Manifest, fields: &mut Map<String, Value>) {
    let Some(load) = app.services.load_plugin_presets else { return };
    if app.services.save_plugin_presets.is_none() {
        return;
    }
    fields.insert(PRESETS.into(), to_value(&m.valid_presets(&load(&m.id))));
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
    f.get(PRESETS).and_then(Value::as_object).map(|o| o.iter().filter_map(|(k, v)| v.as_object().map(|m| (k.clone(), m.clone()))).collect()).unwrap_or_default()
}

/// Puts the preset `name`'s values into the dialog; parameters it doesn't set go back to their
/// defaults. The live preview follows, as for any change of the dialog's values.
pub fn apply(m: &Manifest, f: &mut Map<String, Value>, name: &str) -> bool {
    let Some(values) = listed(f).remove(name) else { return false };
    let Ok(resolved) = m.resolve_params(&Value::Object(values)) else { return false };
    f.extend(resolved);
    f.insert(CHOSEN.into(), json!(name));
    true
}

/// Saves the dialog's current values as `name` (replacing a preset of that name).
pub fn save_as(app: &mut PhotocraftApp, m: &Manifest, f: &mut Map<String, Value>, name: &str) -> Result<(), String> {
    if !valid_name(name) {
        return Err(format!("{name:?} can't be used as a preset name"));
    }
    let current = crate::filter_dialog::params_of(f);
    let values = current.as_object().and_then(|c| m.preset_values(c)).ok_or("this plug-in has no values to save")?;
    // The dialog lists the preset only once the store has accepted it.
    store(app, &m.id, |all| {
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
    Ok(())
}

/// Deletes the preset `name`.
pub fn delete(app: &mut PhotocraftApp, m: &Manifest, f: &mut Map<String, Value>, name: &str) -> Result<(), String> {
    store(app, &m.id, |all| {
        all.remove(name);
        Ok(())
    })?;
    let mut list = listed(f);
    list.remove(name);
    f.insert(PRESETS.into(), to_value(&list));
    f.insert(CHOSEN.into(), json!(""));
    Ok(())
}

/// Re-reads the stored presets (so presets this version of the plug-in can't use are kept, and
/// changes from another window aren't lost), changes them and writes them back.
fn store(app: &mut PhotocraftApp, id: &str, change: impl FnOnce(&mut Presets) -> Result<(), String>) -> Result<(), String> {
    let (Some(load), Some(save)) = (app.services.load_plugin_presets, app.services.save_plugin_presets) else { return Ok(()) };
    let mut all = load(id);
    change(&mut all)?;
    save(id, &all)
}

/// The Presets row: a list of saved names (choosing one applies it), "Save As…" and "Delete".
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let Some(plugin) = f.get("id").and_then(Value::as_str).and_then(photocraft_plugins::registry::get) else { return };
    let m = plugin.manifest();
    let t = Tokens::get(ui.ctx());
    let names: Vec<String> = listed(f).into_keys().collect();
    let mut chosen = f.get(CHOSEN).and_then(Value::as_str).unwrap_or_default().to_string();
    let mut opts: Vec<(String, &str)> = vec![(String::new(), tl!("Custom"))];
    opts.extend(names.iter().map(|n| (n.clone(), n.as_str())));
    let (mut picked, mut save_as_clicked, mut delete_clicked) = (false, false, false);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Presets")).color(t.text_dim));
        picked = crate::widgets::dropdown(ui, &format!("plugin-presets-{}", m.id), &mut chosen, &opts, 150.0);
        save_as_clicked = crate::widgets::secondary_button(ui, tl!("Save As…"), 0.0).clicked();
        delete_clicked = ui.add_enabled_ui(!chosen.is_empty(), |ui| crate::widgets::secondary_button(ui, tl!("Delete"), 0.0).clicked()).inner;
    });
    let mut result = Ok(());
    // Opening the name field isn't an action: an earlier error stays until something succeeds.
    // A pick counts only once it applies.
    let mut acted = delete_clicked;
    if picked {
        if chosen.is_empty() {
            f.insert(CHOSEN.into(), json!(""));
            acted = true;
        } else {
            acted = apply(m, f, &chosen);
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
}
