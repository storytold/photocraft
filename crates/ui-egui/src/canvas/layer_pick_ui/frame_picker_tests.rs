//! #13885: with the Frame tool a right-click on the canvas opens the layer picker: the layers under
//! the pointer, topmost first, a frame's image indented under its frame, then a separator and
//! Isolate Layers. The Move tool's picker stays as it is.
use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use super::*;

/// The issue's document: 600 × 400 white, Red (100 × 80 at 50,50) put in a frame ("Red Frame"),
/// Blue (120 × 60 at 300,200) and an empty layer on top.
fn app() -> (PhotocraftApp, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400, "background": "white"})).unwrap();
    let mut red = 0;
    for (name, x, y, w, h, color) in [("Red", 50, 50, 100, 80, "#ff0000"), ("Blue", 300, 200, 120, 60, "#0000ff")] {
        app.run("layer.new.layer", json!({"name": name})).unwrap();
        app.run("select.rect", json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
        app.run("edit.fill", json!({"color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        if name == "Red" {
            red = app.session.active().unwrap().active_layer.unwrap().0;
        }
    }
    app.run("layer.new.frame", json!({"x": 50, "y": 50, "width": 100, "height": 80, "layer": red})).unwrap();
    app.run("layer.new.layer", json!({"name": "Empty"})).unwrap();
    (app, red)
}

fn harness(mut a: PhotocraftApp, tool: Tool) -> Harness<'static, PhotocraftApp> {
    a.ui.tool = tool;
    a.sync_views();
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        a,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
    h.run_steps(4);
    h
}

fn screen(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
    crate::canvas::ViewXform::active(h.state()).unwrap().to_screen(x, y)
}

fn right_click(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32, modifiers: Modifiers) {
    let p = screen(h, x, y);
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers });
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

fn rows(h: &Harness<'static, PhotocraftApp>) -> Vec<(String, u8)> {
    let m = h.state().ui.layer_menu.as_ref().expect("the picker");
    m.layers.iter().enumerate().map(|(i, (_, n))| (n.clone(), m.depth.get(i).copied().unwrap_or(0))).collect()
}

#[test]
fn on_empty_canvas_background_then_isolate_layers() {
    let (a, _) = app();
    let mut h = harness(a, Tool::Frame);
    right_click(&mut h, 500.0, 350.0, Modifiers::NONE);
    assert_eq!(rows(&h), [("Background".to_string(), 0)]);
    assert!(h.state().ui.layer_menu.as_ref().unwrap().isolate);
    assert!(h.state().session.active().unwrap().isolated_layers.is_empty());
    h.get_by_label("Isolate Layers").click();
    h.run_steps(3);
    assert!(h.state().ui.layer_menu.is_none(), "choosing closes the picker");
    let st = h.state().session.active().unwrap();
    assert_eq!(st.isolated_layers, st.selected_layers(), "select.isolateLayers ran on the selected layers");
    assert!(!st.isolated_layers.is_empty());
}

#[test]
fn on_the_frame_its_image_is_indented_under_it() {
    let (a, red) = app();
    let mut h = harness(a, Tool::Frame);
    right_click(&mut h, 100.0, 90.0, Modifiers::NONE);
    assert_eq!(rows(&h), [("Red Frame".to_string(), 0), ("Red".to_string(), 1), ("Background".to_string(), 0)]);
    let red_row = h.get_by_label("Red").rect();
    let frame_row = h.get_by_label("Red Frame").rect();
    assert!(red_row.left() > frame_row.left(), "indented: {red_row:?} vs {frame_row:?}");
    h.get_by_label("Red").click();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().active_layer.map(|l| l.0), Some(red), "choosing selects the layer");
}

#[test]
fn the_move_tools_picker_and_command_right_click_stay_as_they_are() {
    let (a, _) = app();
    let mut h = harness(a, Tool::Move);
    right_click(&mut h, 100.0, 90.0, Modifiers::NONE);
    let m = h.state().ui.layer_menu.clone().expect("the Move tool's picker");
    assert_eq!(m.layers.iter().map(|l| l.1.as_str()).collect::<Vec<_>>(), ["Red", "Background"]);
    assert!(!m.isolate && m.depth.is_empty());
    assert!(h.query_by_label("Isolate Layers").is_none());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    let (a, _) = app();
    let mut h = harness(a, Tool::Brush);
    right_click(&mut h, 100.0, 90.0, Modifiers::COMMAND);
    let m = h.state().ui.layer_menu.clone().expect("⌘ right-click lists layers");
    assert!(!m.isolate && m.depth.is_empty());
    assert!(!is_gesture(Tool::Brush, Modifiers::NONE) && is_gesture(Tool::Frame, Modifiers::NONE));
}
