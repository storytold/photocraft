//! #11761: a Count marker the tool adds or moves sits on the centre of the pixel under the pointer, also on the
//! pasteboard, and a plain click on a marker re-centres it there as one history state. `count.add` and `count.move`
//! called with coordinates keep them as given.

use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool = Tool::Count;
    app
}

fn press(app: &mut PhotocraftApp, x: f64, y: f64) {
    crate::canvas::tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
}

fn release(app: &mut PhotocraftApp, x: f64, y: f64) {
    crate::canvas::tool_event(app, ToolEvent::Up { x, y }, egui::Modifiers::NONE);
}

fn points(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    app.session.active().unwrap().doc.measurement.count_groups.first().map(|g| g.points.clone()).unwrap_or_default()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn a_click_adds_the_marker_on_the_pixel_centre() {
    let mut app = app();
    for (at, centre) in [([99.3, 49.7], [99.5, 49.5]), ([-164.0, 20.2], [-163.5, 20.5])] {
        press(&mut app, at[0], at[1]);
        release(&mut app, at[0], at[1]);
        assert_eq!(points(&app).last(), Some(&centre), "a click at {at:?}");
    }
}

#[test]
fn a_drag_drops_the_marker_on_the_release_pixel_centre() {
    let mut app = app();
    app.run("count.add", json!({"x": 20.5, "y": 20.5})).unwrap();
    let before = steps(&app);
    press(&mut app, 20.5, 20.5);
    release(&mut app, 60.4, 30.6);
    assert_eq!(points(&app), vec![[60.5, 30.5]]);
    assert_eq!(steps(&app), before + 1, "the drag is one step");
}

#[test]
fn a_click_on_a_marker_re_centres_it_once() {
    let mut app = app();
    // Commands keep the coordinates they are given.
    app.run("count.add", json!({"x": 40.25, "y": 50.75})).unwrap();
    assert_eq!(points(&app), vec![[40.25, 50.75]]);
    let before = steps(&app);
    press(&mut app, 43.2, 50.7);
    release(&mut app, 43.2, 50.7);
    assert_eq!(points(&app), vec![[43.5, 50.5]], "the click moved the marker to the clicked pixel's centre");
    assert_eq!(steps(&app), before + 1, "the re-centring is one step");
    // A click in the pixel the marker already sits on changes nothing.
    press(&mut app, 43.9, 50.1);
    release(&mut app, 43.9, 50.1);
    assert_eq!(points(&app), vec![[43.5, 50.5]]);
    assert_eq!(steps(&app), before + 1, "an unchanged centre is no step");
}
