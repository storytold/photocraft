//! Shift while drawing a frame squares from the drag's shorter side (#13865).
use super::*;
use crate::canvas::{ToolEvent, tool_event, tool_event_response};

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":600,"height":400})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Frame;
    app
}

const SHIFT: egui::Modifiers = egui::Modifiers { shift: true, ..egui::Modifiers::NONE };
const ALT: egui::Modifiers = egui::Modifiers { alt: true, ..egui::Modifiers::NONE };
const SHIFT_ALT: egui::Modifiers = egui::Modifiers { shift: true, alt: true, ..egui::Modifiers::NONE };

/// Press at `from` with `down`, move to `to` with `held`, and return the size the drag reads out beside the pointer
/// (`W`, `H`) before releasing there and returning the new frame's transform.
fn draw(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], down: egui::Modifiers, held: egui::Modifiers) -> (Vec<String>, [f64; 6]) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, down);
    let r = tool_event_response(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, held);
    let live = r.readout.expect("a frame is being drawn").rows.into_iter().map(|(_, v)| v).collect();
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, held);
    (live, selected(app).unwrap().frame.as_ref().unwrap().transform.m)
}

/// The readout of a `w` x `h` box.
fn size(w: u32, h: u32) -> Vec<String> {
    vec![format!("{w} px"), format!("{h} px")]
}

#[test]
fn shift_squares_a_rectangle_from_the_shorter_side_at_the_start_corner() {
    let mut app = app();
    let (live, m) = draw(&mut app, [400., 50.], [500., 110.], SHIFT, SHIFT);
    assert_eq!(m, [60., 0., 0., 60., 400., 50.], "400..460 x 50..110");
    assert_eq!(live, size(60, 60), "the live drag shows the committed square");
    assert_eq!(app.session.active().unwrap().history.entries().last().map(String::as_str), Some("New Frame"));
}

#[test]
fn shift_squares_a_polygon_from_the_shorter_side() {
    let mut app = app();
    app.ui.frame_tool.shape = "polygon".into();
    let (live, m) = draw(&mut app, [100., 100.], [260., 180.], SHIFT, SHIFT);
    assert_eq!(m, [80., 0., 0., 80., 100., 100.], "a 160 x 80 drag gives 80 x 80");
    assert_eq!(live, size(80, 80));
}

#[test]
fn shift_alt_squares_the_shorter_half_extent_around_the_start() {
    let mut app = app();
    let (live, m) = draw(&mut app, [150., 300.], [210., 330.], SHIFT_ALT, SHIFT_ALT);
    assert_eq!(m, [60., 0., 0., 60., 120., 270.], "120..180 x 270..330");
    assert_eq!(live, size(60, 60));
}

#[test]
fn alt_alone_draws_from_the_centre_without_squaring() {
    let mut app = app();
    let (live, m) = draw(&mut app, [450., 300.], [500., 330.], egui::Modifiers::NONE, ALT);
    assert_eq!(m, [100., 0., 0., 60., 400., 270.], "400..500 x 270..330");
    assert_eq!(live, size(100, 60));
}
