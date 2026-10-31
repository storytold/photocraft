//! Image › Adjustments › Equalize with a selection: the dialog that asks what to equalize, as in
//! Photoshop. Equalize Selected Area Only (the default) evenly distributes the selected pixels;
//! Equalize Entire Image Based On Selected Area maps the layer within the canvas by the selected
//! pixels' histogram. OK runs `image.adjustments.equalize` with `selectedAreaOnly`. Without a selection,
//! Equalize runs at once.
//!
//! The dialog is an ordinary one in the UI state, so `ui.inspect`, `ui.dialog.confirm` and
//! `ui.dialog.cancel` drive it.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

const MARK: &str = "__equalize";
pub const COMMAND: &str = "image.adjustments.equalize";
const KEY: &str = "selectedAreaOnly";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Whether Equalize asks first: the active document has a selection and Equalize can run.
pub fn asks(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|st| st.doc.selection.is_some()) && app.session.is_enabled(COMMAND)
}

pub fn open(app: &mut PhotocraftApp) -> u64 {
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("Equalize"));
    f.insert(KEY.into(), json!(true));
    app.ui.open_dialog(DialogKind::Command, f)
}

pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let mut only = f.get(KEY).and_then(Value::as_bool).unwrap_or(true);
    ui.radio_value(&mut only, true, tl!("Equalize Selected Area Only"));
    ui.radio_value(&mut only, false, tl!("Equalize Entire Image Based On Selected Area"));
    f.insert(KEY.into(), json!(only));
}

#[cfg(test)]
mod tests;
