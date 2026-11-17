use egui::Modifiers;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::testing::{frame, move_to, press, release};
use crate::state::Tool;
use photocraft_engine::gradient_fill_cmds as cmds;

/// A 200×120 document with a Gradient Fill layer drawn from (20, 60) to (180, 60).
fn app_with_fill_layer() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
    app.ui.tool = Tool::Gradient;
    let m = Modifiers::NONE;
    press(&mut app, [20.0, 60.0], m);
    move_to(&mut app, [100.0, 60.0], m);
    move_to(&mut app, [180.0, 60.0], m);
    release(&mut app, [180.0, 60.0], m);
    app
}

fn past(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn get(app: &mut PhotocraftApp) -> Value {
    app.run(cmds::GET, json!({})).unwrap()
}

#[test]
fn a_live_handle_or_stop_drag_is_one_fill_step_and_shows_before_release() {
    let mut app = app_with_fill_layer();
    let m = Modifiers::NONE;
    let before = past(&app);
    press(&mut app, [180.0, 60.0], m);
    move_to(&mut app, [165.0, 80.0], m);
    move_to(&mut app, [150.0, 100.0], m);
    assert!(app.gradient.preview.is_some(), "the picture shows the proposed fill before release");
    assert_eq!(past(&app), before, "Update passes run no command");
    assert!((get(&mut app)["to"][0].as_f64().unwrap() - 180.0).abs() < 1e-3, "the document is unchanged during the drag");
    release(&mut app, [150.0, 100.0], m);
    let g = get(&mut app);
    assert!((g["to"][0].as_f64().unwrap() - 150.0).abs() < 1e-3 && (g["to"][1].as_f64().unwrap() - 100.0).abs() < 1e-3, "{g}");
    assert_eq!(past(&app), before + 1);

    // A click on the line (now from (20, 60) to (150, 100)) adds a stop: a third of the way, since the midpoint
    // diamond sits half-way and takes the press there, as in production (gradient_ui tests).
    let on_line = [20.0 + 130.0 / 3.0, 60.0 + 40.0 / 3.0];
    let before = past(&app);
    press(&mut app, on_line, m);
    release(&mut app, on_line, m);
    assert_eq!(get(&mut app)["stops"].as_array().unwrap().len(), 3);
    assert_eq!(past(&app), before + 1);

    // A drag of that stop along the line is one step. The clock moves past the double-click time first, so the press
    // is not a double click on the new stop (which asks for its colour).
    for _ in 0..60 {
        frame(&mut app);
    }
    let before = past(&app);
    press(&mut app, on_line, m);
    move_to(&mut app, [60.0, 75.0], m);
    assert_eq!(past(&app), before, "Update passes run no command");
    move_to(&mut app, [50.0, 70.0], m);
    release(&mut app, [50.0, 70.0], m);
    assert_eq!(past(&app), before + 1);
    let g = get(&mut app);
    assert_eq!(g["stops"].as_array().unwrap().len(), 3);
    let moved = g["stops"][1][0].as_f64().unwrap();
    assert!(moved < 0.4, "the stop followed the pointer toward the start: {g}");
}
