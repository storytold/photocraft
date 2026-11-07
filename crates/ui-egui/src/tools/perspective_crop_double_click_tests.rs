//! #11806: a double-click inside a pending Perspective Crop quad commits it, in one "Perspective
//! Crop" history state; the first click doesn't move the quad, and a double-click outside does
//! nothing.

use egui::{Event, Modifiers, PointerButton, pos2};
use serde_json::json;

use super::*;

/// A 400 × 300 document at 100%, centred in a 600 × 400 canvas (document 0,0 at screen 100,50),
/// with the quad 100,50–300,250 pending.
fn harness() -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    let mut a = PhotocraftApp::new(s, crate::Services::default());
    a.ui.tool = Tool::PerspectiveCrop;
    a.ui.extras.rulers = false;
    a.ui.views = vec![crate::state::View {
        zoom: 1.0,
        center: [200.0, 150.0],
        fit_pending: false,
        fill_pending: false,
        doc_size: [400, 300],
        rotation: 0.0,
        zoom_animating: false,
        zoom_anchor: None,
        display_camera: None,
        zoom_transition: None,
    }];
    a.ui.perspective_crop = Some([[100.0, 50.0], [300.0, 50.0], [300.0, 250.0], [100.0, 250.0]]);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 400.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let v = app.ui.views[0].clone();
            crate::canvas::canvas_view(app, ui, 0, ui.max_rect(), v, true);
        },
        a,
    );
    h.run_steps(2);
    h
}

fn double_click(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, at: egui::Pos2) {
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    h.event(Event::PointerMoved(at));
    h.step();
    for _ in 0..2 {
        h.event(button(true));
        h.step();
        h.event(button(false));
        h.step();
    }
    h.run_steps(2);
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
