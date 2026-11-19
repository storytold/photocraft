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
use crate::adjust_editors::is_gray;
use crate::state::Tool;
use craft::imaging::ui::adjust::TonePicker;
use photocraft_doc::{Adjustment, ColorMode, LayerContent, LayerId};

/// An armed Properties eyedropper: on `layer`, for `point` ("black", "gray" or "white"), armed
/// while `tool` was the current tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerPicker {
    pub layer: u64,
    pub point: String,
    pub tool: Tool,
}

/// The point name of a picker, as `layer.setAdjustment`'s `eyedropper` param and [`LayerPicker`] have it.
fn point_name(picker: TonePicker) -> &'static str {
    match picker {
        TonePicker::Black => "black",
        TonePicker::Gray => "gray",
        _ => "white",
    }
}

/// The framework picker a [`LayerPicker`] names; `None` for a point name that is none of the three.
pub fn tone_picker(picker: &LayerPicker) -> Option<TonePicker> {
    TonePicker::ALL.into_iter().find(|p| point_name(*p) == picker.point)
}

/// Arms `picker` on `layer` for the current tool, or disarms the picker when it is `layer`'s.
pub fn arm(app: &mut PhotocraftApp, layer: LayerId, picker: Option<TonePicker>) {
    match picker {
        Some(p) => app.ui.layer_picker = Some(LayerPicker { layer: layer.0, point: point_name(p).into(), tool: app.ui.tool }),
        None if app.ui.layer_picker.as_ref().is_some_and(|armed| armed.layer == layer.0) => app.ui.layer_picker = None,
        None => {}
    }
}

/// The armed picker if it still applies: same tool, the layer still the active Levels or Curves
/// layer, and the point supported in the document's colour mode.
pub fn armed(app: &PhotocraftApp) -> Option<(LayerId, TonePicker)> {
    let picker = app.ui.layer_picker.as_ref()?;
    let st = app.session.active()?;
    let layer = LayerId(picker.layer);
    let tonal = st.doc.layer(layer).is_some_and(|l| matches!(l.content, LayerContent::Adjustment(Adjustment::Levels { .. } | Adjustment::Curves { .. })));
    let point = tone_picker(picker).filter(|p| p.supported(st.doc.mode == ColorMode::Rgb, is_gray(st.doc.mode)))?;
    (picker.tool == app.ui.tool && st.active_layer == Some(layer) && tonal).then_some((layer, point))
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
    let Some((layer, point)) = armed(app) else { return false };
    let size = app.ui.tool_options.eyedropper.size.pixels();
    let p = json!({"layer": layer.0, "eyedropper": {"point": point_name(point), "at": [x, y], "size": size}});
    if let Err(err) = app.run("layer.setAdjustment", p) {
        app.ui.status = err;
        app.ui.status_error = true;
    }
    true
}
