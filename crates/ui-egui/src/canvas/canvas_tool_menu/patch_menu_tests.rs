//! #12122: the Patch tool's canvas right-click opens the patch rows inside the selection, the
//! Lasso's selection menu outside it and the selection tools' no-selection menu without one.

use egui::Modifiers;
use egui_kittest::Harness;
use serde_json::{Value, json};

use super::tests::{any_dialog, app, harness, open_here, pick, right_click, screen_of};
use super::{NO_SELECTION_MENU, PATCH_DESTINATION, PATCH_SOURCE, PATCH_USE_PATTERN, SELECTION_MENU, available};
use crate::PhotocraftApp;
use crate::state::Tool;

/// A 32 × 32 document with the selection 4..20 square and `tool` selected.
fn selected(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = app();
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 16, "height": 16})).unwrap();
    app.ui.tool = tool;
    app
}

/// Right-click document point `doc` with `tool`: the open menu's entries as `ui.inspect` lists them.
fn entries_at(h: &mut Harness<'static, Box<PhotocraftApp>>, tool: Tool, doc: [f32; 2]) -> Vec<Value> {
    h.state_mut().ui.canvas_tool_menu = None;
    h.state_mut().ui.tool = tool;
    let at = screen_of(h.state(), doc);
    right_click(h, at, Modifiers::NONE);
    let inspected = crate::control::inspect(h.state(), &h.ctx);
    assert_eq!(inspected["canvasToolMenu"]["tool"], json!(format!("{tool:?}")), "{tool:?} opens its menu at {doc:?}");
    inspected["canvasToolMenu"]["entries"].as_array().cloned().unwrap_or_default()
}

/// The entries' labels, a separator as `None`.
fn shown(entries: &[Value]) -> Vec<Option<String>> {
    entries.iter().map(|e| if e["separator"] == json!(true) { None } else { e["label"].as_str().map(String::from) }).collect()
}

fn enabled(entries: &[Value], label: &str) -> bool {
    entries.iter().find(|e| e["label"] == json!(label)).unwrap_or_else(|| panic!("{label} is listed"))["enabled"] == json!(true)
}

#[test]
fn inside_the_selection_the_patch_rows_show_in_the_reference_order() {
    let mut h = harness(selected(Tool::Patch));
    let entries = entries_at(&mut h, Tool::Patch, [12.0, 12.0]);
    let want = [
        Some("Use Pattern to Patch Selection"),
        None,
        Some("Use Patch as Source"),
        Some("Use Patch as Destination"),
        None,
        Some("Color Range…"),
        Some("Load Selection…"),
    ];
    assert_eq!(shown(&entries), want.map(|l| l.map(String::from)));
    assert!(!enabled(&entries, "Use Pattern to Patch Selection"), "no pattern patch exists yet: greyed");
    assert!(enabled(&entries, "Use Patch as Source") && enabled(&entries, "Use Patch as Destination"));
    assert!(enabled(&entries, "Color Range…"));
    assert!(!enabled(&entries, "Load Selection…"), "no selection saved: greyed");

    h.state_mut().run("select.saveSelection", json!({"name": "Alpha 1"})).unwrap();
    let entries = entries_at(&mut h, Tool::Patch, [12.0, 12.0]);
    assert!(enabled(&entries, "Load Selection…"), "a saved selection makes Load Selection… live");
}

#[test]
fn outside_the_selection_the_patch_tool_opens_the_lassos_menu() {
    let mut h = harness(selected(Tool::Patch));
    let lasso = entries_at(&mut h, Tool::Lasso, [26.0, 26.0]);
    let patch = entries_at(&mut h, Tool::Patch, [26.0, 26.0]);
    assert_eq!(shown(&patch), shown(&lasso));
    assert_eq!(patch.iter().map(|e| &e["enabled"]).collect::<Vec<_>>(), lasso.iter().map(|e| &e["enabled"]).collect::<Vec<_>>());
    assert_eq!(patch.len(), SELECTION_MENU.len(), "the selection menu, nothing added");
}

#[test]
fn without_a_selection_the_patch_tool_opens_the_no_selection_menu() {
    let mut app = app();
    app.ui.tool = Tool::Patch;
    let mut h = harness(app);
    let entries = entries_at(&mut h, Tool::Patch, [12.0, 12.0]);
    let want: Vec<_> = NO_SELECTION_MENU.iter().map(|row| row.map(|(label, _)| label.to_string())).collect();
    assert_eq!(shown(&entries), want);
}

#[test]
fn the_mode_rows_set_the_options_bar_mode_and_nothing_else() {
    let mut app = selected(Tool::Patch);
    assert_eq!(app.ui.tool_options.patch_mode, "source");
    let journal = app.session.journal.len();
    for (row, mode) in [(PATCH_DESTINATION, "destination"), (PATCH_SOURCE, "source")] {
        assert!(open_here(&mut app, Tool::Patch, [12.0, 12.0]));
        pick(&mut app, row);
        assert!(app.ui.canvas_tool_menu.is_none(), "choosing a row closes the menu");
        assert_eq!(app.ui.tool_options.patch_mode, mode);
    }
    assert_eq!(app.session.journal.len(), journal, "the mode rows run no command");
    assert_eq!(app.ui.tool, Tool::Patch);

    // Use Pattern is greyed, so choosing it does nothing.
    assert!(open_here(&mut app, Tool::Patch, [12.0, 12.0]));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(!available(&app, &menu, PATCH_USE_PATTERN));
    // Outside the selection the mode rows aren't offered.
    assert!(open_here(&mut app, Tool::Patch, [26.0, 26.0]));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(!available(&app, &menu, PATCH_DESTINATION));
}

#[test]
fn color_range_opens_its_dialog() {
    let mut app = selected(Tool::Patch);
    assert!(open_here(&mut app, Tool::Patch, [12.0, 12.0]));
    pick(&mut app, "select.colorRange");
    assert!(app.ui.canvas_tool_menu.is_none());
    assert!(any_dialog(&app), "Color Range… opens the Color Range dialog");
}
