//! Image › Image Rotation › Arbitrary: the reference application's Rotate Canvas dialog (#14462).
//! One row: the Angle field (`°`, two decimals) behind a scrubby label that moves it 1° a point,
//! then the Clockwise and Counter clockwise radios. No slider, no interpolation choice, no
//! preview, and Alt does not turn Cancel into Reset.
//!
//! It stays a schema dialog of `filter_dialog` (`__filter`), so remembering on OK, the ruler
//! pre-fill, Enter / Escape and `ui.dialog.*` work as before; only its fields and body differ.
//! The command's `interpolation` param stays for scripts and recorded actions: the dialog just
//! doesn't send one, so the command's own default applies.

use serde_json::{Map, Value, json};

use crate::theme::Tokens;

pub const COMMAND: &str = "image.rotation.arbitrary";
pub const TITLE: &str = "Rotate Canvas";
/// The Angle field's shown decimals: hundredths of a degree (#12035).
pub const DECIMALS: u32 = 2;
/// Degrees a point of drag on the Angle label moves the angle by, as measured.
pub const SCRUB_STEP: f32 = 1.0;
/// The field's range, should the command's schema ever lose it.
const RANGE: (f32, f32) = (-359.99, 359.99);

/// Whether `f` is the Rotate Canvas dialog (the command's own schema dialog).
pub fn owns(f: &Map<String, Value>) -> bool {
    f.get("__command").and_then(Value::as_str) == Some(COMMAND) && f.contains_key("__filter") && !f.contains_key("__spec")
}

/// What the dialog opens with on top of the schema defaults: its title, no Interpolation control,
/// no Preview and no Alt+Reset.
pub(crate) fn open_fields(f: &mut Map<String, Value>) {
    f.insert("__label".into(), json!(TITLE));
    f.remove("interpolation");
    f.remove("__preview");
    f.insert("__altReset".into(), json!(false));
}

/// The Angle field's range, from the command's schema.
pub fn angle_range() -> (f32, f32) {
    photocraft_engine::commands::find(COMMAND)
        .into_iter()
        .flat_map(|c| super::filter_dialog::parse_spec(c.params))
        .find_map(|p| match p.kind {
            super::filter_dialog::Kind::Range { min, max, .. } if p.key == "angle" => Some((min, max)),
            _ => None,
        })
        .unwrap_or(RANGE)
}

pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let (min, max) = angle_range();
    let scale = 10f64.powi(DECIMALS as i32);
    let round64 = |x: f64| (x * scale).round() / scale;
    // Rounded in f64 before the field narrows it to f32 (#13067): a 43x27 ruler's 32.1249984°
    // would otherwise land on 32.125 and draw 32.13.
    let src = f.get("angle").and_then(Value::as_f64).filter(|x| x.is_finite()).map(|x| round64(x.clamp(f64::from(min), f64::from(max))));
    let mut angle = src.unwrap_or(0.0) as f32;
    let mut cw = f.get("direction").and_then(Value::as_str) != Some("ccw");
    let mut edited = false;
    ui.horizontal(|ui| {
        let l = ui.label(egui::RichText::new(tl!("Angle")).color(t.text_dim));
        crate::widgets::scrub_label(ui, &l);
        edited = crate::widgets::value_field_scrub_step(ui, &mut angle, min..=max, "°", 74.0, DECIMALS, SCRUB_STEP).changed();
        ui.add_space(8.0);
        ui.radio_value(&mut cw, true, tl!("Clockwise"));
        ui.radio_value(&mut cw, false, tl!("Counter clockwise"));
    });
    // An edit this frame makes the field's f32 value authoritative; otherwise the exact f64 stays,
    // so OK sends 32.12 and not the 32.1199989 an f32 widens to (#13067).
    let stored = match src {
        Some(rounded) if !edited => rounded,
        _ => round64(f64::from(angle.clamp(min, max))),
    };
    f.insert("angle".into(), json!(stored));
    f.insert("direction".into(), json!(if cw { "cw" } else { "ccw" }));
}
