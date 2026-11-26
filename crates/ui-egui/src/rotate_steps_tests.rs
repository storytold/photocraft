//! #4911: Rotate View Left and Right turn the active view 15 degrees on screen, wrap at 180, and leave the
//! document, its history, the zoom, the centre and every other tab alone; the default keys fire them.

use egui::{Key, Modifiers};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::shortcut_dispatch::fire_bridged;

/// An app with `count` new documents, the last one active.
fn app_docs(count: usize) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for _ in 0..count {
        app.run("file.new", json!({"width": 100, "height": 50})).unwrap();
    }
    app.sync_views();
    testing::frame(&mut app);
    app
}

/// The rotation the active canvas shows, in degrees.
fn rotation(app: &PhotocraftApp) -> f64 {
    testing::view(app).expect("the canvas shows the document").rotation
}

/// Fires the menu row `id` and runs the frames in which the canvas takes it.
fn fire_row(app: &mut PhotocraftApp, id: &str) {
    assert_eq!(fire_bridged(&testing::ctx(app), id), Some(true), "{id}: the runtime takes it");
    testing::frame(app);
    testing::frame(app);
}

#[test]
fn rotation_steps_are_relative_wrap_and_change_only_the_view() {
    let mut app = app_docs(2);
    app.ui.views[1].rotation = 7.0;
    testing::frame(&mut app);
    let doc = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    let (zoom, center) = (testing::view(&app).unwrap().zoom, testing::view(&app).unwrap().center);
    for _ in 0..3 {
        fire_row(&mut app, "view.rotateRight");
    }
    assert_eq!(rotation(&app), 52.0);
    fire_row(&mut app, "view.rotateLeft");
    assert_eq!(rotation(&app), 37.0);
    app.ui.views[1].rotation = 175.0;
    testing::frame(&mut app);
    fire_row(&mut app, "view.rotateRight");
    assert_eq!(rotation(&app), -170.0);
    fire_row(&mut app, "view.rotateLeft");
    assert_eq!(rotation(&app), 175.0);
    fire_row(&mut app, "view.resetView");
    assert_eq!(rotation(&app), 0.0);
    assert_eq!(app.ui.views[0].rotation, 0.0, "the other tab keeps its view");
    let view = testing::view(&app).unwrap();
    assert_eq!((view.zoom, view.center), (zoom, center));
    assert!(std::sync::Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

#[test]
fn rotation_steps_need_a_document() {
    let empty = app_docs(0);
    let app = app_docs(1);
    for id in ["view.rotateLeft", "view.rotateRight"] {
        assert!(!crate::menus::is_enabled(&empty, id), "{id}: no document");
        assert!(crate::menus::is_enabled(&app, id), "{id}: a document is open");
    }
}

#[test]
fn the_default_keys_rotate_the_view() {
    let mut app = app_docs(1);
    for _ in 0..3 {
        testing::key(&mut app, Key::Period, Modifiers::ALT | Modifiers::SHIFT);
    }
    assert_eq!(rotation(&app), 45.0, "Alt+Shift+. turns clockwise");
    testing::key(&mut app, Key::Comma, Modifiers::ALT | Modifiers::SHIFT);
    assert_eq!(rotation(&app), 30.0, "Alt+Shift+, turns back");
    assert_eq!(app.session.active().unwrap().history.past_len(), 0);
}
