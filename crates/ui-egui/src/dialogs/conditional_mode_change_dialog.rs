//! File › Automate › Conditional Mode Change… (#13676), laid out like Photoshop's dialog: a
//! Source Mode section of eight independent checkboxes in two columns with All and None, and a
//! Target Mode section whose Mode list holds the same eight modes (#13678).
//!
//! The checkboxes are one custom row of the command's form: `from` is the list of ticked source
//! ids and `to` the target id, so OK, Reset and `ui.dialog.set` work as for every form. With Bitmap, Duotone or Indexed
//! Color as the target, OK opens that mode's own dialog first, as Photoshop does (#13678).

use craft::behavior::ColumnWidth;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout::GridFit;
use craft::ui::layout_components::{Grid, Row, Stack};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use photocraft_color::ColorMode;
use serde_json::{Value, json};

use crate::PhotocraftApp;

pub(crate) const COMMAND: &str = "file.automate.conditionalModeChange";

/// Photoshop's mode order, for the sources and the targets alike. The checkboxes fill the left
/// column, then the right one, and the ticked ones go to `from` in this order.
const MODES: [(&str, &str); 8] = [
    ("bitmap", "Bitmap"),
    ("grayscale", "Grayscale"),
    ("duotone", "Duotone"),
    ("indexed", "Indexed Color"),
    ("rgb", "RGB Color"),
    ("cmyk", "CMYK Color"),
    ("lab", "Lab Color"),
    ("multichannel", "Multichannel"),
];

/// The rows of the dialog: every source ticked ("any") and RGB Color.
pub(crate) fn rows() -> Vec<Param> {
    let targets = MODES.iter().map(|(id, label)| ((*id).to_owned(), Label::from(tl!(label)))).collect();
    vec![
        Param::section(tl!("Source Mode")),
        Param::custom("from", Label::Empty, sources_row).opens_with(FieldValue::Text("any".into())).full_width(),
        Param::section(tl!("Target Mode")),
        Param::new("to", tl!("Mode"), ParamKind::choice(targets, Some("rgb".into()))),
    ]
}

/// A mode id spelled as the command reads it.
fn norm(v: &str) -> String {
    let v = v.to_ascii_lowercase();
    if v == "gray" { "grayscale".into() } else { v }
}

/// `from` as JSON: the opening "any", the list the checkboxes write, or what `ui.dialog.set` gave.
fn from_value(fields: &DialogFields) -> Value {
    fields.get("from").and_then(|v| serde_json::to_value(v).ok()).unwrap_or(Value::Null)
}

/// Is source `id` ticked? `from` takes every form the command does ("any", one mode, a list),
/// and keeps it as it is until a checkbox changes.
fn ticked(from: &Value, id: &str) -> bool {
    let is = |v: &str| {
        let v = norm(v);
        v == "any" || v == id
    };
    match from {
        Value::Array(a) => a.iter().filter_map(Value::as_str).any(is),
        Value::String(v) => is(v),
        _ => true,
    }
}

/// The two columns of checkboxes, each as wide as its widest label.
const COLUMNS: &[ColumnWidth] = &[ColumnWidth::Content, ColumnWidth::Content];

/// The eight checkboxes in two columns, All and None under them. Laid out by the framework's grid and row: in an egui
/// `Grid` the framework's checkboxes took the whole height left in the dialog, one row each.
fn sources_row(ui: &mut egui::Ui, fields: &mut DialogFields, _: &mut craft::ui::patterns::form_dialogs::CustomContext) -> egui::Response {
    let from = from_value(fields);
    let mut on = MODES.map(|(id, _)| ticked(&from, id));
    let before = on;
    let response = Stack::new()
        .show(ui, |ui, _| {
            Grid::new(COLUMNS).fit(GridFit::Fixed).show(ui, |ui, _| {
                for row in 0..4 {
                    for i in [row, row + 4] {
                        Checkbox::new(Bind::bound(&mut on[i])).label(tl!(MODES[i].1)).show(ui);
                    }
                }
            });
            Row::new().show(ui, |ui, _| {
                for (label, all) in [(tl!("All"), true), (tl!("None"), false)] {
                    if Button::new(label).show(ui).clicked() {
                        on = [all; 8];
                    }
                }
            });
        })
        .output;
    if on != before {
        let ticked: Vec<&str> = MODES.iter().zip(on).filter(|(_, on)| *on).map(|((id, _), _)| *id).collect();
        fields.set("from", FieldValue::Json(json!(ticked)));
    }
    response
}

/// The captured value of a target mode's dialog opened from this one: `{"conditionalModeChange": {from, to}}`, the
/// values its OK passes on.
const PENDING: &str = "conditionalModeChange";

/// The targets whose own dialog (as from Image › Mode) asks for the options first (#13678).
fn target_dialog(to: &str) -> Option<&'static str> {
    match to {
        "bitmap" => Some("image.mode.bitmap"),
        "duotone" => Some("image.mode.duotone"),
        "indexed" => Some("image.mode.indexedColor"),
        _ => None,
    }
}

fn mode_id(m: ColorMode) -> &'static str {
    match m {
        ColorMode::Bitmap => "bitmap",
        ColorMode::Grayscale => "grayscale",
        ColorMode::Duotone => "duotone",
        ColorMode::Indexed => "indexed",
        ColorMode::Rgb => "rgb",
        ColorMode::Cmyk => "cmyk",
        ColorMode::Lab => "lab",
        ColorMode::Multichannel => "multichannel",
    }
}

/// The `ParamFamily::run` of the dialog: with Bitmap, Duotone or Indexed Color as the target of a document the command
/// converts, that mode's own dialog opens instead and nothing converts yet; otherwise OK runs the command.
pub(crate) fn run(app: &mut PhotocraftApp, ctx: &egui::Context, result: ParamResult) {
    if !open_target_dialog(app, &result) {
        crate::filter_dialog::run(app, ctx, result);
    }
}

/// Opens the target mode's dialog for the confirmed `result`, when its target has one and the document converts.
fn open_target_dialog(app: &mut PhotocraftApp, result: &ParamResult) -> bool {
    let to = norm(result.fields.text("to").unwrap_or("rgb"));
    let from = from_value(&result.fields);
    let (Some(command), Some(cur)) = (target_dialog(&to), app.session.active().map(|st| mode_id(st.doc.mode))) else { return false };
    if cur == to || !ticked(&from, cur) {
        return false;
    }
    let Ok(id) = crate::filter_dialog::open(app, command) else { return false };
    let from = if from.is_null() { json!("any") } else { from };
    app.framework.captured.insert(command.to_owned(), json!({PENDING: {"from": from, "to": to}}));
    // A playing action continues after this dialog's OK.
    crate::actions::hand_over(app, result.id, id);
    true
}

/// Whether `captured` is that of a target mode's dialog opened from this one: its preview would run the mode's command
/// on the document not converted yet (Bitmap and Duotone need Grayscale first), so it shows none.
pub(crate) fn is_pending(captured: &Value) -> bool {
    captured.get(PENDING).is_some()
}

/// Whether the dialog `key` can be a target mode's dialog opened from this one.
pub(crate) fn owns_target(key: &str) -> bool {
    ["bitmap", "duotone", "indexed"].iter().any(|to| target_dialog(to) == Some(key))
}

/// The `ParamFamily::run` of the target modes' dialogs: one opened from this dialog runs the command with its options;
/// one opened from Image › Mode runs its own command.
pub(crate) fn run_target(app: &mut PhotocraftApp, ctx: &egui::Context, mut result: ParamResult) {
    let Some(fields) = pending_fields(app, &result.key, &result.fields) else {
        return crate::filter_dialog::run(app, ctx, result);
    };
    result.fields = fields;
    // A Record Again editor's step is this command, not the target mode's.
    let outcome = if crate::actions::record_again::owns_dialog(app, result.id) {
        let params = crate::filter_dialog::command_params(COMMAND, &result.fields);
        crate::actions::record_again::confirm(app, result.id, COMMAND, params)
    } else {
        crate::filter_dialog::run_command(app, ctx, COMMAND, result)
    };
    if let Err(error) = outcome {
        crate::notices::error(app, error);
    }
}

/// What the OK of a target mode's dialog opened from this one answers for a step that judges it: this command's params.
pub(crate) fn editor_answer(app: &PhotocraftApp, key: &str, fields: &DialogFields) -> Option<Value> {
    pending_fields(app, key, fields).map(|fields| crate::filter_dialog::command_params(COMMAND, &fields))
}

/// The target mode's dialog `key`'s `fields` with this dialog's `from` and `to`, when this dialog opened it.
fn pending_fields(app: &PhotocraftApp, key: &str, fields: &DialogFields) -> Option<DialogFields> {
    let Some(Value::Object(pending)) = crate::dialogs::params::captured(app, key).get(PENDING).cloned() else { return None };
    let mut fields = fields.clone();
    for (key, value) in pending {
        fields.set(key, FieldValue::Json(value));
    }
    Some(fields)
}

#[cfg(test)]
#[path = "../conditional_mode_change_controls_tests.rs"]
mod tests;
