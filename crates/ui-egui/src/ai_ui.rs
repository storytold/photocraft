//! AI forms and previews. All actions dispatch engine commands; state is dialog data.
use crate::{PhotocraftApp, state::DialogKind, theme::Tokens};
use egui::{RichText, TextureOptions};
use serde_json::{Map, Value, json};

const COMMANDS: &[&str] = &["edit.generateImage", "edit.aiEdit", "edit.generativeFill", "edit.generativeRemove"];
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key("__ai")
}
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id == "ai.result.list" {
        return Some(Ok(json!({"dialog":open_results(app)})));
    }
    if !COMMANDS.contains(&id) || params.as_object().is_some_and(|p| !p.is_empty()) {
        return None;
    }
    let Some(spec) = photocraft_engine::command_specs().iter().find(|c| c.id == id) else {
        return Some(Err("Unknown AI command".into()));
    };
    if let Err(reason) = (spec.enabled)(&app.session) {
        return Some(Err(reason));
    }
    let fields = Map::from_iter([
        ("__ai".into(), json!("generate")),
        ("__command".into(), json!(id)),
        ("__label".into(), json!(spec.label)),
        ("prompt".into(), json!("")),
        ("seed".into(), json!(0)),
        ("contextMargin".into(), json!(0)),
    ]);
    Some(Ok(json!({"dialog":app.ui.open_dialog(DialogKind::Command,fields)})))
}
pub fn open_results(app: &mut PhotocraftApp) -> u64 {
    if let Some(dialog) = app.ui.dialogs.iter().find(|d| d.fields.get("__ai").and_then(Value::as_str) == Some("results")) {
        return dialog.id;
    }
    app.ui.open_dialog(
        DialogKind::Command,
        Map::from_iter([("__ai".into(), json!("results")), ("__command".into(), json!("ai.result.list")), ("__label".into(), json!("AI Results"))]),
    )
}
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    if fields.get("__ai").and_then(Value::as_str) == Some("results") {
        results(app, ui);
        return;
    }
    let generation = fields.get("__command").and_then(Value::as_str) == Some("edit.generateImage");
    let integrations = &app.session.prefs().integrations;
    let dim = Tokens::get(ui.ctx()).text_dim;
    // What leaves the machine, and where it goes, before the first request.
    let sends = if generation { tl!("Sends your prompt to") } else { tl!("Sends your prompt, a working copy of the image and the selection mask to") };
    ui.label(RichText::new(format!("{sends} {}", integrations.openai_image_endpoint)).color(dim));
    if generation {
        output_size(app, ui, fields);
    }
    fields.entry("imageModel").or_insert(json!("gpt-image-2.5-sunburst"));
    text(ui, fields, "imageModel", "Image model", 128);
    let mut quality = fields.get("imageQuality").and_then(Value::as_str).unwrap_or("low").to_owned();
    ui.horizontal(|ui| {
        ui.label(tl!("Image quality"));
        egui::ComboBox::from_id_salt("ai-image-quality").selected_text(&quality).show_ui(ui, |ui| {
            for choice in ["low", "medium", "high", "xhigh", "max", "auto"] {
                ui.selectable_value(&mut quality, choice.into(), choice);
            }
        });
    });
    fields.insert("imageQuality".into(), json!(quality));
    ui.label(
        RichText::new(tl!("The result must fit the selected model's limits (up to 4K for GPT Image 2/2.5). API usage is billed by your provider."))
            .small()
            .color(dim),
    );
    ui.label(tl!("Prompt"));
    let mut prompt = fields.get("prompt").and_then(Value::as_str).unwrap_or_default().to_owned();
    ui.add(egui::TextEdit::multiline(&mut prompt).desired_width(400.0).desired_rows(4).char_limit(4096));
    fields.insert("prompt".into(), json!(prompt));
    ui.collapsing(tl!("Advanced"), |ui| {
        number(ui, fields, "contextMargin", "Context margin", 1024);
    });
    ui.label(RichText::new(tl!("AI creates a preview. Apply it as a new layer after reviewing the result.")).color(dim));
    ui.add_space(8.0);
    if crate::widgets::secondary_button(ui, tl!("AI Settings…"), 150.0).clicked() {
        let _ = crate::menus::invoke(app, ui.ctx(), "edit.preferences.integrations", json!({}));
    }
}
fn output_size(app: &PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    let mut preset = fields.get("__sizePreset").and_then(Value::as_str).unwrap_or("document").to_owned();
    ui.horizontal(|ui| {
        ui.label(tl!("Output size"));
        egui::ComboBox::from_id_salt("ai-output-size")
            .selected_text(match preset.as_str() {
                "4k" => "4K · 3840 × 2160",
                "8k" => "8K · 7680 × 4320",
                "custom" => tl!("Custom"),
                _ => tl!("Current document"),
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut preset, "document".into(), tl!("Current document"));
                ui.selectable_value(&mut preset, "4k".into(), "4K · 3840 × 2160");
                ui.selectable_value(&mut preset, "8k".into(), "8K · 7680 × 4320");
                ui.selectable_value(&mut preset, "custom".into(), tl!("Custom"));
            });
    });
    match preset.as_str() {
        "document" => {
            fields.remove("outputSize");
        }
        "4k" => {
            fields.insert("outputSize".into(), json!({"width":3840,"height":2160}));
        }
        "8k" => {
            fields.insert("outputSize".into(), json!({"width":7680,"height":4320}));
        }
        _ => {
            let doc_size = app.session.active().map(|s| s.doc.size);
            let mut size = fields.get("outputSize").and_then(Value::as_object).cloned().unwrap_or_else(|| {
                Map::from_iter([("width".into(), json!(doc_size.map_or(3840, |s| s.width))), ("height".into(), json!(doc_size.map_or(2160, |s| s.height)))])
            });
            number(ui, &mut size, "width", "Width", 300_000);
            number(ui, &mut size, "height", "Height", 300_000);
            fields.insert("outputSize".into(), Value::Object(size));
        }
    }
    fields.insert("__sizePreset".into(), json!(preset));
    ui.label(RichText::new(tl!("The canvas size changes only when you apply the result.")).small().color(Tokens::get(ui.ctx()).text_dim));
}
fn text(ui: &mut egui::Ui, fields: &mut Map<String, Value>, key: &str, label: &str, limit: usize) {
    ui.horizontal(|ui| {
        ui.label(tl!(label));
        let mut value = fields.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        ui.add(egui::TextEdit::singleline(&mut value).char_limit(limit).desired_width(240.0));
        fields.insert(key.into(), json!(value));
    });
}
fn number(ui: &mut egui::Ui, fields: &mut Map<String, Value>, key: &str, label: &str, max: u64) {
    ui.horizontal(|ui| {
        ui.label(tl!(label));
        let mut value = fields.get(key).and_then(Value::as_u64).unwrap_or(0);
        ui.add(egui::DragValue::new(&mut value).range(0..=max));
        fields.insert(key.into(), json!(value));
    });
}
fn results(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let listing = match app.run("ai.result.list", json!({})) {
        Ok(v) => v,
        Err(e) => {
            ui.label(e);
            return;
        }
    };
    let rows = listing.get("results").and_then(Value::as_array).cloned().unwrap_or_default();
    let ids: Vec<u64> = rows.iter().filter_map(|row| row.get("result").and_then(Value::as_u64)).collect();
    ui.ctx().data_mut(|data| {
        let cache = egui::Id::unique("ai-preview-ids");
        for old in data.get_temp::<Vec<u64>>(cache).unwrap_or_default() {
            if !ids.contains(&old) {
                data.remove::<egui::TextureHandle>(egui::Id::unique(("ai-preview", old)));
            }
        }
        data.insert_temp(cache, ids);
    });
    if rows.is_empty() {
        ui.label(tl!("No AI results yet."));
        return;
    }
    egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
        for row in rows {
            let Some(id) = row.get("result").and_then(Value::as_u64) else {
                continue;
            };
            let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(true);
            ui.label(format!("{} #{id}", tl!("AI Result")));
            if let Some(document) = app.session.documents().iter().find(|state| Some(state.doc.id.0) == row.get("document").and_then(Value::as_u64)) {
                ui.label(format!("{}: {}", tl!("Document"), document.doc.name));
            }
            if let Some(preview) = app.session.ai_preview(id, 0) {
                let key = egui::Id::unique(("ai-preview", id));
                let cached = ui.ctx().data_mut(|data| data.get_temp::<egui::TextureHandle>(key));
                let texture = if let Some(texture) = cached {
                    texture
                } else {
                    // load_texture accesses Context itself: never call it while data_mut holds
                    // that context's lock, or the first result preview freezes the UI thread.
                    let texture = ui.ctx().load_texture(
                        format!("ai-result-{id}"),
                        egui::ColorImage::from_rgba_unmultiplied([preview.size.width as usize, preview.size.height as usize], &preview.rgba),
                        TextureOptions::LINEAR,
                    );
                    ui.ctx().data_mut(|data| data.insert_temp(key, texture.clone()));
                    texture
                };
                ui.add(egui::Image::new(&texture).max_size(egui::vec2(380.0, 260.0)));
            }
            for (key, label) in [("outputSize", "Output size"), ("workingSize", "AI working size")] {
                if let Some(size) = row.get(key) {
                    ui.label(format!(
                        "{}: {} × {} px",
                        tl!(label),
                        size.get("width").and_then(Value::as_u64).unwrap_or(0),
                        size.get("height").and_then(Value::as_u64).unwrap_or(0)
                    ));
                }
            }
            if stale {
                ui.label(RichText::new(tl!("The document changed. Generate a new result before applying.")).color(Tokens::get(ui.ctx()).danger));
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(!stale, egui::Button::new(tl!("Apply as New Layer"))).clicked()
                    && let Err(e) = app.run("ai.result.apply", json!({"result":id}))
                {
                    app.ui.status = e;
                    app.ui.status_error = true;
                }
                if crate::widgets::secondary_button(ui, tl!("Discard"), 90.0).clicked() {
                    let _ = app.run("ai.result.discard", json!({"result":id}));
                }
            });
            if crate::widgets::secondary_button(ui, tl!("Open as New Document"), 210.0).clicked()
                && let Err(e) = app.run("ai.result.openDocument", json!({"result":id,"name":format!("{} #{id}",tl!("AI Result"))}))
            {
                app.ui.status = e;
                app.ui.status_error = true;
            }
            ui.separator();
        }
    });
}
pub fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let command = fields.get("__command").and_then(Value::as_str).ok_or("AI dialog has no command")?;
    if command == "ai.result.list" {
        return Ok(Value::Null);
    }
    let params = fields.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect::<Map<_, _>>();
    app.run(command, Value::Object(params))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app_with_ai() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":64,"height":64})).unwrap();
        let endpoint = app.session.prefs().integrations.openai_image_endpoint.clone();
        app.run("prefs.set", json!({"path":"integrations.aiConsentedEndpoint","value":endpoint})).unwrap();
        app
    }
    #[test]
    fn ai_commands_stay_disabled_until_the_user_agrees_to_an_endpoint() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":64,"height":64})).unwrap();
        app.run("select.all", json!({})).unwrap();
        assert!(menu(&mut app, "edit.generateImage", &json!({})).unwrap().is_err());
        let items = crate::menus::menu_items(&app);
        for id in COMMANDS {
            assert!(items.iter().any(|i| i.id == *id && !i.enabled), "{id}");
        }
    }
    #[test]
    fn ai_menu_and_selection_context_rows_are_live_once_configured() {
        let mut app = app_with_ai();
        app.run("select.all", json!({})).unwrap();
        let items = crate::menus::menu_items(&app);
        for id in COMMANDS {
            assert!(items.iter().any(|i| i.id == *id && i.enabled), "{id}");
        }
        assert!(crate::canvas_tool_menu::SELECTION_MENU.iter().flatten().any(|(_, id)| *id == "edit.generativeFill"));
    }
    #[test]
    fn the_form_names_the_endpoint_and_retains_invalid_drafts_without_network() {
        let mut app = app_with_ai();
        let response = menu(&mut app, "edit.generateImage", &json!({})).unwrap().unwrap();
        let id = response["dialog"].as_u64().unwrap();
        assert!(app.session.jobs().is_empty());
        assert!(crate::dialogs::confirm(&mut app, id).is_err());
        assert!(app.ui.dialogs.iter().any(|d| d.id == id));
        crate::dialogs::cancel(&mut app, id).unwrap();
    }
    #[test]
    fn generation_presets_keep_output_dimensions_in_dialog_state_without_editing_the_document() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":64,"height":64})).unwrap();
        let original = app.session.active().unwrap().doc.clone();
        let context = egui::Context::default();
        PhotocraftApp::setup_context(&context, Default::default());
        let mut fields = Map::new();
        for (preset, expected) in
            [("4k", json!({"width":3840,"height":2160})), ("8k", json!({"width":7680,"height":4320})), ("custom", json!({"width":7680,"height":4320}))]
        {
            fields.insert("__sizePreset".into(), json!(preset));
            context.run_ui(Default::default(), |ui| output_size(&app, ui, &mut fields)).textures_delta.clear();
            assert_eq!(fields["outputSize"], expected);
            assert_eq!(app.session.active().unwrap().doc, original);
        }
        fields.insert("__sizePreset".into(), json!("document"));
        context.run_ui(Default::default(), |ui| output_size(&app, ui, &mut fields)).textures_delta.clear();
        assert!(!fields.contains_key("outputSize"));
        assert!(app.session.jobs().is_empty());
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn ready_preview_renders_and_reuses_texture_without_locking_the_context() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            use photocraft_ai::{AiRequest, AiResult, Artifact, ImageSize, Operation, RgbaImage, WorkflowId};
            use photocraft_engine::ai_cmds::{Snapshot, Source};
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width":64,"height":64})).unwrap();
            let size = ImageSize { width: 64, height: 64 };
            let input = Snapshot::capture(app.session.active().unwrap(), Source::Composite, 0)
                .unwrap()
                .prepare(AiRequest {
                    operation: Operation::Generate,
                    workflow: WorkflowId { name: "test".into(), revision: 1 },
                    prompt: "synthetic preview".into(),
                    negative_prompt: None,
                    size,
                    variants: 1,
                    seed: 0,
                    source: None,
                    edit_mask: None,
                    references: vec![],
                })
                .unwrap();
            let result = input
                .finish(AiResult { provider_job: None, artifacts: vec![Artifact::Image(RgbaImage::new(size, [0.1, 0.2, 0.8, 1.0].repeat(64 * 64)).unwrap())] })
                .unwrap();
            let id = app.session.retain_ai_result(result).unwrap();
            let context = egui::Context::default();
            PhotocraftApp::setup_context(&context, Default::default());
            for _ in 0..2 {
                context.run_ui(Default::default(), |ui| results(&mut app, ui)).textures_delta.clear();
                assert!(context.data_mut(|data| data.get_temp::<egui::TextureHandle>(egui::Id::unique(("ai-preview", id)))).is_some());
            }
            sender.send(()).unwrap();
        });
        receiver.recv_timeout(std::time::Duration::from_secs(15)).expect("AI preview rendering stalled or failed");
        worker.join().unwrap();
    }
}
