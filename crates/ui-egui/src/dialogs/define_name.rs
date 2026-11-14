//! Define Pattern and Define Brush Preset need a name from the dialog. The engine commands keep
//! their default-name fallbacks for programmatic calls; only the dialog refuses an empty one.

use serde_json::{Value, json};

use crate::state::{DialogKind, UiState};

const PATTERN: &str = "edit.definePattern";
const BRUSH_PRESET: &str = "edit.defineBrushPreset";
const PATTERN_ALERT: &str = "You must specify a name for the pattern.";

fn blank(fields: &super::Fields) -> bool {
    fields.get("name").is_none_or(|n| n.as_str().is_none_or(|s| s.trim().is_empty()))
}

/// An empty or blank name keeps the dialog open. Pattern says why in an alert; Brush Preset
/// does nothing visible (its OK stays drawn enabled).
pub(super) fn preflight(ui: &mut UiState, id: u64) -> Result<(), String> {
    let Some(d) = ui.dialogs.iter().find(|d| d.id == id) else {
        return Ok(());
    };
    let command = d.fields.get("__command").and_then(Value::as_str);
    let generic = d.fields.get("__filter").and_then(Value::as_bool) == Some(true) && !d.fields.contains_key("__spec");
    if !generic || !matches!(command, Some(PATTERN | BRUSH_PRESET)) || !blank(&d.fields) {
        return Ok(());
    }
    if command == Some(PATTERN) {
        let message = tl!(PATTERN_ALERT).to_owned();
        let shown = ui.dialogs.iter().any(|d| d.kind == DialogKind::Error && d.fields.get("message").and_then(Value::as_str) == Some(&message));
        if !shown {
            ui.open_dialog(DialogKind::Error, json!({ "message": message, "__title": "Pattern Name" }).as_object().cloned().unwrap_or_default());
        }
        return Err(message);
    }
    Err(String::new())
}
