//! Clicking a Recent file on the Home screen whose file is gone asks what to do instead of failing
//! to open it: "Could not find “name”". Cancel leaves the list as it is; Remove from Recent takes
//! the entry off it. The file's name is the last place it was seen at, not a promise it is anywhere.
//!
//! The prompt is an ordinary dialog in the UI state: `ui.dialog.confirm` removes the entry and
//! `ui.dialog.cancel` keeps it.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

const MARK: &str = "__recentMissing";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Ask what to do about the recent file at `path`, which is not there. One prompt at a time.
pub fn open(app: &mut PhotocraftApp, path: &str) -> u64 {
    if let Some(d) = app.ui.dialogs.iter().find(|d| owns(&d.fields)) {
        return d.id;
    }
    let name = crate::file_open::display_name(path);
    let folder = std::path::Path::new(path).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let title = crate::i18n::fmt(tl!("Could not find “{name}”"), &[("name", &name)]);
    let message = crate::i18n::fmt(tl!("It may have been moved, renamed or deleted. It was last at {folder}."), &[("folder", &folder)]);
    let fields = json!({MARK: true, "__label": title, "message": message, "path": path});
    app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap_or_default())
}

pub fn body(ui: &mut egui::Ui, fields: &Map<String, Value>) {
    ui.label(fields.get("message").and_then(Value::as_str).unwrap_or_default());
}

/// Remove from Recent: the entry goes off the list, as the × does. The file is not touched.
pub fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let path = fields.get("path").and_then(Value::as_str).ok_or(tl!("The recent file is not named"))?.to_string();
    app.remove_recent(&path);
    Ok(json!({"removed": path}))
}
