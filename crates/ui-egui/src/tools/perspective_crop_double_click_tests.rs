//! #11806: a double-click inside a pending Perspective Crop quad commits it, in one "Perspective
//! Crop" history state; the first click doesn't move the quad, and a double-click outside does
//! nothing.

use egui::{Event, Modifiers, PointerButton, pos2};
use serde_json::json;

use super::*;

/// A 400 × 300 document at 100%, centred in a 600 × 400 canvas (document 0,0 at screen 100,50),
/// with the quad 100,50–300,250 pending.
fn harness() -> egui_kittest::Harness<'static, PhotocraftApp> {
    harness_with(0.0, [200.0, 150.0], QUAD)
}

const QUAD: [[f64; 2]; 4] = [[100.0, 50.0], [300.0, 50.0], [300.0, 250.0], [100.0, 250.0]];

/// The same document in a 600 × 400 canvas `left` points from the window's left edge, the view
/// centred on `center` (document px), with `quad` pending.
fn harness_with(left: f32, center: [f32; 2], quad: [[f64; 2]; 4]) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    let mut a = PhotocraftApp::new(s, crate::Services::default());
    a.ui.tool = Tool::PerspectiveCrop;
    a.ui.extras.rulers = false;
    a.ui.views = vec![crate::state::View {
        zoom: 1.0,
        center,
        fit_pending: false,
        fill_pending: false,
        doc_size: [400, 300],
        rotation: 0.0,
        zoom_animating: false,
        zoom_anchor: None,
        display_camera: None,
        zoom_transition: None,
    }];
    a.ui.perspective_crop = Some(quad);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(600.0 + left, 400.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            let v = app.ui.views[0].clone();
            let rect = egui::Rect::from_min_size(pos2(left, 0.0), egui::vec2(600.0, 400.0));
            crate::canvas::canvas_view(app, ui, 0, rect, v, true);
        },
        a,
    );
    h.run_steps(2);
    h
}

fn button(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, at: egui::Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}

fn clicks(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, at: egui::Pos2, n: usize) {
    h.event(Event::PointerMoved(at));
    h.step();
    for _ in 0..n {
        button(h, at, true);
        button(h, at, false);
    }
    h.run_steps(2);
}

fn double_click(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, at: egui::Pos2) {
    clicks(h, at, 2);
}

fn steps(h: &egui_kittest::Harness<'static, PhotocraftApp>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

#[test]
fn a_double_click_inside_the_quad_commits_it() {
    let mut h = harness();
    let before = steps(&h);
    // Document (200, 150), the quad's middle, is screen (300, 200).
    double_click(&mut h, pos2(300.0, 200.0));
    let st = h.state();
    assert_eq!(st.ui.perspective_crop, None, "committed");
    assert_eq!(steps(&h), before + 1);
    let d = &h.state().session.active().unwrap();
    assert_eq!(d.history.undo_label(), Some("Perspective Crop"));
    assert_eq!((d.doc.size.width, d.doc.size.height), (200, 200), "the quad as it was: the first click didn't move it");
}

#[test]
fn a_double_click_outside_the_quad_does_nothing() {
    let mut h = harness();
    let before = steps(&h);
    // Document (20, 20), outside the quad, is screen (120, 70).
    double_click(&mut h, pos2(120.0, 70.0));
    assert!(h.state().ui.perspective_crop.is_some(), "still pending");
    assert_eq!(steps(&h), before);
}

#[test]
fn a_single_click_inside_the_quad_keeps_it_pending() {
    let mut h = harness();
    let before = steps(&h);
    clicks(&mut h, pos2(300.0, 200.0), 1);
    assert_eq!(h.state().ui.perspective_crop, Some(QUAD), "pending, unmoved");
    assert_eq!(steps(&h), before);
}

#[test]
fn a_double_click_on_a_corner_handle_does_nothing() {
    let mut h = harness();
    let before = steps(&h);
    // Document (100, 50), the top-left corner, is screen (200, 100).
    double_click(&mut h, pos2(200.0, 100.0));
    assert_eq!(h.state().ui.perspective_crop, Some(QUAD), "still pending, unmoved");
    assert_eq!(steps(&h), before);
}

/// egui starts a drag as soon as the pointer leaves the canvas, even within the click distance,
/// and still reports the click when it comes back: that click doesn't move the quad either.
#[test]
fn a_click_that_briefly_leaves_the_canvas_does_not_move_the_quad() {
    // The canvas starts 40 points into the window, document 0,0 at screen 40,0; the quad's left
    // edge is on the canvas's.
    let quad = [[0.0, 50.0], [200.0, 50.0], [200.0, 250.0], [0.0, 250.0]];
    let mut h = harness_with(40.0, [300.0, 200.0], quad);
    let before = steps(&h);
    let (down, up) = (pos2(40.5, 150.0), pos2(42.5, 150.0));
    h.event(Event::PointerMoved(down));
    h.step();
    button(&mut h, down, true);
    // 5.4 points off the canvas (past egui's 5-point hit radius), 5.9 from the press (within its
    // 6-point click distance).
    h.event(Event::PointerMoved(pos2(34.6, 150.0)));
    h.step();
    h.event(Event::PointerMoved(up));
    h.step();
    button(&mut h, up, false);
    assert_eq!(h.state().ui.perspective_crop, Some(quad), "the first click left the quad where it was");
    // The second click of the double-click commits it.
    button(&mut h, up, true);
    button(&mut h, up, false);
    h.run_steps(2);
    assert_eq!(h.state().ui.perspective_crop, None, "committed");
    assert_eq!(steps(&h), before + 1);
}
