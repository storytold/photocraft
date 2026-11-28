//! #16800: the Search button at the right end of the options bar. It runs `edit.search`, as Edit › Search… does,
//! stays for every tool and hides with the bar.

use egui::accesskit::Role;
use egui::{Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The button's name and tooltip, as in the reference application.
const SEARCH: &str = "Search for tools, help, and more";

/// The window width the trials use, in points.
const WIDTH: f32 = 1440.0;

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

/// The Search button's rect, if the button is shown.
fn search_button(h: &Harness<'_, Box<PhotocraftApp>>) -> Option<Rect> {
    h.query_by_role_and_label(Role::Button, SEARCH).map(|n| n.rect())
}

/// The options bar's Home button: the bar's first member, on the bar's line.
fn home(h: &Harness<'_, Box<PhotocraftApp>>) -> Rect {
    h.get_by_label("Home").rect()
}

#[test]
fn the_search_button_sits_at_the_right_end_of_the_options_bar() {
    let h = harness(Tool::Move);
    let search = search_button(&h).expect("the options bar has a Search button");
    let home = home(&h);
    assert!(search.center().y > home.top() && search.center().y < home.bottom(), "on the bar's line: {search:?} vs Home {home:?}");
    assert!(search.right() > WIDTH - 48.0, "at the bar's right end: {search:?}");
    // Every other control on the bar's line sits left of it, but the workspace switcher (#16799), which ends the bar.
    let switcher = h.query_by_role_and_label(Role::Button, "Choose a Workspace").map(|n| n.rect());
    let line = |r: Rect| r.center().y > home.top() && r.center().y < home.bottom() && Some(r) != switcher;
    let rightmost_other = h
        .query_all_by_role(Role::Button)
        .chain(h.query_all_by_role(Role::CheckBox))
        .chain(h.query_all_by_role(Role::ComboBox))
        .map(|n| n.rect())
        .filter(|r| line(*r) && *r != search)
        .map(|r| r.right())
        .fold(f32::MIN, f32::max);
    assert!(rightmost_other <= search.left(), "nothing on the bar but the switcher is right of Search: {rightmost_other} > {}", search.left());
}

#[test]
fn a_click_opens_the_search_and_a_second_click_closes_it() {
    let mut h = harness(Tool::Move);
    assert!(!h.state().ui.palette.open);
    h.get_by_role_and_label(Role::Button, SEARCH).click();
    h.run_steps(4);
    assert!(h.state().ui.palette.open, "a click runs edit.search");
    h.get_by_role_and_label(Role::Button, SEARCH).click();
    h.run_steps(4);
    assert!(!h.state().ui.palette.open, "a second click closes the search");
}

#[test]
fn the_search_button_stays_in_place_for_every_tool() {
    let at = search_button(&harness(Tool::Move)).expect("Search with the Move tool");
    for tool in [Tool::Brush, Tool::Crop, Tool::Type, Tool::Gradient, Tool::Zoom] {
        let rect = search_button(&harness(tool)).unwrap_or_else(|| panic!("Search with {tool:?}"));
        assert!((rect.right() - at.right()).abs() < 0.5, "{tool:?}: {rect:?} vs {at:?}");
    }
}

#[test]
fn the_search_button_hides_with_the_options_bar() {
    let mut h = harness(Tool::Move);
    assert!(search_button(&h).is_some());
    h.state_mut().ui.panels.options_bar = false;
    h.run_steps(4);
    assert!(search_button(&h).is_none(), "no Search button without the options bar");
    assert!(h.query_by_label("Home").is_none(), "the bar itself is hidden");
}
