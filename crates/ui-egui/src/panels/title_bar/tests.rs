use craft::host::target::{Backend, Os, Runtime, WindowKind};
use craft::window::desktop::Desktop;
use craft::window::policy::{TitleBarPref, decide};
use egui::{ViewportCommand, ViewportId, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use crate::PhotocraftApp;

/// The window size most trials use, in points.
const WINDOW: egui::Vec2 = vec2(1100.0, 600.0);

/// The whole app in a window of `size` points at `pixels_per_point` whose frame the app draws (the Windows decision),
/// with `setup` run on the app before the first frame.
fn integrated_app(size: egui::Vec2, pixels_per_point: f32, setup: impl FnOnce(&mut PhotocraftApp) + 'static) -> Harness<'static, Box<PhotocraftApp>> {
    let mut setup = Some(setup);
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(pixels_per_point).with_max_steps(64).build_eframe(move |cc| {
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
fn commands(h: &mut Harness<'static, Box<PhotocraftApp>>, steps: usize) -> Vec<ViewportCommand> {
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
    let mut h = integrated_app(WINDOW, 1.0, with_unsaved_work);
    h.get_by_label("Close").click();
    let asks = commands(&mut h, 6);
    assert!(!asks.iter().any(|c| matches!(c, ViewportCommand::Close)), "a window that can draw asks first: {asks:?}");
    assert!(!h.state().framework.dialogs.is_empty(), "the discard prompt is open");

    let mut lost = integrated_app(WINDOW, 1.0, |app| {
        with_unsaved_work(app);
        app.window_lost = true;
    });
    lost.get_by_label("Close").click();
    let quits = commands(&mut lost, 6);
    assert!(quits.iter().any(|c| matches!(c, ViewportCommand::Close)), "{quits:?}");
    assert!(lost.state().framework.dialogs.is_empty(), "no prompt opened");
}

/// The whole app with the menu bar set as given.
fn whole_app(menu_bar: bool) -> Harness<'static, Box<PhotocraftApp>> {
    integrated_app(WINDOW, 1.0, move |app| app.ui.panels.menu_bar = menu_bar)
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

/// The top bar holds the brand mark, the menus, the title and the caption buttons, and nothing at its far end, as in
/// the reference application: no appearance button, no command search and no workspace switcher (#13577). Each of
/// them stays in a menu: Window › Appearance, Edit › Search and Window › Workspace.
#[test]
fn the_top_bar_has_no_appearance_search_or_workspace_controls() {
    let h = integrated_app(WINDOW, 1.0, |_| {});
    assert!(h.query_by_label("File").is_some(), "the menus are drawn");
    assert!(h.query_by_label("Close").is_some(), "the caption buttons are drawn");
    assert!(h.query_by_label("Appearance").is_none(), "no appearance button");
    assert!(h.query_by_label_contains("Search commands").is_none(), "no command search button");
    assert!(h.query_by_label("Choose a workspace").is_none(), "no workspace switcher");
}

/// Where `text` was painted in the last frame, and whether it was cut.
fn painted(h: &Harness<'static, Box<PhotocraftApp>>, text: &str) -> Option<(egui::Rect, bool)> {
    fn find(shape: &egui::Shape, text: &str) -> Option<(egui::Rect, bool)> {
        match shape {
            egui::Shape::Text(shape) if shape.galley.text() == text => Some((shape.visual_bounding_rect(), shape.galley.elided)),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, text)),
            _ => None,
        }
    }
    h.output().shapes.iter().find_map(|clipped| find(&clipped.shape, text))
}

/// #15533: in a 1600 × 1000 px window at 125 % (1280 × 800 pt), the menu bar takes its titles' width and leaves the
/// rest of the row to the document title: Help is a menu-bar title, and the title is drawn whole, right of it.
#[test]
fn the_document_title_is_drawn_after_the_last_menu_at_125_percent() {
    use egui_kittest::kittest::NodeT as _;
    let mut h = integrated_app(vec2(1280.0, 800.0), 1.25, |app| {
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    });
    h.run_steps(2);
    let help = h.query_all_by_label("Help").find(|n| n.accesskit_node().role() == egui::accesskit::Role::MenuItem).map(|n| n.rect());
    let help = help.expect("Help is a title of the menu bar, not a row of its \"…\" menu");
    let title = super::title_text(h.state());
    let (drawn, cut) = painted(&h, &title).unwrap_or_else(|| panic!("the document title {title:?} is painted"));
    assert!(drawn.left() > help.right(), "the document title {drawn:?} lies after Help {help:?}");
    assert!(!cut, "the document title is whole");
}
