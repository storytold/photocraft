//! #12928: with Quick Selection, the options-bar brush chip opens the compact tip editor (Size,
//! Hardness, Spacing, the angle/roundness widget, Angle, Roundness and the "Size:" pen control)
//! with no preset list, no search field and no Tolerance dropdown. The Background Eraser keeps its
//! popup with Tolerance, the painting tools the Brush Preset picker.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::bg_eraser_popup::PRESSURE_WARNING;
use crate::state::Tool;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.tool = tool;
    app.sync_views();
    app
}

/// The canvas with the picker open as the brush chip opens it.
fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::paint_mouse::sync_tool_brush(app);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h
}

fn has_fonts(h: &Harness<'static, PhotocraftApp>) -> bool {
    h.ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into())))
}

fn open(h: &mut Harness<'static, PhotocraftApp>) {
    let c = h.state().last_canvas_rect.center();
    let ctx = h.ctx.clone();
    crate::brush_picker::open(&mut h.state_mut().ui, &ctx, [c.x, c.y]);
    h.run_steps(3);
}

fn combo<'a>(h: &'a Harness<'static, PhotocraftApp>, label: &str) -> Option<egui_kittest::Node<'a>> {
    h.get_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().label().as_deref() == Some(label))
}

#[test]
fn quick_selection_opens_the_compact_tip_editor_without_tolerance() {
    let mut h = harness(app(Tool::QuickSelection));
    if !has_fonts(&h) {
        return;
    }
    open(&mut h);
    let order = ["Size", "Hardness", "Spacing", "Angle", "Roundness", "Size:"];
    let tops: Vec<f32> = order
        .iter()
        .map(|l| match *l {
            "Size" => h.get_all_by_role(Role::SpinButton).next().expect("the Size field").rect().top(),
            "Size:" => combo(&h, l).expect(l).rect().top(),
            "Hardness" | "Spacing" => h.query_all_by_label(l).next().expect(l).rect().top(),
            _ => h.get_by_label(l).rect().top(),
        })
        .collect();
    assert!(tops.windows(2).all(|w| w[0] <= w[1]), "top to bottom {order:?}: {tops:?}");
    assert!(combo(&h, "Tolerance:").is_none() && h.query_by_label(PRESSURE_WARNING).is_none(), "no Tolerance dropdown");
    assert!(h.query_by_label("Search Brushes").is_none() && h.query_by_label("Brush Preset Options").is_none(), "no presets, search or gear");
    let size = combo(&h, "Size:").unwrap().accesskit_node().value().unwrap_or_default();
    assert_eq!(size, "Off", "Size: Off by default");
}

#[test]
fn the_size_pen_control_offers_off_pen_pressure_and_stylus_wheel() {
    let mut h = harness(app(Tool::QuickSelection));
    if !has_fonts(&h) {
        return;
    }
    open(&mut h);
    combo(&h, "Size:").unwrap().click();
    h.run_steps(2);
    for entry in ["Off", "Pen Pressure", "Stylus Wheel"] {
        assert!(h.query_all_by_label(entry).next().is_some(), "{entry}");
    }
    h.get_all_by_label("Pen Pressure").last().unwrap().click();
    h.run_steps(3);
    assert!(h.state().session.tools.brush.pressure_size, "Size: Pen Pressure is the brush's pressure for size");
}

#[test]
fn enter_in_a_field_applies_it_as_one_brush_edit_and_closes() {
    let mut h = harness(app(Tool::QuickSelection));
    if !has_fonts(&h) {
        return;
    }
    open(&mut h);
    h.get_all_by_role(Role::SpinButton).next().expect("the Size field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("64".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().session.tools.brush.size, 64.0, "the typed size applies");
    assert_eq!(h.state().ui.brush_picker, None, "Enter closes the picker");
}

#[test]
fn the_background_eraser_and_the_brush_keep_their_pickers() {
    let mut h = harness(app(Tool::BackgroundEraser));
    if !has_fonts(&h) {
        return;
    }
    open(&mut h);
    assert!(combo(&h, "Tolerance:").is_some(), "the Background Eraser's popup keeps Tolerance");
    let mut h = harness(app(Tool::Brush));
    open(&mut h);
    assert!(h.query_by_label("Brush Preset Options").is_some(), "the Brush keeps the Brush Preset picker");
}
