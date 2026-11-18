//! #11767: new count groups take the colour, Marker Size and Label Size of the last group edit,
//! remembered in the preferences (other documents, restarts); Reset Tool and Reset All Tools
//! leave them alone; fresh preferences start at #97FBFD, 2, 8; an explicit `color` still wins.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool = Tool::Count;
    app
}

/// (rgb, markerSize, labelSize) of count group `i` as `count.tool` lists it.
fn group(app: &mut PhotocraftApp, i: usize) -> (Value, Value, Value) {
    let g = app.session.execute("image.analysis.countTool", json!({})).unwrap()["groups"][i].clone();
    (g["color"].clone(), g["markerSize"].clone(), g["labelSize"].clone())
}

fn rgb(c: &Value) -> [i64; 3] {
    let a = c.as_array().unwrap();
    [0, 1, 2].map(|i| (a[i].as_f64().unwrap() * 255.0).round() as i64)
}

#[test]
fn fresh_preferences_start_at_97fbfd_2_8() {
    let mut app = app();
    app.run("count.add", json!({"x": 10, "y": 10})).unwrap();
    let (c, m, l) = group(&mut app, 0);
    assert_eq!(rgb(&c), [0x97, 0xFB, 0xFD]);
    assert_eq!((m, l), (json!(2), json!(8)));
}

#[test]
fn the_last_edit_is_the_next_groups_values_in_any_document() {
    let mut app = app();
    app.run("count.add", json!({"x": 10, "y": 10})).unwrap();
    app.run("count.setGroup", json!({"group": 0, "color": [1.0, 0.0, 0.0]})).unwrap();
    app.run("count.setGroup", json!({"group": 0, "markerSize": 10})).unwrap();
    app.run("count.setGroup", json!({"group": 0, "labelSize": 12})).unwrap();
    // The next group in the same document…
    app.run("count.newGroup", json!({})).unwrap();
    let (c, m, l) = group(&mut app, 1);
    assert_eq!((rgb(&c), m, l), ([255, 0, 0], json!(10), json!(12)));
    // …and the first group of another document.
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("count.add", json!({"x": 10, "y": 10})).unwrap();
    let (c, m, l) = group(&mut app, 0);
    assert_eq!((rgb(&c), m, l), ([255, 0, 0], json!(10), json!(12)));
}

#[test]
fn the_values_survive_a_restart() {
    let mut app = app();
    app.run("count.add", json!({"x": 10, "y": 10})).unwrap();
    app.run("count.setGroup", json!({"group": 0, "color": [0.0, 0.0, 1.0], "markerSize": 5})).unwrap();
    let saved: photocraft_engine::prefs::Preferences = serde_json::from_str(&app.session.prefs_to_json()).unwrap();
    assert_eq!(saved.count_group.color, [0, 0, 255]);
    assert_eq!(saved.count_group.marker_size, 5);
    assert_eq!(saved.count_group.label_size, 8);
}

#[test]
fn reset_tool_and_reset_all_tools_leave_them_alone() {
    let mut app = app();
    app.run("count.add", json!({"x": 10, "y": 10})).unwrap();
    app.run("count.setGroup", json!({"group": 0, "color": [1.0, 0.0, 0.0], "markerSize": 10, "labelSize": 12})).unwrap();
    let before = app.session.prefs().count_group;
    crate::tool_reset::invoke(&mut app, crate::tool_reset::RESET, &json!({})).unwrap().unwrap();
    crate::tool_reset::invoke(&mut app, crate::tool_reset::RESET_ALL, &json!({"confirmed": true})).unwrap().unwrap();
    assert_eq!(app.session.prefs().count_group, before);
}

#[test]
fn an_explicit_color_still_wins_and_a_rename_remembers_nothing() {
    let mut app = app();
    app.run("count.add", json!({"x": 10, "y": 10})).unwrap();
    app.run("count.newGroup", json!({"color": [0.0, 1.0, 0.0]})).unwrap();
    let (c, m, _) = group(&mut app, 1);
    assert_eq!((rgb(&c), m), ([0, 255, 0], json!(2)));
    let before = app.session.prefs().count_group;
    app.run("count.setGroup", json!({"group": 1, "name": "Cells"})).unwrap();
    assert_eq!(app.session.prefs().count_group, before, "only colour and sizes are remembered");
}
