//! Define Pattern and Define Brush Preset need a name from the dialog. The engine commands keep
//! their default-name fallbacks for programmatic calls; only the dialog refuses an empty one.

use craft::ui::dialog::Alert;
use craft::ui::runtime::env::Label;
use serde_json::{Map, Value};

use crate::PhotocraftApp;

const PATTERN: &str = "edit.definePattern";
const BRUSH_PRESET: &str = "edit.defineBrushPreset";
const PATTERN_ALERT: &str = "You must specify a name for the pattern.";

fn blank(fields: &Map<String, Value>) -> bool {
    fields.get("name").is_none_or(|n| n.as_str().is_none_or(|s| s.trim().is_empty()))
}

/// The held-OK check: an empty or blank name keeps the dialog open. Pattern says why in an alert;
/// Brush Preset does nothing visible (its OK stays drawn enabled).
pub(crate) fn check(app: &mut PhotocraftApp, command: &str, fields: &Map<String, Value>) -> Result<(), Label> {
    if !matches!(command, PATTERN | BRUSH_PRESET) || !blank(fields) {
        return Ok(());
    }
    if command == PATTERN {
        let message = tl!(PATTERN_ALERT).to_owned();
        super::alerts::show(app, Alert::new("Pattern Name", message.clone()));
        return Err(Label::from(message));
    }
    Err(Label::from(String::new()))
}

