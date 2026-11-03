//! Generate an image as an editable layer through the engine job system.
use crate::{PhotocraftApp, state::DialogKind, widgets};
use serde_json::{Map, Value, json};
const COMMAND: &str = "image.generate";
const MARK: &str = "__generateImage";
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}
pub fn open(app: &mut PhotocraftApp) -> Result<u64, String> {
    if !app.session.is_enabled(COMMAND) {
        return Err("Open a document on Linux with the Enki runtime available".into());
    }
    let document = app.session.active().ok_or("no document open")?.doc.id.0;
    let mut fields = Map::new();
    for (key, value) in [
        (MARK, json!(true)),
        ("__command", json!(COMMAND)),
        ("__label", json!("Generate Image")),
        ("document", json!(document)),
        ("prompt", json!("")),
        ("negative", json!("")),
        ("model", json!("")),
        ("width", json!(512)),
        ("height", json!(512)),
        ("seed", json!(-1)),
        ("placement", json!("layer")),
    ] {
        fields.insert(key.into(), value);
    }
    Ok(app.ui.open_dialog(DialogKind::Command, fields))
}
pub fn open_edit(app: &mut PhotocraftApp, command: &str) -> Result<u64, String> {
    if !app.session.is_enabled(command) {
        return Err("Select an editable region, or choose a supported pixel document for expansion".into());
    }
    let id = open(app)?;
    let fields = &mut app.ui.dialog_mut(id).ok_or("generation dialog is no longer available")?.fields;
    fields.insert("__command".into(), json!(command));
    let label = match command {
        "image.expandCanvas" => "Expand Canvas",
        "image.replaceBackground" => "Replace Background",
        "image.removeObject" => "Remove Object",
        _ => "Fill Selection",
    };
    fields.insert("__label".into(), json!(label));
    fields.insert("strength".into(), json!(1.0));
    if command == "image.replaceBackground" {
        fields.insert("prompt".into(), json!("Replace the background around the selected subject, matching the subject's perspective and lighting. "));
    }
    if command == "image.removeObject" {
        fields.insert("prompt".into(), json!("Remove the selected object and reconstruct the surrounding background naturally."));
    }
    if command == "image.expandCanvas" {
        for key in ["left", "right", "top", "bottom"] {
            fields.insert(key.into(), json!(32));
        }
    }
    for key in ["placement", "width", "height"] {
        fields.remove(key);
    }
    Ok(id)
}

fn text(ui: &mut egui::Ui, fields: &mut Map<String, Value>, key: &str, label: &str, hint: &str, limit: usize) {
    let mut value = fields.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    ui.horizontal(|ui| {
        let label = ui.label(tl!(label));
        if widgets::text_field_ex(ui, ui.scope_id().with(key), &mut value, tl!(hint), 300.0, false, Some(limit)).labelled_by(label.id).changed() {
            fields.insert(key.into(), json!(value));
        }
    });
}
pub fn body(_app: &PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    let editing = fields.get("__command").and_then(Value::as_str).is_some_and(|id| id != COMMAND);
    if !editing {
        let mut placement = fields.get("placement").and_then(Value::as_str).unwrap_or("layer").to_string();
        if widgets::dropdown(
            ui,
            "generate-placement",
            tl!("Result:"),
            &mut placement,
            &[("layer".into(), "New Layer"), ("background".into(), "Behind Subject")],
            180.0,
        ) {
            fields.insert("placement".into(), json!(placement));
        }
    }
    text(ui, fields, "prompt", "Prompt:", "Describe the image", 2000);
    text(ui, fields, "negative", "Avoid:", "Optional unwanted details", 2000);
    text(ui, fields, "model", "Model:", "Empty uses the Enki recommendation", 256);
    if !editing {
        for (key, label) in [("width", "Width:"), ("height", "Height:")] {
            let mut value = fields.get(key).and_then(Value::as_u64).unwrap_or(512) as u32;
            ui.horizontal(|ui| {
                let label = ui.label(tl!(label));
                if ui.add(egui::DragValue::new(&mut value).range(64..=1024).speed(8)).labelled_by(label.id).changed() {
                    fields.insert(key.into(), json!(value));
                }
            });
        }
    } else {
        if fields.get("__command").and_then(Value::as_str) == Some("image.expandCanvas") {
            for (key, label) in [("left", "Left:"), ("right", "Right:"), ("top", "Top:"), ("bottom", "Bottom:")] {
                let mut value = fields.get(key).and_then(Value::as_u64).unwrap_or(32) as u32;
                ui.horizontal(|ui| {
                    let label = ui.label(tl!(label));
                    if ui.add(egui::DragValue::new(&mut value).range(0..=512)).labelled_by(label.id).changed() {
                        fields.insert(key.into(), json!(value));
                    }
                });
            }
            ui.label(tl!("Expanded dimensions must be multiples of 8 and no larger than 1024 pixels per side."));
        }
        let mut harmonize = fields.get("harmonize").and_then(Value::as_bool).unwrap_or(true);
        if widgets::checkbox(ui, &mut harmonize, tl!("Match generated edge colors")).changed() {
            fields.insert("harmonize".into(), json!(harmonize));
        }
        let mut strength = fields.get("strength").and_then(Value::as_f64).unwrap_or(1.0) as f32;
        if widgets::slider_row(ui, tl!("Edit strength:"), &mut strength, 0.05..=1.0, "", None).changed() {
            fields.insert("strength".into(), json!(strength));
        }
        if fields.get("__command").and_then(Value::as_str) == Some("image.replaceBackground") {
            ui.label(tl!("The selection marks the subject to keep. Its background will be replaced using the original scene as context."));
        }
        ui.label(tl!("Uses original image context and an edit mask. Protected pixels remain unchanged. Requires an updated Enki runtime."));
    }
    let mut override_sampling = fields.contains_key("steps");
    if widgets::checkbox(ui, &mut override_sampling, tl!("Override sampling settings")).changed() {
        if override_sampling {
            fields.insert("steps".into(), json!(20));
            fields.insert("cfg".into(), json!(7.0));
        } else {
            fields.remove("steps");
            fields.remove("cfg");
        }
    }
    if override_sampling {
        let mut steps = fields.get("steps").and_then(Value::as_u64).unwrap_or(20) as u32;
        ui.horizontal(|ui| {
            let label = ui.label(tl!("Steps:"));
            if ui.add(egui::DragValue::new(&mut steps).range(1..=50)).labelled_by(label.id).changed() {
                fields.insert("steps".into(), json!(steps));
            }
        });
        let mut cfg = fields.get("cfg").and_then(Value::as_f64).unwrap_or(7.0) as f32;
        if widgets::slider_row(ui, tl!("Guidance:"), &mut cfg, 1.0..=20.0, "", None).changed() {
            fields.insert("cfg".into(), json!(cfg));
        }
    }
    let mut seed = fields.get("seed").and_then(Value::as_i64).unwrap_or(-1);
    ui.horizontal(|ui| {
        let label = ui.label(tl!("Seed:"));
        if ui.add(egui::DragValue::new(&mut seed)).labelled_by(label.id).changed() {
            fields.insert("seed".into(), json!(seed));
        }
    });
    if !editing {
        ui.label(tl!("Use multiples of 8 for image dimensions. A negative seed creates a new variation."));
    }
    ui.label(tl!("Runs locally through Enki. No models are downloaded. The result is a new layer; Undo removes it."));
}
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    #[cfg(target_os = "linux")]
    fn generation_dialog_is_document_bound_and_cancellable() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width":64,"height":64})).unwrap();
        let mut app = PhotocraftApp::new(s, Default::default());
        let before = app.session.active().unwrap().doc.clone();
        assert!(crate::menus::menu_items(&app).iter().any(|m| m.id == COMMAND && m.path == ["Image"]));
        let id = open(&mut app).unwrap();
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        assert_eq!(fields["document"], json!(before.id.0));
        assert!(crate::dialogs::is_route_marker(MARK));
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| body(&app, ui, &mut fields));
        out.textures_delta.clear();
        crate::dialogs::cancel(&mut app, id).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers, before.layers);
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn a_dialog_cannot_apply_to_another_document() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width":64,"height":64})).unwrap();
        let mut app = PhotocraftApp::new(s, Default::default());
        let id = open(&mut app).unwrap();
        app.ui.dialog_mut(id).unwrap().fields.insert("prompt".into(), json!("landscape"));
        app.session.execute("file.new", json!({"width":64,"height":64})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        assert!(crate::dialogs::confirm(&mut app, id).is_err());
        assert_eq!(app.session.active().unwrap().doc.layers, before.layers);
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn masked_edit_dialogs_use_saved_document_and_are_cancellable() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width":64,"height":64})).unwrap();
        s.execute("select.rect", json!({"x":8,"y":8,"width":24,"height":24})).unwrap();
        let mut app = PhotocraftApp::new(s, Default::default());
        let before = app.session.active().unwrap().doc.clone();
        let ctx = egui::Context::default();
        for command in ["image.fillSelection", "image.removeObject", "image.expandCanvas"] {
            let id = open_edit(&mut app, command).unwrap();
            let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
            assert_eq!(fields["__command"], command);
            assert_eq!(fields["document"], json!(before.id.0));
            assert!(!fields.contains_key("placement"));
            let mut out = ctx.run_ui(Default::default(), |ui| body(&app, ui, &mut fields));
            out.textures_delta.clear();
            crate::dialogs::cancel(&mut app, id).unwrap();
            assert_eq!(app.session.active().unwrap().doc.layers, before.layers);
        }
    }
}
