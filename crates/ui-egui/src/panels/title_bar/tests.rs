use craft::host::target::{Backend, Os, Runtime, WindowKind};
use craft::window::desktop::Desktop;
use craft::window::policy::{TitleBarPref, decide};
use egui::{ViewportCommand, ViewportId, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use crate::PhotocraftApp;

/// The whole app in a 1100 × 600 window whose frame the app draws (the Windows decision), with `setup` run on the
/// app before the first frame.
fn integrated_app(setup: impl FnOnce(&mut PhotocraftApp) + 'static) -> Harness<'static, PhotocraftApp> {
    let mut setup = Some(setup);
    let mut h = Harness::builder().with_size(vec2(1100.0, 600.0)).with_max_steps(64).build_eframe(move |cc| {
        let desktop = Desktop::detect(Backend::X11);
        let decision = decide(Os::Windows, Runtime::Native, Backend::X11, &desktop, WindowKind::Main, TitleBarPref::Integrated, None);
        PhotocraftApp::setup_window(&cc.egui_ctx, Default::default(), decision);
        let mut session = photocraft_engine::Session::new();
        session.edit_prefs(|p| p.general.auto_show_home_screen = false);
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), session, crate::Services::default());
        if let Some(setup) = setup.take() {
            setup(&mut app);
        }
        app
    });
    h.run_steps(4);
    h
}

/// The viewport commands the root window got over `steps` frames.
fn commands(h: &mut Harness<'static, PhotocraftApp>, steps: usize) -> Vec<ViewportCommand> {
    let mut all = Vec::new();
    for _ in 0..steps {
        h.step();
        if let Some(viewport) = h.output().viewport_output.get(&ViewportId::ROOT) {
            all.extend(viewport.commands.iter().cloned());
        }
    }
    all
}

fn with_unsaved_work(app: &mut PhotocraftApp) {
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
}

/// The Close caption runs File › Exit, which asks about unsaved work. After the window's GPU device is lost, the
/// prompt could never be drawn, so Close quits at once.
#[test]
fn the_close_caption_asks_about_unsaved_work_unless_the_device_is_lost() {
    let mut h = integrated_app(with_unsaved_work);
    h.get_by_label("Close").click();
    let asks = commands(&mut h, 6);
    assert!(!asks.iter().any(|c| matches!(c, ViewportCommand::Close)), "a window that can draw asks first: {asks:?}");
    assert!(!h.state().framework.dialogs.is_empty(), "the discard prompt is open");

    let mut lost = integrated_app(|app| {
        with_unsaved_work(app);
        app.window_lost = true;
    });
    lost.get_by_label("Close").click();
    let quits = commands(&mut lost, 6);
    assert!(quits.iter().any(|c| matches!(c, ViewportCommand::Close)), "{quits:?}");
    assert!(lost.state().framework.dialogs.is_empty(), "no prompt opened");
}

/// The whole app with the menu bar set as given.
fn whole_app(menu_bar: bool) -> Harness<'static, PhotocraftApp> {
    integrated_app(move |app| app.ui.panels.menu_bar = menu_bar)
}

/// An app that embeds the UI can hide the menu bar; it is on by default. The dock's panels stay either way, and no
/// separate rail of panel icons exists: the dock's own column of icons holds them.
#[test]
fn the_menu_bar_follows_its_panel_flag() {
    assert!(crate::state::Panels::default().menu_bar);

    for menu_bar in [true, false] {
        let h = whole_app(menu_bar);
        assert_eq!(h.query_by_label("File").is_some(), menu_bar, "the title bar's menus are drawn with the flag");
        assert!(h.query_by_label("Color & Swatches").is_none(), "no panel rail is drawn (menu bar {menu_bar})");
        assert!(h.query_all_by_label("Layers").next().is_some(), "the dock itself stays (menu bar {menu_bar})");
    }
}
