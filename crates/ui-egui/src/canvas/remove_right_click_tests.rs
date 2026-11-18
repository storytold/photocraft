//! #12551: with Remove strokes waiting (Remove after each stroke off), a right-click on the canvas
//! opens nothing, neither the Brush Preset picker nor a menu, and the strokes, their tint and the
//! document stay as they were. With nothing waiting, and with the other painting tools, it still
//! opens the picker.

use egui::{PointerButton, Pos2};
use egui_kittest::Harness;
use serde_json::{Value, json};

use super::tests::{drag, harness, press};
use crate::PhotocraftApp;
use crate::control::{ControlRequest, Reply, handle};
use crate::state::Tool;

/// The Remove tool with Remove after each stroke off, nothing painted yet.
fn remove_harness() -> Harness<'static, PhotocraftApp> {
    let mut h = harness(None);
    h.state_mut().ui.tool = Tool::Remove;
    h.state_mut().ui.tool_options.remove_after_each_stroke = false;
    h.run_steps(2);
    h
}

/// One Remove stroke painted with the mouse, waiting.
fn waiting() -> Harness<'static, PhotocraftApp> {
    let mut h = remove_harness();
    drag(&mut h, PointerButton::Primary);
    assert_eq!(h.state().remove_pending.len(), 1, "one stroke waits");
    assert!(h.state().remove_trail.is_some(), "tinted");
    h
}

/// The waiting strokes, their tint along the stroke's row, the history and the journal.
fn snapshot(app: &PhotocraftApp) -> (Vec<Value>, Option<Vec<u8>>, usize, usize) {
    let st = app.session.active().unwrap();
    let row = st.doc.size.height as usize / 2;
    let tint = app.remove_trail.as_ref().map(|t| (0..st.doc.size.width as usize).map(|x| t.coverage(x, row)).collect());
    (app.remove_pending.clone(), tint, st.history.past_len(), app.session.journal.len())
}

fn nothing_open(app: &PhotocraftApp) -> bool {
    crate::brush_picker::opened_at(app).is_none() && app.ui.canvas_tool_menu.is_none() && !crate::color_hud::is_open(&app.ui)
}

fn mouse_right_click(h: &mut Harness<'static, PhotocraftApp>) -> Pos2 {
    let c = h.state().last_canvas_rect.center();
    h.event(egui::Event::PointerMoved(c));
    h.run_steps(1);
    press(h, c, PointerButton::Secondary, true);
    press(h, c, PointerButton::Secondary, false);
    c
}

/// `ui.pointer` with `"button": "secondary"`: a down and an up on the image's centre.
fn pointer_right_click(h: &mut Harness<'static, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    let events = json!([{"kind": "down", "x": 200, "y": 150}, {"kind": "up", "x": 200, "y": 150}]);
    let (req, _rx) = ControlRequest::new("ui.pointer", json!({"button": "secondary", "events": events}));
    assert!(matches!(handle(h.state_mut(), &ctx, &req), Reply::Done(v) if v["ok"] == true));
    h.run_steps(2);
}

#[test]
fn a_mouse_right_click_with_strokes_waiting_opens_nothing_and_keeps_them() {
    let mut h = waiting();
    let before = snapshot(h.state());
    mouse_right_click(&mut h);
    assert!(nothing_open(h.state()), "no picker, no menu");
    assert_eq!(snapshot(h.state()), before, "the strokes, their tint and the document stay");
}

#[test]
fn a_control_channel_right_click_with_strokes_waiting_opens_nothing_and_keeps_them() {
    let mut h = waiting();
    let before = snapshot(h.state());
    pointer_right_click(&mut h);
    assert!(nothing_open(h.state()), "no picker, no menu");
    assert_eq!(snapshot(h.state()), before, "the strokes, their tint and the document stay");
}
