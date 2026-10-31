//! Edit › Stroke…: a Photoshop-style options dialog for the existing `edit.stroke` command.
//! The engine remains directly callable with parameters by CLI, MCP and Actions.

use egui::{Align2, Sense, pos2, vec2};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

pub const COMMAND: &str = "edit.stroke";
const MARK: &str = "__stroke";
const KEYS: [&str; 6] = ["width", "color", "location", "mode", "opacity", "preserveTransparency"];
const LOCATIONS: [(&str, &str); 3] = [("inside", "Inside"), ("center", "Center"), ("outside", "Outside")];

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Accept remembered options only within their field domains, including finite float RGB.
fn valid(key: &str, v: &Value) -> bool {
    match key {
        "width" => v.as_f64().is_some_and(|x| x.is_finite() && (1.0..=250.0).contains(&x)),
        "color" => crate::color_picker_ui::parse_color(v).is_some(),
        "location" => v.as_str().is_some_and(|s| LOCATIONS.iter().any(|(id, _)| *id == s)),
        _ => crate::dialog_blend_ui::valid(key, v),
    }
}

/// Construct a dialog from defaults and only valid, persisted preferences.
pub fn fields(app: &PhotocraftApp) -> Map<String, Value> {
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("Stroke…"));
    f.insert("width".into(), json!(1.0));
    let fg = app.session.tools.foreground;
    f.insert("color".into(), crate::color_picker_ui::document_color(app, [fg[0], fg[1], fg[2]]));
    f.insert("location".into(), json!("center"));
    f.insert("mode".into(), json!("normal"));
    f.insert("opacity".into(), json!(100.0));
    f.insert("preserveTransparency".into(), json!(false));
    if let Some(Value::Object(saved)) = app.session.prefs().dialogs.get(COMMAND) {
        for key in KEYS {
            if let Some(value) = saved.get(key).filter(|v| valid(key, v)) {
                f.insert(key.into(), value.clone());
            }
        }
    }
    // Keep stored float values for float documents and the legacy hex domain for integer ones.
    if let Some(rgb) = f.get("color").and_then(crate::color_picker_ui::parse_color) {
        f.insert("color".into(), crate::color_picker_ui::document_color(app, rgb));
    }
    f
}

pub fn open(app: &mut PhotocraftApp) -> u64 {
    let f = fields(app);
    app.ui.open_dialog(crate::state::DialogKind::Command, f)
}

/// Only Stroke params are passed to the engine; private UI markers are ignored.
pub fn params(f: &Map<String, Value>) -> Value {
    Value::Object(KEYS.iter().filter_map(|k| f.get(*k).map(|v| (k.to_string(), v.clone()))).collect())
}

pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let remembered = params(f);
    let result = app.run(COMMAND, remembered.clone());
    if result.is_ok() {
        app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), remembered));
    }
    result
}

fn label(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(112.0, 22.0), Sense::hover());
    ui.painter().text(pos2(r.right() - 6.0, r.center().y), Align2::RIGHT_CENTER, text, egui::FontId::proportional(12.0), t.text_dim);
}

pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.spacing_mut().item_spacing.y = 6.0;
    crate::widgets::section_label(ui, tl!("Stroke"));
    crate::widgets::hairline(ui);
    ui.horizontal(|ui| {
        label(ui, tl!("Width:"));
        let mut width = f.get("width").and_then(Value::as_f64).unwrap_or(1.0) as f32;
        if crate::widgets::value_field(ui, &mut width, 1.0..=250.0, "px", 84.0).accessible_name(tl!("Width:")).changed() {
            f.insert("width".into(), json!(width.round()));
        }
    });
    ui.horizontal(|ui| {
        label(ui, tl!("Color:"));
        let rgb = f.get("color").and_then(crate::color_picker_ui::parse_color).unwrap_or([0.0; 3]);
        // As in Photoshop, the swatch opens the Color Picker on the stroke colour.
        if crate::widgets::color_swatch_button(ui, crate::color_picker_ui::c32(rgb), tl!("Stroke color")).clicked() {
            crate::color_picker_ui::request(f, tl!("Color Picker (Stroke Color)"));
        }
    });

    ui.add_space(4.0);
    crate::widgets::section_label(ui, tl!("Location"));
    crate::widgets::hairline(ui);
    ui.horizontal(|ui| {
        label(ui, tl!("Location:"));
        let mut location = f.get("location").and_then(Value::as_str).unwrap_or("center").to_string();
        let opts: Vec<(String, &str)> = LOCATIONS.iter().map(|(key, label)| (key.to_string(), tl!(label))).collect();
        if crate::widgets::dropdown(ui, "stroke-location", tl!("Location"), &mut location, &opts, 170.0) {
            f.insert("location".into(), json!(location));
        }
    });
    crate::dialog_blend_ui::body(ui, f, "stroke");
}

#[cfg(test)]
mod tests;
