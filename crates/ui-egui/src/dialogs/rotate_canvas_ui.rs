//! Image › Image Rotation › Arbitrary: the reference application's Rotate Canvas dialog (#14462).
//! One row: the Angle field (`°`, two decimals) behind a scrubby label, then the Clockwise and
//! Counter clockwise radios. No slider, no interpolation choice, no preview, and Alt does not turn
//! Cancel into Reset.
//!
//! It stays the command's schema dialog (`filter_dialog::open`), so the ruler pre-fill, Enter /
//! Escape and `ui.dialog.*` work as for every other command dialog; only its rows, title and
//! buttons differ. The command's `interpolation` param stays for scripts and recorded actions: the
//! dialog just has no row for it, so the command's own default applies.

use craft::behavior::NumberCodec;
use craft::ui::choice::{Radio, RadioGroup};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, SideLabel, Sizing};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamKind};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::NumberField;
use serde_json::{Map, Value, json};

pub const COMMAND: &str = "image.rotation.arbitrary";
/// The dialog's title, as the reference application names it.
pub fn title() -> Label {
    Label::from(tl!("Rotate Canvas").to_string())
}
/// The Angle field's shown decimals: hundredths of a degree (#12035).
pub const DECIMALS: u32 = 2;
/// What an arrow press moves the angle by: its last shown decimal (Shift moves ten of them).
const STEP: f64 = 0.01;
/// The field's range, should the command's schema ever lose it.
const RANGE: (f64, f64) = (-359.99, 359.99);

/// The Angle field's range, from the command's schema.
pub fn angle_range() -> (f64, f64) {
    photocraft_engine::commands::find(COMMAND)
        .into_iter()
        .flat_map(|c| Param::parse_list(c.params))
        .find_map(|p| match p.kind {
            ParamKind::Range { min, max, .. } if p.key == "angle" && min <= max => Some((min, max)),
            _ => None,
        })
        .unwrap_or(RANGE)
}

/// `angle` kept in the field's range at its two decimals; a non-finite one is 0.
pub fn settle(angle: f64) -> f64 {
    let (min, max) = angle_range();
    if !angle.is_finite() {
        return 0.0;
    }
    let scale = 10f64.powi(DECIMALS as i32);
    ((angle.clamp(min, max)) * scale).round() / scale
}

/// The dialog's one row, in place of the schema's Angle, Direction and Interpolation rows: the row
/// keeps `angle` and its companion `direction`.
pub(crate) fn params() -> Vec<Param> {
    vec![Param::custom("angle", Label::Empty, |ui, fields, _| row(ui, fields)).companions(["direction"]).opens_with(FieldValue::Number(0.0)).full_width()]
}

/// The values the dialog opens with besides the ruler pre-fill: Clockwise, as on a fresh dialog.
pub(crate) fn open_values(fields: &mut DialogFields) {
    if !fields.contains("angle") {
        fields.set("angle", FieldValue::Number(0.0));
    }
    if fields.text("direction").is_none() {
        fields.set("direction", FieldValue::Text("cw".into()));
    }
}

/// The command params of the dialog's values: the angle kept in the field's range at two decimals
/// (an agent's `ui.dialog.set` reaches the row's key unchecked), the direction as chosen.
pub(crate) fn command_params(map: &mut Map<String, Value>) {
    if let Some(angle) = map.get("angle").and_then(Value::as_f64) {
        map.insert("angle".into(), json!(settle(angle)));
    }
}

fn row(ui: &mut egui::Ui, fields: &mut DialogFields) -> egui::Response {
    let (min, max) = angle_range();
    let angle = settle(fields.number("angle").unwrap_or(0.0));
    let cw = fields.text("direction") != Some("ccw");
    let caption = Label::from(tl!("Angle").to_string());
    Row::new()
        .show(ui, |ui, _| {
            Field::new(caption.clone()).side(SideLabel::new().scrub(true)).id_salt("angle").sizing(Sizing::Content).show(ui, |ui, _| {
                let codec = NumberCodec::new(Env::of(ui).units().reader(None), DECIMALS as usize);
                let field = NumberField::new(caption.clone(), Bind::given(Some(angle)))
                    .range(min..=max)
                    .step(STEP)
                    .codec(codec)
                    .commit_decimals(DECIMALS as usize)
                    .unit("°")
                    // The reference application scrubs 1° a point of drag; craft's NumberField scrubs by its key step
                    // (0.01 here) until it takes a scrub step of its own.
                    .scrub_by_label()
                    .show(ui);
                if let Some(next) = field.value().copied().flatten() {
                    fields.set("angle", FieldValue::Number(settle(next)));
                }
            });
            let direction = if cw { "cw" } else { "ccw" }.to_owned();
            let group = RadioGroup::new(tl!("Direction").to_owned(), Bind::given(direction)).show(ui, |ui, _| {
                Row::new().show(ui, |ui, _| {
                    Radio::new("cw".to_owned()).label(tl!("Clockwise").to_owned()).show(ui);
                    Radio::new("ccw".to_owned()).label(tl!("Counter clockwise").to_owned()).show(ui);
                });
            });
            if let Some(picked) = group.output.value().cloned() {
                fields.set("direction", FieldValue::Text(picked));
            }
        })
        .output
}
