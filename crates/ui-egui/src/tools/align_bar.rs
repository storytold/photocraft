//! The Move tool's Align and Distribute controls (Photoshop shows them whenever the Move tool is
//! active; they enable once enough layers are selected). Every button dispatches an existing
//! `layer.align.*` / `layer.distribute.*` command.
//!
//! The bar reads, after Show Transform Controls: Align left, horizontal centres, right and
//! Distribute vertically; Align top, vertical centres, bottom and Distribute horizontally; the
//! "..." button (a compact icon panel); the gear (the Move tool's three hover and group toggles,
//! #11381).

use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{PhotocraftApp, icons};

/// (command id, icon, label). The labels are the Layer menu's, already translated.
pub(crate) const ALIGN: [(&str, &str, &str); 6] = [
    ("layer.align.leftEdges", "align-edge-left", "Left Edges"),
    ("layer.align.horizontalCenters", "align-edge-hcenter", "Horizontal Centers"),
    ("layer.align.rightEdges", "align-edge-right", "Right Edges"),
    ("layer.align.topEdges", "align-edge-top", "Top Edges"),
    ("layer.align.verticalCenters", "align-edge-vcenter", "Vertical Centers"),
    ("layer.align.bottomEdges", "align-edge-bottom", "Bottom Edges"),
];

/// The panel's Distribute group: top, vertical centres, bottom, left, horizontal centres, right.
const DISTRIBUTE: [(&str, &str, &str); 6] = [
    ("layer.distribute.topEdges", "distribute-top", "Top Edges"),
    ("layer.distribute.verticalCenters", "distribute-vcenter", "Vertical Centers"),
    ("layer.distribute.bottomEdges", "distribute-bottom", "Bottom Edges"),
    ("layer.distribute.leftEdges", "distribute-left", "Left Edges"),
    ("layer.distribute.horizontalCenters", "distribute-hcenter", "Horizontal Centers"),
    ("layer.distribute.rightEdges", "distribute-right", "Right Edges"),
];

/// One bar button: (command id, icon, tooltip).
type BarButton = (&'static str, &'static str, &'static str);

/// The two button groups of the bar, in order. The tooltips are the reference application's.
fn bar_groups() -> [[BarButton; 4]; 2] {
    [
        [
            ("layer.align.leftEdges", "align-edge-left", tl!("Align left edges")),
            ("layer.align.horizontalCenters", "align-edge-hcenter", tl!("Align horizontal centers")),
            ("layer.align.rightEdges", "align-edge-right", tl!("Align right edges")),
            ("layer.distribute.vertically", "distribute-vertical", tl!("Distribute vertically")),
        ],
        [
            ("layer.align.topEdges", "align-edge-top", tl!("Align top edges")),
            ("layer.align.verticalCenters", "align-edge-vcenter", tl!("Align vertical centers")),
            ("layer.align.bottomEdges", "align-edge-bottom", tl!("Align bottom edges")),
            ("layer.distribute.horizontally", "distribute-horizontal", tl!("Distribute horizontally")),
        ],
    ]
}

/// Whether the active document has artboards. The reference application greys Align To out
/// there, showing Selection, and viewing such a document turns the setting back to Selection.
fn align_to_off(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|st| st.doc.has_artboards())
}

/// Align To as the reference application keeps it: Selection whenever the active document has
/// artboards (#10337).
pub(crate) fn sync_align_to(app: &mut PhotocraftApp) {
    if align_to_off(app) {
        app.ui.tool_options.move_align_to_canvas = false;
    }
}

/// The parameters of a `layer.align.*` run from the Move tool or Properties: Align To Canvas
/// sends `to: canvas`; Selection leaves the engine's own choice (the selection's bounds, else the
/// layers', a lone layer's canvas or artboard) (#10337).
pub(crate) fn align_params(app: &PhotocraftApp) -> Value {
    if app.ui.tool_options.move_align_to_canvas && !align_to_off(app) { json!({"to": "canvas"}) } else { json!({}) }
}

/// The parameters a bar or panel button's command runs with: Align To for the aligns, none for
/// the distributes.
fn params_for(app: &PhotocraftApp, cmd: &str) -> Value {
    if cmd.starts_with("layer.align.") { align_params(app) } else { json!({}) }
}

fn align_to_label(canvas: bool) -> &'static str {
    if canvas { tl!("Canvas") } else { tl!("Selection") }
}

/// Align To: Selection or Canvas, as a drop-down (Properties' Align and Distribute), greyed out
/// in a document with artboards.
pub(crate) fn align_to(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    sync_align_to(app);
    let on = !align_to_off(app);
    let canvas = &mut app.ui.tool_options.move_align_to_canvas;
    ui.label(tl!("Align To"));
    ui.add_enabled_ui(on, |ui| crate::widgets::dropdown(ui, "align-to", tl!("Align To"), canvas, &[(false, "Selection"), (true, "Canvas")], 130.0));
}

/// Whether the "..." panel is open: a click on the button toggles it and nothing else closes it
/// (Escape included), as in the reference application.
pub(crate) fn panel_open_id() -> egui::Id {
    egui::Id::unique("move-align-panel-open")
}

pub(crate) fn panel_is_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(panel_open_id())).unwrap_or(false)
}

/// The two groups of four icons, then the "..." button and its panel. Each icon uses 20 pt
/// artwork inside a 28 pt hit target for high-DPI legibility without changing enabled-state or
/// command dispatch (#8849).
pub fn options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    sync_align_to(app);
    ui.spacing_mut().item_spacing.x = 0.0;
    for group in bar_groups() {
        for (cmd, icon, tip) in group {
            let on = app.session.is_enabled(cmd);
            if ui.add_enabled_ui(on, |ui| icons::button_with_icon_size(ui, icon, 28.0, 20.0, false, tip)).inner.clicked() {
                let _ = app.run(cmd, params_for(app, cmd));
            }
        }
        ui.add_space(6.0);
    }
    let more = icons::button_with_icon_size(ui, "ellipsis", 28.0, 20.0, false, tl!("Align and distribute"));
    let ctx = ui.ctx().clone();
    let open = panel_is_open(&ctx);
    if more.clicked() {
        ctx.data_mut(|d| d.insert_temp(panel_open_id(), !open));
    }
    if panel_is_open(&ctx) {
        panel(app, &ctx, more.rect);
    }
    ui.add_space(6.0);
}

/// The compact icon panel under the "..." button: Align (6) and Distribute (6). The Distribute
/// Spacing group (#10715) goes below these as a further `group` row; Align To (#10358) is last.
fn panel(app: &mut PhotocraftApp, ctx: &egui::Context, anchor: egui::Rect) {
    let t = Tokens::get(ctx);
    egui::Area::new(egui::Id::unique("move-align-panel")).order(egui::Order::Foreground).fixed_pos(anchor.left_bottom() + egui::vec2(0.0, 2.0)).show(
        ctx,
        |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
                let mut group = |ui: &mut egui::Ui, title: &str, kind: &str, items: &[(&str, &str, &str)]| {
                    ui.label(egui::RichText::new(title).small().color(t.text_dim));
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for (cmd, icon, label) in items {
                            let tip = format!("{kind}: {}", tl!(label));
                            let on = app.session.is_enabled(cmd);
                            if ui.add_enabled_ui(on, |ui| icons::button_with_icon_size(ui, icon, 28.0, 20.0, false, &tip)).inner.clicked() {
                                let _ = app.run(cmd, params_for(app, cmd));
                            }
                        }
                    });
                };
                group(ui, tl!("Align"), tl!("Align"), &ALIGN);
                ui.add_space(4.0);
                group(ui, tl!("Distribute"), tl!("Distribute"), &DISTRIBUTE);
                // Align To, as in the reference application's Move tool (#10337): greyed out at
                // Selection in a document with artboards.
                ui.add_space(4.0);
                ui.label(egui::RichText::new(tl!("Align To")).small().color(t.text_dim));
                let on = !align_to_off(app);
                ui.add_enabled_ui(on, |ui| {
                    ui.horizontal(|ui| {
                        for c in [false, true] {
                            ui.radio_value(&mut app.ui.tool_options.move_align_to_canvas, c, align_to_label(c));
                        }
                    });
                });
            });
        },
    );
}

/// The gear after the "..." button: a menu of the Move tool's three check items, all on by
/// default and saved with the tool options.
pub fn gear(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let gear = icons::button_with_icon_size(ui, "settings", 28.0, 20.0, false, tl!("Options"));
    egui::Popup::menu(&gear).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(200.0);
        let o = &mut app.ui.tool_options;
        ui.checkbox(&mut o.move_show_hover_bounds, tl!("Show layer bounds on hover"));
        ui.checkbox(&mut o.move_expand_group_on_click, tl!("Expand layer group on click"));
        ui.checkbox(&mut o.move_show_layers_hover_bounds, tl!("Show hover bounds from Layers"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_button_is_a_live_command_with_an_icon() {
        let specs = photocraft_engine::command_specs();
        let bar: Vec<BarButton> = bar_groups().into_iter().flatten().collect();
        let panel = ALIGN.iter().chain(DISTRIBUTE.iter());
        for (cmd, icon) in bar.iter().map(|b| (b.0, b.1)).chain(panel.map(|b| (b.0, b.1))) {
            assert!(specs.iter().any(|c| c.id == cmd), "{cmd} is not a command");
            assert!(icons::exists(icon), "{icon} is not embedded");
        }
        assert!(icons::exists("settings"));
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
            h.get_by_label("Align left edges").click();
            h.run_steps(3);
            let st = h.state().session.active().unwrap();
            let lefts: Vec<i32> =
                ids.iter().map(|&id| st.doc.layer(photocraft_doc::LayerId(id)).and_then(|l| l.surface()).unwrap().content_bounds().x0).collect();
            assert_eq!(lefts, if canvas { [0, 0] } else { [40, 40] }, "Align To Canvas: {canvas}");
        }
    }

    /// #10337, as measured in the reference application: in a document with artboards Align To is
    /// greyed out at Selection, so the align icons align the selected layers to each other, and a
    /// Canvas setting turns back to Selection.
    #[test]
    fn align_to_is_selection_in_a_document_with_artboards() {
        use egui_kittest::kittest::Queryable;
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
        app.run("layer.new.artboardFromLayers", json!({})).unwrap();
        app.run("layer.select", json!({"layer": ids[0]})).unwrap();
        app.run("layer.select", json!({"layer": ids[1], "mode": "add"})).unwrap();
        let left = |app: &PhotocraftApp, id: u64| {
            let st = app.session.active().unwrap();
            st.doc.layer(photocraft_doc::LayerId(id)).and_then(|l| l.surface()).unwrap().content_bounds().x0
        };
        let first = left(&app, ids[0]);
        assert!(first > 0, "the layers sit away from the canvas edge: {first}");
        app.ui.tool_options.move_align_to_canvas = true;
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
        assert!(!h.state().ui.tool_options.move_align_to_canvas, "Align To turns back to Selection");
        h.get_by_label("Align left edges").click();
        h.run_steps(3);
        let lefts: Vec<i32> = ids.iter().map(|&id| left(h.state(), id)).collect();
        assert_eq!(lefts, [first, first], "aligned to each other, not to the canvas");
    }
}
