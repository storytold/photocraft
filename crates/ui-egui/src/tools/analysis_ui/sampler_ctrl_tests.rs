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
    held_with(app, at, CMD)
}

/// The temporary tool the modifiers `m` make with the pointer over document point `at`.
fn held_with(app: &mut PhotocraftApp, at: [f64; 2], m: Modifiers) -> Option<crate::hold_keys::Temporary> {
    app.hover_doc = Some(at);
    let ctx = egui::Context::default();
    let raw = egui::RawInput { events: vec![egui::Event::ModifiersChanged(m)], ..Default::default() };
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

/// CodeRabbit on #14684: only a plain ⌘ stays the Color Sampler over a point. ⌘⌥ and ⌘⇧ were the
/// Move tool there before this change (the issue marks them unmeasured) and stay it.
#[test]
fn command_with_alt_or_shift_over_a_point_is_still_the_move_tool() {
    let mut app = app();
    for m in [Modifiers { alt: true, ..CMD }, Modifiers { shift: true, ..CMD }] {
        assert_eq!(held_with(&mut app, [300.5, 200.5], m), Some(crate::hold_keys::Temporary::Move), "{m:?}");
    }
}

/// The same over `ui.pointer`, where the modifiers come with the event: ⌘⌥ on a point still
/// deletes it as the Color Sampler's Alt does, and ⌘⇧ still drags it.
#[test]
fn command_with_alt_or_shift_on_a_point_keeps_the_sampler_handler() {
    let mut app = app();
    drag(&mut app, [300.5, 200.5], [20.0, 10.0], Modifiers { shift: true, ..CMD });
    assert_eq!(samplers(&app), vec![[320.5, 210.5]], "⌘⇧ drag moves the point");
    drag(&mut app, [320.5, 210.5], [0.0, 0.0], Modifiers { alt: true, ..CMD });
    assert!(samplers(&app).is_empty(), "⌘⌥ click deletes the point");
}

/// Off a point, ⌘⇧ over `ui.pointer` still goes to the Color Sampler's handler (a new point), as
/// before ⌘ was the temporary Move tool there.
#[test]
fn command_with_shift_off_a_point_keeps_the_sampler_handler() {
    let mut app = app();
    drag(&mut app, [120.0, 120.0], [30.0, 20.0], Modifiers { shift: true, ..CMD });
    assert_eq!(samplers(&app).len(), 2, "a new point where the drag ends");
    assert!(alpha(&app, 105, 105) > 0.5, "the layer stays");
}

/// CodeRabbit on #14684: over `ui.pointer` the modifiers belong to each call, so a ⌘ Down off a
/// point can be followed by a Move and Up sent without ⌘. The Move drag the Down began still gets
/// them and completes: the layer moves and the sampler handler doesn't swallow the release.
#[test]
fn a_move_drag_begun_with_command_finishes_when_the_later_events_come_without_it() {
    let mut app = app();
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 120.0, y: 120.0, pressure: 1.0 }, CMD);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 150.0, y: 140.0, pressure: 1.0 }, Modifiers::NONE);
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 150.0, y: 140.0 }, Modifiers::NONE);
    assert!(alpha(&app, 105, 105) < 0.5 && alpha(&app, 175, 165) > 0.5, "the layer moved by 30,20");
    assert_eq!(samplers(&app), vec![[300.5, 200.5]], "no point added");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Move"));
}
