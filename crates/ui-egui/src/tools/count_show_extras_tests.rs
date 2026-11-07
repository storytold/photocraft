//! #11770: a Count tool click that adds a marker while View › Show › Count is off turns Show ›
//! Count back on, with no history state of its own; a hidden group's eye stays off.

use serde_json::json;

use super::*;

fn app() -> PhotocraftApp {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    s.execute("count.add", json!({"x": 20, "y": 20})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.ui.tool = Tool::Count;
    app
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64) {
    pointer(app, ToolEvent::Down { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
    pointer(app, ToolEvent::Up { x, y }, egui::Modifiers::NONE);
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn total(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.measurement.count_total()
}

#[test]
fn a_new_marker_turns_show_count_back_on() {
    let mut app = app();
    app.ui.view.show.count = false;
    let before = steps(&app);
    click(&mut app, 120.0, 120.0);
    assert_eq!(total(&app), 2);
    assert!(app.ui.view.show.count, "Show › Count is on again");
    assert_eq!(steps(&app), before + 1, "one state, for the marker");
}

#[test]
fn with_show_count_on_the_click_is_as_before() {
    let mut app = app();
    app.ui.view.show.count = true;
    let before = steps(&app);
    click(&mut app, 120.0, 120.0);
    assert_eq!((total(&app), steps(&app)), (2, before + 1));
    assert!(app.ui.view.show.count);
}

#[test]
fn a_hidden_group_stays_hidden() {
    let mut app = app();
    app.run("count.setGroup", json!({"visible": false})).unwrap();
    app.ui.view.show.count = false;
    click(&mut app, 120.0, 120.0);
    assert!(app.ui.view.show.count);
    let doc = &app.session.active().unwrap().doc;
    assert!(!doc.measurement.count_groups[doc.measurement.active_count_group].visible, "the group's eye stays off");
    assert_eq!(total(&app), 2, "the marker went into that group");
}
