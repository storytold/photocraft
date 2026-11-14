//! Object Selection's Refresh button and gear menu (#12857): Object Finder, Object Subtract and
//! Object Finder Mode are options that Reset Tool restores; Manual Refresh keeps the analysed
//! pixels until Refresh is clicked, Auto Refresh re-analyses after every edit.

use serde_json::json;

use crate::PhotocraftApp;
use crate::object_finder::{Update, refresh, update};
use crate::state::{ObjectFinderMode, Tool, ToolOptions};

const DISC: [f64; 2] = [100.0, 80.0];

/// A 200 x 160 document: a red disc (centre 100,80, radius 45) on blue, nothing selected.
fn app(mode: ObjectFinderMode) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 160})).unwrap();
    app.sync_views();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 160})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#3366cc"})).unwrap();
    app.run("select.rect", json!({"x": 55, "y": 35, "width": 90, "height": 90, "ellipse": true})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#e63319"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool = Tool::ObjectSelection;
    app.ui.tool_options.object_finder = true;
    app.ui.tool_options.object_finder_mode = mode;
    app
}

/// Hovers at `at` until the lookup settles (time advances past the debounce).
fn hover(app: &mut PhotocraftApp, at: [f64; 2]) -> Update {
    let mut t = 100.0;
    loop {
        let u = update(app, Some(at), false, t);
        if !u.busy || t > 130.0 {
            return u;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        t += 0.05;
    }
}

/// A pixel edit away from the disc.
fn edit_pixels(app: &mut PhotocraftApp) {
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#000000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
}

#[test]
fn defaults_and_old_settings() {
    let d = ToolOptions::default();
    assert!(d.object_finder && d.object_subtract);
    assert_eq!(d.object_finder_mode, ObjectFinderMode::Auto);
    let o: ToolOptions = serde_json::from_value(json!({})).unwrap();
    assert!(o.object_finder && o.object_subtract);
    assert_eq!(o.object_finder_mode, ObjectFinderMode::Auto);
    let mut v = serde_json::to_value(&d).unwrap();
    let m = v.as_object_mut().unwrap();
    m.remove("object_subtract");
    m.remove("object_finder_mode");
    let o: ToolOptions = serde_json::from_value(v).unwrap();
    assert_eq!(o, d);
}

#[test]
fn options_round_trip() {
    let o = ToolOptions { object_subtract: false, object_finder_mode: ObjectFinderMode::Manual, ..ToolOptions::default() };
    let back: ToolOptions = serde_json::from_value(serde_json::to_value(&o).unwrap()).unwrap();
    assert!(!back.object_subtract);
    assert_eq!(back.object_finder_mode, ObjectFinderMode::Manual);
}

#[test]
fn reset_tool_restores_them() {
    let mut o = ToolOptions { object_finder: false, object_subtract: false, object_finder_mode: ObjectFinderMode::Manual, ..ToolOptions::default() };
    crate::tools::tool_reset::reset_options(&mut o, Tool::Lasso);
    assert!(!o.object_finder && !o.object_subtract && o.object_finder_mode == ObjectFinderMode::Manual);
    crate::tools::tool_reset::reset_options(&mut o, Tool::ObjectSelection);
    assert_eq!(o, ToolOptions::default());
}

/// Hovers long past the debounce and checks nothing was shown or started.
fn assert_idle(app: &mut PhotocraftApp) {
    let u = hover(app, DISC);
    assert!(u.shown.is_none() && !u.busy);
    let u = update(app, Some(DISC), false, 500.0);
    assert!(u.shown.is_none() && !u.busy);
}

#[test]
fn manual_refresh_analyses_nothing_before_refresh() {
    let mut app = app(ObjectFinderMode::Manual);
    assert_idle(&mut app);
    refresh(&mut app);
    assert!(hover(&mut app, DISC).shown.is_some());
}

#[test]
fn manual_refresh_needs_refresh_on_another_document() {
    let mut app = app(ObjectFinderMode::Manual);
    refresh(&mut app);
    assert!(hover(&mut app, DISC).shown.is_some());
    app.run("file.new", json!({"width": 200, "height": 160})).unwrap();
    app.sync_views();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 55, "y": 35, "width": 90, "height": 90, "ellipse": true})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#e63319"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    assert_idle(&mut app);
    refresh(&mut app);
    assert!(hover(&mut app, DISC).shown.is_some());
}

#[test]
fn manual_refresh_waits_for_refresh_after_a_pixel_edit() {
    let mut app = app(ObjectFinderMode::Manual);
    refresh(&mut app);
    assert!(hover(&mut app, DISC).shown.is_some());
    edit_pixels(&mut app);
    assert_idle(&mut app);
    refresh(&mut app);
    assert!(hover(&mut app, DISC).shown.is_some());
}

#[test]
fn manual_refresh_survives_selection_changes_and_leaving() {
    let mut app = app(ObjectFinderMode::Manual);
    refresh(&mut app);
    assert!(hover(&mut app, DISC).shown.is_some());
    assert!(update(&mut app, None, false, 200.0).shown.is_none());
    app.run("select.all", json!({})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    assert!(hover(&mut app, DISC).shown.is_some());
}

#[test]
fn auto_refresh_reanalyses_after_a_pixel_edit() {
    let mut app = app(ObjectFinderMode::Auto);
    assert!(hover(&mut app, DISC).shown.is_some());
    edit_pixels(&mut app);
    assert!(hover(&mut app, DISC).shown.is_some());
}
