//! Properties Quick Action for a background beneath a masked subject.

use photocraft_color::{Color, ColorMode};
#[cfg(test)]
use photocraft_doc::LayerContent;
use serde_json::{Map, Value, json};

use crate::{PhotocraftApp, state::DialogKind, widgets};

pub const COMMAND: &str = "layer.backgroundFill";
const MARK: &str = "__backgroundFill";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

pub fn open(app: &mut PhotocraftApp) -> Result<u64, String> {
    if !app.session.is_enabled(COMMAND) {
        return Err("Remove the background or add an enabled subject mask first".into());
    }
    let st = app.session.active().ok_or("no document open")?;
    let layer = st.active_layer.ok_or("no subject layer selected")?;
    let mut fields = Map::new();
    fields.insert(MARK.into(), json!(true));
    fields.insert("__command".into(), json!(COMMAND));
    fields.insert("__label".into(), json!("Background Fill"));
    fields.insert("document".into(), json!(st.doc.id.0));
    fields.insert("layer".into(), json!(layer.0));
    fields.insert("kind".into(), json!("solid"));
    fields.insert("color".into(), json!(app.session.tools.foreground));
    fields.insert("to".into(), json!(app.session.tools.background));
    fields.insert("angle".into(), json!(90.0));
    Ok(app.ui.open_dialog(DialogKind::Command, fields))
}

fn colour(ui: &mut egui::Ui, fields: &mut Map<String, Value>, key: &str, label: &str) {
    let original = fields.get(key).and_then(|v| photocraft_engine::solid_fill_cmds::color_param(v, Color::rgb(0.0, 0.0, 0.0), ColorMode::Rgb).ok());
    let mut rgb = original.map(|c| c.to_rgb()).unwrap_or([0.0; 3]);
    ui.horizontal(|ui| {
        let label = ui.label(tl!(label));
        if widgets::color_edit_button_rgb(ui, &mut rgb, tl!("Background Color")).labelled_by(label.id).changed() {
            fields.insert(key.into(), json!([rgb[0], rgb[1], rgb[2], original.map_or(1.0, |c| c.alpha)]));
        }
    });
}

pub fn body(app: &PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    let mut kind = fields.get("kind").and_then(Value::as_str).unwrap_or("solid").to_string();
    let options = [("solid".to_string(), "Solid Color"), ("gradient".to_string(), "Gradient"), ("image".to_string(), "Image Layer")];
    if widgets::dropdown(ui, "background-fill-kind", tl!("Background:"), &mut kind, &options, 180.0) {
        fields.insert("kind".into(), json!(kind));
    }
    match kind.as_str() {
        "solid" => colour(ui, fields, "color", "Color:"),
        "gradient" => {
            colour(ui, fields, "color", "From:");
            colour(ui, fields, "to", "To:");
            let mut angle = fields.get("angle").and_then(Value::as_f64).unwrap_or(90.0) as f32;
            if widgets::slider_row(ui, tl!("Angle:"), &mut angle, -180.0..=180.0, "°", None).changed() {
                fields.insert("angle".into(), json!(angle));
            }
        }
        "image" => {
            let subject = fields.get("layer").and_then(Value::as_u64);
            let layers: Vec<(u64, String)> = app
                .session
                .active()
                .map(|st| {
                    st.doc
                        .walk()
                        .into_iter()
                        .filter(|(_, _, layer)| {
                            Some(layer.id.0) != subject
                                && photocraft_engine::cmd::background_fill_cmds::image_source(
                                    &app.session,
                                    photocraft_doc::LayerId(subject.unwrap_or(0)),
                                    layer.id,
                                )
                                .is_ok()
                        })
                        .map(|(_, _, layer)| (layer.id.0, layer.name.clone()))
                        .collect()
                })
                .unwrap_or_default();
            let mut options = vec![(0, tl!("Choose an image layer"))];
            options.extend(layers.iter().map(|(id, name)| (*id, name.as_str())));
            let mut source = fields.get("sourceLayer").and_then(Value::as_u64).unwrap_or(0);
            if widgets::dropdown_literal(ui, "background-fill-source", tl!("Layer:"), &mut source, &options, 220.0) {
                fields.insert("sourceLayer".into(), json!(source));
            }
            ui.label(tl!("Moves the chosen image layer behind the subject. Its size and position are kept."));
            ui.label(tl!("Use File > Place Embedded to add an image layer first."));
        }
        _ => {}
    }
    ui.add_space(crate::theme::ROW_GAP);
    ui.label(tl!("The subject's pixels and mask stay unchanged. Undo restores the previous background."));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{ControlRequest, handle};

    fn app() -> PhotocraftApp {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width":64,"height":64})).unwrap();
        s.execute("layer.new.layer", json!({"name":"Subject"})).unwrap();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        PhotocraftApp::new(s, Default::default())
    }

    #[test]
    fn quick_action_dialog_is_cancellable_and_commits_through_automation() {
        let mut app = app();
        let before = app.session.active().unwrap().doc.clone();
        let ctx = egui::Context::default();
        let result = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
        let id = result["dialog"].as_u64().unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers, before.layers);
        crate::dialogs::cancel(&mut app, id).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers, before.layers);
        let id = open(&mut app).unwrap();
        for (field, value) in [("kind", json!("gradient")), ("color", json!("#224466")), ("to", json!("#eeeeee"))] {
            let (request, _) = ControlRequest::new("ui.dialog.set", json!({"dialog":id,"field":field,"value":value}));
            let crate::control::Outcome::Done(reply) = handle(&mut app, &ctx, &request) else { panic!("expected immediate reply") };
            assert_eq!(reply["ok"], true, "{reply}");
        }
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers.len(), before.layers.len() + 1);
        assert!(
            app.session.active().unwrap().doc.layers.iter().any(|layer| matches!(layer.content, LayerContent::Fill(photocraft_doc::Fill::Gradient { .. })))
        );
        assert!(app.session.undo());
        assert_eq!(app.session.active().unwrap().doc.layers, before.layers);
    }

    #[test]
    fn dialog_renders_using_the_shared_widgets() {
        let mut app = app();
        let id = open(&mut app).unwrap();
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        let ctx = egui::Context::default();
        for kind in ["solid", "gradient", "image"] {
            fields.insert("kind".into(), json!(kind));
            let mut out = ctx.run_ui(Default::default(), |ui| body(&app, ui, &mut fields));
            out.textures_delta.clear();
        }
        assert!(crate::dialogs::is_route_marker(MARK));
    }
}
