//! The Move tool's Align and Distribute controls (Photoshop shows them whenever the Move tool is
//! active; they enable once enough layers are selected). Every button dispatches an existing
//! `layer.align.*` / `layer.distribute.*` command.

use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{PhotocraftApp, icons};

/// (command id, icon, label). The labels are the Layer menu's, already translated.
const ALIGN: [(&str, &str, &str); 6] = [
    ("layer.align.leftEdges", "align-edge-left", "Left Edges"),
    ("layer.align.horizontalCenters", "align-edge-hcenter", "Horizontal Centers"),
    ("layer.align.rightEdges", "align-edge-right", "Right Edges"),
    ("layer.align.topEdges", "align-edge-top", "Top Edges"),
    ("layer.align.verticalCenters", "align-edge-vcenter", "Vertical Centers"),
    ("layer.align.bottomEdges", "align-edge-bottom", "Bottom Edges"),
];

const DISTRIBUTE: [(&str, &str, &str); 8] = [
    ("layer.distribute.topEdges", "distribute-top", "Top Edges"),
    ("layer.distribute.verticalCenters", "distribute-vcenter", "Vertical Centers"),
    ("layer.distribute.bottomEdges", "distribute-bottom", "Bottom Edges"),
    ("layer.distribute.leftEdges", "distribute-left", "Left Edges"),
    ("layer.distribute.horizontalCenters", "distribute-hcenter", "Horizontal Centers"),
    ("layer.distribute.rightEdges", "distribute-right", "Right Edges"),
    ("layer.distribute.horizontally", "distribute-horizontal", "Horizontally"),
    ("layer.distribute.vertically", "distribute-vertical", "Vertically"),
];

/// The parameters of a `layer.align.*` run from the Move tool or Properties: Align To Canvas
/// sends `to: artboard` (the artboard the layers share, else the canvas); Selection leaves the
/// engine's own choice (the selection's bounds, else the layers', a lone layer's canvas or
/// artboard) (#10337).
pub(crate) fn align_params(app: &PhotocraftApp) -> Value {
    if app.ui.tool_options.move_align_to_canvas { json!({"to": "artboard"}) } else { json!({}) }
}

fn align_to_label(canvas: bool) -> &'static str {
    if canvas { tl!("Canvas") } else { tl!("Selection") }
}

/// Align To: Selection or Canvas, as a drop-down (Properties' Align and Distribute).
pub(crate) fn align_to(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let canvas = &mut app.ui.tool_options.move_align_to_canvas;
    ui.label(tl!("Align To"));
    crate::widgets::dropdown(ui, "align-to", tl!("Align To"), canvas, &[(false, "Selection"), (true, "Canvas")], 130.0);
}

/// The six align icons followed by a menu holding both groups with icons and names.
/// Each icon uses 20 pt artwork inside a 28 pt hit target for high-DPI legibility
/// without changing enabled-state or command dispatch (#8849).
pub fn options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    ui.spacing_mut().item_spacing.x = 0.0;
    for (cmd, icon, label) in ALIGN {
        let tip = format!("{}: {}", tl!("Align"), tl!(label));
        let on = app.session.is_enabled(cmd);
        if ui.add_enabled_ui(on, |ui| icons::button_with_icon_size(ui, icon, 28.0, 20.0, false, &tip)).inner.clicked() {
            let _ = app.run(cmd, align_params(app));
        }
    }
    ui.add_space(6.0);
    let more = icons::button_with_icon_size(ui, "ellipsis", 28.0, 20.0, false, tl!("Align and distribute"));
    egui::Popup::menu(&more).show(|ui| {
        ui.set_min_width(200.0);
        // Align To, as in Photoshop's Move tool: radio rows, since a drop-down inside the menu
        // would close it.
        ui.label(egui::RichText::new(tl!("Align To")).small().color(t.text_dim));
        for c in [false, true] {
            ui.radio_value(&mut app.ui.tool_options.move_align_to_canvas, c, align_to_label(c));
        }
        ui.separator();
        let align = align_params(app);
        let mut section = |ui: &mut egui::Ui, title: &str, items: &[(&str, &str, &str)], params: Value| {
            ui.label(egui::RichText::new(title).small().color(t.text_dim));
            for (cmd, icon, label) in items {
                let on = app.session.is_enabled(cmd);
                let tint = if on { t.icon } else { t.text_dim };
                let button = egui::Button::image_and_text(icons::image(icon, 16.0, tint), tl!(label));
                if ui.add_enabled(on, button).clicked() {
                    let _ = app.run(cmd, params.clone());
                    ui.close();
                }
            }
        };
        section(ui, tl!("Align"), &ALIGN, align);
        ui.separator();
        section(ui, tl!("Distribute"), &DISTRIBUTE, json!({}));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_button_is_a_live_command_with_an_icon() {
        let specs = photocraft_engine::command_specs();
        for (cmd, icon, _) in ALIGN.iter().chain(DISTRIBUTE.iter()) {
            assert!(specs.iter().any(|c| c.id == *cmd), "{cmd} is not a command");
            assert!(icons::exists(icon), "{icon} is not embedded");
        }
    }

    /// #10337: the Move tool's Align To (radio rows in the ⋯ menu) sends its icons' aligns to the
    /// canvas; Selection, the default, aligns the selected layers to each other.
    #[test]
    fn align_to_canvas_in_the_move_options_aligns_to_the_canvas() {
        use egui_kittest::kittest::Queryable;
        for canvas in [false, true] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
            let mut ids = Vec::new();
            for x in [40, 90] {
                ids.push(app.run("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
                app.run("select.rect", json!({"x": x, "y": 30, "width": 20, "height": 20})).unwrap();
                app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
                app.run("select.deselect", json!({})).unwrap();
            }
            app.run("layer.select", json!({"layer": ids[0], "mode": "add"})).unwrap();
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(400.0, 400.0)).build_ui_state(
                |ui, app: &mut PhotocraftApp| {
                    if ui.ctx().cumulative_pass_nr() == 0 {
                        PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                        return;
                    }
                    ui.horizontal(|ui| options(app, ui));
                },
                app,
            );
            h.run_steps(3);
            if canvas {
                h.get_by_label("Align and distribute").click();
                h.run_steps(3);
                h.get_by_label("Canvas").click();
                h.run_steps(3);
                assert!(h.state().ui.tool_options.move_align_to_canvas, "the radio sets Align To Canvas");
                h.key_press(egui::Key::Escape);
                h.run_steps(3);
            }
            h.get_by_label("Align: Left Edges").click();
            h.run_steps(3);
            let st = h.state().session.active().unwrap();
            let lefts: Vec<i32> =
                ids.iter().map(|&id| st.doc.layer(photocraft_doc::LayerId(id)).and_then(|l| l.surface()).unwrap().content_bounds().x0).collect();
            assert_eq!(lefts, if canvas { [0, 0] } else { [40, 40] }, "Align To Canvas: {canvas}");
        }
    }
}
