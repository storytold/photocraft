//! A save asks before replacing a file that the save dialog did not ask about. The dialog asks
//! only about the name it answers, and the app can change that name afterwards: it gives a name
//! without an extension the suggested file's (#5039), and lowercases the extension when
//! Preferences ▸ File Handling › Lowercase Extension is on. When a file exists under the changed
//! name, Replace writes over it and Cancel ends the save like the save dialog's Cancel.
//!
//! The prompt is an ordinary dialog in the UI state: `ui.dialog.confirm` replaces the file and
//! `ui.dialog.cancel` keeps it. The file dialog stays open until it is answered
//! ([`PhotocraftApp::file_dialog_open`]).

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::file_open::display_name;
use crate::state::DialogKind;

const MARK: &str = "__replaceSaved";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Opens the prompt about `path`, the file a save writes to for `answered`, the save dialog's
/// answer.
pub(crate) fn open(app: &mut PhotocraftApp, answered: &str, path: &str) {
    let message = crate::i18n::fmt(tl!("“{name}” already exists. Replace it?"), &[("name", display_name(path).as_str())]);
    let detail = crate::i18n::fmt(tl!("The name in the save dialog was “{name}”."), &[("name", display_name(answered).as_str())]);
    let fields = json!({MARK: true, "__label": "Save", "message": message, "detail": detail, "path": path});
    app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
}

pub fn body(ui: &mut egui::Ui, fields: &Map<String, Value>) {
    // One direction-aware body: in a left-to-right UI the blocks keep egui's default layout; in a
    // right-to-left one they hug the reading start and the question reads from the right. The
    // file name inside the message keeps its own left-to-right run.
    ui.scope_builder(egui::UiBuilder::new().layout(crate::text::direction::body_layout()), |ui| {
        ui.label(fields.get("message").and_then(Value::as_str).unwrap_or_default());
        ui.add_space(6.0);
        ui.label(fields.get("detail").and_then(Value::as_str).unwrap_or_default());
    });
}

/// Replace: the save the prompt held back.
pub fn confirm(app: &mut PhotocraftApp) -> Result<Value, String> {
    app.answer_replace(true)
}

/// Cancel: nothing is written.
pub fn cancel(app: &mut PhotocraftApp) -> Result<Value, String> {
    app.answer_replace(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    /// The prompt's message and detail drawn in `dir`, with the content area they were drawn in.
    fn drawn(dir: crate::text::direction::Dir) -> (egui::Rect, egui::Rect, egui::Rect) {
        let _scope = crate::text::direction::direction_scope(dir);
        let mut fields = Map::new();
        fields.insert("message".into(), json!("Replace it?"));
        fields.insert("detail".into(), json!("The name in the save dialog changed."));
        let area = std::cell::Cell::new(egui::Rect::ZERO);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(400.0, 200.0)).build_ui(|ui| {
            area.set(ui.max_rect());
            body(ui, &fields);
        });
        h.run();
        (h.get_by_label("Replace it?").rect(), h.get_by_label("The name in the save dialog changed.").rect(), area.get())
    }

    /// The question reads from the reading start: left in a left-to-right UI, right in a
    /// right-to-left one.
    #[test]
    fn the_prompt_reads_from_the_reading_start() {
        let (ltr, ltr_detail, ltr_area) = drawn(crate::text::direction::Dir::Ltr);
        let (rtl, rtl_detail, rtl_area) = drawn(crate::text::direction::Dir::Rtl);
        assert!(
            (ltr.left() - ltr_area.left()).abs() < 1.0 && (ltr_detail.left() - ltr_area.left()).abs() < 1.0,
            "left-to-right: the prompt starts at the left edge: {ltr:?} {ltr_detail:?} in {ltr_area:?}"
        );
        assert!(
            (rtl.right() - rtl_area.right()).abs() < 1.0 && (rtl_detail.right() - rtl_area.right()).abs() < 1.0,
            "right-to-left: the prompt ends at the right edge: {rtl:?} {rtl_detail:?} in {rtl_area:?}"
        );
    }
}
