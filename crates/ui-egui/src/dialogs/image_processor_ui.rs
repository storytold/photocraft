//! The File Type section of the Image Processor dialog. Its rows (`<format>/<option>`, one group per format) are
//! built with the rest of the form by `view_cmds::dialogs::params_for`; this module folds their values into the
//! per-format objects the command takes and refuses an OK with no format ticked.

use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use photocraft_engine::file_cmds::IMAGE_PROCESSOR_FORMATS;
use serde_json::{Map, Value};

use crate::PhotocraftApp;

/// The command id of the dialog.
pub(crate) const COMMAND: &str = "file.scripts.imageProcessor";

/// All formats share resize options. Keep the saved dimensions and resize tick when a format is switched off.
pub(crate) fn resize_rows(format: &str) -> [Param; 3] {
    [
        Param::new(format!("{format}/resize"), tl!("Resize to Fit"), ParamKind::bool(false)).enabled_when(format!("{format}/enabled"), true),
        Param::new(format!("{format}/width"), tl!("W"), ParamKind::int(Some(1), Some(30_000), 1)).enabled_when(format!("{format}/resize"), true),
        Param::new(format!("{format}/height"), tl!("H"), ParamKind::int(Some(1), Some(30_000), 1)).enabled_when(format!("{format}/resize"), true),
    ]
}

/// Build the dimension rows' enabled condition from both switches. The hosted schema accepts one predicate per row.
pub(crate) fn apply_resize_gates(params: &mut [Param], fields: &DialogFields) {
    for param in params {
        let Some((format, "width" | "height")) = param.key.split_once('/') else { continue };
        let enabled = format!("{format}/enabled");
        if !fields.bool(&enabled).unwrap_or(format == "jpeg") {
            param.enabled_when = Some((enabled, vec![true.into()]));
        }
    }
}

/// Folds the `<format>/<option>` values into the `{jpeg: {..}, psd: {..}, tiff: {..}}` objects the command takes.
pub(crate) fn nest(map: &mut Map<String, Value>) {
    for (format, _) in IMAGE_PROCESSOR_FORMATS {
        let prefix = format!("{format}/");
        let keys: Vec<String> = map.keys().filter(|key| key.starts_with(&prefix)).cloned().collect();
        let mut options = Map::new();
        for key in keys {
            if let Some(value) = map.remove(&key) {
                options.insert(key[prefix.len()..].to_owned(), value);
            }
        }
        if !options.is_empty() {
            map.insert(format.to_owned(), Value::Object(options));
        }
    }
}

/// Whether the Image Processor on the source `source` waits for the open Free Transform: document sources read
/// committed pixels, not the pending preview; Folder processing stays usable. The command (`run_command_at_depth`)
/// and the dialog's OK ([`check`]) both refuse by it.
pub(crate) fn waits_for_transform(app: &PhotocraftApp, source: Option<&str>) -> bool {
    app.ui.transform.is_some()
        && match source {
            Some("open") => true,
            Some("current") => app.transform_preview.as_ref().is_none_or(|preview| app.session.active().is_some_and(|st| st.doc.id == preview.doc.id)),
            _ => false,
        }
}

/// The `ParamFamily::check` of the dialog: OK is refused, and the dialog stays open, while a document source waits for
/// the open Free Transform or no file type is ticked.
pub(crate) fn check(app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    if waits_for_transform(app, result.fields.text("source")) {
        return Err(Label::from(tl!("Commit or cancel the transform first")));
    }
    if IMAGE_PROCESSOR_FORMATS.iter().any(|(format, _)| result.fields.bool(&format!("{format}/enabled")) == Some(true)) {
        Ok(())
    } else {
        Err(Label::from(tl!("You must save to at least one file type.")))
    }
}

#[cfg(test)]
mod tests;
