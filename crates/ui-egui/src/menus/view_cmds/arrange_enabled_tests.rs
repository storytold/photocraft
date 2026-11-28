//! #16890: which Window › Arrange entries grey. The n-up entries grey until their cells could be filled,
//! Consolidate All to Tabs and the Match entries work from one document, and Cascade waits for a floating document
//! window. The rows are read as `ui.menu.list` reports them, by command id, so the entry order and the labels stay
//! the menu's own business.

use serde_json::json;

use super::*;

/// A document area with `n` documents, every one tabbed in the single tab group.
fn documents(n: usize) -> (Box<PhotocraftApp>, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for i in 0..n {
        app.run("file.new", json!({ "width": 64, "height": 64, "name": format!("W{}", i + 1) })).unwrap();
    }
    app.sync_views();
    let ctx = app.ctx().clone();
    (app, ctx)
}

/// Every menu row as `ui.menu.list` reports it: `(id, enabled)`.
fn menu_rows(app: &mut PhotocraftApp, ctx: &egui::Context) -> Vec<(String, bool)> {
    let (req, _rx) = crate::control::ControlRequest::new("ui.menu.list", json!({}));
    let reply = match crate::control::handle(app, ctx, &req) {
        crate::control::Reply::Done(listed) => listed,
        _ => panic!("ui.menu.list: expected an immediate reply"),
    };
    reply["result"]
        .as_array()
        .expect("ui.menu.list returns an array")
        .iter()
        .map(|it| (it["id"].as_str().unwrap_or_default().to_string(), it["enabled"].as_bool().unwrap_or(false)))
        .collect()
}

/// Whether the row `id` of Window › Arrange is greyed out.
fn on(rows: &[(String, bool)], id: &str) -> bool {
    rows.iter().find(|(row_id, _)| row_id == id).unwrap_or_else(|| panic!("the menu has no {id} row")).1
}

fn enabled(rows: &[(String, bool)], ids: &[&str], state: &str) {
    for id in ids {
        assert!(on(rows, id), "{state}: {id} is enabled: {rows:?}");
    }
}

fn greyed(rows: &[(String, bool)], ids: &[&str], state: &str) {
    for id in ids {
        assert!(!on(rows, id), "{state}: {id} is greyed: {rows:?}");
    }
}

const THREE_UP: [&str; 3] = ["window.arrange.threeUpHorizontal", "window.arrange.threeUpVertical", "window.arrange.threeUpStacked"];
const FOUR_AND_SIX_UP: [&str; 2] = ["window.arrange.fourUp", "window.arrange.sixUp"];
const MATCH_ONE_ENOUGH: [&str; 3] = ["window.arrange.matchZoom", "window.arrange.matchLocation", "window.arrange.matchAll"];

/// Two documents: 2-up is on, and everything that would leave a cell empty is greyed.
#[test]
fn two_documents_grey_every_layout_that_needs_more_documents() {
    let (mut app, ctx) = documents(2);
    let rows = menu_rows(&mut app, &ctx);
    enabled(&rows, &["window.arrange.twoUpHorizontal", "window.arrange.twoUpVertical"], "two documents");
    greyed(&rows, &THREE_UP, "two documents");
    greyed(&rows, &FOUR_AND_SIX_UP, "two documents");
}

/// Three documents: the three 3-up entries are enabled, 4-up and 6-up stay greyed.
#[test]
fn three_documents_enable_the_three_up_entries_only() {
    let (mut app, ctx) = documents(3);
    let rows = menu_rows(&mut app, &ctx);
    enabled(&rows, &THREE_UP, "three documents");
    greyed(&rows, &FOUR_AND_SIX_UP, "three documents");
}

/// Six documents: 4-up and 6-up are enabled.
#[test]
fn six_documents_enable_four_up_and_six_up() {
    let (mut app, ctx) = documents(6);
    let rows = menu_rows(&mut app, &ctx);
    enabled(&rows, &FOUR_AND_SIX_UP, "six documents");
}

/// Four and five documents were not measured, so 4-up and 6-up keep the state they had there: enabled.
#[test]
fn four_and_five_documents_keep_four_up_and_six_up_enabled() {
    for n in [4usize, 5] {
        let (mut app, ctx) = documents(n);
        let rows = menu_rows(&mut app, &ctx);
        enabled(&rows, &FOUR_AND_SIX_UP, "{n} documents (not measured, kept as today)");
    }
}

/// One document in one tab bar: Consolidate All to Tabs and Match Zoom, Location and All are enabled, Cascade is
/// greyed. Match Rotation was not measured and keeps its rule of two documents.
#[test]
fn one_tabbed_document_greys_only_cascade_and_match_rotation() {
    let (mut app, ctx) = documents(1);
    let rows = menu_rows(&mut app, &ctx);
    enabled(&rows, &["window.arrange.consolidateAllToTabs"], "one tabbed document");
    enabled(&rows, &MATCH_ONE_ENOUGH, "one tabbed document");
    greyed(&rows, &["window.arrange.cascade"], "one tabbed document");
    greyed(&rows, &["window.arrange.matchRotation"], "one tabbed document (not measured, kept as today)");
    let (mut two, two_ctx) = documents(2);
    enabled(&menu_rows(&mut two, &two_ctx), &["window.arrange.matchRotation"], "two documents (not measured, kept as today)");
}

/// Cascade goes grey while every document is tabbed, comes up as soon as one document floats, and goes grey again
/// after Consolidate All to Tabs put it back; Consolidate itself is never greyed while a document is open.
#[test]
fn cascade_follows_the_floating_document_windows() {
    let (mut app, ctx) = documents(1);
    greyed(&menu_rows(&mut app, &ctx), &["window.arrange.cascade"], "the document is tabbed");
    crate::menus::invoke(&mut app, &ctx, "window.arrange.floatInWindow", json!({})).unwrap();
    assert_eq!(app.ui.document_windows.floating.len(), 1, "Float in Window put the document in a floating window");
    let floated = menu_rows(&mut app, &ctx);
    enabled(&floated, &["window.arrange.cascade", "window.arrange.consolidateAllToTabs"], "the document floats");
    crate::menus::invoke(&mut app, &ctx, "window.arrange.consolidateAllToTabs", json!({})).unwrap();
    assert!(app.ui.document_windows.floating.is_empty(), "Consolidate All to Tabs closed the floating window");
    let back = menu_rows(&mut app, &ctx);
    greyed(&back, &["window.arrange.cascade"], "after Consolidate All to Tabs");
    enabled(&back, &["window.arrange.consolidateAllToTabs"], "after Consolidate All to Tabs");
}

/// An entry that is newly enabled in a state where it has nothing to do runs without an error and changes nothing:
/// Consolidate with one tab bar, Match with one document.
#[test]
fn a_newly_enabled_entry_with_nothing_to_do_runs_and_changes_nothing() {
    let (mut app, ctx) = documents(1);
    let groups_before = app.ui.document_windows.docked.clone();
    let view_before = app.ui.views.first().cloned().expect("the document has a view");
    crate::menus::invoke(&mut app, &ctx, "window.arrange.consolidateAllToTabs", json!({})).unwrap();
    assert_eq!(app.ui.document_windows.docked, groups_before, "Consolidate with one tab bar changes no group");
    assert!(app.ui.document_windows.floating.is_empty(), "Consolidate with one tab bar opens no window");
    for id in MATCH_ONE_ENOUGH {
        crate::menus::invoke(&mut app, &ctx, id, json!({})).unwrap();
        let view = app.ui.views.first().cloned().expect("the document keeps its view");
        assert_eq!(view.zoom, view_before.zoom, "{id} with one document keeps the zoom");
        assert_eq!(view.center, view_before.center, "{id} with one document keeps the location");
        assert_eq!(view.rotation, view_before.rotation, "{id} with one document keeps the rotation");
    }
}

/// Only Window › Arrange rows react to how the documents are split up: putting every document in its own floating
/// window, and taking them back, changes no enabled flag outside the submenu.
#[test]
fn only_the_arrange_rows_react_to_the_window_state() {
    let (mut app, ctx) = documents(3);
    let tabbed = menu_rows(&mut app, &ctx);
    crate::menus::invoke(&mut app, &ctx, "window.arrange.floatAllInWindows", json!({})).unwrap();
    let floating = menu_rows(&mut app, &ctx);
    for (id, state) in &floating {
        let Some((_, before)) = tabbed.iter().find(|(row_id, _)| row_id == id) else {
            panic!("{id} disappears from the menu when the documents float");
        };
        if state != before {
            assert!(id.starts_with("window.arrange."), "{id} greys or comes up because the windows changed: {before} -> {state}");
        }
    }
    crate::menus::invoke(&mut app, &ctx, "window.arrange.consolidateAllToTabs", json!({})).unwrap();
    let back = menu_rows(&mut app, &ctx);
    assert_eq!(back, tabbed, "taking the documents back to tabs gives the menu its states again");
}
