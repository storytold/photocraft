//! File › Automate › Conditional Mode Change… (#13676), laid out like Photoshop's dialog: a
//! Source Mode group of eight independent checkboxes in two columns with All and None, and a
//! Target Mode group whose Mode: list holds the modes the command converts to.
//!
//! It is the command's form dialog with its own body: `from` is the list of ticked source ids
//! and `to` the target id, so OK, Alt+Reset and `ui.dialog.set` work as for every form.

use serde_json::{Map, Value, json};

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

/// The fields the dialog opens with: the sources and target of the last run that converted a
/// document (#13680), else Photoshop's first open, RGB Color to Grayscale. Neither follows the
/// document's mode.
pub(crate) fn fields(app: &crate::PhotocraftApp) -> Value {
    let saved = app.session.prefs().dialogs.get(COMMAND).and_then(Value::as_object);
    saved.and_then(|s| settings(s.get("from"), s.get("to"))).unwrap_or_else(|| json!({"from": ["rgb"], "to": "grayscale"}))
}

/// OK ran the command: keep its sources and target for the next open, but only when it converted
/// the document. Cancel never reaches here, and a run that converted nothing keeps the settings
/// from before, as in Photoshop.
pub(crate) fn remember(app: &mut crate::PhotocraftApp, params: &Value, result: &Value) {
    if result.get("changed").and_then(Value::as_bool) != Some(true) {
        return;
    }
    if let Some(saved) = settings(params.get("from"), params.get("to")) {
        app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), saved));
    }
}

/// `from` and `to` as stored settings: the known source ids in list order (`"any"` meaning every
/// one) and a known mode id. Anything else (a hand-edited preferences file, a value set over the
/// control channel) is `None`, so the defaults are used instead of it.
fn settings(from: Option<&Value>, to: Option<&Value>) -> Option<Value> {
    let to = norm(to?.as_str()?);
    if !SOURCES.iter().any(|(id, _)| *id == to) {
        return None;
    }
    let given: Vec<String> = match from? {
        Value::String(v) => vec![norm(v)],
        Value::Array(a) => a.iter().map(|v| v.as_str().map(norm)).collect::<Option<_>>()?,
        _ => return None,
    };
    let from: Vec<&str> = SOURCES.iter().map(|(id, _)| *id).filter(|id| given.iter().any(|v| v == "any" || v == id)).collect();
    (!from.is_empty()).then(|| json!({"from": from, "to": to}))
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
    let mut on = SOURCES.map(|(id, _)| ticked(f, id));
    let mut changed = false;

    crate::widgets::section_label(ui, tl!("Source Mode"));
    crate::widgets::hairline(ui);
    egui::Grid::new("conditional-mode-sources").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
        for row in 0..4 {
            for i in [row, row + 4] {
                changed |= crate::widgets::checkbox(ui, &mut on[i], SOURCES[i].1).changed();
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
        let from: Vec<&str> = SOURCES.iter().zip(on).filter(|(_, on)| *on).map(|((id, _), _)| *id).collect();
        f.insert("from".into(), json!(from));
    }

    ui.add_space(4.0);
    crate::widgets::section_label(ui, tl!("Target Mode"));
    crate::widgets::hairline(ui);
    ui.horizontal(|ui| {
        ui.label(tl!("Mode:"));
        let mut to = norm(f.get("to").and_then(Value::as_str).unwrap_or("rgb"));
        let opts: Vec<(String, &str)> = TARGETS.iter().map(|(id, l)| (id.to_string(), tl!(l))).collect();
        if crate::widgets::dropdown(ui, "conditional-mode-target", tl!("Mode"), &mut to, &opts, 150.0) {
            f.insert("to".into(), json!(to));
        }
    });
}

#[cfg(test)]
#[path = "../conditional_mode_change_controls_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../conditional_mode_change_sticky_tests.rs"]
mod sticky_tests;
