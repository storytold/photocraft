//! Dialogs rendered from `UiState::dialogs`. Field values live in the dialog data, so automation can
//! set them (`ui.dialog.set`) and confirm (`ui.dialog.confirm`) exactly like a user.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::{Dialog, DialogKind};

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let dialogs = app.ui.dialogs.clone();
    for d in dialogs {
        let mut fields = d.fields.clone();
        let mut outcome: Option<bool> = None; // Some(true)=OK, Some(false)=Cancel
        let title = title(&d);
        // Photoshop doesn't dim the window behind dialogs: previews must be judged at true contrast.
        let modal = egui::Modal::new(egui::Id::new(("dialog", d.id))).backdrop_color(egui::Color32::TRANSPARENT).show(ctx, |ui| {
            ui.set_min_width(380.0);
            let wide = crate::prefs_ui::width(&d.fields);
            if let Some(w) = wide {
                ui.set_min_width(w.min(460.0));
            }
            if d.kind == DialogKind::NewDocument {
                ui.set_min_width(800.0);
            }
            ui.set_max_width(wide.unwrap_or(if d.kind == DialogKind::NewDocument {
                800.0
            } else if d.kind == DialogKind::LayerStyle || d.fields.contains_key("__export") || crate::color_picker_ui::owns(&d.fields) {
                600.0
            } else {
                440.0
            }));
            if let Some(w) = crate::file_ui::dialog_width(&d.fields) {
                ui.set_min_width(w);
                ui.set_max_width(w);
            }
            ui.label(egui::RichText::new(&title).font(crate::theme::semibold(15.0)));
            ui.add_space(4.0);
            crate::widgets::hairline(ui);
            ui.add_space(8.0);
            match d.kind {
                DialogKind::NewDocument => crate::new_doc_ui::body(ui, &mut fields),
                DialogKind::About => {
                    ui.label("PhotoCraft — an open-source, native image editor written in Rust.");
                    ui.label(format!("Version {}", photocraft_engine::build_info::long_version()));
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| {
                        crate::links::discord_button(app, ui, 220.0);
                        ui.add_space(8.0);
                        crate::links::link_row(app, ui);
                    });
                    ui.add_space(10.0);
                    ui.weak("egui · wgpu · photocraft-engine");
                }
                DialogKind::Command if crate::variables_ui::owns(&fields) => crate::variables_ui::body(app, ui, &mut fields),
                DialogKind::Command if crate::file_ui::owns(&fields) => crate::file_ui::body(app, ui, &mut fields),
                DialogKind::Command if crate::color_picker_ui::owns(&fields) => crate::color_picker_ui::body(ui, &mut fields),
                DialogKind::Command if crate::prefs_ui::owns(&fields) => crate::prefs_ui::body(app, ui, &mut fields),
                DialogKind::Command if fields.contains_key("__export") => crate::export_dialog::body(app, ui, &mut fields),
                DialogKind::Command if fields.contains_key("__sizing") => crate::sizing::body(ui, &mut fields),
                DialogKind::Command if fields.contains_key("__filter") => crate::filter_dialog::body(ui, &mut fields),
                DialogKind::Command if fields.contains_key("__form") => crate::view_cmds::form_body(ui, &mut fields),
                DialogKind::Command => command_fields(ui, &mut fields),
                DialogKind::LayerStyle => crate::layer_style::body(ui, &mut fields),
                DialogKind::Error => {
                    ui.label(fields.get("message").and_then(Value::as_str).unwrap_or("Error"));
                }
            }
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if matches!(d.kind, DialogKind::About | DialogKind::Error) {
                    if crate::widgets::primary_button(ui, "OK", 84.0).clicked() {
                        outcome = Some(false);
                    }
                } else {
                    let ok_label = if d.kind == DialogKind::NewDocument {
                        "Create"
                    } else if d.fields.contains_key("__export") {
                        "Export"
                    } else {
                        crate::file_ui::ok_label(&d.fields).unwrap_or("OK")
                    };
                    if crate::widgets::primary_button(ui, ok_label, 84.0).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        outcome = Some(true);
                    }
                    if crate::widgets::secondary_button(ui, if d.kind == DialogKind::NewDocument { "Close" } else { "Cancel" }, 84.0).clicked() {
                        outcome = Some(false);
                    }
                }
            });
        });
        if modal.should_close() && outcome.is_none() {
            outcome = Some(false);
        }
        if let Some(dm) = app.ui.dialog_mut(d.id) {
            dm.fields = fields;
        }
        match outcome {
            Some(true) => {
                let _ = confirm(app, d.id);
            }
            Some(false) => {
                app.ui.close_dialog(d.id);
                app.filter_preview = None;
            }
            None => {}
        }
    }
}

pub fn title(d: &Dialog) -> String {
    match d.kind {
        DialogKind::NewDocument => "New Document".into(),
        DialogKind::About => "About PhotoCraft".into(),
        DialogKind::LayerStyle => "Layer Style".into(),
        DialogKind::Command => d.fields.get("__label").and_then(Value::as_str).unwrap_or("Command").trim_end_matches('…').to_string(),
        DialogKind::Error => "Error".into(),
    }
}

/// Confirm a dialog: run its action and close it. Used by the OK button and by automation.
pub fn confirm(app: &mut PhotocraftApp, id: u64) -> Result<Value, String> {
    let d = app.ui.close_dialog(id).ok_or_else(|| format!("no dialog {id}"))?;
    match d.kind {
        DialogKind::NewDocument => {
            let r = app.run("file.new", crate::new_doc_ui::command_params(&d.fields));
            if let Some(i) = app.session.active_index() {
                app.ui.views[i].fit_pending = true;
            }
            r
        }
        DialogKind::Command if crate::variables_ui::owns(&d.fields) => crate::variables_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::file_ui::owns(&d.fields) => crate::file_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::color_picker_ui::owns(&d.fields) => crate::color_picker_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::prefs_ui::owns(&d.fields) => crate::prefs_ui::confirm(app, &d.fields),
        DialogKind::Command if d.fields.contains_key("__export") => crate::export_dialog::confirm(app, &d.fields),
        DialogKind::Command => {
            app.filter_preview = None;
            let cmd = d.fields.get("__command").and_then(|v| v.as_str().map(str::to_string)).ok_or("dialog has no command")?;
            let (cmd, params) = crate::smart_ui::confirm_command(&d.fields, cmd, crate::filter_dialog::params_of(&d.fields));
            app.run(&cmd, params)
        }
        DialogKind::LayerStyle => crate::layer_style::confirm(app, &d.fields),
        DialogKind::About | DialogKind::Error => Ok(Value::Null),
    }
}

fn command_fields(ui: &mut egui::Ui, f: &mut serde_json::Map<String, Value>) {
    let kind = f.get("__kind").and_then(Value::as_str).unwrap_or_default().to_string();
    for &(key, label, min, max, default) in crate::panels::adjustment_sliders(&kind) {
        let mut v = f.get(key).and_then(Value::as_f64).unwrap_or(default as f64) as f32;
        if ui.add(egui::Slider::new(&mut v, min..=max).text(label)).changed() {
            f.insert(key.into(), json!(v));
        }
    }
    if kind == "hueSaturation" {
        let mut c = f.get("colorize").and_then(Value::as_bool).unwrap_or(false);
        if ui.checkbox(&mut c, "Colorize").changed() {
            f.insert("colorize".into(), json!(c));
        }
    }
}

/// Open a parameter dialog for a destructive `image.adjustments.*` command.
pub fn open_command_dialog(app: &mut PhotocraftApp, command: &str, label: &str) -> u64 {
    let kind = command.rsplit('.').next().unwrap_or_default();
    let mut fields = serde_json::Map::new();
    fields.insert("__command".into(), json!(command));
    fields.insert("__label".into(), json!(label));
    fields.insert("__kind".into(), json!(kind));
    for &(key, _, _, _, default) in crate::panels::adjustment_sliders(kind) {
        fields.insert(key.into(), json!(default));
    }
    app.ui.open_dialog(DialogKind::Command, fields)
}
