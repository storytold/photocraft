//! Fit Image's bounds belong to its dialog. The engine still accepts an unconstrained side.

use std::cell::Cell;

use serde_json::{Value, json};

use super::Fields;
use crate::panels::options_bar::range_alert;
use crate::state::UiState;

const COMMAND: &str = "file.automate.fitImage";
const RANGE: std::ops::RangeInclusive<f64> = 1.0..=300_000.0;
const WIDTH: (&str, &str) = ("width", "__fitImageWidth");
const HEIGHT: (&str, &str) = ("height", "__fitImageHeight");
const SIDES: [(&str, &str); 2] = [WIDTH, HEIGHT];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Alert {
    Closest,
    Invalid,
}

impl Alert {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Closest => "A value between 1 and 300000 is required. Closest value inserted.",
            Self::Invalid => "Invalid numeric entry. A value between 1 and 300000 is required. Previous value inserted.",
        }
    }
}

pub(super) fn owns(fields: &Fields) -> bool {
    fields.contains_key("__form") && fields.get("__command").and_then(Value::as_str) == Some(COMMAND)
}

fn bounded(number: Option<f64>, alert: &Cell<Option<Alert>>) -> Option<f64> {
    let Some(number) = number.filter(|value| value.is_finite()) else {
        alert.set(Some(Alert::Invalid));
        return None;
    };
    if !RANGE.contains(&number) {
        alert.set(Some(Alert::Closest));
    }
    Some(number.clamp(*RANGE.start(), *RANGE.end()))
}

/// Covers dialog confirmation when automation supplies values without rendering the fields.
pub(super) fn preflight(ui: &mut UiState, id: u64) -> Result<(), String> {
    if let Some(dialog) = ui.dialog_mut(id).filter(|dialog| owns(&dialog.fields))
        && let Some(alert) = validate(&mut dialog.fields)
    {
        range_alert(ui, alert.message());
        if alert == Alert::Invalid {
            return Err(tl!(alert.message()).into());
        }
    }
    Ok(())
}

fn validate(fields: &mut Fields) -> Option<Alert> {
    let mut result = None;
    for (key, accepted) in SIDES {
        let alert = Cell::new(None);
        let number = fields.get(key).and_then(|value| value.as_f64().or_else(|| value.as_str().and_then(crate::widgets::parse_num)));
        let value = bounded(number, &alert)
            .or_else(|| fields.get(accepted).and_then(Value::as_f64))
            .or_else(|| fields.get("__formOrig")?.get(key)?.as_f64())
            .unwrap_or(*RANGE.start())
            .clamp(*RANGE.start(), *RANGE.end()) as i64;
        fields.insert(key.into(), json!(value));
        fields.insert(accepted.into(), json!(value));
        // An invalid entry needs acknowledgment before the dialog can run.
        if result != Some(Alert::Invalid) {
            result = alert.get().or(result);
        }
    }
    result
}

pub(super) fn finish(ui: &mut UiState, interactive: bool, alert: Option<Alert>, outcome: Option<bool>) -> Option<bool> {
    if interactive
        && outcome != Some(false)
        && let Some(alert) = alert
    {
        range_alert(ui, alert.message());
        if alert == Alert::Invalid {
            return None;
        }
    }
    outcome
}

/// Drops the character a consumed shortcut types, from position `at` on (where the shortcut was): the
/// first text event after it that is one character and not a digit (Option+W types `∑` on macOS). Text
/// typed before the shortcut, and what follows the character in the same frame (digits, a number's
/// following characters), is an entry and stays.
pub(super) fn drop_text_after(events: &mut Vec<egui::Event>, at: usize) {
    let typed = events
        .iter()
        .enumerate()
        .skip(at)
        .find_map(|(i, e)| matches!(e, egui::Event::Text(t) if t.chars().count() == 1 && !t.chars().any(|c| c.is_ascii_digit())).then_some(i));
    if let Some(i) = typed {
        events.remove(i);
    }
}

/// The dialog as the reference application lays it out (#13851): Width, Height and Don't Enlarge
/// in a "Constrain Within" group, each size followed by "pixels". Alt+W and Alt+H go to the sizes;
/// Tab goes Width, Height, Don't Enlarge, then on to OK and Cancel (drawn after this).
pub(super) fn body(ui: &mut egui::Ui, fields: &mut Fields) -> Option<Alert> {
    let mnemonic = |ui: &mut egui::Ui, key: egui::Key| {
        ui.input_mut(|i| {
            let at = i.events.iter().position(|e| matches!(e, egui::Event::Key { key: k, pressed: true, modifiers, .. } if *k == key && modifiers.alt));
            let hit = i.consume_key(egui::Modifiers::ALT, key);
            // The character Alt+key types (on macOS) isn't an entry: it arrives after the shortcut. Text that
            // came before it in the same frame is one, and stays for the field that has the focus.
            if let (true, Some(at)) = (hit, at) {
                drop_text_after(&mut i.events, at);
            }
            hit
        })
    };
    let (alt_w, alt_h) = (mnemonic(ui, egui::Key::W), mnemonic(ui, egui::Key::H));
    let mut alert = None;
    ui.label(tl!("Constrain Within"));
    // As narrow as the fields (#4673): a tight group around a tight grid.
    egui::Frame::group(ui.style()).inner_margin(6.0).show(ui, |ui| {
        let keys = crate::view_cmds::form_keys(fields);
        let has = |key: &str| keys.iter().any(|f| f == key);
        egui::Grid::new("form-dialog").num_columns(3).spacing([6.0, 6.0]).show(ui, |ui| {
            for (key, side_keys, label, jump) in [("width", WIDTH, tl!("Width"), alt_w), ("height", HEIGHT, tl!("Height"), alt_h)] {
                if has(key) {
                    alert = side(ui, fields, side_keys, label, jump).or(alert);
                    ui.end_row();
                }
            }
        });
        // Under the sizes, outside their columns so its label doesn't widen them.
        if has("dontEnlarge") {
            let mut enabled = fields.get("dontEnlarge").and_then(Value::as_bool).unwrap_or(false);
            crate::field_tab::register(ui.ctx(), ui.next_auto_id());
            ui.checkbox(&mut enabled, tl!("Don't Enlarge"));
            fields.insert("dontEnlarge".into(), json!(enabled));
        }
    });
    alert
}

/// One size row: "`label`:", its field (named `label`) and "pixels". `jump` (its Alt key) moves
/// the keyboard to the field with its text selected.
fn side(ui: &mut egui::Ui, fields: &mut Fields, (key, accepted): (&str, &str), label: &str, jump: bool) -> Option<Alert> {
    let input = fields.get(key).and_then(Value::as_i64);
    let mut value = input.or_else(|| fields.get(accepted).and_then(Value::as_i64)).or_else(|| fields.get("__formOrig")?.get(key)?.as_i64()).unwrap_or(1);
    let alert = Cell::new(None);
    let parsed = Cell::new(false);
    ui.label(format!("{label}:"));
    let field = ui.next_auto_id();
    crate::field_tab::register(ui.ctx(), field);
    if jump {
        crate::field_tab::focus(ui.ctx(), field);
    }
    let response = ui
        .add(egui::DragValue::new(&mut value).range(RANGE).clamp_existing_to_range(false).update_while_editing(false).custom_parser(|text| {
            parsed.set(true);
            bounded(crate::widgets::parse_num(text), &alert)
        }))
        .accessible_name(label);
    if response.changed() || parsed.get() || input.is_some_and(|input| RANGE.contains(&(input as f64))) {
        let value = value.clamp(*RANGE.start() as i64, *RANGE.end() as i64);
        fields.insert(key.into(), json!(value));
        fields.insert(accepted.into(), json!(value));
    }
    ui.label(tl!("pixels"));
    alert.get()
}
