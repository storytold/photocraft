//! Pasting vector artwork (an SVG app's clipboard, such as VectorCraft's) offers a choice instead
//! of silently flattening it: Smart Object keeps the original source editable (Layer ▸ Smart
//! Objects ▸ Edit Contents), Path and Shape extract editable geometry, Pixels flattens it, exactly
//! as the ordinary bitmap-clipboard Paste already does (#4410).

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

const MARK: &str = "__pasteAs";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Opens the dialog in place of an implicit Paste, only when the clipboard's vector source
/// survived paste preparation (an SVG app's clipboard, not an ordinary bitmap copy): `None`
/// otherwise, so the caller falls through to the ordinary route. Automation already states its
/// mode through `edit.paste`'s own `as` param and never sees this dialog.
pub fn offer(app: &mut PhotocraftApp) -> Option<Result<Value, String>> {
    if app.automation_input || !app.session.clipboard.as_ref().is_some_and(|clip| clip.svg.is_some()) {
        return None;
    }
    if app.ui.dialogs.iter().any(|d| owns(&d.fields)) {
        return Some(Ok(json!({"confirmationPending": true})));
    }
    // `params_of` (`filter_dialog.rs`) sends every field not prefixed `__` as the command's own
    // params, flat: `as` is `edit.paste`'s own param, not a nested one.
    let fields = json!({MARK: true, "__label": "Paste", "__command": "edit.paste", "as": "smartObject"});
    let id = app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
    Some(Ok(json!({"dialog": id, "confirmationPending": true})))
}

pub fn body(ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    ui.label(tl!("The clipboard holds vector artwork. Paste it as:"));
    ui.add_space(4.0);
    let mut mode = fields.get("as").and_then(Value::as_str).unwrap_or("smartObject").to_string();
    for (value, label) in [("smartObject", "Smart Object"), ("pixels", "Pixels"), ("path", "Path"), ("shape", "Shape Layer")] {
        ui.radio_value(&mut mode, value.to_string(), tl!(label));
    }
    fields.insert("as".into(), json!(mode));
}
