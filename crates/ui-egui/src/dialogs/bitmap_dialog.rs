//! Image › Mode › Bitmap… (#5559), laid out like Photoshop's dialog: Resolution (Input, read-only,
//! and Output, in pixels/inch or pixels/centimeter), Method (Use: 50% Threshold, Pattern Dither,
//! Diffusion Dither, Halftone Screen, Custom Pattern) with the Custom Pattern picker, enabled only
//! for Custom Pattern. The Halftone Screen settings (Frequency in lines/inch or lines/centimeter,
//! Angle, Shape), which Photoshop asks for in a second dialog after OK, show here only while
//! Halftone Screen is the method.
//!
//! It is the schema dialog of `image.mode.bitmap` with its own body: the fields are the command's
//! params (`resolution` in pixels/inch, `frequency` in lines/inch), so OK, the live preview,
//! `ui.dialog.set` and `ui.dialog.confirm` work as for every schema dialog. The units are view
//! state (`__resUnit`, `__freqUnit`).

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

pub(crate) const COMMAND: &str = "image.mode.bitmap";

/// Centimetres per inch.
const CM: f32 = 2.54;
/// The command's Output resolution range (pixels/inch).
const RESOLUTION: (f32, f32) = (1.0, 30_000.0);
/// The command's Halftone Screen frequency range (lines/inch).
const FREQUENCY: (f32, f32) = (1.0, 999.0);

const METHODS: [(&str, &str); 5] = [
    ("threshold", "50% Threshold"),
    ("pattern", "Pattern Dither"),
    ("diffusion", "Diffusion Dither"),
    ("halftone", "Halftone Screen"),
    ("customPattern", "Custom Pattern"),
];

/// Photoshop's order of the Halftone Screen shapes.
const SHAPES: [(&str, &str); 6] =
    [("round", "Round"), ("diamond", "Diamond"), ("ellipse", "Ellipse"), ("line", "Line"), ("square", "Square"), ("cross", "Cross")];

/// Is this dialog the Bitmap dialog? (A plug-in's own `__spec` dialog never is.)
pub(crate) fn owns(f: &Map<String, Value>) -> bool {
    f.get("__filter").and_then(Value::as_bool) == Some(true) && f.get("__command").and_then(Value::as_str) == Some(COMMAND) && !f.contains_key("__spec")
}

/// Open defaults: Output starts at the document's resolution (Input, as in Photoshop), Custom
/// Pattern at the Patterns panel's pattern.
pub(crate) fn defaults(app: &PhotocraftApp, f: &mut Map<String, Value>) {
    let input = app.session.active().map_or(72.0, |d| d.doc.resolution_dpi);
    f.insert("__inputResolution".into(), json!(input));
    f.insert("resolution".into(), json!(input));
    match photocraft_engine::pattern_cmds::resolve(&app.session, "") {
        Some(p) => f.insert("pattern".into(), json!(p.id)),
        None => f.remove("pattern"),
    };
    f.entry("__resUnit").or_insert_with(|| json!("ppi"));
    f.entry("__freqUnit").or_insert_with(|| json!("lpi"));
}

fn num(f: &Map<String, Value>, key: &str, default: f32) -> f32 {
    f.get(key).and_then(Value::as_f64).map_or(default, |v| v as f32)
}

fn text(f: &Map<String, Value>, key: &str, default: &str) -> String {
    f.get(key).and_then(Value::as_str).unwrap_or(default).to_string()
}

fn label(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_sized([96.0, 20.0], egui::Label::new(egui::RichText::new(text).color(t.text_dim)).truncate());
}

/// A value stored in per-inch units, edited in per-inch or per-centimeter ones: `field` holds the
/// per-inch value, `unit` the view's unit (`per_cm` when its value is per centimeter).
// The explicit accessible name accompanies the existing numeric-field parameters.
#[expect(clippy::too_many_arguments)]
fn per_unit_row(ui: &mut egui::Ui, f: &mut Map<String, Value>, field: &str, name: &str, range: (f32, f32), default: f32, unit: &str, units: [(&str, &str); 2]) {
    let per_inch = num(f, field, default);
    let mut cur = text(f, unit, units[0].0);
    let per_cm = cur == units[1].0;
    let k = if per_cm { CM } else { 1.0 };
    let mut shown = per_inch / k;
    if crate::widgets::value_field(ui, &mut shown, range.0 / k..=range.1 / k, "", 70.0).accessible_name(name).changed() {
        f.insert(field.into(), json!(((shown * k).clamp(range.0, range.1) * 1000.0).round() / 1000.0));
    }
    let opts: Vec<(String, &str)> = units.iter().map(|(id, l)| (id.to_string(), *l)).collect();
    if crate::widgets::dropdown(ui, &format!("bitmap-{unit}"), tl!("Units"), &mut cur, &opts, 150.0) {
        f.insert(unit.into(), json!(cur));
    }
}

/// The dialog body.
pub(crate) fn body(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    // One direction-aware body: in a left-to-right UI the layout is egui's default, so nothing
    // moves; in a right-to-left UI the blocks hug the reading start and every row mirrors.
    ui.with_layout(crate::direction::body_layout(), |ui| body_fields(app, ui, f));
}

/// The Bitmap dialog's fields.
fn body_fields(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    ui.spacing_mut().item_spacing.y = 6.0;
    let res_units = [("ppi", tl!("Pixels/Inch")), ("ppcm", tl!("Pixels/Centimeter"))];

    crate::widgets::section_label(ui, tl!("Resolution"));
    crate::widgets::hairline(ui);
    ui.horizontal(|ui| {
        label(ui, tl!("Input:"));
        let input = num(f, "__inputResolution", 72.0);
        let (v, unit) = if text(f, "__resUnit", "ppi") == "ppcm" { (input / CM, res_units[1].1) } else { (input, res_units[0].1) };
        ui.label(egui::RichText::new(format!("{} {unit}", crate::widgets::fmt_num_prec(f64::from(v), 3))).color(t.text));
    });
    ui.horizontal(|ui| {
        label(ui, tl!("Output:"));
        let input = num(f, "__inputResolution", 72.0);
        per_unit_row(ui, f, "resolution", tl!("Resolution"), RESOLUTION, input, "__resUnit", res_units);
    });

    ui.add_space(4.0);
    crate::widgets::section_label(ui, tl!("Method"));
    crate::widgets::hairline(ui);
    let mut method = text(f, "method", "diffusion");
    ui.horizontal(|ui| {
        label(ui, tl!("Use:"));
        let opts: Vec<(String, &str)> = METHODS.iter().map(|(id, l)| (id.to_string(), tl!(l))).collect();
        if crate::widgets::dropdown(ui, "bitmap-method", tl!("Method"), &mut method, &opts, 190.0) {
            f.insert("method".into(), json!(method));
        }
    });
    // Photoshop greys the pattern out unless the method is Custom Pattern.
    ui.horizontal(|ui| {
        ui.add_enabled_ui(method == "customPattern", |ui| {
            label(ui, tl!("Custom Pattern:"));
            let pat = text(f, "pattern", "");
            if app.session.patterns.items.is_empty() {
                ui.label(egui::RichText::new(tl!("No patterns")).color(t.text_faint));
            } else if let Some(id) = crate::preset_panels::pattern_picker(app, ui, &pat) {
                f.insert("pattern".into(), json!(id));
            }
        });
    });

    if method == "halftone" {
        ui.add_space(4.0);
        crate::widgets::section_label(ui, tl!("Halftone Screen"));
        crate::widgets::hairline(ui);
        ui.horizontal(|ui| {
            label(ui, tl!("Frequency:"));
            per_unit_row(ui, f, "frequency", tl!("Frequency"), FREQUENCY, 53.0, "__freqUnit", [("lpi", tl!("Lines/Inch")), ("lpcm", tl!("Lines/Centimeter"))]);
        });
        ui.horizontal(|ui| {
            label(ui, tl!("Angle:"));
            let mut angle = num(f, "angle", 45.0);
            if crate::widgets::value_field(ui, &mut angle, -180.0..=180.0, "°", 70.0).accessible_name(tl!("Angle:")).changed() {
                f.insert("angle".into(), json!((angle.clamp(-180.0, 180.0) * 10.0).round() / 10.0));
            }
        });
        ui.horizontal(|ui| {
            label(ui, tl!("Shape:"));
            let mut shape = text(f, "shape", "round");
            let opts: Vec<(String, &str)> = SHAPES.iter().map(|(id, l)| (id.to_string(), tl!(l))).collect();
            if crate::widgets::dropdown(ui, "bitmap-shape", tl!("Shape"), &mut shape, &opts, 150.0) {
                f.insert("shape".into(), json!(shape));
            }
        });
    }

    if let Some(mut preview) = f.get("__preview").and_then(Value::as_bool) {
        ui.add_space(4.0);
        crate::widgets::checkbox(ui, &mut preview, tl!("Preview"));
        f.insert("__preview".into(), json!(preview));
    }
}

#[cfg(test)]
mod tests;

/// The Bitmap body mirrors in a right-to-left UI: the rows read from the right and the units
/// dropdown ends the row at the reading end.
#[cfg(test)]
mod rtl_tests {
    use super::*;
    use crate::direction::Dir;
    use egui_kittest::kittest::Queryable;

    /// The Bitmap body drawn in `dir`.
    fn harness(dir: Dir) -> egui_kittest::Harness<'static, (PhotocraftApp, Map<String, Value>)> {
        let _dir = crate::direction::direction_scope(dir);
        let mut fields = Map::new();
        fields.insert("__inputResolution".into(), json!(72.0));
        fields.insert("__resUnit".into(), json!("ppi"));
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(440.0, 360.0))
            .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ALL[0]))
            .build_ui_state(|ui, s: &mut (PhotocraftApp, Map<String, Value>)| body(&s.0, ui, &mut s.1), (app, fields));
        h.run_steps(3);
        h
    }

    /// The rows read from the right in a right-to-left UI: the caption sits at the reading start
    /// and the units dropdown ends the row.
    #[test]
    fn the_rows_mirror_in_a_right_to_left_ui() {
        let ltr = harness(Dir::Ltr);
        let rtl = harness(Dir::Rtl);

        let ltr_output = ltr.get_by_label("Output:").rect();
        let ltr_units = ltr.get_by_label("Units").rect();
        assert!(ltr_units.left() > ltr_output.right(), "LTR: the dropdown ends the row: {ltr_output:?} {ltr_units:?}");

        let rtl_output = rtl.get_by_label("Output:").rect();
        let rtl_units = rtl.get_by_label("Units").rect();
        assert!(rtl_units.right() < rtl_output.left(), "RTL: the row mirrors: {rtl_output:?} {rtl_units:?}");
    }

    /// The rows hug the reading start of the column: the left in LTR, the right in RTL.
    #[test]
    fn the_rows_hug_the_reading_start() {
        let ltr = harness(Dir::Ltr);
        let rtl = harness(Dir::Rtl);
        let ltr_input = ltr.get_by_label("Input:").rect();
        let ltr_output = ltr.get_by_label("Output:").rect();
        assert!((ltr_input.left() - ltr_output.left()).abs() < 1.0, "LTR: the rows start at the left: {ltr_input:?} {ltr_output:?}");
        let rtl_input = rtl.get_by_label("Input:").rect();
        let rtl_output = rtl.get_by_label("Output:").rect();
        assert!((rtl_input.right() - rtl_output.right()).abs() < 1.0, "RTL: the rows end at the right: {rtl_input:?} {rtl_output:?}");
    }
}
