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

/// The shell's slots of the harness's window, in the order the shell shows them (`AppShell`): the title bar, the
/// options bar, the status bar.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ShellSlot {
    Title,
    Options,
    Status,
}

/// How the shell's view of `slot` took part in `runs`: `(ran, replayed)`. The shell's slot views share one props type,
/// so they are told apart by id, in the order the shell decides them in every pass, which is the order it shows its
/// slots in.
pub(crate) fn slot_runs(runs: &[(&'static str, egui::Id, MemoRun)], slot: ShellSlot) -> (usize, usize) {
    let mut ids: Vec<egui::Id> = Vec::new();
    for (name, id, _) in runs {
        if name.contains("app_shell::SlotView<") && !ids.contains(id) {
            ids.push(*id);
        }
    }
    let Some(&id) = ids.get(slot as usize) else { return (0, 0) };
    let count = |how: MemoRun| runs.iter().filter(|(_, of, run)| *of == id && *run == how).count();
    (count(MemoRun::Ran), count(MemoRun::Replayed))
}

/// How the shell's view of `slot` took part in the last frame: `(ran, replayed)`.
fn slot_views(h: &H, slot: ShellSlot) -> (usize, usize) {
    slot_runs(&last_frame_memo_runs(&h.ctx), slot)
}

/// The panel tab strips the last pass ran.
fn tab_strip_runs(h: &H) -> Vec<(&'static str, u32)> {
    last_pass_runs(&h.ctx).into_iter().filter(|(ty, _)| ty.contains("TabStrip<'_, craft_ui_panels::docking::PanelKey>")).collect()
}

/// A hover over the canvas replays the dock's tab rows and column bars (in the columns' views, which replay whole) and
/// the menu bar (memoized views) and runs no panel tab strip.
#[test]
fn a_hover_over_the_canvas_replays_the_dock_chrome_and_the_menu_bar() {
    let mut h = harness();
    let at = h.state().last_canvas_rect.center();
    assert!(!last_pass_runs(&h.ctx).is_empty(), "the frame runs components");
    for step in 1..=3 {
        h.event(Event::PointerMoved(at + vec2(step as f32, 0.0)));
        h.run_steps(1);
        // The menu bar sits in the title slot's view: while that replays, the menu bar replays with it.
        let title_replayed = slot_views(&h, ShellSlot::Title).1 == 1;
        let columns = (views_ran(&h, "area::ColumnView<"), views(&h, "area::ColumnView<", MemoRun::Replayed));
        assert!(columns.0 == 0 && columns.1 >= 1, "hover step {step}: the dock's columns replay: {columns:?}");
        let views = chrome_views(&h);
        for kind in ["TabRow", "dock_column::bar::BarProps", "menu::bar::BarProps"] {
            let replayed_with = if kind == "menu::bar::BarProps" { title_replayed } else { columns.1 >= 1 };
            let shown = views.iter().any(|(name, _)| name.contains(kind)) || replayed_with;
            assert!(shown, "hover step {step}: the chrome shows a {kind} view: {views:?}");
        }
        let ran: Vec<_> = views.iter().filter(|(_, run)| *run != MemoRun::Replayed).collect();
        assert_eq!(ran, Vec::<&(&str, MemoRun)>::new(), "hover step {step}: the chrome's views replay their last run");
        assert_eq!(tab_strip_runs(&h), [], "hover step {step}: no panel tab strip runs");
    }
}

/// A hover over the canvas replays the title bar, the options bar, the status bar (its own view) and every panel body
/// (in the columns' views, which replay whole): the panels read the modifiers, not the pointer frame
/// (storytold/craft-libs#2812), so a pointer move runs none of them.
#[test]
fn a_hover_over_the_canvas_replays_the_bars_and_the_panel_bodies() {
    let mut h = harness();
    let at = h.state().last_canvas_rect.center();
    for step in 1..=3 {
        h.event(Event::PointerMoved(at + vec2(step as f32, 0.0)));
        h.run_steps(1);
        for slot in [ShellSlot::Title, ShellSlot::Options, ShellSlot::Status] {
            assert_eq!(slot_views(&h, slot), (0, 1), "hover step {step}: the {slot:?} slot's view replays");
        }
        let columns = (views_ran(&h, "area::ColumnView<"), views(&h, "area::ColumnView<", MemoRun::Replayed));
        assert!(
            views_ran(&h, "region_versions::PanelBody<") == 0 && columns == (0, columns.1) && columns.1 >= 2,
            "hover step {step}: every panel body replays with its column: {columns:?}"
        );
    }
}

/// A hover over the canvas replays the toolbar's footer (the colour chips, Quick Mask and Screen Mode) with the toolbar's
/// column: the rail replays its tool buttons but calls its footer every pass, so before it was a view of its own it ran
/// on every frame (#15493).
#[test]
fn a_hover_over_the_canvas_replays_the_toolbar_footer() {
    let mut h = harness();
    let at = h.state().last_canvas_rect.center();
    for step in 1..=3 {
        h.event(Event::PointerMoved(at + vec2(step as f32, 0.0)));
        h.run_steps(1);
        let columns = (views_ran(&h, "area::ColumnView<"), views(&h, "area::ColumnView<", MemoRun::Replayed));
        assert!(views_ran(&h, "toolbar::FooterProps<") == 0 && columns.0 == 0 && columns.1 >= 1, "hover step {step}: the toolbar footer replays: {columns:?}");
    }
}

/// The pointer-move gate (#17148): a move within one toolbar button changes nothing under the pointer, so it runs no
/// frame; a move over the canvas runs one, since the canvas follows the pointer (its cursor, the rulers' marks and the
/// readouts).
#[test]
fn a_move_within_a_button_runs_no_frame_and_one_over_the_canvas_does() {
    let h = harness();
    let viewport = egui::ViewportId::ROOT;
    let button = h.query_all_by_label("Move Tool").last().map(|node| node.rect()).expect("the toolbar shows the Move tool");
    let inside = button.center();
    assert!(!h.ctx.pointer_move_needs_pass(viewport, Some(inside), inside + vec2(1.0, 1.0)), "a move within the button");
    let canvas = h.state().last_canvas_rect.center();
    assert!(h.ctx.pointer_move_needs_pass(viewport, Some(canvas), canvas + vec2(1.0, 1.0)), "a move over the canvas");
}

/// Zoom In from the menu runs no panel body that does not show the view: before, every command counted as a command
/// run, so a zoom ran every panel body in the next frame (#15493).
#[test]
fn a_zoom_runs_only_the_panels_that_show_the_view() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "view.zoomIn", json!({})).unwrap();
    h.run_steps(1);
    let columns = views(&h, "area::ColumnView<", MemoRun::Replayed);
    let bodies = views_ran(&h, "region_versions::PanelBody<");
    assert!(bodies <= 1 && columns >= 1, "only the Navigator's body may run, the other columns replay: {bodies} ran, {columns} columns replayed");
}

/// The footer runs again when a colour it shows changes without a command (the Color panel's sliders, a Color Picker
/// preview) and when the document goes into Quick Mask, so its chips and button never show a stale state.
#[test]
fn the_toolbar_footer_runs_when_what_it_shows_changes() {
    let mut h = harness();
    assert_eq!(views_ran(&h, "toolbar::FooterProps<"), 0, "settled");
    h.state_mut().session.tools.foreground = [0.2, 0.4, 0.6, 1.0];
    h.run_steps(1);
    assert!(views_ran(&h, "toolbar::FooterProps<") > 0, "a new foreground colour runs the footer");
    h.run_steps(2);
    assert_eq!(views_ran(&h, "toolbar::FooterProps<"), 0, "settled again");
    h.state_mut().session.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
    h.run_steps(1);
    assert!(views_ran(&h, "toolbar::FooterProps<") > 0, "Quick Mask on runs the footer");
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
        let (tabs, title) = (views_ran(&h, "docking::group::TabRow<"), slot_views(&h, ShellSlot::Title).0);
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
    assert!(slot_views(&h, ShellSlot::Title).0 > 0, "an unsaved edit runs the title bar");
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

/// What `ran` counts in each of the next `steps` frames, summed.
fn ran_over(h: &mut H, steps: usize, ran: impl Fn(&H) -> usize) -> usize {
    (0..steps)
        .map(|_| {
            h.run_steps(1);
            ran(h)
        })
        .sum()
}

/// The options bar's slot view, if it ran in the last frame.
fn options_ran(h: &H) -> usize {
    slot_views(h, ShellSlot::Options).0
}

/// The status bar's body view, if it ran in the last frame.
fn status_ran(h: &H) -> usize {
    views_ran(h, "status_bar::StatusProps<")
}

/// A pan with a tool whose bar shows no view field runs neither the options bar nor the status bar (it shows the zoom
/// only); a zoom runs the status bar, and the options bar only while the Zoom tool's bar shows the zoom.
#[test]
fn a_pan_runs_neither_bar_and_a_zoom_runs_the_status_bar() {
    let mut h = harness();
    h.state_mut().ui.tool = crate::state::Tool::Move;
    h.run_steps(4);
    h.state_mut().ui.views[0].center[0] += 40.0;
    assert_eq!(ran_over(&mut h, 3, options_ran), 0, "a pan runs no options bar");
    h.state_mut().ui.views[0].center[0] += 40.0;
    assert_eq!(ran_over(&mut h, 3, status_ran), 0, "a pan runs no status bar");

    h.state_mut().ui.views[0].zoom *= 2.0;
    let status = ran_over(&mut h, 3, status_ran);
    assert!(status > 0, "a zoom runs the status bar: {status}");
}

/// The Zoom tool's bar shows the zoom, so a zoom runs it.
#[test]
fn a_zoom_runs_the_zoom_tools_options_bar() {
    let mut h = harness();
    h.state_mut().ui.tool = crate::state::Tool::Zoom;
    h.run_steps(4);
    h.state_mut().ui.views[0].zoom *= 2.0;
    assert!(ran_over(&mut h, 3, options_ran) > 0, "a zoom runs the Zoom tool's bar");
}

/// A tool option changed outside the bar runs the bar, and the frames after it replay it.
#[test]
fn a_tool_option_runs_the_options_bar_once() {
    let mut h = harness();
    h.state_mut().ui.tool = crate::state::Tool::Move;
    h.run_steps(4);
    assert_eq!(ran_over(&mut h, 2, options_ran), 0, "settled");
    let on = h.state().ui.tool_options.move_auto_select;
    h.state_mut().ui.tool_options.move_auto_select = !on;
    assert!(ran_over(&mut h, 1, options_ran) > 0, "a changed option runs the bar");
    // What the bar's frame remembers of the option (the saved preferences) moves no version.
    assert_eq!(ran_over(&mut h, 3, options_ran), 0, "the frames after the changed one run no bar");
}

/// A bar that runs and writes every control's value back leaves the tool options' revision alone, so idle frames never
/// move what the bar's version holds, whichever tool's bar is showing.
#[test]
fn an_idle_options_bar_leaves_the_tool_options_revision() {
    let mut h = harness();
    let mut moved = Vec::new();
    for tool in crate::state::Tool::ALL {
        h.state_mut().ui.tool = tool;
        h.run_steps(4);
        let settled = h.state().ui.tool_options.revision();
        h.run_steps(6);
        if h.state().ui.tool_options.revision() != settled {
            moved.push(tool);
        }
    }
    assert!(moved.is_empty(), "idle frames moved the tool options under {moved:?}");
    // A changed option runs the bar, whose own writes then change nothing more.
    h.state_mut().ui.tool = crate::state::Tool::Move;
    h.run_steps(4);
    let on = h.state().ui.tool_options.move_auto_select;
    h.state_mut().ui.tool_options.move_auto_select = !on;
    let changed = h.state().ui.tool_options.revision();
    h.run_steps(10);
    assert_eq!(h.state().ui.tool_options.revision(), changed, "the bar's run after a change");
}
