//! Stamp Down (#217) through the real key handling: ⌘⌥E stamps into the layer below.

use egui::{Key, Modifiers};
use serde_json::json;

use crate::PhotocraftApp;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
    app.run("layer.new.layer", json!({"name": "Paint"})).unwrap();
    app.run("edit.fill", json!({"color": "#3366cc"})).unwrap();
    app.sync_views();
    crate::canvas::testing::frame(&mut app);
    app
}

fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.walk().len()
}

#[test]
fn cmd_alt_e_stamps_down() {
    let mut app = app();
    let stamp = Modifiers { alt: true, command: true, ..Default::default() };
    // Stamp Down into the Background: pixels change there, no layer is added.
    crate::canvas::testing::key(&mut app, Key::E, stamp);
    assert_eq!(layers(&app), 2);
    assert!(app.session.journal.iter().any(|(id, _)| id == "layer.stampDown"));
    let st = app.session.active().unwrap();
    let px = st.doc.layers[0].surface().unwrap().rgba(5, 5);
    assert!((px[2] - 0.8).abs() < 0.01, "the copy was merged into the Background: {px:?}");
    // On the Background there is nothing below: a status message, no crash.
    let bg = app.session.active().unwrap().doc.layers[0].id.0;
    app.run("layer.select", json!({"layer": bg})).unwrap();
    crate::canvas::testing::key(&mut app, Key::E, stamp);
    assert!(app.ui.status.contains("below"), "{}", app.ui.status);
}
