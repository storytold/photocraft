//! Photoshop's New Group dialog (Name, Color, Mode, Opacity), opened by Layer › New › Group… and
//! Group from Layers…, the Layers panel menu's New Group… / New Group from Layers… and ⌥-click on
//! the panel's New Group button. OK runs `layer.new.group` / `layer.new.groupFromLayers` with the
//! options as one history step; explicit params from Actions, CLI and MCP run the engine directly.

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use photocraft_color::BlendMode;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params;

/// Layer › New › Group…; also the key of its dialog.
pub const NEW_GROUP: &str = "layer.new.group";
/// Layer › New › Group from Layers…; also the key of its dialog.
pub const FROM_LAYERS: &str = "layer.new.groupFromLayers";

/// Whether the dialog fronts command `id`.
pub fn fronts(id: &str) -> bool {
    id == NEW_GROUP || id == FROM_LAYERS
}

/// Whether the hosted dialog with `key` is a New Group dialog.
pub(crate) fn owns(key: &str) -> bool {
    fronts(key)
}

/// Open the dialog for `command` with Photoshop's defaults: the next "Group N", no colour, Pass
/// Through, 100 %. Unlike Stroke the choices are not remembered, as in Photoshop.
pub fn open(app: &mut PhotocraftApp, command: &str) -> DialogId {
    let command = if command == FROM_LAYERS { FROM_LAYERS } else { NEW_GROUP };
    let name = app.session.active().map(|st| st.doc.next_layer_name("Group")).unwrap_or_else(|| "Group 1".into());
    // The window title, as in Photoshop.
    let title = if command == FROM_LAYERS { tl!("New Group from Layers…") } else { tl!("New Group…") };
    params::open_param(app, command, DialogFields::new(), Value::Null, move |fields: &mut DialogFields| {
        let rows = vec![
            Param::new("name", tl!("Name:"), ParamKind::text(name.clone())),
            super::new_layer_ui::color_param(),
            super::blend_options::blend_param("mode", tl!("Mode:"), true, BlendMode::PassThrough),
            super::new_layer_ui::opacity_param(),
        ];
        ParamDialog::command(command, title, rows).values(Bind::bound(fields))
    })
}

/// Only the New Group params are passed to the engine.
pub fn params(f: &DialogFields) -> Value {
    let mut p = Map::new();
    for key in ["name", "color", "mode"] {
        if let Some(text) = f.text(key) {
            p.insert(key.into(), json!(text));
        }
    }
    if let Some(opacity) = f.number("opacity") {
        p.insert("opacity".into(), json!(opacity));
    }
    Value::Object(p)
}

/// OK: runs the command the dialog's key names.
pub(crate) fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let outcome = app.run(&result.key, params(&result.fields));
    super::alerts::report(app, outcome);
}

#[cfg(test)]
mod tests;
