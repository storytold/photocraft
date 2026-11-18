//! #13870: drawing a frame shows its width and height beside the pointer, live, as the marquee
//! does, and the readout goes away on release or Escape.
use super::*;
use crate::canvas::{ToolEvent, testing, tool_event, tool_event_response};
use egui::Modifiers;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Frame;
    app
}

/// The readout's W and H values in the response of the pass `ev` ran, if the tool asked for one.
fn readout(app: &mut PhotocraftApp, ev: ToolEvent, m: Modifiers) -> Option<(String, String)> {
    let rows = tool_event_response(app, ev, m).readout?.rows;
    match rows.as_slice() {
        [(_, w), (_, h)] => Some((w.clone(), h.clone())),
        _ => None,
    }
}

fn mv(x: f64, y: f64) -> ToolEvent {
    ToolEvent::Move { x, y, pressure: 1.0 }
}

fn frames(app: &PhotocraftApp) -> Vec<[f64; 6]> {
    let st = app.session.active().unwrap();
    st.doc.walk().into_iter().filter_map(|(_, _, l)| l.frame.as_ref().map(|f| f.transform.m)).collect()
}

#[test]
fn shift_and_alt_read_the_size_the_release_makes() {
    let mut app = app();
    let shift = Modifiers::SHIFT;
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE);
    let shown = readout(&mut app, mv(130.0, 110.0), shift).expect("a readout");
    tool_event(&mut app, ToolEvent::Up { x: 130.0, y: 110.0 }, shift);
    let f = frames(&app)[0];
    assert_eq!(shown, (format!("{} px", f[0]), format!("{} px", f[3])), "Shift: {f:?}");
    assert_eq!(f[0], f[3], "a square");

    let mut app = self::app();
    let alt = Modifiers::ALT;
    tool_event(&mut app, ToolEvent::Down { x: 200.0, y: 200.0, pressure: 1.0 }, Modifiers::NONE);
    assert_eq!(readout(&mut app, mv(240.0, 220.0), alt), Some(("80 px".into(), "40 px".into())), "Alt: twice the pointer's offset");
    tool_event(&mut app, ToolEvent::Up { x: 240.0, y: 220.0 }, alt);
    assert_eq!(frames(&app), vec![[80., 0., 0., 40., 160., 180.]]);
}

#[test]
fn the_readout_goes_when_escape_ends_the_drag() {
    let mut app = app();
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, m);
    assert!(readout(&mut app, mv(130.0, 110.0), m).is_some());
    testing::key(&mut app, egui::Key::Escape, m);
    testing::frame(&mut app);
    assert_eq!(app.canvas_tools.last_response.take().and_then(|r| r.readout), None);
}
