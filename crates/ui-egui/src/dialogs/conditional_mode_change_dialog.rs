//! File › Automate › Conditional Mode Change… (#13676), laid out like Photoshop's dialog: a
//! Source Mode section of eight independent checkboxes in two columns with All and None, and a
//! Target Mode section whose Mode list holds the modes the command converts to.
//!
//! The checkboxes are one custom row of the command's form: `from` is the list of ticked source
//! ids and `to` the target id, so OK, Reset and `ui.dialog.set` work as for every form.

use craft::behavior::ColumnWidth;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout::GridFit;
use craft::ui::layout_components::{Grid, Row, Stack};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamKind};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};

pub(crate) const COMMAND: &str = "file.automate.conditionalModeChange";

/// Photoshop's mode order. The checkboxes fill the left column, then the right one, and the
/// ticked ones go to `from` in this order.
const SOURCES: [(&str, &str); 8] = [
    ("bitmap", "Bitmap"),
    ("grayscale", "Grayscale"),
    ("duotone", "Duotone"),
    ("indexed", "Indexed Color"),
    ("rgb", "RGB Color"),
    ("cmyk", "CMYK Color"),
    ("lab", "Lab Color"),
    ("multichannel", "Multichannel"),
];

/// The targets the command converts to, in the same order.
const TARGETS: [(&str, &str); 4] = [("grayscale", "Grayscale"), ("rgb", "RGB Color"), ("cmyk", "CMYK Color"), ("lab", "Lab Color")];

/// The rows of the dialog: every source ticked ("any") and RGB Color.
pub(crate) fn rows() -> Vec<Param> {
    let targets = TARGETS.iter().map(|(id, label)| ((*id).to_owned(), Label::from(tl!(label)))).collect();
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
    let mut on = SOURCES.map(|(id, _)| ticked(&from, id));
    let before = on;
    let response = Stack::new()
        .show(ui, |ui, _| {
            Grid::new(COLUMNS).fit(GridFit::Fixed).show(ui, |ui, _| {
                for row in 0..4 {
                    for i in [row, row + 4] {
                        Checkbox::new(Bind::bound(&mut on[i])).label(tl!(SOURCES[i].1)).show(ui);
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
        let ticked: Vec<&str> = SOURCES.iter().zip(on).filter(|(_, on)| *on).map(|((id, _), _)| *id).collect();
        fields.set("from", FieldValue::Json(json!(ticked)));
    }
    response
}

#[cfg(test)]
#[path = "../conditional_mode_change_controls_tests.rs"]
mod tests;
