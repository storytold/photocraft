//! AI forms and previews. All actions dispatch engine commands; state is dialog data.
use crate::{PhotocraftApp, state::DialogKind, theme::Tokens};
use egui::{RichText, TextureOptions};
use serde_json::{Map, Value, json};

const COMMANDS: &[&str] = &[
    "edit.aiDescribeImage",
    "edit.generateImage",
    "edit.aiEdit",
    "edit.aiBackground",
    "edit.generativeFill",
    "edit.generativeRemove",
    "layer.aiRemoveBackground",
    "image.generativeExpand",
    "image.aiUpscale",
];
pub fn open_dependencies(app: &mut PhotocraftApp, result: Option<&Value>) -> u64 {
    app.ui.open_dialog(
        DialogKind::Command,
        Map::from_iter([
            ("__ai".into(), json!("dependencies")),
            ("__command".into(), json!("comfy.dependencies.check")),
            ("__label".into(), json!("ComfyUI Dependencies")),
            ("__report".into(), result.cloned().unwrap_or(Value::Null)),
        ]),
    )
}
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key("__ai")
}
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id == "comfy.dependencies.check" && params.as_object().is_none_or(|p| p.is_empty()) {
        return Some(Ok(json!({"dialog":open_dependencies(app, None)})));
    }
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
    let mut fields = Map::from_iter([
        ("__ai".into(), json!("generate")),
        ("__command".into(), json!(id)),
        ("__label".into(), json!(spec.label)),
        ("prompt".into(), json!("")),
        ("negativePrompt".into(), json!("")),
        ("preparePrompt".into(), json!(false)),
        ("textModel".into(), json!(app.session.ai_prompt_model())),
        ("seed".into(), json!(0)),
        ("contextMargin".into(), json!(0)),
    ]);
    if id == "image.aiUpscale" {
        fields.retain(|key, _| key.starts_with("__"));
        fields.insert("scale".into(), json!(2));
    }
    if id == "edit.aiDescribeImage" {
        fields.retain(|key, _| key.starts_with("__") || key == "textModel");
        fields.insert("instruction".into(), json!(""));
    }
    Some(Ok(json!({"dialog":app.ui.open_dialog(DialogKind::Command,fields)})))
}
pub fn open_interpretation(app: &mut PhotocraftApp, result: &Value) {
    app.ui.open_dialog(
        DialogKind::Command,
        Map::from_iter([
            ("__ai".into(), json!("interpretation")),
            ("__command".into(), json!("edit.generateImage")),
            ("__label".into(), json!("Review Recreation Prompt")),
            ("description".into(), result.get("description").cloned().unwrap_or(Value::Null)),
            ("prompt".into(), result.get("positivePrompt").cloned().unwrap_or(Value::Null)),
            ("negativePrompt".into(), result.get("negativePrompt").cloned().unwrap_or(json!(""))),
        ]),
    );
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
    if fields.get("__ai").and_then(Value::as_str) == Some("dependencies") {
        dependency_form(app, ui, fields);
        return;
    }
    if fields.get("__ai").and_then(Value::as_str) == Some("interpretation") {
        ui.label(tl!("Image description"));
        ui.label(fields.get("description").and_then(Value::as_str).unwrap_or_default());
        text(ui, fields, "prompt", "Recreation prompt", 4096);
        text(ui, fields, "negativePrompt", "Negative prompt", 4096);
        ui.label(tl!("Review the prompt, then continue to image generation. Matching pixels are not guaranteed."));
        return;
    }
    if fields.get("__command").and_then(Value::as_str) == Some("edit.aiDescribeImage") {
        ui.label(tl!("Analyze the visible image and create an editable prompt for recreating it."));
        text(ui, fields, "textModel", "Prompt model", 128);
        text(ui, fields, "instruction", "Analysis instructions", 4096);
        ui.label(tl!("A working image up to 1024 pixels is sent to your prompt provider. Choose a vision-capable model."));
        return;
    }
    if fields.get("__ai").and_then(Value::as_str) == Some("results") {
        results(app, ui, fields);
        return;
    }
    if fields.get("__command").and_then(Value::as_str) == Some("image.aiUpscale") {
        upscale_form(app, ui, fields);
        return;
    }
    let segment = fields.get("__command").and_then(Value::as_str) == Some("layer.aiRemoveBackground");
    let generation = fields.get("__command").and_then(Value::as_str) == Some("edit.generateImage");
    if generation {
        output_size(app, ui, fields);
        let mut mode = fields.get("resolutionMode").and_then(Value::as_str).unwrap_or("native").to_owned();
        ui.horizontal(|ui| {
            ui.label(tl!("Generation resolution"));
            egui::ComboBox::from_id_salt("ai-resolution-mode")
                .selected_text(if mode == "native" { tl!("Direct generation") } else { tl!("Reduced generation") })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut mode, "native".into(), tl!("Direct generation"));
                    ui.selectable_value(&mut mode, "reduced".into(), tl!("Reduced generation"));
                });
        });
        fields.insert("resolutionMode".into(), json!(mode));
        fields.insert("detailUpscale".into(), json!(false));
        ui.label(
            RichText::new(tl!("Direct generation uses the output size. Model and GPU limits apply; use AI Upscale separately for enlargement."))
                .small()
                .color(Tokens::get(ui.ctx()).text_dim),
        );
    }
    ui.label(RichText::new(tl!("AI creates a preview. Apply it as a new layer after reviewing the result.")).color(Tokens::get(ui.ctx()).text_dim));
    if !segment && !matches!(fields.get("__command").and_then(Value::as_str), Some("image.generativeExpand" | "edit.aiBackground")) {
        let mut provider = fields.get("imageProvider").and_then(Value::as_str).unwrap_or("comfyui").to_owned();
        egui::ComboBox::from_id_salt("ai-provider").selected_text(if provider == "openai" { "OpenAI Images" } else { "ComfyUI" }).show_ui(ui, |ui| {
            ui.selectable_value(&mut provider, "comfyui".into(), "ComfyUI");
            ui.selectable_value(&mut provider, "openai".into(), "OpenAI Images");
        });
        if provider == "openai" {
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
                RichText::new(tl!(
                    "Direct OpenAI generation must fit the selected model limits (up to 4K for GPT Image 2/2.5). API usage is billed by your provider."
                ))
                .small()
                .color(Tokens::get(ui.ctx()).text_dim),
            );
        }
        fields.insert("imageProvider".into(), json!(provider));
    }
    if !segment {
        if !generation {
            let mut detail = fields.get("detailUpscale").and_then(Value::as_bool).unwrap_or(false);
            ui.checkbox(&mut detail, tl!("Restore high-resolution detail with ComfyUI (Real-ESRGAN)"));
            fields.insert("detailUpscale".into(), json!(detail));
        }
        ui.label(tl!("Prompt"));
        let mut prompt = fields.get("prompt").and_then(Value::as_str).unwrap_or_default().to_owned();
        ui.add(egui::TextEdit::multiline(&mut prompt).desired_width(400.0).desired_rows(4).char_limit(4096));
        fields.insert("prompt".into(), json!(prompt));
        let mut prepare = fields.get("preparePrompt").and_then(Value::as_bool).unwrap_or(false);
        ui.checkbox(&mut prepare, tl!("Prepare the prompt automatically with OpenAI"));
        fields.insert("preparePrompt".into(), json!(prepare));
        if prepare {
            text(ui, fields, "textModel", "Prompt model", 128);
            ui.label(
                RichText::new(tl!("Only your text instruction is sent to OpenAI. API usage is billed by your provider."))
                    .small()
                    .color(Tokens::get(ui.ctx()).text_dim),
            );
        }
        ui.collapsing(tl!("Advanced"), |ui| {
            if fields.get("imageProvider").and_then(Value::as_str) != Some("openai") {
                fields.entry("samplingSteps").or_insert(json!(if generation { 25 } else { 8 }));
                number(ui, fields, "samplingSteps", "Sampling steps", 100);
                text(ui, fields, "negativePrompt", "Negative prompt", 4096);
                number(ui, fields, "seed", "Seed", u64::MAX);
            }
            number(ui, fields, "contextMargin", "Context margin", 1024);
        });
    }
    if fields.get("__command").and_then(Value::as_str) == Some("image.generativeExpand") {
        let mut edges = fields.get("expansion").and_then(Value::as_object).cloned().unwrap_or_else(|| Map::from_iter([("right".into(), json!(128))]));
        for (key, label) in [("left", "Left"), ("top", "Top"), ("right", "Right"), ("bottom", "Bottom")] {
            number(ui, &mut edges, key, label, 300_000);
        }
        fields.insert("expansion".into(), Value::Object(edges));
    }
    ui.add_space(8.0);
    if crate::widgets::secondary_button(ui, tl!("API Settings…"), 150.0).clicked() {
        let _ = crate::menus::invoke(app, ui.ctx(), "edit.preferences.integrations", json!({}));
    }
}
fn upscale_form(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    let source = app.session.active().map(|s| s.doc.size);
    let mut preset = fields.get("__upscalePreset").and_then(Value::as_str).unwrap_or("2").to_owned();
    ui.horizontal(|ui| {
        ui.label(tl!("Upscale factor"));
        egui::ComboBox::from_id_salt("ai-upscale-factor")
            .selected_text(match preset.as_str() {
                "2" => "2×",
                "4" => "4×",
                _ => tl!("Custom"),
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut preset, "2".into(), "2×");
                ui.selectable_value(&mut preset, "4".into(), "4×");
                ui.selectable_value(&mut preset, "custom".into(), tl!("Custom"));
            });
    });
    let factor = if preset == "4" { 4u64 } else { 2 };
    fields.insert("scale".into(), json!(factor));
    if preset == "custom" {
        let mut size = fields.get("outputSize").and_then(Value::as_object).cloned().unwrap_or_else(|| {
            Map::from_iter([
                ("width".into(), json!(u64::from(source.map_or(512, |s| s.width)) * 2)),
                ("height".into(), json!(u64::from(source.map_or(512, |s| s.height)) * 2)),
            ])
        });
        let before_w = size.get("width").and_then(Value::as_u64).unwrap_or(0);
        let before_h = size.get("height").and_then(Value::as_u64).unwrap_or(0);
        number(ui, &mut size, "width", "Width", 300_000);
        number(ui, &mut size, "height", "Height", 300_000);
        if let Some(source) = source.filter(|s| s.width > 0 && s.height > 0) {
            let w = size.get("width").and_then(Value::as_u64).unwrap_or(0);
            let h = size.get("height").and_then(Value::as_u64).unwrap_or(0);
            if w != before_w {
                size.insert("height".into(), json!((w as f64 * f64::from(source.height) / f64::from(source.width)).round() as u64));
            } else if h != before_h {
                size.insert("width".into(), json!((h as f64 * f64::from(source.width) / f64::from(source.height)).round() as u64));
            }
        }
        fields.insert("outputSize".into(), Value::Object(size));
    } else {
        fields.remove("outputSize");
        if let Some(source) = source {
            ui.label(format!("{}: {} × {} px", tl!("Output size"), u64::from(source.width) * factor, u64::from(source.height) * factor));
        }
    }
    fields.insert("__upscalePreset".into(), json!(preset));
    ui.label(tl!("Upscale the visible image from its original pixels with ComfyUI (Real-ESRGAN). Transparency is preserved."));
    ui.label(tl!("AI creates a preview. Apply it as a new layer after reviewing the result."));
    if crate::widgets::secondary_button(ui, tl!("API Settings…"), 150.0).clicked() {
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
fn results(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
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
            if let Some(result) = app.session.ai_result(id)
                && let Some(prompt) = &result.prepared_prompt
            {
                ui.label(&prompt.positive);
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
            if let Some(model) = row.get("detailUpscaler").and_then(Value::as_str) {
                ui.label(format!("{}: {model}", tl!("AI detail restoration")));
            } else if row.get("resampled").and_then(Value::as_bool) == Some(true) {
                ui.label(
                    RichText::new(tl!("Upsampled to the destination; this is not native high-resolution AI detail."))
                        .small()
                        .color(Tokens::get(ui.ctx()).text_dim),
                );
            }
            if stale {
                ui.label(RichText::new(tl!("The document changed. Generate a new result before applying.")).color(Tokens::get(ui.ctx()).danger));
            }
            let upscale = row.get("operation").and_then(Value::as_str) == Some("upscale");
            let foreground = row.get("operation").and_then(Value::as_str) == Some("segmentSubject") || upscale;
            let mut hide = fields.get("hideOriginals").and_then(Value::as_bool).unwrap_or(true);
            if foreground {
                ui.checkbox(&mut hide, if upscale { tl!("Hide original layers") } else { tl!("Hide original layers to show the cutout") });
                fields.insert("hideOriginals".into(), json!(hide));
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(!stale, egui::Button::new(tl!("Apply as New Layer"))).clicked()
                    && let Err(e) = app.run("ai.result.apply", json!({"result":id,"hideOriginals":foreground && hide}))
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
fn dependency_form(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    ui.label(tl!("Apply your AI settings before checking dependencies. Downloads run on the ComfyUI host."));
    if ui.button(tl!("Check dependencies")).clicked() {
        match app.run("comfy.dependencies.check", json!({})) {
            Ok(value) => {
                if let Some(report) = value.get("dependencyReport") {
                    fields.insert("__report".into(), report.clone());
                }
            }
            Err(error) => {
                fields.insert("__error".into(), json!(error));
            }
        }
    }
    let report = fields.get("__report").cloned().unwrap_or(Value::Null);
    if let Some(note) = report.pointer("/nodes/note").and_then(Value::as_str) {
        ui.label(note);
    }
    if let Some(missing) = report.pointer("/nodes/missing").and_then(Value::as_array) {
        for node in missing {
            if let Some(node) = node.as_str() {
                ui.colored_label(Tokens::get(ui.ctx()).danger, format!("Missing node: {node}"));
            }
        }
    }
    if let Some(note) = report.pointer("/models/note").and_then(Value::as_str) {
        ui.label(note);
    }
    let mut selected: Vec<String> = fields.get("__selected").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    if let Some(rows) = report.pointer("/models/models").and_then(Value::as_array) {
        for row in rows {
            let id = row.pointer("/model/id").and_then(Value::as_str).unwrap_or_default();
            let state = row.get("state").and_then(Value::as_str).unwrap_or("unknown");
            let mut checked = selected.iter().any(|value| value == id);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        state == "missing",
                        egui::Checkbox::new(
                            &mut checked,
                            format!("{id} · {state} · {:.2} GB", row.pointer("/model/size").and_then(Value::as_u64).unwrap_or(0) as f64 / 1e9),
                        ),
                    )
                    .changed()
                {
                    selected.retain(|value| value != id);
                    if checked {
                        selected.push(id.into());
                    }
                }
                if let (Some(license), Some(url)) =
                    (row.pointer("/model/license").and_then(Value::as_str), row.pointer("/model/licenseUrl").and_then(Value::as_str))
                    && url.starts_with("https://")
                {
                    ui.hyperlink_to(license, url);
                }
            });
        }
    }
    fields.insert("__selected".into(), json!(selected));
    let mut accept = fields.get("__licenses").and_then(Value::as_bool).unwrap_or(false);
    ui.checkbox(&mut accept, tl!("I have reviewed and accept the selected model licenses."));
    fields.insert("__licenses".into(), json!(accept));
    if ui.add_enabled(accept && !selected.is_empty(), egui::Button::new(tl!("Download selected models"))).clicked() {
        match app.run("comfy.dependencies.install", json!({"models":selected,"acceptLicenses":true})) {
            Ok(value) => {
                if let Some(report) = value.get("dependencyReport") {
                    fields.insert("__report".into(), report.clone());
                }
            }
            Err(error) => {
                fields.insert("__error".into(), json!(error));
            }
        }
    }
    if let Some(error) = fields.get("__error").and_then(Value::as_str) {
        ui.colored_label(Tokens::get(ui.ctx()).danger, error);
    }
}

pub fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    if fields.get("__ai").and_then(Value::as_str) == Some("interpretation") {
        let opened = menu(app, "edit.generateImage", &json!({})).ok_or("Generation form unavailable")??;
        let id = opened.get("dialog").and_then(Value::as_u64).ok_or("Generation form unavailable")?;
        if let Some(dialog) = app.ui.dialog_mut(id) {
            for key in ["prompt", "negativePrompt"] {
                dialog.fields.insert(key.into(), fields.get(key).filter(|v| v.is_string()).cloned().unwrap_or(json!("")));
            }
        }
        return Ok(opened);
    }
    let command = fields.get("__command").and_then(Value::as_str).ok_or("AI dialog has no command")?;
    if command == "ai.result.list" || command == "comfy.dependencies.check" {
        return Ok(Value::Null);
    }
    let mut params = fields.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect::<Map<_, _>>();
    if params.get("imageProvider").and_then(Value::as_str) == Some("openai") {
        params.remove("negativePrompt");
    }
    if params.get("negativePrompt").and_then(Value::as_str) == Some("") {
        params.remove("negativePrompt");
    }
    app.run(command, Value::Object(params))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn platform_prompt_model_default_is_used_and_form_overrides_remain_editable() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":64,"height":64})).unwrap();
        assert_eq!(app.session.ai_prompt_model(), "gpt-4.1-mini");
        app.session.configure_ai_prompt_model("qwen3.6:35b@GPU-HOST").unwrap();
        assert!(app.session.configure_ai_prompt_model("bad\nmodel").is_err());
        let id = menu(&mut app, "edit.generateImage", &json!({})).unwrap().unwrap()["dialog"].as_u64().unwrap();
        let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
        assert_eq!(fields["textModel"], "qwen3.6:35b@GPU-HOST");
        fields.insert("textModel".into(), json!("individual-model"));
        assert_eq!(fields["textModel"], "individual-model");
        assert_eq!(app.session.ai_prompt_model(), "qwen3.6:35b@GPU-HOST");
    }
    #[test]
    fn native_generation_and_separate_upscale_forms_keep_distinct_command_parameters() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":1800,"height":1200})).unwrap();
        let context = egui::Context::default();
        PhotocraftApp::setup_context(&context, Default::default());
        let opened = menu(&mut app, "edit.generateImage", &json!({})).unwrap().unwrap();
        let id = opened["dialog"].as_u64().unwrap();
        let mut fields = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
        fields.insert("detailUpscale".into(), json!(true));
        context.run_ui(Default::default(), |ui| body(&mut app, ui, &mut fields)).textures_delta.clear();
        assert_eq!(fields["resolutionMode"], "native");
        assert_eq!(fields["detailUpscale"], false);
        crate::dialogs::cancel(&mut app, id).unwrap();
        let opened = menu(&mut app, "image.aiUpscale", &json!({})).unwrap().unwrap();
        let id = opened["dialog"].as_u64().unwrap();
        let mut fields = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
        context.run_ui(Default::default(), |ui| body(&mut app, ui, &mut fields)).textures_delta.clear();
        assert_eq!(fields["scale"], 2);
        assert!(!fields.contains_key("prompt"));
        assert!(!fields.contains_key("preparePrompt"));
        assert!(app.session.jobs().is_empty());
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
    #[test]
    fn menus_open_editable_ai_forms_without_network_and_retain_invalid_drafts() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":64,"height":64})).unwrap();
        let response = menu(&mut app, "edit.generateImage", &json!({})).unwrap().unwrap();
        let id = response["dialog"].as_u64().unwrap();
        assert!(app.session.jobs().is_empty());
        assert!(crate::dialogs::confirm(&mut app, id).is_err());
        assert!(app.ui.dialogs.iter().any(|d| d.id == id));
        let dialog = app.ui.dialogs.iter_mut().find(|d| d.id == id).unwrap();
        dialog.fields.insert("prompt".into(), json!("manual instruction"));
        dialog.fields.insert("preparePrompt".into(), json!(true));
        assert_eq!(dialog.fields["preparePrompt"], true);
        crate::dialogs::cancel(&mut app, id).unwrap();
    }
    #[test]
    fn ai_menu_and_selection_context_rows_are_live() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":64,"height":64})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let items = crate::menus::menu_items(&app);
        for id in COMMANDS {
            assert!(items.iter().any(|i| i.id == *id && i.enabled), "{id}");
        }
        assert!(crate::canvas_tool_menu::SELECTION_MENU.iter().flatten().any(|(_, id)| *id == "edit.generativeFill"));
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn ready_preview_renders_and_reuses_texture_without_locking_the_context() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            use photocraft_ai::{AiRequest, AiResult, Artifact, ImageSize, Operation, PromptPreparation, RgbaImage, WorkflowId};
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
                    prompt_preparation: PromptPreparation::Manual,
                })
                .unwrap();
            let result = input
                .finish(AiResult {
                    provider_job: None,
                    prepared_prompt: None,
                    artifacts: vec![Artifact::Image(RgbaImage::new(size, [0.1, 0.2, 0.8, 1.0].repeat(64 * 64)).unwrap())],
                })
                .unwrap();
            let id = app.session.retain_ai_result(result).unwrap();
            let context = egui::Context::default();
            PhotocraftApp::setup_context(&context, Default::default());
            let mut fields = Map::new();
            for _ in 0..2 {
                context.run_ui(Default::default(), |ui| results(&mut app, ui, &mut fields)).textures_delta.clear();
                assert!(context.data_mut(|data| data.get_temp::<egui::TextureHandle>(egui::Id::unique(("ai-preview", id)))).is_some());
            }
            sender.send(()).unwrap();
        });
        receiver.recv_timeout(std::time::Duration::from_secs(15)).expect("AI preview rendering stalled or failed");
        worker.join().unwrap();
    }
    #[test]
    fn interpreted_prompt_opens_generation_for_review_without_starting_a_job() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":32,"height":24})).unwrap();
        let revision = app.session.active().unwrap().revision;
        open_interpretation(&mut app, &json!({"description":"A cube","positivePrompt":"A blue cube","negativePrompt":null}));
        let fields = app.ui.dialogs.last().unwrap().fields.clone();
        let opened = confirm(&mut app, &fields).unwrap();
        let fields = &app.ui.dialog_mut(opened["dialog"].as_u64().unwrap()).unwrap().fields;
        assert_eq!(fields["prompt"], "A blue cube");
        assert_eq!(fields["preparePrompt"], false);
        assert!(app.session.jobs().is_empty());
        assert_eq!(app.session.active().unwrap().revision, revision);
    }
}
