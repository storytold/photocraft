//! #12013: a right-click on the canvas with the Gradient tool opens the gradient preset picker at the pointer, in
//! Gradient and Classic gradient mode, with a pixel layer active. A double click on a preset selects it for the tool
//! (`gradient.presets.select`, no history state) and closes the picker; Escape closes it. A gradient fill layer opens
//! nothing, and Shift+right-click keeps the Mode menu.

use super::*;

/// The paint test harness with the Gradient tool, in Classic gradient mode or not.
fn gradient_harness(classic: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = harness(None);
    h.state_mut().ui.tool = Tool::Gradient;
    h.state_mut().ui.tool_options.gradient_classic = classic;
    h.run_steps(2);
    h
}

/// Where the gradient preset picker is open, if it is.
fn gradient_picker_at(h: &Harness<'static, Box<PhotocraftApp>>) -> Option<Pos2> {
    h.state().ui.gradient_picker.map(|at| egui::pos2(at[0], at[1]))
}

/// A right-click at the canvas centre with `mods` held; returns where it was.
fn right_click(h: &mut Harness<'static, Box<PhotocraftApp>>, mods: Modifiers) -> Pos2 {
    mode_right_click(h, mods);
    h.state().last_canvas_rect.center()
}

/// The `gradient.presets.select` calls the journal holds.
fn selects(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<serde_json::Value> {
    h.state().session.journal.iter().filter(|(id, _)| id == "gradient.presets.select").map(|(_, p)| p.clone()).collect()
}

fn history(h: &Harness<'static, Box<PhotocraftApp>>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

/// One press and release at `p`, twice, each in its own pass, as the window delivers a double click.
fn double_click(h: &mut Harness<'static, Box<PhotocraftApp>>, p: Pos2) {
    h.hover_at(p);
    h.run_steps(1);
    for pressed in [true, false, true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
}

#[test]
fn a_right_click_opens_the_gradient_picker_at_the_pointer_in_both_modes() {
    for classic in [false, true] {
        let mut h = gradient_harness(classic);
        let at = right_click(&mut h, Modifiers::NONE);
        assert_eq!(gradient_picker_at(&h), Some(at), "classic {classic}: open at the pointer");
        assert!(h.state().ui.canvas_tool_menu.is_none() && picker_at(&h).is_none(), "classic {classic}: no other popup");
        use egui_kittest::kittest::Queryable;
        assert!(h.query_by_label("Black, White").is_some(), "classic {classic}: the Basics gradients are listed");
    }
}

#[test]
fn a_double_click_selects_the_preset_and_closes_without_a_history_state() {
    use egui_kittest::kittest::Queryable;
    let mut h = gradient_harness(false);
    let before = history(&h);
    right_click(&mut h, Modifiers::NONE);
    let at = h.get_by_label("Black, White").rect().center();
    double_click(&mut h, at);
    assert_eq!(selects(&h).last(), Some(&json!({"preset": "Black, White"})), "the tool's gradient");
    assert!(gradient_picker_at(&h).is_none(), "the double click closed the picker");
    assert_eq!(history(&h), before, "picking a gradient is no history state");
}

#[test]
fn escape_closes_the_picker_and_picks_nothing() {
    let mut h = gradient_harness(false);
    right_click(&mut h, Modifiers::NONE);
    assert!(gradient_picker_at(&h).is_some());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(gradient_picker_at(&h).is_none());
    assert!(selects(&h).is_empty());
}

#[test]
fn a_gradient_fill_layer_opens_nothing() {
    let mut h = gradient_harness(false);
    h.state_mut().run("gradient.fill.create", json!({"from": [100, 150], "to": [300, 150]})).unwrap();
    h.run_steps(2);
    assert!(crate::gradient_ui::fill_layer_active(h.state()), "the fill layer is active");
    right_click(&mut h, Modifiers::NONE);
    assert!(gradient_picker_at(&h).is_none() && h.state().ui.canvas_tool_menu.is_none() && picker_at(&h).is_none());
}

#[test]
fn shift_right_click_keeps_the_mode_menu() {
    let mut h = gradient_harness(false);
    right_click(&mut h, Modifiers::SHIFT);
    assert!(mode_menu_at(&h).is_some(), "the Gradient tool's blend modes");
    assert!(gradient_picker_at(&h).is_none());
}

/// The control channel's secondary press (`ui.pointer` with `"button": "secondary"`) goes through the same tool host,
/// and `ui.inspect` reports the open picker (`gradientPicker`).
#[test]
fn a_secondary_ui_pointer_opens_the_picker_too() {
    use crate::control::{ControlRequest, Reply, handle};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = Tool::Gradient;
    app.sync_views();
    crate::canvas::testing::frame(&mut app);
    let ctx = app.ctx().clone();
    let params = json!({"button": "secondary", "events": [{"kind": "down", "x": 200, "y": 150}, {"kind": "up", "x": 200, "y": 150}]});
    let (req, _rx) = ControlRequest::new("ui.pointer", params);
    assert!(matches!(handle(&mut app, &ctx, &req), Reply::Done(_)));
    for _ in 0..8 {
        crate::canvas::testing::frame(&mut app);
    }
    let (req, _rx) = ControlRequest::new("ui.inspect", json!({}));
    let Reply::Done(inspected) = handle(&mut app, &ctx, &req) else { panic!("ui.inspect replies at once") };
    assert!(inspected["result"]["gradientPicker"]["pos"].is_array(), "the agent sees the picker open: {}", inspected["result"]["gradientPicker"]);
    assert!(app.ui.canvas_tool_menu.is_none());
}
