//! Saving and restoring the workspace: what the preferences keep, what a save leaves to the host, and what
//! a workspace switch restores.

use craft::behavior::dock::{Action, Workspace};
use craft::color_ui::ui::ColorPanelMode;
use serde_json::json;

use super::*;
use crate::panels::registry;

fn app_with_layers() -> (Box<PhotocraftApp>, photocraft_doc::LayerId, photocraft_doc::LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let pixel = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    let adj = app.session.active().unwrap().active_layer.unwrap();
    assert_ne!(pixel, adj);
    app.sync_views();
    (app, pixel, adj)
}

/// A new app that starts from `prefs`, with Remember Workspace Changes `remember`.
fn app_from_prefs(prefs: &str, remember: bool) -> Box<PhotocraftApp> {
    let mut session = photocraft_engine::Session::new();
    session.load_prefs_json(prefs).unwrap();
    session.prefs.edit(|p| p.workspace.remember_workspace_changes = remember);
    PhotocraftApp::new(session, crate::Services::default())
}

/// A changed layout and the Color panel's mode are remembered across launches, and only while
/// Remember Workspace Changes is on.
#[test]
fn the_layout_and_color_panel_options_restore_only_with_remember_workspace_changes_on() {
    let (mut app, _, _) = app_with_layers();
    app.session.prefs.edit(|p| p.workspace.remember_workspace_changes = true);
    app.ui.color_panel.panel.mode = ColorPanelMode::HsbSliders;
    crate::color_panel_ui::persist(&mut app);
    crate::panel_docking::apply(&mut app, vec![Action::resize_group("layers".to_owned(), Some(333.0))]).unwrap();
    persist(&mut app);
    let default = registry::default_workspace();
    assert_ne!(app.ui.docking.layout(), default.layout(), "the resize changed the layout");

    let prefs = app.session.prefs_to_json();
    let mut remembered = app_from_prefs(&prefs, true);
    restore(&mut remembered);
    crate::color_panel_ui::restore(&mut remembered);
    assert_eq!(remembered.ui.docking.layout(), app.ui.docking.layout());
    assert_eq!(remembered.ui.color_panel.panel.mode, ColorPanelMode::HsbSliders, "the Color panel's mode too");

    let mut forgotten = app_from_prefs(&prefs, false);
    restore(&mut forgotten);
    assert_eq!(forgotten.ui.docking.layout(), default.layout(), "the default shows when changes are not remembered");
}

/// Reset Workspace brings back the current workspace's own arrangement, and a named workspace keeps
/// the arrangement it was saved with.
#[test]
fn reset_workspace_restores_the_default_layout_and_new_workspaces_keep_theirs() {
    let (mut app, _, _) = app_with_layers();
    let ctx = egui::Context::default();
    let default = registry::default_workspace().layout().clone();
    crate::panel_docking::apply(&mut app, vec![Action::resize_group("properties".to_owned(), Some(120.0)), Action::set_minimized("color".to_owned(), true)])
        .unwrap();
    let mine = app.ui.docking.layout().clone();
    assert_ne!(mine, default);
    crate::menus::invoke(&mut app, &ctx, "workspace.new", json!({"name": "Tall Layers"})).unwrap();
    let named = app.workspaces.set.saved.iter().find(|w| w.name == "Tall Layers").expect("the saved workspace").id.clone();

    crate::menus::invoke(&mut app, &ctx, "window.workspace.essentials", json!({})).unwrap();
    assert_eq!(*app.ui.docking.layout(), default, "the preset replaces the arrangement");
    let essentials = app.workspaces.set.current.clone();

    crate::panel_docking::apply(&mut app, vec![Action::resize_group("layers".to_owned(), Some(300.0))]).unwrap();
    assert_ne!(*app.ui.docking.layout(), default);
    crate::menus::invoke(&mut app, &ctx, "workspace.reset", json!({})).unwrap();
    assert_eq!(*app.ui.docking.layout(), default, "Reset Workspace restores the preset's arrangement");
    assert_eq!(app.workspaces.set.current, essentials, "and stays in that workspace");

    crate::menus::invoke(&mut app, &ctx, &format!("window.workspace.{named}"), json!({})).unwrap();
    assert_eq!(*app.ui.docking.layout(), mine, "the named workspace kept its own arrangement");
    assert_eq!(app.workspaces.set.current, named);
}

/// The load robustness of PhotoCraft's own payload: an unreadable `docking` value is dropped for the
/// default layout, and the rest of the save still applies.
#[test]
fn an_unreadable_docking_layout_is_dropped_and_the_rest_applies() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut saved = snapshot(&app);
    saved["docking"] = json!({"layout": "not-a-dock-layout"});
    saved["panels"]["status_bar"] = json!(false);
    app.run("window.panel.close", json!({"panel": "layers"})).unwrap();
    let default = registry::default_workspace();
    assert_ne!(app.ui.docking.layout(), default.layout(), "the app starts from a changed layout");
    assert!(app.ui.panels.status_bar);

    apply(&mut app, &saved);
    assert_eq!(app.ui.docking.layout(), default.layout(), "the unreadable docking value is dropped for the default");
    assert_eq!(app.ui.docking.hidden(), default.hidden());
    assert!(!app.ui.panels.status_bar, "the rest of the save still applies");
}

/// Whether the toolbar's column is in the end dock.
fn toolbar_at_end(app: &PhotocraftApp) -> bool {
    let layout = app.ui.docking.layout();
    match layout.place(&registry::TOOLBAR.to_owned()) {
        Some(craft::behavior::dock::PanelPlace::Dock { path, .. }) => layout.dock_path(craft::behavior::dock::DockEdge::End) == Some(path),
        _ => false,
    }
}

/// A save from before the toolbar was a panel names its side in `panels.toolbar_side`: a side that resolves to the right
/// puts the toolbar's column in the end dock once; any other side, or one that does not read, leaves the default's
/// start dock. A save that holds the toolbar panel keeps the place it saved, whatever the flags say.
#[test]
fn an_old_saves_toolbar_side_places_the_toolbar_once() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for (side, at_end) in [(json!("right"), true), (json!("left"), false), (json!("start"), false), (json!("sideways"), false), (json!(7), false)] {
        let old = json!({"panels": {"toolbar": true, "options_bar": true, "status_bar": true, "toolbar_side": side.clone()}});
        apply(&mut app, &old);
        assert_eq!(toolbar_at_end(&app), at_end, "{side}");
    }

    // A save from the toolbar panel's version holds its place; its chrome flags no longer mention the toolbar.
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, "window.panel.dock", json!({"panel": "toolbar", "column": true, "side": "end"})).unwrap();
    assert!(toolbar_at_end(&app));
    let mut saved = snapshot(&app);
    saved["panels"].as_object_mut().unwrap().insert("toolbar_side".into(), json!("left"));
    let mut restored = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert!(!toolbar_at_end(&restored));
    apply(&mut restored, &saved);
    assert!(toolbar_at_end(&restored), "the saved place wins over an old flag");
}

/// An old save's `toolbar_double` makes the toolbar two tool columns wide, and `toolbar: false` closes it.
#[test]
fn an_old_saves_toolbar_flags_set_its_width_and_visibility() {
    let steps = |app: &PhotocraftApp| {
        let layout = app.ui.docking.layout();
        layout.place(&registry::TOOLBAR.to_owned()).and_then(|place| layout.column(&place)).and_then(|column| column.steps)
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    apply(&mut app, &json!({"panels": {"toolbar": true, "options_bar": true, "status_bar": true, "toolbar_double": true}}));
    assert_eq!(steps(&app), Some(2));
    apply(&mut app, &json!({"panels": {"toolbar": false, "options_bar": true, "status_bar": true}}));
    assert!(!crate::panel_docking::is_shown(&app, registry::TOOLBAR), "the toolbar is closed");
    assert!(app.ui.docking.hidden().iter().any(|(panel, _)| panel == registry::TOOLBAR), "and keeps its place to reopen at");
}

/// A workspace from a newer version shows the default, and its file stays as saved until the user
/// changes the layout.
#[test]
fn a_workspace_from_a_newer_version_shows_the_default_and_stays_saved() {
    let (mut app, _, _) = app_with_layers();
    app.session.prefs.edit(|p| p.workspace.remember_workspace_changes = true);
    let mut saved = snapshot(&app);
    saved["docking"] = json!({"version": 99, "layout": {"root": null, "floating": []}, "future": true});
    app.session.prefs.edit(|p| p.panel_layout = saved.clone());
    restore(&mut app);
    assert_eq!(app.ui.docking.layout(), registry::default_workspace().layout(), "the default layout shows");
    persist(&mut app);
    assert_eq!(app.session.prefs().panel_layout, saved, "a persist with no change leaves the newer file saved");

    // `close` saves through `apply`'s own persist.
    app.run("window.panel.close", json!({"panel": "layers"})).unwrap();
    let docking = &app.session.prefs().panel_layout["docking"];
    assert_eq!(docking["version"], json!(Workspace::<String>::VERSION), "a changed layout replaces it: {docking}");
}
