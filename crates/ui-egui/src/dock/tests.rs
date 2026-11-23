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
    assert_ne!(app.ui.docking.layout, default.layout, "the resize changed the layout");

    let prefs = app.session.prefs_to_json();
    let mut remembered = app_from_prefs(&prefs, true);
    restore(&mut remembered);
    crate::color_panel_ui::restore(&mut remembered);
    assert_eq!(remembered.ui.docking.layout, app.ui.docking.layout);
    assert_eq!(remembered.ui.color_panel.panel.mode, ColorPanelMode::HsbSliders, "the Color panel's mode too");

    let mut forgotten = app_from_prefs(&prefs, false);
    restore(&mut forgotten);
    assert_eq!(forgotten.ui.docking.layout, default.layout, "the default shows when changes are not remembered");
}

/// Reset Workspace brings back the current workspace's own arrangement, and a named workspace keeps
/// the arrangement it was saved with.
#[test]
fn reset_workspace_restores_the_default_layout_and_new_workspaces_keep_theirs() {
    let (mut app, _, _) = app_with_layers();
    let ctx = egui::Context::default();
    let default = registry::default_workspace().layout;
    crate::panel_docking::apply(&mut app, vec![Action::resize_group("properties".to_owned(), Some(120.0)), Action::set_minimized("color".to_owned(), true)])
        .unwrap();
    let mine = app.ui.docking.layout.clone();
    assert_ne!(mine, default);
    crate::menus::invoke(&mut app, &ctx, "workspace.new", json!({"name": "Tall Layers"})).unwrap();
    let named = app.workspaces.set.saved.iter().find(|w| w.name == "Tall Layers").expect("the saved workspace").id.clone();

    crate::menus::invoke(&mut app, &ctx, "window.workspace.essentials", json!({})).unwrap();
    assert_eq!(app.ui.docking.layout, default, "the preset replaces the arrangement");
    let essentials = app.workspaces.set.current.clone();

    crate::panel_docking::apply(&mut app, vec![Action::resize_group("layers".to_owned(), Some(300.0))]).unwrap();
    assert_ne!(app.ui.docking.layout, default);
    crate::menus::invoke(&mut app, &ctx, "workspace.reset", json!({})).unwrap();
    assert_eq!(app.ui.docking.layout, default, "Reset Workspace restores the preset's arrangement");
    assert_eq!(app.workspaces.set.current, essentials, "and stays in that workspace");

    crate::menus::invoke(&mut app, &ctx, &format!("window.workspace.{named}"), json!({})).unwrap();
    assert_eq!(app.ui.docking.layout, mine, "the named workspace kept its own arrangement");
    assert_eq!(app.workspaces.set.current, named);
}

/// An unreadable `docking` value shows the default without losing the original save.
#[test]
fn an_unreadable_docking_layout_is_retained_until_a_panel_changes() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut saved = snapshot(&app);
    saved["docking"] = json!({"layout": "not-a-dock-layout"});
    saved["dockingHidden"] = json!(["future panel metadata"]);
    saved["panels"]["status_bar"] = json!(false);
    app.run("window.panel.close", json!({"panel": "layers"})).unwrap();
    let default = registry::default_workspace();
    assert_ne!(app.ui.docking.layout, default.layout, "the app starts from a changed layout");
    assert!(app.ui.panels.status_bar);

    apply(&mut app, &saved);
    assert_eq!(app.ui.docking.layout, default.layout, "the default layout shows");
    assert_eq!(app.ui.docking.hidden, default.hidden);
    assert!(!app.ui.panels.status_bar, "the rest of the save still applies");
    assert_eq!(snapshot(&app), saved, "the fallback keeps the original layout and its companion data");
    assert!(crate::notices::shown_titled(&app, RECOVERY_TITLE));

    assert!(crate::panel_docking::show_panel(&mut app, "layers", Some(true)).unwrap());
    assert_eq!(snapshot(&app), saved, "showing the already visible panel keeps the saved layout");

    app.run("window.panel.close", json!({"panel": "layers"})).unwrap();
    assert_eq!(snapshot(&app)["docking"]["version"], json!(Workspace::<String>::VERSION));
    assert!(!crate::notices::shown_titled(&app, RECOVERY_TITLE));
}

#[test]
fn choosing_or_resetting_to_the_same_arrangement_replaces_the_unreadable_layout() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.session.prefs.edit(|p| p.workspace.remember_workspace_changes = true);
    let default = snapshot(&app);
    let named = crate::workspace_ui::save(&mut app, "Same arrangement").unwrap();
    let mut saved = default.clone();
    saved["docking"] = json!({"version": 99, "fromFuture": [1, 2, 3]});
    app.session.prefs.edit(|p| p.panel_layout = saved.clone());
    restore(&mut app);
    crate::workspace_ui::select(&mut app, &named).unwrap();
    assert_eq!(snapshot(&app), saved, "reselecting the same workspace without a layout change keeps the original");

    for command in ["window.workspace.essentials", "workspace.reset"] {
        app.session.prefs.edit(|p| p.panel_layout = saved.clone());
        restore(&mut app);
        assert_eq!(snapshot(&app), saved);
        assert!(crate::workspace_ui::select(&mut app, "missing-workspace").is_err());
        assert_eq!(snapshot(&app), saved);

        crate::menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
        assert_eq!(app.session.prefs().panel_layout, default);
        assert!(!crate::notices::shown_titled(&app, RECOVERY_TITLE));
    }
}

#[test]
fn an_overflowing_floating_rect_falls_back_and_keeps_the_saved_value() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("window.panel.float", json!({"panel": "layers"})).unwrap();
    let mut saved = snapshot(&app);
    let floated = saved["docking"]["layout"]["floating"].as_array_mut().unwrap();
    assert!(!floated.is_empty());
    floated[0]["rect"][0] = json!(1e300);

    apply(&mut app, &saved);
    assert_eq!(app.ui.docking.layout, registry::default_workspace().layout);
    assert_eq!(snapshot(&app), saved);
}

#[test]
fn recovery_notice_survives_full_screen_and_a_full_queue_of_sticky_notices() {
    use crate::{Services, i18n, notices};
    use craft::ui::runtime::{env::WindowKey, toast};
    use egui_kittest::Harness;

    let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::AppTheme::pro());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.session.prefs.edit(|p| {
            p.interface.language = "en".into();
            p.interface.notification_auto_hide = false;
        });
        let mut saved = snapshot(&app);
        saved["docking"] = json!({"layout": "unreadable"});
        saved["panels"]["status_bar"] = json!(false);
        apply(&mut app, &saved);
        app.ui.view.screen_mode = craft::ui::shell::ScreenMode::FullScreen;
        app
    });
    let has_card = |ctx: &egui::Context, title: &str| {
        toast::cards(ctx, &WindowKey::main()).iter().any(|card| card.toast.title.as_ref().is_some_and(|label| label.as_str() == title))
    };
    harness.run_steps(3);
    assert!(has_card(&harness.ctx, RECOVERY_TITLE));

    for id in 0..toast::MAX_CARDS {
        notices::post(harness.state_mut(), format!("Notice {id}"), Vec::new(), notices::Tone::Neutral, None);
    }
    harness.run_steps(4);
    assert!(has_card(&harness.ctx, RECOVERY_TITLE));
    assert!(has_card(&harness.ctx, &format!("Notice {}", toast::MAX_CARDS - 1)));

    if let Some(lang) = i18n::Lang::from_code("ru") {
        let title = i18n::with_language(lang, || i18n::t(RECOVERY_TITLE).to_string());
        harness.state_mut().session.prefs.edit(|p| p.interface.language = "ru".into());
        harness.run_steps(4);
        assert!(has_card(&harness.ctx, &title));
    }
}

/// Whether the toolbar's column is in the end dock.
fn toolbar_at_end(app: &PhotocraftApp) -> bool {
    let layout = &app.ui.docking.layout;
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
        let layout = &app.ui.docking.layout;
        layout.place(&registry::TOOLBAR.to_owned()).and_then(|place| layout.column(&place)).and_then(|column| column.steps)
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    apply(&mut app, &json!({"panels": {"toolbar": true, "options_bar": true, "status_bar": true, "toolbar_double": true}}));
    assert_eq!(steps(&app), Some(2));
    apply(&mut app, &json!({"panels": {"toolbar": false, "options_bar": true, "status_bar": true}}));
    assert!(!crate::panel_docking::is_shown(&app, registry::TOOLBAR), "the toolbar is closed");
    assert!(app.ui.docking.hidden.iter().any(|(panel, _)| panel == registry::TOOLBAR), "and keeps its place to reopen at");
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
    assert_eq!(app.ui.docking.layout, registry::default_workspace().layout, "the default layout shows");
    persist(&mut app);
    assert_eq!(app.session.prefs().panel_layout, saved, "a persist with no change leaves the newer file saved");

    // `close` saves through `apply`'s own persist.
    app.run("window.panel.close", json!({"panel": "layers"})).unwrap();
    let docking = &app.session.prefs().panel_layout["docking"];
    assert_eq!(docking["version"], json!(Workspace::<String>::VERSION), "a changed layout replaces it: {docking}");
}
