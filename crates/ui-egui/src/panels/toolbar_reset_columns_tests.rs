//! #13045: Reset Workspace puts the toolbar back to one column; picking a workspace keeps it.

use serde_json::json;

use crate::PhotocraftApp;

fn two_column_app() -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.ui.panels.toolbar_double = true;
    (app, egui::Context::default())
}

#[test]
fn reset_workspace_sets_one_column() {
    let (mut app, ctx) = two_column_app();
    crate::menus::invoke(&mut app, &ctx, "window.workspace.resetWorkspace", json!({})).unwrap();
    assert!(!app.ui.panels.toolbar_double);
    let (mut app, ctx) = two_column_app();
    crate::menus::invoke(&mut app, &ctx, "window.workspace.photography", json!({})).unwrap();
    app.ui.panels.toolbar_double = true;
    crate::menus::invoke(&mut app, &ctx, "window.workspace.resetWorkspace", json!({})).unwrap();
    assert!(!app.ui.panels.toolbar_double, "Reset Photography");
}

#[test]
fn choosing_a_workspace_keeps_two_columns() {
    let (mut app, ctx) = two_column_app();
    crate::menus::invoke(&mut app, &ctx, "window.workspace.photography", json!({})).unwrap();
    assert!(app.ui.panels.toolbar_double);
    crate::menus::invoke(&mut app, &ctx, "window.workspace.essentials", json!({})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "window.workspace.essentials", json!({})).unwrap();
    assert!(app.ui.panels.toolbar_double);
}
