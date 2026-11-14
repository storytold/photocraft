//! #11731: with the Color Sampler, ⌘ (Ctrl off the Mac) over one of its points keeps the point:
//! a ⌘-drag that starts on it moves it. Off a point ⌘ is the temporary Move tool: the drag moves
//! the active layer (History step "Move"). The tool stays Color Sampler either way, in the app
//! (the held key) and over `ui.pointer` (the modifier sent with the event).

use super::*;
use egui::Modifiers;

const CMD: Modifiers = Modifiers::COMMAND;

/// A 400 × 300 document: a pixel layer with a red square at 100,100–150,150, active, and one
/// Color Sampler point at 300.5,200.5; the Color Sampler tool.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 50, "height": 50})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("view.colorSamplers.add", json!({"x": 300.5, "y": 200.5})).unwrap();
    app.ui.tool = Tool::ColorSampler;
    app
}

/// A drag from `from` by `by` with `m` held, as `ui.pointer` sends it (the modifier with each
/// event, no held key).
fn drag(app: &mut PhotocraftApp, from: [f64; 2], by: [f64; 2], m: Modifiers) {
    crate::canvas::tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    for k in 1..=3 {
        let t = f64::from(k) / 3.0;
        crate::canvas::tool_event(app, ToolEvent::Move { x: from[0] + by[0] * t, y: from[1] + by[1] * t, pressure: 1.0 }, m);
    }
    crate::canvas::tool_event(app, ToolEvent::Up { x: from[0] + by[0], y: from[1] + by[1] }, m);
}

fn samplers(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    app.session.active().unwrap().color_samplers.clone()
}

/// The active layer's alpha at document pixel (`x`, `y`).
fn alpha(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)[3]
}

/// The temporary tool ⌘ held makes with the pointer over document point `at`.
fn held_with_command(app: &mut PhotocraftApp, at: [f64; 2]) -> Option<crate::hold_keys::Temporary> {
    app.hover_doc = Some(at);
    let ctx = egui::Context::default();
    let raw = egui::RawInput { events: vec![egui::Event::ModifiersChanged(CMD)], ..Default::default() };
    ctx.begin_pass(raw);
    let t = crate::hold_keys::held_tool(app, &ctx);
    ctx.end_pass().textures_delta.clear();
    t
}

#[test]
fn command_drag_on_a_point_moves_the_point() {
    let mut app = app();
    drag(&mut app, [300.5, 200.5], [20.0, 10.0], CMD);
    assert_eq!(samplers(&app), vec![[320.5, 210.5]]);
    assert!(alpha(&app, 105, 105) > 0.5 && alpha(&app, 155, 155) < 0.5, "the layer didn't move");
    assert_eq!(app.ui.tool, Tool::ColorSampler);
}

#[test]
fn command_drag_off_a_point_moves_the_layer() {
    let mut app = app();
    drag(&mut app, [120.0, 120.0], [30.0, 20.0], CMD);
    assert!(alpha(&app, 105, 105) < 0.5 && alpha(&app, 175, 165) > 0.5, "the layer moved by 30,20");
    assert_eq!(samplers(&app), vec![[300.5, 200.5]], "no point added or moved");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Move"));
    assert_eq!(app.ui.tool, Tool::ColorSampler, "the tool stays the Color Sampler");
}

#[test]
fn held_command_is_the_move_tool_only_off_a_point() {
    let mut app = app();
    assert_eq!(held_with_command(&mut app, [300.5, 200.5]), None, "over a point: the Color Sampler");
    assert_eq!(held_with_command(&mut app, [50.0, 50.0]), Some(crate::hold_keys::Temporary::Move), "off a point: the Move tool");
    // While a point is dragged, ⌘ stays the Color Sampler wherever the pointer is.
    app.ui.analysis.drag = Some(Drag::Sampler(0, [50.0, 50.0]));
    assert_eq!(held_with_command(&mut app, [50.0, 50.0]), None);
}

#[test]
fn without_command_a_drag_off_a_point_adds_one_as_before() {
    let mut app = app();
    drag(&mut app, [120.0, 120.0], [30.0, 20.0], Modifiers::NONE);
    assert_eq!(samplers(&app).len(), 2, "a new point where the drag ends");
    assert!(alpha(&app, 105, 105) > 0.5, "the layer stays");
}
