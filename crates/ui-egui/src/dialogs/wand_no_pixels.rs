//! The Magic Wand's "No pixels are selected." warning (#11781): a click whose result leaves
//! nothing selected (Subtract everything, Intersect with a region that doesn't overlap) says so in
//! an alert with OK, instead of the selection silently disappearing. `select.magicWand` itself
//! still just reports `"selected": false`, so scripts and agents see the result without a dialog.

use serde_json::{Map, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

/// The alert's text.
pub fn message() -> &'static str {
    tl!("No pixels are selected.")
}

/// Show the warning, once: a click while it is already open adds no second one.
pub fn warn(app: &mut PhotocraftApp) {
    let text = message();
    if app.ui.dialogs.iter().any(|d| d.kind == DialogKind::Error && d.fields.get("message").and_then(|m| m.as_str()) == Some(text)) {
        return;
    }
    let mut fields = Map::new();
    fields.insert("message".into(), json!(text));
    app.ui.open_dialog(DialogKind::Error, fields);
}
