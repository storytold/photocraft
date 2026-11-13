//! File › Automate › Conditional Mode Change… (#13676), laid out like Photoshop's dialog: a
//! Source Mode group of eight independent checkboxes in two columns with All and None, and a
//! Target Mode group whose Mode: list holds the same eight modes (#13678).
//!
//! It is the command's form dialog with its own body: `from` is the list of ticked source ids
//! and `to` the target id, so OK, Alt+Reset and `ui.dialog.set` work as for every form.

use serde_json::{Map, Value, json};

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

/// The fields the dialog opens with: every source ticked ("any") and RGB Color.
pub(crate) fn fields() -> Value {
    json!({"from": "any", "to": "rgb"})
}

/// Is this form the Conditional Mode Change dialog?
pub(crate) fn owns(f: &Map<String, Value>) -> bool {
    f.get("__command").and_then(Value::as_str) == Some(COMMAND) && f.contains_key("__form")
}

/// A mode id spelled as the command reads it.
fn norm(v: &str) -> String {
    let v = v.to_ascii_lowercase();
    if v == "gray" { "grayscale".into() } else { v }
}

/// Is source `id` ticked? `from` takes every form the command does ("any", one mode, a list),
/// and keeps it as it is until a checkbox changes.
fn ticked(f: &Map<String, Value>, id: &str) -> bool {
    let is = |v: &str| {
        let v = norm(v);
        v == "any" || v == id
    };
    match f.get("from") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).any(is),
        Some(Value::String(v)) => is(v),
        _ => true,
    }
}

/// The dialog body.
pub(crate) fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.spacing_mut().item_spacing.y = 6.0;
    let mut on = MODES.map(|(id, _)| ticked(f, id));
    let mut changed = false;

    crate::widgets::section_label(ui, tl!("Source Mode"));
    crate::widgets::hairline(ui);
    egui::Grid::new("conditional-mode-sources").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
        for row in 0..4 {
            for i in [row, row + 4] {
                changed |= crate::widgets::checkbox(ui, &mut on[i], MODES[i].1).changed();
            }
            ui.end_row();
        }
        for (label, all) in [(tl!("All"), true), (tl!("None"), false)] {
            if crate::widgets::secondary_button(ui, label, 84.0).clicked() {
                on = [all; 8];
                changed = true;
            }
        }
        ui.end_row();
    });
    if changed {
        let from: Vec<&str> = MODES.iter().zip(on).filter(|(_, on)| *on).map(|((id, _), _)| *id).collect();
        f.insert("from".into(), json!(from));
    }

    ui.add_space(4.0);
    crate::widgets::section_label(ui, tl!("Target Mode"));
    crate::widgets::hairline(ui);
    ui.horizontal(|ui| {
        ui.label(tl!("Mode:"));
        let mut to = norm(f.get("to").and_then(Value::as_str).unwrap_or("rgb"));
        let opts: Vec<(String, &str)> = MODES.iter().map(|(id, l)| (id.to_string(), tl!(l))).collect();
        if crate::widgets::dropdown(ui, "conditional-mode-target", tl!("Mode"), &mut to, &opts, 150.0) {
            f.insert("to".into(), json!(to));
        }
    });
}

#[cfg(test)]
#[path = "../conditional_mode_change_controls_tests.rs"]
mod tests;
