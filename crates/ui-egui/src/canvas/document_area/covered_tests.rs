//! A full-window workspace covers the main window's canvas: its document is not drawn underneath.

use super::MAIN_PASSES;
use crate::PhotocraftApp;
use serde_json::json;

fn passes(app: &mut PhotocraftApp) -> usize {
    let before = MAIN_PASSES.with(std::cell::Cell::get);
    crate::canvas::testing::frame(app);
    MAIN_PASSES.with(std::cell::Cell::get) - before
}

/// The main canvas is drawn without a workspace, not while Liquify, Select and Mask or Filter Gallery is open, and
/// again once it closes.
#[test]
fn a_full_window_workspace_skips_the_main_canvas() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#e05030"})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.sync_views();
    assert!(passes(&mut app) > 0, "the canvas is drawn");
    let ctx = app.ctx().clone();
    for key in ["filter.liquify", "select.selectAndMask", "filter.filterGallery"] {
        crate::menus::invoke(&mut app, &ctx, key, json!({})).unwrap();
        crate::canvas::testing::frame(&mut app);
        assert_eq!(crate::distort_ui::modal_workspace(&app), Some(key), "{key} is open");
        assert_eq!(passes(&mut app), 0, "nothing drawn under {key}");
        crate::control::dialogs::workspace_ui(&mut app, &ctx, key, &json!({"cancel": true})).unwrap();
        crate::canvas::testing::frame(&mut app);
        assert_eq!(crate::distort_ui::modal_workspace(&app), None, "{key} closed");
        assert!(passes(&mut app) > 0, "the canvas is drawn again after {key}");
    }
}
