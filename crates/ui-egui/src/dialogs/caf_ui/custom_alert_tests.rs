//! #14249: choosing Custom in the Content-Aware Fill workspace's Sampling Area Options shows the
//! reference's info alert, "Use the Sampling Brush Tool to add to the sampling area in order to
//! fill the selected region.", with "Don't show again" and OK, once per choice. OK closes it and the
//! Sampling Brush adds. With "Don't show again" ticked it stays closed. Other choices show nothing.

use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::lifecycle::{CUSTOM_SAMPLING_ALERT, CUSTOM_SAMPLING_PREF};
use super::*;
use crate::control::dialogs::workspace_ui;

/// A 160 × 120 document with a striped layer and a 24 px square selected.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 160, "height": 120, "background": "white"})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    for x in (0..160).step_by(10) {
        app.session.execute("select.rect", json!({"x": x, "y": 0, "width": 5, "height": 120})).unwrap();
        app.session.execute("edit.fill", json!({"color": "#2e8b57"})).unwrap();
    }
    app.session.execute("select.rect", json!({"x": 68, "y": 48, "width": 24, "height": 24})).unwrap();
    app.sync_views();
    app
}

/// One pass of the host with the app's taker, as a frame runs them.
fn pass(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    crate::canvas::testing::frame(app);
    take(app, &ctx);
}

/// Chooses `sampling` in the open workspace, then the passes that report it.
fn choose(app: &mut PhotocraftApp, sampling: &str) {
    let ctx = app.ctx().clone();
    workspace_ui(app, &ctx, CMD, &json!({"sampling": sampling})).unwrap();
    pass(app);
    pass(app);
}

/// The open "Don't show again" alerts.
fn alerts(app: &PhotocraftApp) -> Vec<craft::ui::dialog::DialogId> {
    crate::dialogs::alerts::alerts(app)
}

/// Whether the dialog host, drawn, shows `text` (the prompt keeps its message to itself).
fn shows(app: Box<PhotocraftApp>, text: &str) -> (bool, Box<PhotocraftApp>) {
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(16)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    h.run_steps(2);
    let found = h.query_all_by_label_contains(text).next().is_some();
    (found, h.into_state())
}

fn facts(app: &PhotocraftApp) -> Value {
    describe(app).expect("the workspace is open")
}

/// OK on the open alert.
fn press_ok(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    let id = crate::dialogs::alerts::alerts(app)[0];
    assert!(app.framework.dialogs.confirm_by_id(&ctx, id));
    pass(app);
    pass(app);
}

/// The alert's "Don't show again" box is there (an `alert_once` prompt).
fn shows_dont_show_again(app: &mut PhotocraftApp) -> bool {
    crate::dialogs::alerts::prompt_tagged::<crate::dialogs::alerts::AlertOnce>(app).is_some()
}

#[test]
fn choosing_custom_shows_the_alert_once_and_ok_leaves_the_brush_adding() {
    let mut app = app();
    open(&mut app).unwrap();
    pass(&mut app);
    assert!(alerts(&app).is_empty());
    choose(&mut app, "custom");
    assert_eq!(alerts(&app).len(), 1, "one alert");
    let (found, mut app) = shows(app, CUSTOM_SAMPLING_ALERT);
    assert!(found, "the alert says to use the Sampling Brush");
    assert!(shows_dont_show_again(&mut app));
    pass(&mut app);
    assert_eq!(alerts(&app).len(), 1, "shown once, not again on the next pass");
    press_ok(&mut app);
    assert!(alerts(&app).is_empty(), "OK closes it");
    assert!(app.caf.is_some(), "the workspace stays open");
    let d = facts(&app);
    assert_eq!((d["settings"]["sampling"].as_str(), d["brushMode"].as_str()), (Some("custom"), Some("add")));
}

#[test]
fn each_choice_of_custom_shows_it() {
    let mut app = app();
    open(&mut app).unwrap();
    choose(&mut app, "custom");
    press_ok(&mut app);
    choose(&mut app, "rectangular");
    assert!(alerts(&app).is_empty(), "Rectangular shows nothing");
    choose(&mut app, "custom");
    assert_eq!(alerts(&app).len(), 1, "Custom chosen again: the alert again");
}

#[test]
fn dont_show_again_keeps_it_closed() {
    let mut app = app();
    crate::dialogs::alerts::remember_skip(&mut app, CUSTOM_SAMPLING_PREF);
    open(&mut app).unwrap();
    choose(&mut app, "custom");
    assert!(alerts(&app).is_empty());
    assert_eq!(facts(&app)["brushMode"].as_str(), Some("add"));
}

#[test]
fn other_choices_show_nothing() {
    let mut app = app();
    open(&mut app).unwrap();
    for sampling in ["rectangular", "auto"] {
        choose(&mut app, sampling);
        assert!(alerts(&app).is_empty(), "{sampling}");
    }
}
