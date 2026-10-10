//! Non-destructive layer-mask controls (pixel and vector), shared by docked and floating Properties.

use photocraft_doc::Layer;
use serde_json::json;

use crate::{PhotocraftApp, widgets};

/// The Layers panel targets the active layer's pixel mask (not its vector mask).
pub fn targeted(app: &PhotocraftApp, layer: &Layer) -> bool {
    app.ui.mask_target && !app.ui.vector_mask_target && layer.mask.is_some()
}

/// The Layers panel targets the active layer's vector mask.
pub fn vector_targeted(app: &PhotocraftApp, layer: &Layer) -> bool {
    app.ui.vector_mask_target && layer.vector_mask.is_some()
}

/// The pixel-mask section: Density, Feather and the Select and Mask… button. Pixels stay
/// intact; the two properties ride on the mask and are applied by the compositor.
pub fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    let Some(mask) = &layer.mask else { return };
    let Some(document) = app.session.active().map(|s| s.doc.id.0) else { return };
    ui.push_id((document, layer.id, "pixel-mask-properties"), |ui| {
        if !crate::props_layout::section(ui, "pixel-mask", "Layer Mask") {
            return;
        }
        let (density, feather) = (mask.density, mask.feather);
        sliders(app, ui, layer.id.0, document, "layer.layerMask.edit", "pixel-mask", density, feather);
        ui.add_space(6.0);
        // Keep Select and Mask… one click away.
        if widgets::secondary_button(ui, tl!("Select and Mask…"), ui.available_width()).clicked() {
            let _ = crate::menus::invoke(app, ui.ctx(), "select.selectAndMask", json!({}));
        }
    });
}

/// The vector-mask section: Density and Feather, likewise non-destructive.
pub fn vector_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    let Some(vm) = &layer.vector_mask else { return };
    let Some(document) = app.session.active().map(|s| s.doc.id.0) else { return };
    ui.push_id((document, layer.id, "vector-mask-properties"), |ui| {
        if !crate::props_layout::section(ui, "vector-mask", "Vector Mask") {
            return;
        }
        let (density, feather) = (vm.density, vm.feather);
        sliders(app, ui, layer.id.0, document, "layer.vectorMask.edit", "vector-mask", density, feather);
    });
}

/// Density (%) and Feather (px) sliders writing one undoable edit per drag: the value lives on the
/// mask, not in its pixels. `command` is the mask's edit command and `prefix` keys its coalescing.
#[allow(clippy::too_many_arguments)]
fn sliders(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: u64, document: u64, command: &str, prefix: &str, density: f32, feather: f32) {
    // Like brush settings, track pointer presses even on frames without a value change.
    // slider_row also merges numeric scrubbing, whose drag flags are not in its Response.
    let key = ui.id().with("gesture");
    let pass = ui.ctx().cumulative_pass_nr();
    let pressed = ui.input(|i| i.pointer.any_pressed());
    let gesture = ui.data_mut(|d| {
        let g = d.get_temp_mut_or_default::<(u64, u64)>(key);
        if pressed && g.1 != pass {
            g.0 = g.0.wrapping_add(1);
            g.1 = pass;
        }
        g.0
    });
    for (field, label, mut value, max, unit) in [("density", tl!("Density"), density * 100.0, 100.0, "%"), ("feather", tl!("Feather"), feather, 1000.0, "px")] {
        let response = widgets::slider_row(ui, label, &mut value, 0.0..=max, unit, None);
        response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(value), label));
        if response.changed() {
            let mut params = json!({"document": document, "layer": layer,
                "coalesce": format!("{prefix}:{document}:{layer}:{field}:{gesture}")});
            params[field] = json!(value);
            if let Err(error) = app.run(command, params) {
                app.ui.status = error;
                app.ui.status_error = true;
            }
            if !ui.input(|i| i.pointer.any_down()) {
                ui.data_mut(|d| {
                    let g = d.get_temp_mut_or_default::<(u64, u64)>(key);
                    g.0 = g.0.wrapping_add(1);
                });
            }
        }
    }
}
