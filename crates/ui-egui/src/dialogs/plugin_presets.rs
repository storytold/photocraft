//! Plug-in preset selection, editing baseline, and persistent view choices.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

const NEW: &str = "__pluginPresetNew";

pub(super) fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(NEW)
}

pub(super) fn manages_preset(fields: &Map<String, Value>) -> bool {
    fields.get("id").and_then(Value::as_str).and_then(photocraft_plugins::registry::get).is_some_and(|p| !p.manifest().preset_values.is_empty())
}

/// View choices and the selected preset persist even when the filter is cancelled.
pub(super) fn remember_choices(app: &mut PhotocraftApp, fields: &Map<String, Value>) {
    let Some(id) = fields.get("id").and_then(Value::as_str) else { return };
    let key = format!("plugin.choices.{id}");
    let mut choices: Map<String, Value> =
        ["__preview", "skinOnly"].into_iter().filter_map(|k| fields.get(k).filter(|v| v.is_boolean()).map(|v| (k.into(), v.clone()))).collect();
    if let Some(selection) = fields.get("__pluginPreset").filter(|v| v.is_string()) {
        choices.insert("__pluginPreset".into(), selection.clone());
    }
    let value = Value::Object(choices);
    if app.session.prefs().dialogs.get(&key) != Some(&value) {
        app.session.prefs.edit(|prefs| prefs.dialogs.insert(key, value));
    }
}

fn default_selection(m: &photocraft_plugins::Manifest) -> String {
    m.preset_values.get_key_value("natural").or_else(|| m.preset_values.first_key_value()).map(|(k, _)| format!("builtin:{k}")).unwrap_or_default()
}

pub(super) fn restore_choices(app: &PhotocraftApp, fields: &mut Map<String, Value>) {
    let Some(plugin) = fields.get("id").and_then(Value::as_str).and_then(photocraft_plugins::registry::get) else { return };
    let saved = app.session.prefs().dialogs.get(&format!("plugin.choices.{}", plugin.id())).and_then(Value::as_object);
    for key in ["__preview", "skinOnly"] {
        if fields.get(key).is_some_and(Value::is_boolean)
            && let Some(v) = saved.and_then(|s| s.get(key)).filter(|v| v.is_boolean())
        {
            fields.insert(key.into(), v.clone());
        }
    }
    let selection = saved.and_then(|s| s.get("__pluginPreset")).and_then(Value::as_str).unwrap_or_default();
    if !select(app, fields, selection) {
        select(app, fields, &default_selection(plugin.manifest()));
    }
}

fn values(app: &PhotocraftApp, fields: &Map<String, Value>, selection: &str) -> Option<Map<String, Value>> {
    let plugin = fields.get("id").and_then(Value::as_str).and_then(photocraft_plugins::registry::get)?;
    let m = plugin.manifest();
    let input = if let Some(name) = selection.strip_prefix("builtin:") {
        Value::Object(m.preset_values.get(name)?.clone())
    } else {
        app.session.prefs().dialogs.get(&format!("plugin.presets.{}", plugin.id()))?.get(selection.strip_prefix("saved:")?)?.clone()
    };
    let mut values = m.resolve_params(&input).ok()?;
    // These switches are independent of preset selection and Reset.
    values.remove("skinOnly");
    if super::skin_smooth_ui::owns(fields) {
        values.remove("positiveColors");
        values.remove("negativeColors");
    }
    if !m.preset_values.is_empty() {
        // Keep the algorithm in manual mode so it uses the editable values and remembered Skin Only.
        values.insert("preset".into(), json!("manual"));
    }
    Some(values)
}

fn select(app: &PhotocraftApp, fields: &mut Map<String, Value>, selection: &str) -> bool {
    if selection.is_empty() {
        for key in ["__pluginPreset", "__presetValues", "__presetDirty"] {
            fields.remove(key);
        }
        return false;
    }
    let Some(values) = values(app, fields, selection) else { return false };
    fields.extend(values.clone());
    fields.insert("__presetValues".into(), Value::Object(values));
    fields.insert("__pluginPreset".into(), json!(selection));
    fields.insert("__presetDirty".into(), json!(false));
    true
}

pub(super) fn reset(app: &PhotocraftApp, fields: &mut Map<String, Value>) {
    let selection = fields.get("__pluginPreset").and_then(Value::as_str).unwrap_or_default().to_string();
    select(app, fields, &selection);
}

pub(super) fn sync_values(fields: &mut Map<String, Value>) {
    let Some(baseline) = fields.get("__presetValues").and_then(Value::as_object) else { return };
    let dirty = baseline.iter().any(|(k, v)| (k != "preset" || !manages_preset(fields)) && !same_value(fields.get(k), Some(v)));
    fields.insert("__presetDirty".into(), json!(dirty));
}

fn same_value(a: Option<&Value>, b: Option<&Value>) -> bool {
    a == b || a.zip(b).is_some_and(|(a, b)| a.as_f64().is_some() && a.as_f64() == b.as_f64())
}

pub(super) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>, parent: u64) {
    let Some(plugin) = fields.get("id").and_then(Value::as_str).and_then(photocraft_plugins::registry::get) else { return };
    let m = plugin.manifest();
    let id = plugin.id();
    let presets = app.session.prefs().dialogs.get(&format!("plugin.presets.{id}")).and_then(Value::as_object).cloned().unwrap_or_default();
    let mut selection = fields.get("__pluginPreset").and_then(Value::as_str).unwrap_or_default().to_string();
    // A preset removed in another open dialog must not leave a stale Reset target.
    if !selection.is_empty() && values(app, fields, &selection).is_none() {
        selection = default_selection(m);
        select(app, fields, &selection);
    }
    let selected_name = if let Some(name) = selection.strip_prefix("builtin:") {
        crate::filter_dialog::label(name)
    } else {
        selection.strip_prefix("saved:").unwrap_or(tl!("Preset")).to_string()
    };
    let dirty = fields.get("__presetDirty").and_then(Value::as_bool) == Some(true);
    let label = if dirty { format!("{selected_name} ({})", tl!("Modified")) } else { selected_name };
    ui.horizontal(|ui| {
        ui.label(tl!("Preset"));
        let width = (ui.available_width() - 170.0).clamp(120.0, 220.0);
        let before = selection.clone();
        craft_ui::ComboBox::from_id_salt(("plugin-presets", id))
            .selected_text(&label)
            .truncate()
            .width(width)
            .height(300.0)
            .show_ui(ui, |ui| {
                ui.set_max_width(360.0);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                if let Some((_, photocraft_plugins::manifest::ParamSpec::Choice { options, .. })) = m.params.iter().find(|(k, _)| k == "preset") {
                    for name in options.iter().filter(|n| m.preset_values.contains_key(*n)) {
                        ui.selectable_value(&mut selection, format!("builtin:{name}"), crate::filter_dialog::label(name));
                    }
                }
                if !m.preset_values.is_empty() && !presets.is_empty() {
                    ui.separator();
                }
                for name in presets.keys() {
                    ui.selectable_value(&mut selection, format!("saved:{name}"), name).on_hover_text(name);
                }
            })
            .response
            .accessible_name(tl!("Preset"))
            .on_hover_text(&label);
        if selection != before && !select(app, fields, &selection) {
            crate::notices::error(app, tl!("This preset is no longer available.").into());
        }
        let has_baseline = fields.contains_key("__presetValues");
        if ui
            .add_enabled_ui(dirty && has_baseline, |ui| crate::icons::button_with_icon_size(ui, "rotate-cw", 28.0, 18.0, false, "Reset preset"))
            .inner
            .clicked()
        {
            reset(app, fields);
        }
        if ui.button(tl!("New…")).clicked() {
            let mut params = crate::filter_dialog::params_of(fields);
            if super::skin_smooth_ui::owns(fields)
                && let Some(params) = params.as_object_mut()
            {
                params.remove("positiveColors");
                params.remove("negativeColors");
            }
            let f = json!({NEW: true, "__command":"plugin.preset.save", "__label":"New Preset", "__parent":parent,
                "id":id, "name":"", "params":params});
            app.ui.open_dialog(crate::state::DialogKind::Command, f.as_object().cloned().unwrap_or_default());
        }
        let saved_name = selection.strip_prefix("saved:");
        if ui.add_enabled(saved_name.is_some(), egui::Button::new(tl!("Delete"))).clicked()
            && let Some(name) = saved_name
        {
            if let Err(error) = app.run("plugin.preset.remove", json!({"id":id,"name":name})) {
                crate::notices::error(app, error);
            } else {
                select(app, fields, &default_selection(m));
            }
        }
    });
    ui.separator();
}

pub(super) fn validate_new(app: &PhotocraftApp, fields: &Map<String, Value>) -> Result<(), String> {
    let name = fields.get("name").and_then(Value::as_str).unwrap_or_default().trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(tl!("Enter a preset name (1–64 characters).").into());
    }
    let id = fields.get("id").and_then(Value::as_str).ok_or("missing plug-in id")?;
    if app.session.prefs().dialogs.get(&format!("plugin.presets.{id}")).and_then(|v| v.get(name)).is_some() {
        return Err(tl!("A preset with this name already exists.").into());
    }
    let parent = fields.get("__parent").and_then(Value::as_u64).ok_or("missing preset dialog")?;
    if !app.ui.dialogs.iter().any(|d| {
        d.id == parent && d.fields.get("id").and_then(Value::as_str) == Some(id) && d.fields.get("__command").and_then(Value::as_str) == Some("plugin.run")
    }) {
        return Err(tl!("The filter dialog was closed.").into());
    }
    Ok(())
}

pub(super) fn new_body(app: &PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    let mut name = fields.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
    ui.label(tl!("Preset name"));
    let response = ui.add(egui::TextEdit::singleline(&mut name).char_limit(64).desired_width(280.0)).accessible_name(tl!("Preset name"));
    if !ui.is_sizing_pass() && !fields.contains_key("__focused") {
        response.request_focus();
        fields.insert("__focused".into(), json!(true));
    }
    fields.insert("name".into(), json!(name));
    if !fields.get("name").and_then(Value::as_str).unwrap_or_default().trim().is_empty()
        && let Err(error) = validate_new(app, fields)
    {
        ui.add(egui::Label::new(error).wrap());
    }
}

pub(super) fn confirm_new(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    validate_new(app, fields)?;
    let result = app.run("plugin.preset.save", json!({"id":fields.get("id"), "name":fields.get("name"), "params":fields.get("params")}))?;
    let parent = fields.get("__parent").and_then(Value::as_u64).ok_or("missing preset dialog")?;
    let name = fields.get("name").and_then(Value::as_str).unwrap_or_default().trim();
    let mut f = app.ui.dialogs.iter().find(|d| d.id == parent).map(|d| d.fields.clone()).ok_or("the filter dialog was closed")?;
    if !select(app, &mut f, &format!("saved:{name}")) {
        return Err("the saved preset could not be loaded".into());
    }
    remember_choices(app, &f);
    if let Some(d) = app.ui.dialog_mut(parent) {
        d.fields = f;
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
