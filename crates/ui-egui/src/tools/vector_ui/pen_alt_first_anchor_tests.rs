//! Corrected first/later-anchor measurements from #12895, through canvas tool dispatch.

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;
use egui::Modifiers;
use serde_json::json;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Pen;
    app
}

fn down(app: &mut PhotocraftApp, x: f64, y: f64) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
}

fn move_to(app: &mut PhotocraftApp, x: f64, y: f64, mods: Modifiers) {
    tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, mods);
}

fn up(app: &mut PhotocraftApp, x: f64, y: f64, mods: Modifiers) {
    tool_event(app, ToolEvent::Up { x, y }, mods);
}

fn knot(app: &PhotocraftApp) -> [[f64; 2]; 3] {
    *app.ui.pen.as_ref().unwrap().knots.last().unwrap()
}

// Reference measurements land on half pixels; PhotoCraft's document coordinates are exact.
fn handles(app: &PhotocraftApp, incoming: [f64; 2], outgoing: [f64; 2]) {
    let k = knot(app);
    for (actual, expected) in k[1].into_iter().zip(incoming).chain(k[2].into_iter().zip(outgoing)) {
        assert!((actual - expected).abs() <= 0.51, "{k:?}: expected in {incoming:?}, out {outgoing:?}");
    }
}

fn broken_drag(later: bool) -> PhotocraftApp {
    let mut app = app();
    if later {
        down(&mut app, 100.0, 300.0);
        up(&mut app, 100.0, 300.0, Modifiers::NONE);
    }
    let x = if later { 300.0 } else { 100.0 };
    down(&mut app, x, 300.0);
    move_to(&mut app, x + 100.0, 300.0, Modifiers::NONE);
    handles(&app, [x - 100.0, 300.0], [x + 100.0, 300.0]);
    move_to(&mut app, x + 100.0, 200.0, Modifiers::ALT);
    app
}

#[test]
fn measured_a_first_anchor_alt_held_freezes_incoming_and_keeps_outgoing_live() {
    let mut app = broken_drag(false);
    handles(&app, [0.5, 300.0], [199.5, 200.0]);
    move_to(&mut app, 210.0, 210.0, Modifiers::ALT);
    handles(&app, [0.0, 300.0], [210.0, 210.0]);
    move_to(&mut app, 200.0, 200.0, Modifiers::ALT);
    up(&mut app, 200.0, 200.0, Modifiers::ALT);
    // Alt released AFTER mouse-up must not relink a completed drag.
    move_to(&mut app, 200.0, 200.0, Modifiers::NONE);
    handles(&app, [0.5, 300.0], [199.5, 200.0]);
    assert_eq!(app.ui.pen.as_ref().unwrap().unlinked, vec![0]);
}

#[test]
fn measured_b_alt_release_immediately_before_mouse_up_without_movement_relinks() {
    let mut app = broken_drag(false);
    up(&mut app, 200.0, 200.0, Modifiers::NONE);
    handles(&app, [0.5, 400.0], [199.5, 200.0]);
    assert!(app.ui.pen.as_ref().unwrap().unlinked.is_empty());
}

#[test]
fn measured_b_stationary_modifier_update_relinks_before_mouse_up() {
    let mut app = broken_drag(false);
    // The native canvas sends the held position on a modifier-only frame.
    move_to(&mut app, 200.0, 200.0, Modifiers::NONE);
    handles(&app, [0.5, 400.0], [199.5, 200.0]);
    up(&mut app, 200.0, 200.0, Modifiers::NONE);
    handles(&app, [0.5, 400.0], [199.5, 200.0]);
}

#[test]
fn measured_c_release_then_movement_keeps_handles_mirrored() {
    let mut app = broken_drag(false);
    move_to(&mut app, 200.0, 200.0, Modifiers::NONE);
    handles(&app, [0.0, 400.0], [200.0, 200.0]);
    move_to(&mut app, 250.0, 150.0, Modifiers::NONE);
    handles(&app, [-49.5, 450.0], [249.5, 150.0]);
    up(&mut app, 250.0, 150.0, Modifiers::NONE);
    handles(&app, [-49.5, 450.0], [249.5, 150.0]);
}

#[test]
fn measured_d_alt_before_first_movement_freezes_at_anchor() {
    let mut app = app();
    down(&mut app, 100.0, 300.0);
    move_to(&mut app, 200.0, 300.0, Modifiers::ALT);
    handles(&app, [100.0, 300.0], [199.5, 300.0]);
    move_to(&mut app, 200.0, 200.0, Modifiers::ALT);
    up(&mut app, 200.0, 200.0, Modifiers::ALT);
    handles(&app, [100.0, 300.0], [199.5, 200.0]);
}

#[test]
fn measured_e_later_anchor_alt_held_keeps_incoming_frozen() {
    let mut app = broken_drag(true);
    handles(&app, [200.5, 300.0], [399.5, 200.0]);
    move_to(&mut app, 410.0, 210.0, Modifiers::ALT);
    handles(&app, [200.0, 300.0], [410.0, 210.0]);
    move_to(&mut app, 400.0, 200.0, Modifiers::ALT);
    up(&mut app, 400.0, 200.0, Modifiers::ALT);
    move_to(&mut app, 400.0, 200.0, Modifiers::NONE);
    handles(&app, [200.5, 300.0], [399.5, 200.0]);
    assert_eq!(app.ui.pen.as_ref().unwrap().unlinked, vec![1]);
}

#[test]
fn measured_f_later_anchor_release_without_movement_relinks() {
    let mut app = broken_drag(true);
    up(&mut app, 400.0, 200.0, Modifiers::NONE);
    handles(&app, [200.5, 400.0], [399.5, 200.0]);
    assert!(app.ui.pen.as_ref().unwrap().unlinked.is_empty());
    assert_eq!(app.ui.pen.as_ref().unwrap().knots[0], [[100.0, 300.0]; 3]);
}

#[test]
fn later_anchor_release_with_movement_relinks() {
    let mut app = broken_drag(true);
    move_to(&mut app, 450.0, 150.0, Modifiers::NONE);
    handles(&app, [150.0, 450.0], [450.0, 150.0]);
    up(&mut app, 450.0, 150.0, Modifiers::NONE);
    handles(&app, [150.0, 450.0], [450.0, 150.0]);
}

#[test]
fn first_anchor_without_alt_mirrors_handles() {
    let mut app = app();
    down(&mut app, 100.0, 300.0);
    move_to(&mut app, 200.0, 200.0, Modifiers::NONE);
    handles(&app, [0.0, 400.0], [200.0, 200.0]);
    up(&mut app, 200.0, 200.0, Modifiers::NONE);
    handles(&app, [0.0, 400.0], [200.0, 200.0]);
}

#[test]
fn close_click_preserves_relinked_geometry_and_undo_redo() {
    let mut app = broken_drag(false);
    up(&mut app, 200.0, 200.0, Modifiers::NONE);
    down(&mut app, 300.0, 300.0);
    up(&mut app, 300.0, 300.0, Modifiers::NONE);
    let n = app.session.active().unwrap().history.past_len();
    down(&mut app, 100.0, 300.0);
    up(&mut app, 100.0, 300.0, Modifiers::NONE);
    assert!(app.ui.pen.is_none());
    let st = app.session.active().unwrap();
    let path = st.doc.work_path.clone().unwrap();
    assert!(path.subpaths[0].closed);
    assert_eq!(path.subpaths[0].knots.len(), 2);
    let k = &path.subpaths[0].knots[0];
    assert_eq!([k.in_ctrl.x, k.in_ctrl.y], [0.0, 400.0]);
    assert_eq!([k.out_ctrl.x, k.out_ctrl.y], [200.0, 200.0]);
    assert_eq!(st.history.past_len(), n + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.work_path.is_none());
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.work_path.as_ref(), Some(&path));
}

#[test]
fn unmeasured_close_drag_alt_release_keeps_upstream_frozen_geometry() {
    let mut app = app();
    down(&mut app, 100.0, 300.0);
    up(&mut app, 100.0, 300.0, Modifiers::NONE);
    down(&mut app, 300.0, 300.0);
    up(&mut app, 300.0, 300.0, Modifiers::NONE);
    down(&mut app, 100.0, 300.0);
    move_to(&mut app, 200.0, 300.0, Modifiers::NONE);
    move_to(&mut app, 200.0, 200.0, Modifiers::ALT);
    move_to(&mut app, 250.0, 150.0, Modifiers::NONE);
    handles(&app, [0.0, 300.0], [250.0, 150.0]);
    up(&mut app, 250.0, 150.0, Modifiers::NONE);
    let path = app.session.active().unwrap().doc.work_path.as_ref().unwrap();
    let k = &path.subpaths[0].knots[0];
    assert_eq!([k.in_ctrl.x, k.in_ctrl.y], [0.0, 300.0]);
    assert_eq!([k.out_ctrl.x, k.out_ctrl.y], [250.0, 150.0]);
}

#[test]
fn unmeasured_second_alt_press_keeps_existing_frozen_behavior() {
    let mut app = broken_drag(false);
    move_to(&mut app, 200.0, 200.0, Modifiers::NONE);
    let frozen = knot(&app)[1];
    move_to(&mut app, 250.0, 150.0, Modifiers::ALT);
    move_to(&mut app, 260.0, 140.0, Modifiers::NONE);
    up(&mut app, 260.0, 140.0, Modifiers::NONE);
    handles(&app, frozen, [260.0, 140.0]);
}

#[test]
fn releasing_alt_during_separate_handle_grab_does_not_relink() {
    for incoming in [false, true] {
        let mut app = app();
        down(&mut app, 100.0, 300.0);
        move_to(&mut app, 200.0, 200.0, Modifiers::NONE);
        up(&mut app, 200.0, 200.0, Modifiers::NONE);
        let x = if incoming { 0.0 } else { 200.0 };
        let y = if incoming { 400.0 } else { 200.0 };
        assert!(super::pen_grab_handle(&mut app, x, y));
        move_to(&mut app, x + 20.0, y + 20.0, Modifiers::ALT);
        up(&mut app, x + 20.0, y + 20.0, Modifiers::NONE);
        if incoming {
            handles(&app, [20.0, 420.0], [200.0, 200.0]);
        } else {
            handles(&app, [0.0, 400.0], [220.0, 220.0]);
        }
        assert_eq!(app.ui.pen.as_ref().unwrap().unlinked, vec![0]);
    }
}
