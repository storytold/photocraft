//! #13874: the Frame tool's options-bar stroke is the tool's own, for the next frame drawn; a
//! selected frame keeps its stroke (no history step), and the Properties panel still edits it.
use super::*;
use crate::canvas::tool_event;
use egui::{Event, Key, Modifiers, accesskit::Role, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Frame;
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 120.0, y: 90.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Up { x: 120.0, y: 90.0 }, m);
    assert!(selected(&app).is_some(), "the new frame is selected");
    app
}

/// The options bar (`properties: false`) or the selected frame's Properties section.
fn harness(app: PhotocraftApp, properties: bool) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1100.0, 700.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            if properties {
                let l = selected(app).cloned().unwrap();
                ui.set_max_width(320.0);
                super::properties(app, ui, &l);
            } else {
                ui.horizontal(|ui| options_bar(app, ui, Tool::Frame));
            }
            crate::dialogs::show(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(3);
    h
}

fn entries(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries()
}

fn frame_stroke(app: &PhotocraftApp) -> Option<photocraft_doc::ShapeStroke> {
    selected(app).and_then(|l| l.frame.as_ref()).unwrap().stroke.clone()
}

fn width_field<'a>(h: &'a Harness<'_, PhotocraftApp>) -> egui_kittest::Node<'a> {
    h.get_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Set Frame stroke width")).expect("the width field")
}

fn type_width(h: &mut Harness<'_, PhotocraftApp>, text: &str) {
    width_field(h).click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(Key::Enter);
    h.run_steps(3);
}

#[test]
fn options_bar_changes_leave_the_selected_frame_alone() {
    let h = harness(app(), false);
    let before = entries(h.state());
    let mut h = h;
    // On (a colour), width, then off: none reaches Frame 1.
    h.get_by_label("Set Frame stroke type").click();
    h.run_steps(3);
    if h.query_by_label("RGB Red").is_none() {
        h.get_by_label("RGB").click();
        h.run_steps(2);
    }
    h.get_by_label("RGB Red").click();
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.frame_tool.stroke.is_some(), "the tool's stroke is on");
    type_width(&mut h, "4");
    assert_eq!(h.state().ui.frame_tool.stroke.as_ref().unwrap().width, 4.0);
    assert_eq!(frame_stroke(h.state()), None, "Frame 1 keeps no stroke");
    assert_eq!(entries(h.state()), before, "no history step");
}

#[test]
fn the_bar_shows_the_tools_stroke_while_a_frame_with_its_own_is_selected() {
    let mut app = app();
    let id = selected(&app).unwrap().id;
    app.run("layer.frame.set", json!({"layer": id.0, "stroke": {"color": [0.0, 0.0, 1.0, 1.0], "width": 6.0, "align": "inside"}})).unwrap();
    assert_eq!(frame_stroke(&app).unwrap().width, 6.0);
    let h = harness(app, false);
    assert_eq!(width_field(&h).value().as_deref(), Some("1"), "the tool's 1 px, not the frame's 6 px");
    let combos: Vec<String> = h.get_all_by_role(Role::ComboBox).filter_map(|n| n.value()).collect();
    assert!(combos.iter().any(|v| v == "Center"), "the tool's Center, not the frame's Inside: {combos:?}");
}

#[test]
fn the_next_frame_gets_the_options_bar_stroke() {
    let mut app = app();
    app.ui.frame_tool.stroke = Some(photocraft_doc::ShapeStroke { width: 4.0, ..default_stroke_style() });
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 200.0, y: 150.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 300.0, y: 230.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Up { x: 300.0, y: 230.0 }, m);
    let s = frame_stroke(&app).expect("the new frame's stroke");
    assert_eq!((s.width, s.align), (4.0, photocraft_doc::StrokeAlign::Center));
    assert_eq!(s.paint, photocraft_doc::Fill::Solid(photocraft_doc::Color::BLACK));
}

#[test]
fn the_properties_panel_still_edits_the_frame_in_one_step() {
    let mut h = harness(app(), true);
    let before = entries(h.state()).len();
    h.get_by_label("Stroke").click();
    h.run_steps(3);
    assert!(frame_stroke(h.state()).is_some(), "the frame's stroke is on");
    assert_eq!(entries(h.state()).len(), before + 1, "one history step");
    assert_eq!(h.state().ui.frame_tool.stroke, None, "the tool's own stroke is untouched");
}
