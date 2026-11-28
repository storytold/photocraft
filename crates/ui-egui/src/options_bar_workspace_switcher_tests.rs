//! #16799: the workspace switcher at the right end of the options bar. Its menu has the built-in workspaces sorted by
//! name with the current one checked, the saved ones, then Reset, New and Delete, and runs the Window › Workspace
//! commands. It stays for every tool and hides with the bar.

use egui::accesskit::{Role, Toggled};
use egui::{Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The switcher's name and tooltip, as in the reference application.
const SWITCHER: &str = "Choose a Workspace";

/// The Search button's name (#16800), which sits left of the switcher.
const SEARCH: &str = "Search for tools, help, and more";

/// The window width the trials use, in points.
const WIDTH: f32 = 1440.0;

/// The roles a menu row can have.
const ROWS: [Role; 3] = [Role::MenuItem, Role::MenuItemCheckBox, Role::MenuItemRadio];

/// The app with a new document and `tool` chosen.
fn harness(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(WIDTH, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.ui.tool = tool;
        app
    });
    h.run_steps(8);
    h
}

/// The switcher button's rect, if it is shown.
fn switcher(h: &Harness<'_, Box<PhotocraftApp>>) -> Option<Rect> {
    h.query_by_role_and_label(Role::Button, SWITCHER).map(|n| n.rect())
}

/// Opens the switcher's menu with a click on its button.
fn open_menu(h: &mut Harness<'_, Box<PhotocraftApp>>) {
    h.get_by_role_and_label(Role::Button, SWITCHER).click();
    h.run_steps(6);
}

/// The open menu's rows below the options bar, top to bottom: (name, checked, rect).
fn menu_rows(h: &Harness<'_, Box<PhotocraftApp>>) -> Vec<(String, bool, Rect)> {
    let bar_bottom = switcher(h).map_or(0.0, |r| r.bottom());
    let mut rows: Vec<(String, bool, Rect)> = ROWS
        .iter()
        .flat_map(|role| h.query_all_by_role(*role))
        .map(|n| (n.accesskit_node().label().unwrap_or_default(), n.accesskit_node().toggled() == Some(Toggled::True), n.rect()))
        .filter(|(_, _, rect)| rect.top() >= bar_bottom)
        .collect();
    rows.sort_by(|a, b| a.2.top().total_cmp(&b.2.top()));
    rows
}

/// Clicks the open menu's row named `name`.
fn pick(h: &mut Harness<'_, Box<PhotocraftApp>>, name: &str) {
    let row = ROWS.iter().find_map(|role| h.query_by_role_and_label(*role, name)).unwrap_or_else(|| panic!("no row {name}: {:?}", menu_rows(h)));
    row.click();
    h.run_steps(6);
}

/// The names of the menu's rows.
fn names(rows: &[(String, bool, Rect)]) -> Vec<&str> {
    rows.iter().map(|(n, ..)| n.as_str()).collect()
}

#[test]
fn the_switcher_sits_at_the_right_end_right_of_search() {
    let h = harness(Tool::Move);
    let switcher = switcher(&h).expect("the options bar has a workspace switcher");
    let search = h.get_by_role_and_label(Role::Button, SEARCH).rect();
    assert!(search.right() <= switcher.left(), "Search is left of the switcher: {search:?} {switcher:?}");
    assert!((search.center().y - switcher.center().y).abs() < 1.0, "both on the bar's line");
    assert!(switcher.right() > WIDTH - 48.0, "at the bar's right end: {switcher:?}");
}

#[test]
fn the_switcher_keeps_its_place_for_every_tool() {
    let at = switcher(&harness(Tool::Move)).expect("the switcher with the Move tool");
    for tool in [Tool::Brush, Tool::Crop, Tool::Type, Tool::Gradient, Tool::Zoom] {
        let rect = switcher(&harness(tool)).unwrap_or_else(|| panic!("the switcher with {tool:?}"));
        assert!((rect.right() - at.right()).abs() < 0.5, "{tool:?}: {rect:?} vs {at:?}");
    }
}

#[test]
fn the_menu_lists_the_built_in_workspaces_sorted_then_reset_new_and_delete() {
    let mut h = harness(Tool::Move);
    open_menu(&mut h);
    let rows = menu_rows(&h);
    assert_eq!(
        names(&rows),
        ["Essentials", "Graphic and Web", "Motion", "Painting", "Photography", "Pixel Art", "Reset Essentials", "New Workspace…", "Delete Workspace…"],
        "{rows:?}"
    );
    let checked: Vec<&str> = rows.iter().filter(|(_, on, _)| *on).map(|(n, ..)| n.as_str()).collect();
    assert_eq!(checked, ["Essentials"], "only the current workspace is checked");
    // The menu hangs under the button, not over the bar.
    let button = switcher(&h).unwrap();
    assert!(rows.iter().all(|(_, _, r)| r.top() >= button.bottom()), "below the button");
    // At the window's right edge the menu ends near the button's right edge: it slides left to fit (#16799).
    let right = rows.iter().map(|(_, _, r)| r.right()).fold(f32::MIN, f32::max);
    assert!((right - button.right()).abs() <= 8.0 && right <= WIDTH, "under the button's right end: rows end at {right}, {button:?}");
}

#[test]
fn picking_a_workspace_makes_it_current_as_window_workspace_does() {
    let mut h = harness(Tool::Move);
    open_menu(&mut h);
    pick(&mut h, "Painting");
    assert_eq!(h.state().workspaces.set.current, "painting");
    assert!(menu_rows(&h).is_empty(), "the menu closes");
    open_menu(&mut h);
    let rows = menu_rows(&h);
    let checked: Vec<&str> = rows.iter().filter(|(_, on, _)| *on).map(|(n, ..)| n.as_str()).collect();
    assert_eq!(checked, ["Painting"]);
    assert!(names(&rows).contains(&"Reset Painting"), "Reset names the current workspace: {rows:?}");
}

#[test]
fn saved_workspaces_follow_the_built_in_ones() {
    let mut h = harness(Tool::Move);
    crate::workspace_ui::save(h.state_mut(), "My Layout").unwrap();
    h.run_steps(4);
    open_menu(&mut h);
    let rows = menu_rows(&h);
    let names = names(&rows);
    let at = names.iter().position(|n| *n == "My Layout").unwrap_or_else(|| panic!("the saved workspace is listed: {names:?}"));
    assert_eq!(names.get(at - 1), Some(&"Pixel Art"), "after the built-in ones: {names:?}");
    assert_eq!(names.get(at + 1), Some(&"Reset My Layout"), "then Reset: {names:?}");
    assert!(rows.iter().any(|(n, on, _)| n == "My Layout" && *on), "a new workspace is current");
}

#[test]
fn the_menu_has_no_lock_or_keyboard_shortcuts_rows() {
    let mut h = harness(Tool::Move);
    open_menu(&mut h);
    let rows = menu_rows(&h);
    assert!(!rows.iter().any(|(n, ..)| n.contains("Lock") || n.contains("Keyboard")), "{rows:?}");
}

#[test]
fn new_workspace_opens_the_window_menus_dialog() {
    let mut h = harness(Tool::Move);
    assert!(h.state().framework.dialogs.is_empty());
    open_menu(&mut h);
    pick(&mut h, "New Workspace…");
    assert!(!h.state().framework.dialogs.is_empty(), "New Workspace… opens its dialog");
}

#[test]
fn delete_workspace_opens_the_window_menus_dialog() {
    let mut h = harness(Tool::Move);
    crate::workspace_ui::save(h.state_mut(), "Spare").unwrap();
    h.run_steps(4);
    assert!(h.state().framework.dialogs.is_empty());
    open_menu(&mut h);
    pick(&mut h, "Delete Workspace…");
    assert!(!h.state().framework.dialogs.is_empty(), "Delete Workspace… opens its dialog");
}

#[test]
fn reset_runs_window_workspace_reset_and_keeps_the_workspace() {
    let mut h = harness(Tool::Move);
    let id = crate::workspace_ui::save(h.state_mut(), "Spare").unwrap();
    h.run_steps(4);
    let runs = h.state().commands_run;
    open_menu(&mut h);
    pick(&mut h, "Reset Spare");
    assert_eq!(h.state().workspaces.set.current, id, "Reset keeps the workspace");
    assert!(h.state().commands_run > runs, "Reset runs a command");
    assert!(!h.state().ui.status_error, "Reset runs without an error: {}", h.state().ui.status);
}

#[test]
fn the_switcher_hides_with_the_options_bar() {
    let mut h = harness(Tool::Move);
    assert!(switcher(&h).is_some());
    h.state_mut().ui.panels.options_bar = false;
    h.run_steps(4);
    assert!(switcher(&h).is_none(), "no switcher without the options bar");
}
