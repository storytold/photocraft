//! Filter › Blur Gallery: Field Blur, Iris Blur, Spin Blur and Tilt-Shift are edited on the image.
//!
//! The pins are the framework's `BlurPins`: the dialog row shows `BlurPinsPanel` and the canvas body shows
//! `BlurPinsOverlay`. This module is the glue to the app: the filter's bounds, the dialog's fields in the
//! engine's command shape, and the row and the overlay that read and write them.
//!
//! The fields hold what the command takes: `pins` (a JSON array, `FieldValue::Json`) with x and y as fractions of the
//! filter's bounds and radii as fractions of their shorter side; Tilt-Shift keeps the command's own `centerX`,
//! `centerY`, `blur`, `angle`, `focus` and `transition`. `selected` is the selected pin's index (-1 for none), shared
//! by the panel and the overlay, which run in different passes. The bounds are the engine's: the selection's when
//! there is one, else the canvas.

use craft::imaging::ui::blur_pins::{DEFAULT_AMOUNT, MAX_PINS};
use craft::imaging::ui::{BlurKind, BlurPin, BlurPins, BlurPinsOverlay, BlurPinsPanel, IrisShape, TiltBand};
use craft::ui::contract::{Bind, Show as _};
use craft::ui::dialog::StackDialog as _;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param};
use serde_json::{Value, json};

use crate::PhotocraftApp;

pub const COMMAND: &str = "filter.blurGallery.fieldBlur";
pub const IRIS: &str = "filter.blurGallery.irisBlur";
pub const SPIN: &str = "filter.blurGallery.spinBlur";
pub const TILT: &str = "filter.blurGallery.tiltShift";

/// The Blur Gallery filters that open as this workspace.
pub fn handles(command: &str) -> bool {
    kind_of(command).is_some()
}

/// The Blur Gallery kind of `command`; `None` for any other command.
pub(crate) fn kind_of(command: &str) -> Option<BlurKind> {
    match command {
        COMMAND => Some(BlurKind::Field),
        IRIS => Some(BlurKind::Iris),
        SPIN => Some(BlurKind::Spin),
        TILT => Some(BlurKind::TiltShift),
        _ => None,
    }
}

/// The dialog field the selected pin's amount is mirrored to (remembered for the next opening).
fn amount_key(kind: BlurKind) -> &'static str {
    if kind == BlurKind::Spin { "blurAngle" } else { "blur" }
}

/// The filter's bounds in document pixels: the selection's, else the canvas.
fn document_bounds(app: &mut PhotocraftApp) -> Option<[f64; 4]> {
    let (doc, canvas, selection) = app.session.active().map(|st| (st.doc.id, st.doc.bounds(), st.doc.selection.clone()))?;
    let b = selection.map(|s| app.cached_bounds(u64::MAX - doc.0, &s)).filter(|b| !b.is_empty()).unwrap_or(canvas);
    Some([f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)])
}

/// The kind, bounds and start pin a Blur Gallery `command` opens with; `None` for another command or without a
/// document. Takes `&mut` because the selection's bounds are cached in the app.
pub(crate) fn initial(app: &mut PhotocraftApp, command: &str) -> Option<(BlurKind, [f64; 4], BlurPin)> {
    let kind = kind_of(command)?;
    let bounds = document_bounds(app)?;
    Some((kind, bounds, start_of(&DialogFields::new(), kind)))
}

/// The start pin from the remembered values in `fields` (`blur`/`blurAngle`, the shape keys, Tilt-Shift's centre and
/// band), defaults for what is missing. `BlurPins::new` places it. `initial` starts from the defaults (an empty
/// field set); a caller that holds the restored fields passes them here.
pub(crate) fn start_of(fields: &DialogFields, kind: BlurKind) -> BlurPin {
    let num = |key: &str, d: f64| fields.number(key).filter(|v| v.is_finite()).unwrap_or(d);
    let mut pin = BlurPin::new(0.5, 0.5, num(amount_key(kind), DEFAULT_AMOUNT).clamp(0.0, kind.max()));
    match kind {
        BlurKind::Iris | BlurKind::Spin => pin.iris = Some(shape_from(kind, &num)),
        BlurKind::TiltShift => {
            pin.x = num("centerX", 0.5);
            pin.y = num("centerY", 0.5);
            pin.tilt = Some(TiltBand::new(num("angle", 0.0), num("focus", 0.1), num("transition", 0.15)));
        }
        // Field Blur has no shape; the enum is non-exhaustive.
        _ => {}
    }
    pin
}

/// An Iris or Spin shape from the numbers `num` reads under the command's keys.
fn shape_from(kind: BlurKind, num: &dyn Fn(&str, f64) -> f64) -> IrisShape {
    let d = if kind == BlurKind::Spin { IrisShape::spin() } else { IrisShape::iris() };
    // Roundness and feather are Iris only.
    let (roundness, feather) = if kind == BlurKind::Iris { (num("roundness", d.roundness), num("feather", d.feather)) } else { (d.roundness, d.feather) };
    IrisShape::new(num("radiusX", d.radius_x), num("radiusY", d.radius_y), num("angle", d.angle), roundness, feather)
}

/// Whether `fields` hold the pins (or Tilt-Shift's band) yet.
fn seeded(fields: &DialogFields, kind: BlurKind) -> bool {
    fields.contains(if kind == BlurKind::TiltShift { "centerX" } else { "pins" })
}

/// The pin objects under `pins`, in either form an untagged read gives: `Json(Array)` or `List([Json, ..])`.
fn json_pins(fields: &DialogFields) -> Vec<Value> {
    match fields.get("pins") {
        Some(FieldValue::Json(Value::Array(list))) => list.clone(),
        Some(FieldValue::List(items)) => items
            .iter()
            .filter_map(|item| match item {
                FieldValue::Json(v) => Some(v.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// One pin from its command object.
fn pin_of(kind: BlurKind, v: &Value) -> BlurPin {
    let num = |key: &str, d: f64| v.get(key).and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(d);
    let mut pin = BlurPin::new(num("x", 0.5), num("y", 0.5), num(amount_key(kind), DEFAULT_AMOUNT));
    if matches!(kind, BlurKind::Iris | BlurKind::Spin) {
        pin.iris = Some(shape_from(kind, &num));
    }
    pin
}

/// One pin as the command takes it.
fn pin_json(kind: BlurKind, p: &BlurPin) -> Value {
    match kind {
        BlurKind::Iris => {
            let s = p.iris.unwrap_or_else(IrisShape::iris);
            json!({"x": p.x, "y": p.y, "blur": p.amount, "radiusX": s.radius_x, "radiusY": s.radius_y, "angle": s.angle, "roundness": s.roundness, "feather": s.feather})
        }
        BlurKind::Spin => {
            let s = p.iris.unwrap_or_else(IrisShape::spin);
            json!({"x": p.x, "y": p.y, "blurAngle": p.amount, "radiusX": s.radius_x, "radiusY": s.radius_y, "angle": s.angle})
        }
        _ => json!({"x": p.x, "y": p.y, "blur": p.amount}),
    }
}

/// The pins in `fields`, cleaned, over `bounds`. A missing or hostile value takes its default.
pub(crate) fn pins_of(fields: &DialogFields, kind: BlurKind, bounds: [f64; 4]) -> BlurPins {
    let mut pins = BlurPins::new(kind, bounds, BlurPin::new(0.5, 0.5, DEFAULT_AMOUNT));
    pins.pins =
        if kind == BlurKind::TiltShift { vec![start_of(fields, kind)] } else { json_pins(fields).iter().take(MAX_PINS).map(|v| pin_of(kind, v)).collect() };
    pins.selected = fields.int("selected").and_then(|i| usize::try_from(i).ok());
    pins.clean();
    pins
}

/// Gives `set` each field the pins map to: the command's keys, the selected pin's amount (remembered for the next
/// opening) and `selected`. The row writes with `DialogFields::set`, the overlay with `HostedParamDialog::set_field`.
pub(crate) fn write_pins(pins: &BlurPins, mut set: impl FnMut(&'static str, FieldValue)) {
    if pins.kind == BlurKind::TiltShift {
        if let Some(p) = pins.pins.first() {
            let t = p.tilt.unwrap_or_else(TiltBand::default_band);
            for (key, v) in [("centerX", p.x), ("centerY", p.y), ("blur", p.amount), ("angle", t.angle), ("focus", t.focus), ("transition", t.transition)] {
                set(key, FieldValue::Number(v));
            }
        }
    } else {
        set("pins", FieldValue::Json(Value::Array(pins.pins.iter().map(|p| pin_json(pins.kind, p)).collect())));
        if let Some(p) = pins.selected.and_then(|i| pins.pins.get(i)) {
            set(amount_key(pins.kind), FieldValue::Number(p.amount));
        }
    }
    set("selected", FieldValue::Int(pins.selected.map_or(-1, |i| i as i64)));
}

/// The dialog row of the pins: the selected pin's amount and the hints, with the canvas's presses passed through.
/// A dialog that opens without the pins gets `start` as its one pin.
pub(crate) fn blur_row(kind: BlurKind, bounds: [f64; 4], start: BlurPin) -> Param {
    Param::custom("pins", tl!("Blur Pins"), move |ui, fields, cx| {
        if !seeded(fields, kind) {
            write_pins(&BlurPins::new(kind, bounds, start), |k, v| fields.set(k, v));
        }
        let mut pins = pins_of(fields, kind, bounds);
        let before = pins.clone();
        let mut panel = BlurPinsPanel::new(&mut pins);
        if let Some(canvas) = cx.canvas() {
            panel = panel.canvas(canvas);
        }
        let response = panel.show(ui).response;
        if pins != before {
            write_pins(&pins, |k, v| fields.set(k, v));
        }
        response
    })
}

/// The pins over the canvas while a Blur Gallery dialog is the top dialog. Called among the canvas body's controls
/// with the canvas's rect. Each pass reads the pins from the dialog's fields and writes back only its own edit, so a
/// second pass in one frame applies only the input it has.
pub(crate) fn overlay(app: &mut PhotocraftApp, ui: &mut egui::Ui, at: egui::Rect) {
    let top = app.framework.dialogs.stack().top();
    let kind = app.framework.dialogs.get::<HostedParamDialog>().filter(|d| Some(d.id()) == top).and_then(|d| d.key()).and_then(kind_of);
    let Some(kind) = kind else { return };
    let Some(bounds) = document_bounds(app) else { return };
    let Some(dialog) = app.framework.dialogs.get_mut::<HostedParamDialog>() else { return };
    let mut pins = pins_of(dialog.fields(), kind, bounds);
    let before = pins.clone();
    ui.scope_builder(egui::UiBuilder::new().max_rect(at), |ui| {
        BlurPinsOverlay::new(tl!("Blur Pins"), Bind::bound(&mut pins)).show(ui);
    });
    if pins != before {
        write_pins(&pins, |k, v| dialog.set_field(k, v));
    }
}

/// Preview proxies run at 1/`k` of the document: the pins' pixel blur scales with them, as the
/// top-level pixel params (Tilt-Shift's `blur`) already do. Spin's blur angle doesn't.
pub fn scale_for_proxy(params: &mut Value, k: u32) {
    if k <= 1 {
        return;
    }
    let Some(list) = params.get_mut("pins").and_then(Value::as_array_mut) else { return };
    for pin in list {
        if let Some(b) = pin.get("blur").and_then(Value::as_f64) {
            pin["blur"] = json!(b / f64::from(k));
        }
    }
}
