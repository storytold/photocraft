//! Image Size and Canvas Size on the dialog host.
//!
//! The framework's forms own the fields, the linked size model and the column conversion. This file
//! builds their values from the document and the preferences, and maps the confirmed result to
//! `image.imageSize` / `image.canvasSize`.

use craft::behavior::{Anchor9, PixelSize};
use craft::ui::contract::Bind;
use craft::ui::dialog::{DialogId, DialogResult, dialog_id};
use craft::ui::patterns::form_dialogs::{
    CanvasExtension, CanvasSize, CanvasSizeResult, CanvasSizeValues, HostedCanvasSize, HostedImageSize, ImageSize, ImageSizeResult, ImageSizeValues, Resample,
};
use craft::ui::runtime::env::{ColumnSize, Label, UnitKind};
use serde_json::{Value, json};

use crate::PhotocraftApp;

const IMAGE_COMMAND: &str = "image.imageSize";
const CANVAS_COMMAND: &str = "image.canvasSize";

/// The dialog unit of the Units & Rulers preference.
pub(crate) fn pref_unit(u: photocraft_engine::prefs::Unit) -> UnitKind {
    use photocraft_engine::prefs::Unit;
    match u {
        Unit::Pixels => UnitKind::Pixels,
        Unit::Percent => UnitKind::Percent,
        Unit::Inches => UnitKind::Inches,
        Unit::Centimeters => UnitKind::Centimetres,
        Unit::Millimeters => UnitKind::Millimetres,
        Unit::Points => UnitKind::Points,
        Unit::Picas => UnitKind::Picas,
    }
}

/// Image Size resampling for Preferences › General › Image Interpolation.
pub(crate) fn pref_resample(i: photocraft_engine::prefs::Interpolation) -> &'static str {
    use photocraft_engine::prefs::Interpolation;
    match i {
        Interpolation::BicubicAutomatic => "automatic",
        Interpolation::Nearest => "nearest",
        Interpolation::Bilinear => "bilinear",
        Interpolation::Bicubic => "bicubic",
        Interpolation::BicubicSmoother => "bicubicSmoother",
        Interpolation::BicubicSharper => "bicubicSharper",
        Interpolation::PreserveDetails => "preserveDetails",
    }
}

/// The name `image.imageSize` reads a method by.
fn resample_key(method: Resample) -> &'static str {
    match method {
        Resample::Automatic => "automatic",
        Resample::PreserveDetails => "preserveDetails",
        Resample::BicubicSmoother => "bicubicSmoother",
        Resample::BicubicSharper => "bicubicSharper",
        Resample::Bicubic => "bicubic",
        Resample::NearestNeighbor => "nearest",
        Resample::Bilinear => "bilinear",
        Resample::Lanczos => "lanczos",
        // The enum is non-exhaustive: a method added later falls back to the preference default.
        _ => "automatic",
    }
}

/// The method an engine name stands for, through the one name table.
fn resample_from_key(key: &str) -> Option<Resample> {
    Resample::ALL.into_iter().find(|method| resample_key(*method) == key)
}

/// The method the Image Interpolation preference opens the dialog with.
fn resample_of(i: photocraft_engine::prefs::Interpolation) -> Resample {
    resample_from_key(pref_resample(i)).unwrap_or(Resample::Automatic)
}

/// The anchor name `image.canvasSize` reads.
fn anchor_key(anchor: Anchor9) -> &'static str {
    match anchor {
        Anchor9::TopLeft => "topLeft",
        Anchor9::Top => "top",
        Anchor9::TopRight => "topRight",
        Anchor9::Left => "left",
        Anchor9::Centre => "center",
        Anchor9::Right => "right",
        Anchor9::BottomLeft => "bottomLeft",
        Anchor9::Bottom => "bottom",
        Anchor9::BottomRight => "bottomRight",
        _ => "center",
    }
}

/// The `extensionColor` parameter of a canvas extension.
fn extension_param(extension: &CanvasExtension) -> Value {
    let hex = |rgb: [f32; 3]| {
        let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02x}{:02x}{:02x}", byte(rgb[0]), byte(rgb[1]), byte(rgb[2]))
    };
    match extension {
        CanvasExtension::Foreground => json!("foreground"),
        CanvasExtension::Background => json!("background"),
        CanvasExtension::White => json!("white"),
        CanvasExtension::Black => json!("black"),
        CanvasExtension::Gray => json!(hex([0.5; 3])),
        CanvasExtension::Other(rgb) => json!(hex(*rgb)),
        CanvasExtension::Transparent => json!("transparent"),
        _ => json!("background"),
    }
}

/// Whether `command` is Image Size or Canvas Size.
pub fn is_sizing(command: &str) -> bool {
    matches!(command, IMAGE_COMMAND | CANVAS_COMMAND)
}

/// A byte count as the dialogs show it (`138.1M`).
pub fn human_bytes(b: f64) -> String {
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2}G", b / (1024.0 * 1024.0 * 1024.0))
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.1}M", b / (1024.0 * 1024.0))
    } else {
        format!("{:.0}K", (b / 1024.0).max(1.0))
    }
}

/// The bytes a pixel of the flat image takes, for Image Size's "Image Size:" readout (#14768): the mode's colour
/// channels at the document's depth, no alpha, whatever the layers store. An Indexed image is one index channel and a
/// Bitmap one 1-bit channel, though both are stored expanded for editing. A Multichannel image counts its inks; with
/// none it counts the mode's channels.
fn flat_bytes_per_pixel(d: &photocraft_doc::Document) -> f64 {
    use photocraft_color::ColorMode;
    match d.mode {
        ColorMode::Bitmap => 1.0 / 8.0,
        ColorMode::Indexed => 1.0,
        ColorMode::Multichannel => {
            let inks = d.channels.iter().filter(|c| c.spot.is_some()).count();
            (if inks == 0 { d.mode.color_channels() } else { inks } * d.depth.bytes()) as f64
        }
        mode => (mode.color_channels() * d.depth.bytes()) as f64,
    }
}

/// Opens Image Size or Canvas Size for the active document; `None` without one.
pub(crate) fn open(app: &mut PhotocraftApp, command: &str) -> Option<DialogId> {
    open_with(app, command, &Value::Null)
}

/// [`open`] with the `width`, `height`, `resolution` and `resample` of a recorded step applied, in pixels.
pub(crate) fn open_with(app: &mut PhotocraftApp, command: &str, params: &Value) -> Option<DialogId> {
    let prefs = app.session.prefs();
    let ur = &prefs.units_and_rulers;
    let seeded = params.get("width").is_some() || params.get("height").is_some();
    // A recorded size is in pixels, whatever the ruler unit.
    let unit = if seeded { UnitKind::Pixels } else { pref_unit(ur.rulers) };
    let points = ur.point_size.per_inch();
    let columns = ColumnSize::new(ur.column_width, ur.gutter);
    let interpolation = prefs.general.image_interpolation;
    let d = &app.session.active()?.doc;
    let size = PixelSize::new(d.size.width, d.size.height);
    let dpi = f64::from(d.resolution_dpi);
    let bytes_per_pixel = d.layers.first().and_then(|l| l.surface()).map_or(4, |s| s.format().bytes_per_pixel()) as f64;
    let image_bytes_per_pixel = flat_bytes_per_pixel(d);
    let id = if command == IMAGE_COMMAND {
        // Indexed Color resamples with Nearest Neighbor only, as the command does (#6019).
        let indexed = d.mode == photocraft_color::ColorMode::Indexed;
        let mut method = resample_of(interpolation);
        if indexed && method != Resample::Automatic {
            method = Resample::NearestNeighbor;
        }
        let methods = if indexed { vec![Resample::Automatic, Resample::NearestNeighbor] } else { Vec::new() };
        let values = ImageSizeValues::new(size).pixels_per_inch(dpi).points_per_inch(points).unit(unit).method(method).column_size(columns);
        app.framework.dialogs.open(HostedImageSize::new(values, move |values: Bind<'_, ImageSizeValues>| {
            ImageSize::new(values).methods(methods.clone()).bytes_per_pixel(image_bytes_per_pixel)
        }))
    } else {
        let values =
            CanvasSizeValues::new(size).pixels_per_inch(dpi).points_per_inch(points).unit(unit).has_background(d.background().is_some()).column_size(columns);
        app.framework
            .dialogs
            .open(HostedCanvasSize::new(values, move |values: Bind<'_, CanvasSizeValues>| CanvasSize::new(values).bytes_per_pixel(bytes_per_pixel)))
    };
    seed(app, id, command == IMAGE_COMMAND, params);
    Some(id)
}

/// Sets the recorded fields of `params` on the dialog `id`. A value the dialog refuses leaves the opening one.
fn seed(app: &mut PhotocraftApp, id: DialogId, image: bool, params: &Value) {
    let ctx = app.ctx().clone();
    // Resample first: it decides which fields take input.
    let keys: &[&str] = if image { &["resample", "resolution", "width", "height"] } else { &["width", "height"] };
    for key in keys {
        let Some(value) = params.get(*key) else { continue };
        let value = match (*key, value.as_str()) {
            ("resample", Some("none")) => Value::Null,
            ("resample", Some(name)) => resample_from_key(name).and_then(|m| serde_json::to_value(m).ok()).unwrap_or_else(|| value.clone()),
            _ => value.clone(),
        };
        let _ = app.framework.dialogs.set_json_field(&ctx, id, key, value);
    }
}

/// Judges the held OK of the sizing editor of Record Again or of a paused menu step (the only sizing dialogs that hold
/// their OK): a stale step, a changed document or a denied size refuses it and the editor stays open.
fn answer_held(app: &mut PhotocraftApp) {
    let id = dialog_id(HostedImageSize::KEY);
    if let Some(result) = app.framework.dialogs.get_mut_by_id::<HostedImageSize>(id).and_then(HostedImageSize::take_ok) {
        let answer = crate::actions::check_editor(app, id, IMAGE_COMMAND, &image_params(app, &result));
        if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedImageSize>(id) {
            dialog.answer_ok(answer.map_err(Label::from));
        }
    }
    let id = dialog_id(HostedCanvasSize::KEY);
    if let Some(result) = app.framework.dialogs.get_mut_by_id::<HostedCanvasSize>(id).and_then(HostedCanvasSize::take_ok) {
        let answer = crate::actions::check_editor(app, id, CANVAS_COMMAND, &canvas_params(&result));
        if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedCanvasSize>(id) {
            dialog.answer_ok(answer.map_err(Label::from));
        }
    }
}

/// Runs the answer of Image Size or Canvas Size, when one finished this pass.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    answer_held(app);
    if let Some(result) = app.framework.dialogs.take::<HostedImageSize>() {
        let params = match result {
            DialogResult::Ok(r) => Some(image_params(app, &r)),
            _ => None,
        };
        // One dialog of each type is open at a time, so its key names the dialog a paused playback holds.
        finish(app, dialog_id(HostedImageSize::KEY), IMAGE_COMMAND, params);
    }
    if let Some(result) = app.framework.dialogs.take::<HostedCanvasSize>() {
        let params = match result {
            DialogResult::Ok(r) => Some(canvas_params(&r)),
            _ => None,
        };
        finish(app, dialog_id(HostedCanvasSize::KEY), CANVAS_COMMAND, params);
    }
}

/// The `image.imageSize` parameters of a result. While the resolution is the opening one the
/// document keeps its own, which may lie outside the dialog's range (#9260).
fn image_params(app: &PhotocraftApp, r: &ImageSizeResult) -> Value {
    let own = app.session.active().map(|st| f64::from(st.doc.resolution_dpi));
    let resolution = if r.resolution_changed { r.pixels_per_inch } else { own.unwrap_or(r.pixels_per_inch) };
    json!({
        "width": r.size.width,
        "height": r.size.height,
        "resolution": resolution,
        "resample": r.resample.map_or("none", resample_key),
    })
}

/// The `image.canvasSize` parameters of a result: the size is absolute.
fn canvas_params(r: &CanvasSizeResult) -> Value {
    json!({
        "width": r.size.width,
        "height": r.size.height,
        "relative": false,
        "anchor": anchor_key(r.anchor),
        "extensionColor": extension_param(&r.extension),
    })
}

/// Runs `command` with `params` (`None`: the dialog was cancelled), then continues or abandons the
/// paused playback that opened the dialog.
fn finish(app: &mut PhotocraftApp, id: DialogId, command: &str, params: Option<Value>) {
    // The editor Record Again opened answers the step it was opened for, not a new run of the command.
    if crate::actions::record_again::owns_dialog(app, id) {
        match params {
            Some(params) => {
                if let Err(e) = crate::actions::record_again::confirm(app, id, command, params) {
                    app.ui.status = e;
                    app.ui.status_error = true;
                }
            }
            None => crate::actions::record_again::cancel(app, id),
        }
        return;
    }
    let token = crate::actions::token_of(app, id);
    let abandon = |app: &mut PhotocraftApp| {
        if let Some(token) = token {
            crate::actions::abandoned(app, token);
        }
    };
    let Some(params) = params else {
        abandon(app);
        return;
    };
    match crate::actions::run_step_dialog(app, id, command, params) {
        Ok(_) => {
            if let Some(token) = token
                && let Err(e) = crate::actions::resume(app, token)
            {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            abandon(app);
        }
    }
}

#[cfg(test)]
mod tests;
