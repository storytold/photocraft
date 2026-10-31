//! The Set Black / Neutral Gray / White Point eyedroppers of a Levels or Curves adjustment
//! layer's Properties panel (UI-217-15, #3116), as Photoshop has them next to the histogram or
//! curve.
//!
//! Arming one turns a click on the image into one `layer.setAdjustment` with the `eyedropper`
//! param (one undoable, recordable step, the same thing an agent sends), which samples the image
//! the layer adjusts. Unlike the modal Image › Adjustments › Levels and Curves dialogs (`adjust_dialog`), the
//! panel is not modal, so the picker stays armed until Esc, another tool, another layer, or a
//! second click on its button.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;
use photocraft_doc::{Adjustment, LayerContent, LayerId};
use photocraft_engine::adjust_params::CurvesEyedropper;

/// An armed Properties eyedropper: on `layer`, for `point` ("black", "gray" or "white"), armed
/// while `tool` was the current tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerPicker {
    pub layer: u64,
    pub point: String,
    pub tool: Tool,
}

const POINTS: [(&str, CurvesEyedropper); 3] = [("black", CurvesEyedropper::Black), ("gray", CurvesEyedropper::NeutralGray), ("white", CurvesEyedropper::White)];

fn eyedropper(point: &str) -> Option<CurvesEyedropper> {
    POINTS.iter().find(|(id, _)| *id == point).map(|(_, e)| *e)
}

/// The armed picker if it still applies: same tool, the layer still the active Levels or Curves
/// layer, and the point supported in the document's colour mode.
pub fn armed(app: &PhotocraftApp) -> Option<(LayerId, &str)> {
    let picker = app.ui.layer_picker.as_ref()?;
    let st = app.session.active()?;
    let layer = LayerId(picker.layer);
    let tonal = st.doc.layer(layer).is_some_and(|l| matches!(l.content, LayerContent::Adjustment(Adjustment::Levels { .. } | Adjustment::Curves { .. })));
    let supported = eyedropper(&picker.point).is_some_and(|e| crate::adjust_dialog::picker_supported(st.doc.mode, e));
    (picker.tool == app.ui.tool && st.active_layer == Some(layer) && tonal && supported).then_some((layer, picker.point.as_str()))
}

/// Drops a picker that no longer applies, so returning to its tool or layer doesn't re-arm it.
pub fn prune(app: &mut PhotocraftApp) {
    if app.ui.layer_picker.is_some() && armed(app).is_none() {
        app.ui.layer_picker = None;
    }
}

/// Esc: disarms the picker. Returns whether there was one.
pub fn cancel(app: &mut PhotocraftApp) -> bool {
    app.ui.layer_picker.take().is_some()
}

/// Sets the armed point from the image at document point (x, y): one `layer.setAdjustment`.
/// Returns whether a picker was armed (and so owns the click), even if the sample failed (off
/// the image, transparent): that is reported in the status bar and the picker stays armed.
pub fn sample_at(app: &mut PhotocraftApp, x: f64, y: f64) -> bool {
    let Some((layer, point)) = armed(app).map(|(l, p)| (l, p.to_string())) else { return false };
    let size = app.ui.tool_options.eyedropper_size;
    let p = json!({"layer": layer.0, "eyedropper": {"point": point, "at": [x, y], "size": size}});
    if let Err(err) = app.run("layer.setAdjustment", p) {
        app.ui.status = err;
        app.ui.status_error = true;
    }
    true
}

/// The three eyedropper buttons above the Levels or Curves editor of adjustment layer `layer`.
/// A double-click on one also opens the Color Picker on its target colour ([`open_target_picker`]),
/// leaving it armed.
pub fn controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    let Some(mode) = app.session.active().map(|s| s.doc.mode) else { return };
    let current = armed(app).filter(|(l, _)| *l == layer).map(|(_, p)| p.to_string());
    let swatches = target_swatches(app);
    let mut clicked = None;
    let mut double = None;
    ui.horizontal(|ui| {
        // The swatch is the colour the picker maps the sample to, as in the dialog.
        for (((id, e), label), swatch) in POINTS.iter().zip(labels()).zip(swatches) {
            let enabled = crate::adjust_dialog::picker_supported(mode, *e);
            let selected = current.as_deref() == Some(*id);
            let response = crate::adjust_dialog::picker_button(ui, selected, enabled, label, swatch);
            if response.double_clicked() {
                double = Some(*id);
            } else if response.clicked() {
                clicked = Some((*id, selected));
            }
        }
    });
    match clicked {
        Some((_, true)) => app.ui.layer_picker = None,
        Some((id, false)) => app.ui.layer_picker = Some(LayerPicker { layer: layer.0, point: id.into(), tool: app.ui.tool }),
        None => {}
    }
    if let Some(id) = double {
        // The double-click's first click armed it and its second would disarm it: keep it armed.
        app.ui.layer_picker = Some(LayerPicker { layer: layer.0, point: id.into(), tool: app.ui.tool });
        open_target_picker(app, id);
    }
}

/// The eyedroppers' tooltips, in [`POINTS`] order.
pub(crate) fn labels() -> [&'static str; 3] {
    [tl!("Set Black Point"), tl!("Set Neutral Gray Point"), tl!("Set White Point")]
}

/// The target colour (0..1 RGB levels) that picker `point` ("black", "gray" or "white") maps a
/// sample to: the `targetColors` preference.
pub fn target_rgb(app: &PhotocraftApp, point: &str) -> Option<[f32; 3]> {
    eyedropper(point).map(|e| e.target_in(&app.session))
}

/// The three targets as the eyedropper buttons' swatches, in [`POINTS`] order.
pub(crate) fn target_swatches(app: &PhotocraftApp) -> [egui::Color32; 3] {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    POINTS.map(|(_, e)| {
        let [r, g, b] = e.target_in(&app.session);
        egui::Color32::from_rgb(q(r), q(g), q(b))
    })
}

/// The Color Picker's `__colorPicker` target prefix for an eyedropper's target colour.
const TARGET_PICKER: &str = "eyedropperTarget.";

/// Photoshop's Color Picker title for picker `point`'s target.
fn target_title(point: &str) -> Option<&'static str> {
    match point {
        "black" => Some(tl!("Select target shadow color:")),
        "gray" => Some(tl!("Select target midtone color:")),
        "white" => Some(tl!("Select target highlight color:")),
        _ => None,
    }
}

/// Double-click on a Set Black / Gray / White Point eyedropper (Levels, Curves; dialog or
/// Properties): opens the Color Picker on that point's target colour. OK stores it in the
/// `targetColors` preference ([`confirm_target`]); Cancel leaves it as it was.
pub fn open_target_picker(app: &mut PhotocraftApp, point: &str) -> Option<u64> {
    let (rgb, title) = (target_rgb(app, point)?, target_title(point)?);
    Some(crate::color_picker_ui::open_for_target(app, &format!("{TARGET_PICKER}{point}"), title, rgb))
}

/// OK in a target colour's Color Picker (`target` is its `__colorPicker`): one `prefs.set` of
/// `targetColors.<key>`, so it is kept across restarts. `None` when `target` isn't one.
pub fn confirm_target(app: &mut PhotocraftApp, target: &str, color: &str) -> Option<Result<serde_json::Value, String>> {
    let point = target.strip_prefix(TARGET_PICKER)?;
    let Some(e) = eyedropper(point) else { return Some(Err(format!("no eyedropper `{point}`"))) };
    Some(app.run("prefs.set", json!({"path": format!("targetColors.{}", e.target_key()), "value": color})))
}

/// Set by a modal dialog's body (to "black", "gray" or "white") to open that eyedropper's target
/// Color Picker over it; the dialog loop opens it ([`open_requested_target`]).
const REQUEST: &str = "__pickTarget";

/// A modal dialog body asks for the target Color Picker of picker `point`.
pub(crate) fn request_target(fields: &mut serde_json::Map<String, serde_json::Value>, point: &str) {
    fields.insert(REQUEST.into(), json!(point));
}

/// Opens the target Color Picker dialog `id`'s body asked for ([`request_target`]), if any.
pub fn open_requested_target(app: &mut PhotocraftApp, id: u64) {
    let Some(d) = app.ui.dialog_mut(id) else { return };
    if let Some(serde_json::Value::String(point)) = d.fields.remove(REQUEST) {
        open_target_picker(app, &point);
    }
}
