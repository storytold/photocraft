//! #13867: holding Space while a frame is drawn moves the frame being drawn, as the marquee does;
//! after Space is released the drag sizes again from the moved start.
use super::*;
use crate::canvas::{ToolEvent, testing, tool_event};
use craft::keymap::Key;
use craft::ui::runtime::windows::{Released, WindowHost};
use egui::Modifiers;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":600,"height":400})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Frame;
    app
}

fn entries(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries()
}

fn frames(app: &PhotocraftApp) -> Vec<[f64; 6]> {
    let st = app.session.active().unwrap();
    st.doc.walk().into_iter().filter_map(|(_, _, l)| l.frame.as_ref().map(|f| f.transform.m)).collect()
}

fn mv(app: &mut PhotocraftApp, x: f64, y: f64, m: Modifiers) {
    tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
}

/// Space goes down (held) or comes up, through the window as the real key does.
fn space(app: &mut PhotocraftApp, down: bool) {
    let host = WindowHost::of(app.ctx());
    if down {
        host.inject_key(egui::ViewportId::ROOT, Key::Space, craft::keymap::Mods::default(), true);
    } else {
        host.inject_release(egui::ViewportId::ROOT, None, Released::Key(Key::Space));
    }
    testing::frame(app);
}

/// The measured sequence: press at (50,50), drag to (122,120), hold Space and move +50,+30,
/// release Space, release the button at (202,180). Space repositions; it is not the Hand tool.
#[test]
fn the_measured_sequence_makes_100_202_by_80_180_in_one_step() {
    let mut app = app();
    let before = entries(&app);
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 50., y: 50., pressure: 1.0 }, m);
    mv(&mut app, 122., 120., m);
    let center = testing::view(&app).unwrap().center;
    space(&mut app, true);
    mv(&mut app, 147., 135., m);
    mv(&mut app, 172., 150., m);
    space(&mut app, false);
    mv(&mut app, 202., 180., m);
    tool_event(&mut app, ToolEvent::Up { x: 202., y: 180. }, m);
    assert_eq!(testing::view(&app).unwrap().center, center, "Space didn't pan");
    assert_eq!(app.ui.tool, Tool::Frame);
    assert_eq!(frames(&app), vec![[102., 0., 0., 100., 100., 80.]], "100..202 x 80..180");
    let after = entries(&app);
    assert_eq!(after.len(), before.len() + 1);
    assert_eq!(after.last().map(String::as_str), Some("New Frame"));
}

#[test]
fn released_with_space_down_the_frame_keeps_its_size() {
    let mut app = app();
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 50., y: 50., pressure: 1.0 }, m);
    mv(&mut app, 150., 110., m);
    space(&mut app, true);
    mv(&mut app, 200., 150., m);
    tool_event(&mut app, ToolEvent::Up { x: 230., y: 170. }, m);
    space(&mut app, false);
    assert_eq!(frames(&app), vec![[100., 0., 0., 60., 130., 110.]], "the 100 x 60 frame moved by (80, 60)");
}

#[test]
fn space_moves_a_square_and_a_centred_frame_too() {
    // Shift: a square from the moved start.
    let mut app = app();
    let shift = Modifiers::SHIFT;
    tool_event(&mut app, ToolEvent::Down { x: 50., y: 50., pressure: 1.0 }, shift);
    mv(&mut app, 130., 110., shift);
    space(&mut app, true);
    mv(&mut app, 150., 120., shift);
    space(&mut app, false);
    tool_event(&mut app, ToolEvent::Up { x: 150., y: 120. }, shift);
    let moved = frames(&app);
    // The same as a Shift drag from the moved start (70,60) to the release (150,120).
    let mut plain = self::app();
    tool_event(&mut plain, ToolEvent::Down { x: 70., y: 60., pressure: 1.0 }, shift);
    mv(&mut plain, 150., 120., shift);
    tool_event(&mut plain, ToolEvent::Up { x: 150., y: 120. }, shift);
    assert_eq!(moved, frames(&plain));
    assert_eq!(moved, vec![[60., 0., 0., 60., 70., 60.]], "a square from (70,60)");
    // Alt: centred on the moved centre.
    let mut app = self::app();
    let alt = Modifiers::ALT;
    tool_event(&mut app, ToolEvent::Down { x: 200., y: 200., pressure: 1.0 }, alt);
    mv(&mut app, 240., 220., alt);
    space(&mut app, true);
    mv(&mut app, 260., 230., alt);
    space(&mut app, false);
    tool_event(&mut app, ToolEvent::Up { x: 260., y: 230. }, alt);
    assert_eq!(frames(&app), vec![[80., 0., 0., 40., 180., 190.]], "80 x 40 around (220,210)");
}

#[test]
fn without_space_the_drag_sizes() {
    let mut app = app();
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 50., y: 50., pressure: 1.0 }, m);
    mv(&mut app, 122., 120., m);
    mv(&mut app, 202., 180., m);
    tool_event(&mut app, ToolEvent::Up { x: 202., y: 180. }, m);
    assert_eq!(frames(&app), vec![[152., 0., 0., 130., 50., 50.]]);
}
