//! The window's chrome settles (#15493, #16118): a frame whose only input is the pointer moving over the canvas runs no
//! panel tab strip or grip, no title bar, options bar or status bar and no panel body; each repaints its last run.
//! Before, every frame ran them. What the chrome shows runs it again: the UI language, the toolbar's slots, the title
//! and whether the menu bar takes the menu keys. A dock group itself holds its tab strip and body, which are memoized
//! views, and runs with the dock area around them, so it is not counted.

use craft::ui::runtime::measure::{MemoRun, last_frame_memo_runs, last_pass_runs};
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
    const CHROME: [&str; 3] = ["docking::group::GroupGrip<", "TabStrip<'_, craft_ui_panels::docking::PanelKey>", "title_bar::bar::TitleBar<"];
    last_pass_runs(&h.ctx).into_iter().filter(|(ty, _)| CHROME.iter().any(|name| ty.contains(name))).collect()
}

/// The memoized views whose type name holds `name` that took part in the last frame as `how`.
fn views(h: &H, name: &str, how: MemoRun) -> usize {
    last_frame_memo_runs(&h.ctx).into_iter().filter(|(ty, _, run)| ty.contains(name) && *run == how).count()
}

/// The memoized views whose type name holds `name` that ran in the last frame.
fn views_ran(h: &H, name: &str) -> usize {
    views(h, name, MemoRun::Ran)
}

/// The chrome's memoized views in the last frame, by type name, and how each took part: a view that ran in a discarded
/// pass and replayed in the last one counts as run.
fn chrome_views(h: &H) -> Vec<(&'static str, MemoRun)> {
    const CHROME: [&str; 3] = ["docking::group::TabRow<", "dock_column::bar::BarProps<", "menu::bar::BarProps<"];
    last_frame_memo_runs(&h.ctx).into_iter().filter(|(name, _, _)| CHROME.iter().any(|c| name.contains(c))).map(|(name, _, run)| (name, run)).collect()
}

/// The panel tab strips the last pass ran.
fn tab_strip_runs(h: &H) -> Vec<(&'static str, u32)> {
    last_pass_runs(&h.ctx).into_iter().filter(|(ty, _)| ty.contains("TabStrip<'_, craft_ui_panels::docking::PanelKey>")).collect()
}

/// A hover over the canvas replays the dock's tab rows and column bars and the menu bar (memoized views) and runs no
/// panel tab strip.
#[test]
fn a_hover_over_the_canvas_replays_the_dock_chrome_and_the_menu_bar() {
    let mut h = harness();
    let at = h.state().last_canvas_rect.center();
    assert!(!last_pass_runs(&h.ctx).is_empty(), "the frame runs components");
    for step in 1..=3 {
        h.event(Event::PointerMoved(at + vec2(step as f32, 0.0)));
        h.run_steps(1);
        // The menu bar sits in the title bar's own view: while that replays, the menu bar replays with it.
        let title_replayed = views(&h, "TitleSlot<", MemoRun::Replayed) == 1;
        let views = chrome_views(&h);
        for kind in ["TabRow", "dock_column::bar::BarProps", "menu::bar::BarProps"] {
            let shown = views.iter().any(|(name, _)| name.contains(kind)) || (kind == "menu::bar::BarProps" && title_replayed);
            assert!(shown, "hover step {step}: the chrome shows a {kind} view: {views:?}");
        }
        let ran: Vec<_> = views.iter().filter(|(_, run)| *run != MemoRun::Replayed).collect();
        assert_eq!(ran, Vec::<&(&str, MemoRun)>::new(), "hover step {step}: the chrome's views replay their last run");
        assert_eq!(tab_strip_runs(&h), [], "hover step {step}: no panel tab strip runs");
    }
}

/// A hover over the canvas replays the title bar, the options bar, the status bar (its own view) and every panel body: the
/// panels read the modifiers, not the pointer frame (storytold/craft-libs#2812), so a pointer move runs none of them.
#[test]
fn a_hover_over_the_canvas_replays_the_bars_and_the_panel_bodies() {
    let mut h = harness();
    let at = h.state().last_canvas_rect.center();
    for step in 1..=3 {
        h.event(Event::PointerMoved(at + vec2(step as f32, 0.0)));
        h.run_steps(1);
        for slot in ["TitleSlot<", "OptionsSlot<", "status_bar::StatusProps<"] {
            assert_eq!((views_ran(&h, slot), views(&h, slot, MemoRun::Replayed)), (0, 1), "hover step {step}: the {slot} view replays");
        }
        let bodies = (views_ran(&h, "region_versions::PanelBody<"), views(&h, "region_versions::PanelBody<", MemoRun::Replayed));
        assert!(bodies.0 == 0 && bodies.1 >= 2, "hover step {step}: every panel body replays: {bodies:?}");
    }
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
        // The new tab titles change label widths, so the frame takes a second pass that replays what the first ran:
        // the frame's views tell.
        let (tabs, title) = (views_ran(&h, "docking::group::TabRow<"), views_ran(&h, "TitleSlot<"));
        assert!(tabs > 0 && title > 0, "a new language runs the tab rows and the title bar: {tabs} tab rows, {title} title bars");
        h.run_steps(2);
    }

    // Move and Artboard are the first slot's tools: hiding both takes the slot off the rail.
    h.state_mut().session.prefs.edit(|prefs| prefs.toolbar.hidden.extend(["Move".to_string(), "Artboard".to_string()]));
    h.run_steps(1);
    assert!(views_ran(&h, "Tool>") > 0, "a slot taken off the rail runs the toolbar rail");
    h.run_steps(2);

    assert!(!h.state().session.active().unwrap().is_dirty(), "the new document is saved");
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.run_steps(1);
    // The edit's frame takes two passes, and its second replays what the first ran: the frame's views tell.
    assert!(views_ran(&h, "TitleSlot<") > 0, "an unsaved edit runs the title bar");
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
