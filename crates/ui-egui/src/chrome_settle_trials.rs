//! The window's chrome settles (#15493): a frame whose only input is the pointer moving over the canvas runs no dock
//! group header, panel tab strip or grip and no title bar; each repaints its last run. Before, every frame ran them.
//! What the chrome shows runs it again: the UI language, the toolbar's slots, the title and whether the menu bar
//! takes the menu keys.

use craft::ui::runtime::measure::last_pass_runs;
use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The whole app on a new document, with the pointer resting over the canvas.
fn harness() -> H {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).with_step_dt(1.0 / 60.0).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
        PhotocraftApp::with_context(cc.egui_ctx.clone(), session, crate::Services::default())
    });
    h.run_steps(8);
    let at = h.state().last_canvas_rect.center();
    h.event(Event::PointerMoved(at));
    h.run_steps(4);
    h
}

/// The chrome components the last pass ran, by type name.
fn chrome_runs(h: &H) -> Vec<(&'static str, u32)> {
    const CHROME: [&str; 4] =
        ["docking::group::DockGroup<", "docking::group::GroupGrip<", "TabStrip<'_, craft_ui_panels::docking::PanelKey>", "title_bar::bar::TitleBar<"];
    last_pass_runs(&h.ctx).into_iter().filter(|(ty, _)| CHROME.iter().any(|name| ty.contains(name))).collect()
}

/// Whether the chrome component named `name` ran in the last pass.
fn ran(h: &H, name: &str) -> bool {
    chrome_runs(h).iter().any(|(ty, _)| ty.contains(name))
}

#[test]
fn a_hover_over_the_canvas_runs_no_dock_chrome_and_no_title_bar() {
    let mut h = harness();
    let at = h.state().last_canvas_rect.center();
    assert!(!last_pass_runs(&h.ctx).is_empty(), "the frame runs components");
    for step in 1..=3 {
        h.event(Event::PointerMoved(at + vec2(step as f32, 0.0)));
        h.run_steps(1);
        assert_eq!(chrome_runs(&h), [], "hover step {step}: the dock chrome and the title bar repaint their last run");
    }
}

/// With no input reaching it, the chrome runs in the frame after what it shows changes: a new UI language (panel names
/// and menu titles), a toolbar slot taken off the rail (the toolbar column's grid) and an edit that marks the document unsaved
/// (the title). A version that missed one would replay the old chrome.
#[test]
fn the_chrome_runs_when_what_it_shows_changes() {
    let mut h = harness();
    assert_eq!(chrome_runs(&h), [], "settled");

    // A build without the catalogs shows English only, so the language cannot change.
    if crate::i18n::needs_catalogs() {
        h.state_mut().session.prefs.edit(|prefs| prefs.interface.language = "fr".into());
        h.run_steps(1);
        assert!(ran(&h, "TabStrip<") && ran(&h, "TitleBar<"), "a new language runs the tab strips and the title bar: {:?}", chrome_runs(&h));
        h.run_steps(2);
    }

    // Move and Artboard are the first slot's tools: hiding both takes the slot off the rail.
    h.state_mut().session.prefs.edit(|prefs| prefs.toolbar.hidden.extend(["Move".to_string(), "Artboard".to_string()]));
    h.run_steps(1);
    assert!(ran(&h, "DockGroup<"), "a slot taken off the rail runs the dock groups: {:?}", chrome_runs(&h));
    h.run_steps(2);

    assert!(!h.state().session.active().unwrap().is_dirty(), "the new document is saved");
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.run_steps(1);
    assert!(ran(&h, "TitleBar<"), "an unsaved edit runs the title bar: {:?}", chrome_runs(&h));
}

/// A dialog closed with Escape runs no command, and the menu bar takes the menu keys again: Alt+F opens the File menu.
#[test]
fn the_menu_keys_open_a_menu_after_a_dialog_closes_with_escape() {
    let mut h = harness();
    let items = |h: &H| h.query_all_by_role(Role::MenuItem).count();
    let titles = items(&h);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "image.imageSize", json!({})).unwrap();
    h.run_steps(4);
    assert!(h.state().dialog_open(), "Image Size is open");
    h.key_press(Key::Escape);
    h.run_steps(4);
    assert!(!h.state().dialog_open(), "Escape closed it");
    h.key_press_modifiers(Modifiers::ALT, Key::F);
    h.run_steps(4);
    assert!(items(&h) > titles, "Alt+F opens the File menu: {} menu items, {titles} titles", items(&h));
}
