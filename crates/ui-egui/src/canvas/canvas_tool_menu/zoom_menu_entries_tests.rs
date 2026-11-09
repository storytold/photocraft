//! #12069: the Zoom tool's canvas right-click lists Fit on Screen, 100%, 200%, Print Size, a
//! separator, Zoom In and Zoom Out, as measured in the reference application: no Fill Screen.

use egui::Context;
use serde_json::json;

use super::{Row, choose, open_view, rows};
use crate::PhotocraftApp;
use crate::state::Tool;

fn labels(rows: &[Row]) -> Vec<Option<&'static str>> {
    rows.iter().map(|r| r.map(|(label, _)| label)).collect()
}

/// A 400 × 300 document, optionally with everything selected, and the Zoom tool's menu open.
fn menu_app(selection: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    if selection {
        app.run("select.all", json!({})).unwrap();
    }
    app.sync_views();
    assert!(open_view(&mut app, &Context::default(), Tool::Zoom, [100.0, 100.0], [300.0, 100.0]));
    app
}

#[test]
fn the_zoom_menu_lists_the_fixed_zooms_then_the_steps_without_fill_screen() {
    for selection in [false, true] {
        let app = menu_app(selection);
        let menu = app.ui.canvas_tool_menu.clone().unwrap();
        assert_eq!(
            labels(rows(&menu)),
            vec![Some("Fit on Screen"), Some("100%"), Some("200%"), Some("Print Size"), None, Some("Zoom In"), Some("Zoom Out")],
            "selection {selection}"
        );
    }
}

#[test]
fn fill_screen_cannot_be_run_through_the_zoom_menu_and_the_rest_add_no_history() {
    let mut app = menu_app(false);
    let zoom = app.ui.views[0].zoom;
    choose(&mut app, &Context::default(), "view.fillScreen");
    assert_eq!(app.ui.views[0].zoom, zoom);
    assert!(!app.ui.views[0].fill_pending, "Fill Screen left the menu");
    let mut app = menu_app(false);
    let before = app.session.active().unwrap().history.past_len();
    choose(&mut app, &Context::default(), "view.twoHundredPercent");
    assert_eq!(app.ui.views[0].zoom, 2.0);
    assert_eq!(app.session.active().unwrap().history.past_len(), before, "no history state");
}
