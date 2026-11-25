//! The hosted filter form for installed plug-ins.

use craft::ui::{
    contract::{Bind, CanvasGesture, Sampler as _},
    dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id},
    patterns::form_dialogs::FormDialog,
};
use photocraft_engine::plugin_cmds::RUN;
use photocraft_plugins::manifest::{Manifest, ParamSpec};
use serde_json::{Map, Value, json};

use super::{filter_dialog, plugin_presets, skin_smooth_ui};
use crate::PhotocraftApp;

pub(crate) struct PluginDialog {
    pub(crate) fields: Map<String, Value>,
    title: String,
    opening: Option<Map<String, Value>>,
    canvas: Option<egui::Id>,
    presets: Map<String, Value>,
    action: Option<plugin_presets::Action>,
    sampling: Result<(), &'static str>,
    mask_error: Option<String>,
    busy: Option<&'static str>,
    can_confirm: bool,
}

pub(super) fn label(key: &str) -> String {
    let mut text = String::new();
    let camel_case = !key.contains(' ');
    for (i, ch) in key.chars().enumerate() {
        if i == 0 {
            text.extend(ch.to_uppercase());
        } else {
            if ch.is_uppercase() && camel_case {
                text.push(' ');
            }
            text.push(ch);
        }
    }
    tl!(&text).to_owned()
}

fn params_ui(ui: &mut egui::Ui, manifest: &Manifest, fields: &mut Map<String, Value>) {
    for (key, spec) in &manifest.params {
        if key == "preset" && fields.contains_key("__presetValues") && !manifest.preset_values.is_empty() {
            continue;
        }
        let label = label(key);
        ui.push_id(key, |ui| match spec {
            ParamSpec::Number { min, max, default, .. } => {
                let mut v = fields.get(key).and_then(Value::as_f64).unwrap_or(*default);
                ui.add(egui::Slider::new(&mut v, *min..=*max).text(label));
                fields.insert(key.clone(), json!(v));
            }
            ParamSpec::Int { min, max, default, .. } => {
                let mut v = fields.get(key).and_then(Value::as_i64).unwrap_or(*default);
                ui.add(egui::Slider::new(&mut v, *min..=*max).text(label));
                fields.insert(key.clone(), json!(v));
            }
            ParamSpec::Bool { default } => {
                let mut v = fields.get(key).and_then(Value::as_bool).unwrap_or(*default);
                ui.checkbox(&mut v, label);
                fields.insert(key.clone(), json!(v));
            }
            ParamSpec::Choice { options, .. } => {
                let mut v = fields.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
                egui::ComboBox::from_label(label).selected_text(self::label(&v)).show_ui(ui, |ui| {
                    for option in options {
                        ui.selectable_value(&mut v, option.clone(), self::label(option));
                    }
                });
                fields.insert(key.clone(), json!(v));
            }
            ParamSpec::Colors => {}
        });
    }
}

impl StackDialog for PluginDialog {
    type Result = Map<String, Value>;

    fn id(&self) -> DialogId {
        dialog_id(RUN)
    }

    fn key(&self) -> Option<&str> {
        Some(RUN)
    }

    fn canvas(&self) -> Option<egui::Id> {
        self.canvas
    }

    fn show(&mut self, ui: &mut egui::Ui, _stack: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let plugin = self.fields.get("id").and_then(Value::as_str).and_then(photocraft_plugins::registry::get);
        let skin = skin_smooth_ui::owns(&self.fields);
        let mut preview = self.fields.get("__preview").and_then(Value::as_bool).unwrap_or(false);
        let mut mask = self.fields.get("__skinMask").and_then(Value::as_bool).unwrap_or(false);
        let mut form = FormDialog::new(self.title.clone(), Bind::bound(&mut self.fields))
            .enter_confirms(false)
            .preview(Some(Bind::bound(&mut preview)))
            .footer_extra(|ui| {
                if skin {
                    skin_smooth_ui::mask_switch(self.sampling.is_ok(), ui, &mut mask);
                }
                if let Some(msg) = self.busy {
                    ui.spinner();
                    ui.label(tl!(&msg));
                }
            })
            .reset_with(|fields, opening, _| {
                let mut reset = fields.clone();
                if reset.contains_key("__presetValues") {
                    plugin_presets::reset(&self.presets, &mut reset);
                } else {
                    let Some(plugin) = &plugin else { return reset };
                    for (key, _) in &plugin.manifest().params {
                        if let Some(value) = opening.get(key) {
                            reset.insert(key.clone(), value.clone());
                        }
                    }
                }
                reset
            });
        if let Some(opening) = self.opening.take() {
            form = form.opening(opening);
        }
        let mut action = None;
        let out = form.show(ui, |ui, cx, fields| {
            let Some(plugin) = &plugin else { return false };
            action = plugin_presets::body(&self.presets, ui, fields);
            if skin {
                skin_smooth_ui::body(self.sampling, self.mask_error.as_deref(), ui, fields, cx.first());
            } else {
                params_ui(ui, plugin.manifest(), fields);
            }
            plugin_presets::sync_values(fields);
            if let Some(canvas) = self.canvas {
                cx.keep_view_open(canvas);
                if skin && self.sampling.is_ok() && fields.contains_key("__skinTool") && fields.get("skinOnly").and_then(Value::as_bool) == Some(true) {
                    cx.pass_canvas_gesture(canvas, CanvasGesture::PRIMARY);
                }
            }
            self.can_confirm
        });
        self.fields.insert("__preview".into(), json!(preview));
        if skin {
            self.fields.insert("__skinMask".into(), json!(mask));
        }
        if action.is_some() {
            self.action = action;
        }
        match (out.accepted, out.closed) {
            (Some(_), _) => Some(DialogResult::Ok(self.fields.clone())),
            (_, Some(_)) => Some(DialogResult::Cancel),
            _ => None,
        }
    }

    fn confirm(&mut self, _ctx: &egui::Context) -> Option<Self::Result> {
        Some(self.fields.clone())
    }

    fn json_fields(&self) -> Option<Value> {
        Some(Value::Object(self.fields.clone()))
    }

    fn set_json_field(&mut self, _ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let current = self.fields.get(field).ok_or_else(|| format!("no plug-in field {field}"))?;
        if field == "id" || field == "target" || (field.starts_with("__") && !matches!(field, "__preview" | "__skinMask" | "__skinTool")) {
            return Err(format!("plug-in field {field} is not editable"));
        }
        if !current.is_null() && !value.is_null() && std::mem::discriminant(current) != std::mem::discriminant(&value) {
            return Err(format!("invalid value for plug-in field {field}"));
        }
        self.fields.insert(field.into(), value);
        Ok(())
    }
}

pub(super) fn open(app: &mut PhotocraftApp, title: &str, mut fields: Map<String, Value>) -> DialogId {
    let id = fields.get("id").and_then(Value::as_str).unwrap_or_default();
    let presets = plugin_presets::saved(app, id);
    if let Some(plugin) = photocraft_plugins::registry::get(id) {
        fields.entry("__preview").or_insert(json!(plugin.manifest().preview_default));
        if let Ok(defaults) = plugin.manifest().resolve_params(&Value::Null) {
            for (key, value) in defaults {
                fields.entry(key).or_insert(value);
            }
        }
    }
    fields.insert("__command".into(), json!(RUN));
    if !fields.contains_key("target")
        && let Some(target) = filter_dialog::captured_target(app, RUN, true)
    {
        fields.insert("target".into(), target);
    }
    plugin_presets::restore_choices(app, &mut fields);
    if skin_smooth_ui::owns(&fields) {
        skin_smooth_ui::bind(app, &mut fields);
    }
    let canvas = app.session.active_index().map(|idx| crate::canvas::document_display::CanvasHost::primary(app, idx).canvas_id());
    app.framework.dialogs.open(PluginDialog {
        opening: Some(fields.clone()),
        fields,
        title: title.trim_end_matches('…').to_owned(),
        canvas,
        presets,
        action: None,
        sampling: skin_smooth_ui::supported(app).map(|_| ()),
        mask_error: None,
        busy: None,
        can_confirm: true,
    })
}

pub(super) fn top(app: &PhotocraftApp) -> Option<&PluginDialog> {
    app.framework.dialogs.get_by_id(app.framework.dialogs.stack().top()?)
}

pub(super) fn get_mut(app: &mut PhotocraftApp) -> Option<&mut PluginDialog> {
    app.framework.dialogs.get_mut()
}

pub(crate) fn command(app: &PhotocraftApp, id: DialogId) -> Option<(String, Value)> {
    if let Some(d) = app.framework.dialogs.get_by_id::<PluginDialog>(id) {
        return Some((RUN.into(), app.with_mask_target(RUN, filter_dialog::params_of(&d.fields))));
    }
    let d = app.framework.dialogs.get_by_id::<plugin_presets::PresetName>(id)?;
    Some((d.key()?.into(), d.json_fields()?))
}

pub(crate) fn preview(app: &PhotocraftApp) -> Option<(DialogId, Value)> {
    let d = app.framework.dialogs.get::<PluginDialog>()?;
    (d.fields.get("__preview").and_then(Value::as_bool) == Some(true) && valid_source(app, &d.fields))
        .then(|| (d.id(), app.with_mask_target(RUN, filter_dialog::params_of(&d.fields))))
}

fn valid_source(app: &PhotocraftApp, fields: &Map<String, Value>) -> bool {
    !skin_smooth_ui::owns(fields) || fields.get("__skinSource") == Some(&skin_smooth_ui::source(app))
}

pub(super) fn check(app: &mut PhotocraftApp, id: DialogId) -> Result<(), String> {
    if let Some(d) = app.framework.dialogs.get_by_id::<PluginDialog>(id)
        && !valid_source(app, &d.fields)
    {
        app.filter_preview = None;
        app.skin_smooth_mask = None;
        return Err("the Skin Smooth source changed; reopen the dialog".into());
    }
    if let Some(d) = app.framework.dialogs.get_by_id::<plugin_presets::PresetName>(id) {
        plugin_presets::check_new(app, d)?;
        plugin_presets::feed_new(app);
    }
    Ok(())
}

pub(super) fn feed(app: &mut PhotocraftApp) {
    plugin_presets::feed_new(app);
    let Some(d) = app.framework.dialogs.get::<PluginDialog>() else { return };
    let id = d.id();
    let can_confirm = valid_source(app, &d.fields);
    let presets = plugin_presets::saved(app, d.fields.get("id").and_then(Value::as_str).unwrap_or_default());
    let skin = skin_smooth_ui::owns(&d.fields);
    let sampling = if skin { skin_smooth_ui::supported(app).map(|_| ()) } else { Ok(()) };
    let canvas = app.session.active_index().map(|idx| crate::canvas::document_display::CanvasHost::primary(app, idx).canvas_id());
    let mask_error = skin_smooth_ui::mask_error(app);
    #[cfg(not(target_arch = "wasm32"))]
    let busy = if skin_smooth_ui::busy_for(app, id) {
        Some("Updating Mask…")
    } else if app.filter_preview_worker.busy_for(id) {
        Some("Updating preview…")
    } else {
        None
    };
    #[cfg(target_arch = "wasm32")]
    let busy = None;
    if let Some(d) = get_mut(app) {
        d.can_confirm = can_confirm;
        d.canvas = canvas;
        d.presets = presets;
        d.sampling = sampling;
        d.mask_error = mask_error;
        d.busy = busy;
    }
}

pub(super) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    while let Some(answer) = app.framework.dialogs.take::<PluginDialog>() {
        let id = dialog_id(RUN);
        let token = crate::actions::token_of(app, id);
        app.skin_smooth_mask = None;
        let outcome = match answer {
            DialogResult::Ok(fields) if valid_source(app, &fields) => {
                plugin_presets::remember_choices(app, &fields);
                let params = app.with_mask_target(RUN, filter_dialog::params_of(&fields));
                if crate::actions::record_again::owns_dialog(app, id) {
                    crate::actions::record_again::confirm(app, id, RUN, params)
                } else {
                    crate::actions::run_step_dialog(app, id, RUN, params)
                }
            }
            DialogResult::Ok(_) => Err("the Skin Smooth source changed; reopen the dialog".into()),
            _ => {
                app.filter_preview = None;
                if let Some(token) = token {
                    crate::actions::abandoned(app, token);
                }
                continue;
            }
        };
        let running = app
            .session
            .active()
            .is_some_and(|st| app.filter_preview.as_ref().is_some_and(|p| p.key.doc == st.doc.id) && app.session.job_on(st.doc.id).is_some());
        match app.filter_preview.as_mut() {
            Some(p) if outcome.is_ok() && running => p.committing = true,
            _ => app.filter_preview = None,
        }
        match outcome {
            Ok(_) => {
                if let Some(token) = token
                    && let Err(error) = crate::actions::resume(app, token)
                {
                    crate::notices::error(app, error);
                }
            }
            Err(error) => {
                if let Some(token) = token {
                    crate::actions::abandoned(app, token);
                }
                crate::notices::error(app, error);
            }
        }
    }
    while let Some(answer) = app.framework.dialogs.take::<plugin_presets::PresetName>() {
        if let DialogResult::Ok(params) = answer {
            let plugin = params["id"].as_str().unwrap_or_default().to_owned();
            let name = params["name"].as_str().unwrap_or_default().to_owned();
            match app.run("plugin.preset.save", params) {
                Ok(_) => plugin_presets::select_after_write(app, &plugin, Some(&name)),
                Err(error) => crate::notices::error(app, error),
            }
        }
    }
    let Some(d) = get_mut(app) else { return };
    let id = d.id();
    let plugin = d.fields["id"].as_str().unwrap_or_default().to_owned();
    let action = d.action.take();
    let fields = d.fields.clone();
    plugin_presets::remember_choices(app, &fields);
    match action {
        Some(plugin_presets::Action::New(params)) => plugin_presets::open_new(app, id, plugin, params),
        Some(plugin_presets::Action::Delete(name)) => match app.run("plugin.preset.remove", json!({"id": plugin, "name": name})) {
            Ok(_) => plugin_presets::select_after_write(app, &plugin, None),
            Err(error) => crate::notices::error(app, error),
        },
        None => {}
    }
}
