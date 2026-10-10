//! Non-destructive pixel-mask controls, shared by docked and floating Properties.

use photocraft_doc::Layer;
use serde_json::json;

use crate::{PhotocraftApp, widgets};

pub fn targeted(app: &PhotocraftApp, layer: &Layer) -> bool {
    app.ui.mask_target && !app.ui.vector_mask_target && layer.mask.is_some()
}

pub fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    let Some(mask) = &layer.mask else { return };
    let Some(document) = app.session.active().map(|s| s.doc.id.0) else { return };
    ui.push_id((document, layer.id, "pixel-mask-properties"), |ui| {
        if !crate::props_layout::section(ui, "pixel-mask", "Layer Mask") {
            return;
        }
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
        for (field, label, mut value, max, unit) in
            [("density", tl!("Density"), mask.density * 100.0, 100.0, "%"), ("feather", tl!("Feather"), mask.feather, 1000.0, "px")]
        {
            let response = widgets::slider_row(ui, label, &mut value, 0.0..=max, unit, None);
            response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(value), label));
            if response.changed() {
                let mut params = json!({"document": document, "layer": layer.id.0,
                    "coalesce": format!("pixel-mask:{document}:{}:{field}:{gesture}", layer.id.0)});
                params[field] = json!(value);
                if let Err(error) = app.run("layer.layerMask.edit", params) {
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
    });
}
