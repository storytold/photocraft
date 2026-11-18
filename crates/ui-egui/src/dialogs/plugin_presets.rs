//! Plug-in preset selection, editing baseline, and persistent view choices.

use craft::ui::{
    contract::Bind,
    dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id},
    patterns::form_dialogs::FormDialog,
    runtime::overlay::CloseReason,
};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

pub(super) enum Action {
    New(Value),
    Delete(String),
}

pub(super) fn saved(app: &PhotocraftApp, id: &str) -> Map<String, Value> {
    app.session.prefs().dialogs.get(&format!("plugin.presets.{id}")).and_then(Value::as_object).cloned().unwrap_or_default()
}

/// Reports whether installed manifest presets let the host manage the preset choice.
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

/// Prefers the natural built-in preset, then the first built-in; empty means no built-in baseline.
fn default_selection(m: &photocraft_plugins::Manifest) -> String {
    m.preset_values.get_key_value("natural").or_else(|| m.preset_values.first_key_value()).map(|(k, _)| format!("builtin:{k}")).unwrap_or_default()
}

/// Restores independent view switches and the selected preset, falling back if that preset disappeared.
/// Unsaved slider drafts are not restored.
pub(super) fn restore_choices(app: &PhotocraftApp, fields: &mut Map<String, Value>) {
    let Some(plugin) = fields.get("id").and_then(Value::as_str).and_then(photocraft_plugins::registry::get) else { return };
    let choices = app.session.prefs().dialogs.get(&format!("plugin.choices.{}", plugin.id())).and_then(Value::as_object);
    for key in ["__preview", "skinOnly"] {
        if fields.get(key).is_some_and(Value::is_boolean)
            && let Some(v) = choices.and_then(|s| s.get(key)).filter(|v| v.is_boolean())
        {
            fields.insert(key.into(), v.clone());
        }
    }
    let selection = choices.and_then(|s| s.get("__pluginPreset")).and_then(Value::as_str).unwrap_or_default();
    let presets = saved(app, plugin.id());
    if !select(&presets, fields, selection) {
        select(&presets, fields, &default_selection(plugin.manifest()));
    }
}

/// Resolves built-in or saved values for editing without replacing independent switches or Skin Smooth samples.
/// Host-managed presets run in manual mode so subsequent slider edits take effect.
fn values(presets: &Map<String, Value>, fields: &Map<String, Value>, selection: &str) -> Option<Map<String, Value>> {
    let plugin = fields.get("id").and_then(Value::as_str).and_then(photocraft_plugins::registry::get)?;
    let m = plugin.manifest();
    let mut input = if let Some(name) = selection.strip_prefix("builtin:") {
        Value::Object(m.preset_values.get(name)?.clone())
    } else {
        presets.get(selection.strip_prefix("saved:")?)?.clone()
    };
    if !m.preset_values.is_empty() {
        input.as_object_mut()?.insert("preset".into(), json!("manual"));
    }
    let mut values = m.resolve_params(&input).ok()?;
    // These switches are independent of preset selection and Reset.
    if m.id == "local.skinsmooth.skin-smooth" {
        values.remove("skinOnly");
        values.remove("positiveColors");
        values.remove("negativeColors");
    }
    if !m.preset_values.is_empty() {
        // Keep the algorithm in manual mode so it uses the editable values and remembered Skin Only.
        values.insert("preset".into(), json!("manual"));
    }
    Some(values)
}

/// Applies an available preset and records its clean Reset baseline; returns false for an unavailable selection.
fn select(presets: &Map<String, Value>, fields: &mut Map<String, Value>, selection: &str) -> bool {
    if selection.is_empty() {
        for key in ["__pluginPreset", "__presetValues", "__presetDirty"] {
            fields.remove(key);
        }
        return false;
    }
    let Some(values) = values(presets, fields, selection) else { return false };
    fields.extend(values.clone());
    fields.insert("__presetValues".into(), Value::Object(values));
    fields.insert("__pluginPreset".into(), json!(selection));
    fields.insert("__presetDirty".into(), json!(false));
    true
}

/// Restores the currently selected preset, preserving switches and photo samples excluded from its baseline.
pub(super) fn reset(presets: &Map<String, Value>, fields: &mut Map<String, Value>) {
    let selection = fields.get("__pluginPreset").and_then(Value::as_str).unwrap_or_default().to_string();
    select(presets, fields, &selection);
}

/// Marks the selected baseline modified when editable values differ, ignoring a host-managed preset selector.
pub(super) fn sync_values(fields: &mut Map<String, Value>) {
    let Some(baseline) = fields.get("__presetValues").and_then(Value::as_object) else { return };
    let dirty = baseline.iter().any(|(k, v)| (k != "preset" || !manages_preset(fields)) && !same_value(fields.get(k), Some(v)));
    fields.insert("__presetDirty".into(), json!(dirty));
}

/// Treats numerically equal JSON integers and floats alike when comparing a draft with its baseline.
fn same_value(a: Option<&Value>, b: Option<&Value>) -> bool {
    a == b || a.zip(b).is_some_and(|(a, b)| a.as_f64().is_some() && a.as_f64() == b.as_f64())
}

/// Draws built-in and saved presets with Reset, New and Delete actions; repairs selections removed elsewhere.
pub(super) fn body(presets: &Map<String, Value>, ui: &mut egui::Ui, fields: &mut Map<String, Value>) -> Option<Action> {
    let plugin = fields.get("id").and_then(Value::as_str).and_then(photocraft_plugins::registry::get)?;
    let m = plugin.manifest();
    let id = plugin.id();
    let mut selection = fields.get("__pluginPreset").and_then(Value::as_str).unwrap_or_default().to_string();
    // A preset removed in another open dialog must not leave a stale Reset target.
    if !selection.is_empty() && values(presets, fields, &selection).is_none() {
        selection = default_selection(m);
        select(presets, fields, &selection);
    }
    let selected_name = if let Some(name) = selection.strip_prefix("builtin:") {
        super::plugin_dialog::label(name)
    } else {
        selection.strip_prefix("saved:").unwrap_or(tl!("Preset")).to_string()
    };
    let dirty = fields.get("__presetDirty").and_then(Value::as_bool) == Some(true);
    let label = if dirty { format!("{selected_name} ({})", tl!("Modified")) } else { selected_name };
    let action = ui
        .horizontal(|ui| {
            ui.label(tl!("Preset"));
            let width = (ui.available_width() - 170.0).clamp(120.0, 220.0);
            let before = selection.clone();
            let combo = ui.allocate_ui(egui::vec2(width, ui.spacing().interact_size.y), |ui| {
                ui.set_max_width(width);
                egui::ComboBox::from_id_salt(("plugin-presets", id))
                    .selected_text(&label)
                    .width(width)
                    .truncate()
                    .height(300.0)
                    .show_ui(ui, |ui| {
                        ui.set_max_width(360.0);
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                        if let Some((_, photocraft_plugins::manifest::ParamSpec::Choice { options, .. })) = m.params.iter().find(|(k, _)| k == "preset") {
                            for name in options.iter().filter(|n| m.preset_values.contains_key(*n)) {
                                ui.selectable_value(&mut selection, format!("builtin:{name}"), super::plugin_dialog::label(name));
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
            });
            combo.inner.accessible_name(tl!("Preset")).on_hover_text(&label);
            if selection != before {
                select(presets, fields, &selection);
            }
            let has_baseline = fields.contains_key("__presetValues");
            if ui.add_enabled_ui(dirty && has_baseline, |ui| ui.button(tl!("Reset")).accessible_name(tl!("Reset preset"))).inner.clicked() {
                reset(presets, fields);
            }
            if ui.button(tl!("New…")).clicked() {
                let mut params = crate::filter_dialog::params_of(fields);
                if super::skin_smooth_ui::owns(fields)
                    && let Some(params) = params.as_object_mut()
                {
                    params.remove("positiveColors");
                    params.remove("negativeColors");
                }
                return Some(Action::New(params));
            }
            let saved_name = selection.strip_prefix("saved:");
            if ui.add_enabled(saved_name.is_some(), egui::Button::new(tl!("Delete"))).clicked()
                && let Some(name) = saved_name
            {
                return Some(Action::Delete(name.to_owned()));
            }
            None
        })
        .inner;
    ui.separator();
    action
}

const KEY: &str = "plugin.preset.save";

pub(super) struct PresetName {
    pub(super) parent: DialogId,
    id: String,
    name: String,
    params: Value,
    presets: Map<String, Value>,
    parent_open: bool,
}

impl PresetName {
    fn validate(name: &str, presets: &Map<String, Value>, parent_open: bool) -> Result<(), String> {
        let name = name.trim();
        if !parent_open {
            return Err(tl!("The filter dialog was closed.").into());
        }
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
            return Err(tl!("Enter a preset name (1–64 characters).").into());
        }
        if presets.contains_key(name) {
            return Err(tl!("A preset with this name already exists.").into());
        }
        if presets.len() >= 64 {
            return Err(tl!("At most 64 presets per plug-in.").into());
        }
        Ok(())
    }

    fn params_with(&self, name: &str) -> Value {
        json!({"id": self.id, "name": name.trim(), "params": self.params})
    }
}

impl StackDialog for PresetName {
    type Result = Value;

    fn id(&self) -> DialogId {
        dialog_id(KEY)
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut egui::Ui, _stack: &DialogStack) -> Option<DialogResult<Value>> {
        let out = FormDialog::new(tl!("New Preset").to_string(), Bind::bound(&mut self.name))
            .ok_label(tl!("Save").to_string())
            .alt_resets(false)
            .enter_confirms(false)
            .show(ui, |ui, form, name| {
                let escape = !ui.is_sizing_pass() && ui.is_enabled() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                let enter = !ui.is_sizing_pass() && ui.is_enabled() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                let row = ui.add(egui::TextEdit::singleline(name).char_limit(64).desired_width(280.0)).accessible_name(tl!("Preset name"));
                if form.first() && !ui.is_sizing_pass() {
                    row.request_focus();
                }
                let check = Self::validate(name, &self.presets, self.parent_open);
                if !name.trim().is_empty()
                    && let Err(error) = &check
                {
                    ui.label(error);
                }
                if escape {
                    form.request_close_as(CloseReason::Escape);
                } else if enter && check.is_ok() {
                    form.request_close_as(CloseReason::Activated);
                }
                check.is_ok()
            });
        match (out.accepted, out.closed) {
            (Some(name), _) => Some(DialogResult::Ok(self.params_with(&name))),
            (_, Some(_)) => Some(DialogResult::Cancel),
            _ => None,
        }
    }

    fn confirm(&mut self, _ctx: &egui::Context) -> Option<Value> {
        Self::validate(&self.name, &self.presets, self.parent_open).ok()?;
        Some(self.params_with(&self.name))
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({"id": self.id, "name": self.name, "params": self.params}))
    }

    fn set_json_field(&mut self, _ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        if field != "name" {
            return Err(format!("no editable preset field {field}"));
        }
        self.name = value.as_str().ok_or("expected a preset name")?.to_owned();
        Ok(())
    }
}

pub(super) fn open_new(app: &mut PhotocraftApp, parent: DialogId, id: String, params: Value) {
    let presets = saved(app, &id);
    app.framework.dialogs.open_child(PresetName { parent, id, name: String::new(), params, presets, parent_open: true }, parent);
}

pub(super) fn check_new(app: &PhotocraftApp, dialog: &PresetName) -> Result<(), String> {
    let presets = saved(app, &dialog.id);
    let parent_open =
        app.framework.dialogs.get_by_id::<super::plugin_dialog::PluginDialog>(dialog.parent).is_some_and(|parent| parent.fields["id"] == dialog.id);
    PresetName::validate(&dialog.name, &presets, parent_open)
}

pub(super) fn feed_new(app: &mut PhotocraftApp) {
    let Some(dialog) = app.framework.dialogs.get::<PresetName>() else { return };
    let presets = saved(app, &dialog.id);
    let parent_open = app.framework.dialogs.get_by_id::<super::plugin_dialog::PluginDialog>(dialog.parent).is_some();
    if let Some(dialog) = app.framework.dialogs.get_mut::<PresetName>() {
        dialog.presets = presets;
        dialog.parent_open = parent_open;
    }
}

pub(super) fn select_after_write(app: &mut PhotocraftApp, id: &str, name: Option<&str>) {
    let Some(plugin) = photocraft_plugins::registry::get(id) else { return };
    let presets = saved(app, id);
    let Some(d) = app.framework.dialogs.get_mut::<super::plugin_dialog::PluginDialog>().filter(|d| d.fields["id"] == id) else { return };
    if select(&presets, &mut d.fields, &name.map_or_else(|| default_selection(plugin.manifest()), |name| format!("saved:{name}"))) {
        let fields = d.fields.clone();
        remember_choices(app, &fields);
    }
}

#[cfg(test)]
mod tests;
