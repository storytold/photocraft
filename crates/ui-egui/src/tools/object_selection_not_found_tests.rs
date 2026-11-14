//! #12861: an Object Selection rectangle that finds nothing opens the reference application's
//! alert with OK and "Don't show again" and selects nothing. Over an area where the active layer
//! has no pixels it still adds an "Object Selection" History state; inside a uniform colour it adds
//! none. A click that finds nothing shows no message and adds no state.

use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::cam_no_pixels_alert_tests::{drag, selected, steps};
use crate::state::{DialogKind, Tool};

const NOT_FOUND: &str = "Couldn't find an object to select. There might not be enough edge contrast, or the object might be too small.";
/// A drag inside the uniform red rectangle.
const IN_RED: &[[f64; 2]] = &[[150.0, 130.0], [250.0, 220.0]];
/// A drag over the white Background where Shapes has no pixels.
const OVER_WHITE: &[[f64; 2]] = &[[650.0, 20.0], [780.0, 80.0]];

/// The issue's document: 800 x 600 white Background and the active pixel layer Shapes with a red
/// rectangle 100,100-300,250, a blue disc (centre 500,175, radius 75) and a grey ramp at y
/// 350-450, transparent elsewhere. Object Selection with Object Finder on, Sample All Layers off.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Shapes"})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            let s = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            s.fill_rect(photocraft_geom::Rect::new(100, 100, 300, 250), &[220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0]);
            for y in 100..250 {
                for x in 425..575 {
                    let (dx, dy) = (x as f32 + 0.5 - 500.0, y as f32 + 0.5 - 175.0);
                    if dx * dx + dy * dy <= 75.0 * 75.0 {
                        s.fill_rect(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &[30.0 / 255.0, 30.0 / 255.0, 220.0 / 255.0, 1.0]);
                    }
                }
            }
            for x in 0..800 {
                let v = x as f32 / 799.0;
                s.fill_rect(photocraft_geom::Rect::new(x, 350, x + 1, 450), &[v, v, v, 1.0]);
            }
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::ObjectSelection;
    app.ui.tool_options.object_finder = true;
    app.ui.tool_options.sample_all.object_selection = false;
    app
}

/// The open "Couldn't find an object" alerts.
fn alerts(app: &PhotocraftApp) -> Vec<u64> {
    app.ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error && d.fields.get("message").and_then(Value::as_str) == Some(NOT_FOUND)).map(|d| d.id).collect()
}

/// The name of the latest History state.
fn last_step(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.undo_label().map(str::to_owned)
}

#[test]
fn a_drag_inside_the_red_alerts_and_adds_no_state() {
    let mut app = app();
    let before = steps(&app);
    drag(&mut app, IN_RED);
    assert_eq!(alerts(&app).len(), 1, "the alert");
    assert!(!selected(&app), "nothing selected");
    assert_eq!(steps(&app), before, "no History state");
    assert!(app.ui.status.is_empty(), "no status-bar message: {}", app.ui.status);
}

#[test]
fn a_drag_over_empty_white_alerts_and_adds_one_object_selection_state() {
    let mut app = app();
    let before = steps(&app);
    drag(&mut app, OVER_WHITE);
    assert_eq!(alerts(&app).len(), 1, "the alert");
    assert!(!selected(&app), "nothing selected");
    assert_eq!(steps(&app), before + 1, "one History state");
    assert_eq!(last_step(&app).as_deref(), Some("Object Selection"));
    assert!(app.ui.status.is_empty() && !app.ui.status_error, "no engine error in the status bar: {}", app.ui.status);
}

#[test]
fn a_drag_over_a_transparent_gap_inside_the_layers_bounds_adds_one_state_too() {
    // Between the red rectangle, the disc and the ramp: inside Shapes' content bounds, no pixels.
    let mut app = app();
    let before = steps(&app);
    drag(&mut app, &[[320.0, 270.0], [400.0, 330.0]]);
    assert_eq!(alerts(&app).len(), 1, "the alert");
    assert!(!selected(&app), "nothing selected");
    assert_eq!(steps(&app), before + 1, "one History state");
    assert_eq!(last_step(&app).as_deref(), Some("Object Selection"));
}

#[test]
fn the_alert_shows_ok_and_dont_show_again_and_once_ticked_stops() {
    let _gpu = photocraft_testkit::gpu_lock();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(800.0, 600.0))
        .build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app());
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    drag(h.state_mut(), IN_RED);
    h.run_steps(3);
    assert!(h.query_by_label(NOT_FOUND).is_some(), "the alert's text");
    h.get_by_label("Don't show again").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(alerts(h.state()).is_empty(), "OK closed it");
    assert_eq!(h.state().session.prefs().dialogs.get("objectSelection.notFound"), Some(&json!({"skip": true})));
    for (name, points, added) in [("inside the red", IN_RED, 0), ("over white", OVER_WHITE, 1)] {
        let before = steps(h.state());
        drag(h.state_mut(), points);
        h.run_steps(3);
        assert!(alerts(h.state()).is_empty() && h.query_by_label(NOT_FOUND).is_none(), "{name}: no alert any more");
        assert!(!selected(h.state()), "{name}: nothing selected");
        assert_eq!(steps(h.state()), before + added, "{name}: History states");
    }
}

#[test]
fn an_unticked_alert_comes_back_on_the_next_drag() {
    let mut app = app();
    drag(&mut app, OVER_WHITE);
    let id = alerts(&app)[0];
    crate::dialogs::confirm(&mut app, id).unwrap();
    drag(&mut app, IN_RED);
    assert_eq!(alerts(&app).len(), 1);
}

#[test]
fn a_click_that_finds_nothing_is_silent() {
    // Empty white (no Shapes pixels there) and transparency between the shapes.
    for at in [[700.0, 50.0], [380.0, 300.0]] {
        let mut app = app();
        let before = steps(&app);
        drag(&mut app, &[at]);
        assert!(app.ui.status.is_empty(), "{at:?}: no message: {}", app.ui.status);
        assert!(alerts(&app).is_empty() && app.ui.dialogs.is_empty(), "{at:?}: no alert");
        assert!(!selected(&app), "{at:?}: nothing selected");
        assert_eq!(steps(&app), before, "{at:?}: no History state");
    }
    // Sample All Layers on: the click reaches the engine and finds nothing on the white.
    let mut app = app();
    app.ui.tool_options.sample_all.object_selection = true;
    let before = steps(&app);
    drag(&mut app, &[[700.0, 50.0]]);
    assert!(app.ui.status.is_empty() && app.ui.dialogs.is_empty(), "no message: {}", app.ui.status);
    assert_eq!(steps(&app), before);
    // The control channel still answers that nothing changed.
    let r = app.run("select.object", json!({"point": [700, 50], "sampleAllLayers": true})).unwrap();
    assert_eq!(r["changed"], json!(false), "{r}");
}

#[test]
fn scripts_keep_the_no_pixels_error() {
    // #5246: without the tool's `recordEmptyArea` a script still hears why nothing was found.
    let mut app = app();
    let before = steps(&app);
    let e = app.run("select.object", json!({"rect": [650, 20, 130, 60]})).unwrap_err();
    assert!(e.contains("no pixels in that area"), "{e}");
    assert_eq!(steps(&app), before);
}

#[test]
fn a_non_boolean_record_empty_area_is_refused_without_a_state() {
    let mut app = app();
    let before = steps(&app);
    for v in [json!("yes"), json!(1), json!(null)] {
        let e = app.run("select.object", json!({"rect": [650, 20, 130, 60], "recordEmptyArea": v})).unwrap_err();
        assert!(e.contains("`recordEmptyArea` must be true or false"), "{v}: {e}");
        assert_eq!(steps(&app), before, "{v}: no History state");
        // A point request is refused the same way, before it looks for an object.
        let e = app.run("select.object", json!({"point": [700, 50], "sampleAllLayers": true, "recordEmptyArea": v})).unwrap_err();
        assert!(e.contains("`recordEmptyArea` must be true or false"), "{v}, point: {e}");
        assert_eq!(steps(&app), before, "{v}, point: no History state");
    }
}
