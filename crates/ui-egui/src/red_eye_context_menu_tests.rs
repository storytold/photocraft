//! #12577: a right-click on the canvas with the Red Eye tool opens the layer menu, or the
//! selection menu with the pointer inside a selection, as measured in the reference application on
//! a plain one-layer RGB document. The rows grey out like their menu-bar twins; the right-click
//! itself corrects no red eye and adds no history state. The cases that weren't measured keep
//! opening nothing.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas_tool_menu::count_tests::{call, listed, right_click};
use crate::canvas_tool_menu::{entry_enabled, rows};
use crate::state::Tool;

/// A 64 × 48 RGB document with its one Background layer red, after a cleared selection (so
/// Reselect has one to restore). The Red Eye tool is the tool.
fn app() -> (Box<PhotocraftApp>, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48, "background": "white"})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#d01818"})).unwrap();
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 10, "height": 10})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::RedEye;
    let ctx = app.ctx().clone();
    (app, ctx)
}

/// The open menu's rows as `ui.inspect` lists them: (label, enabled), "―" for separators.
fn entries(app: &mut PhotocraftApp, ctx: &egui::Context) -> Vec<(String, bool)> {
    let inspected = call(app, ctx, "ui.inspect", json!({}));
    let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).cloned().unwrap_or_default();
    entries.iter().map(|e| (e["label"].as_str().unwrap_or("―").to_string(), e["enabled"] != false)).collect()
}

/// The labels of the open menu's greyed rows, sorted.
fn greyed(app: &mut PhotocraftApp, ctx: &egui::Context) -> Vec<String> {
    let rows = entries(app, ctx);
    assert!(!rows.is_empty(), "the menu is open");
    let mut grey: Vec<String> = rows.into_iter().filter(|(label, on)| !on && label != "―").map(|(label, _)| label).collect();
    grey.sort();
    grey
}

fn sorted(labels: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = labels.iter().map(|l| l.to_string()).collect();
    v.sort();
    v
}

/// The document's revision, history depth and journal length.
fn state(app: &PhotocraftApp) -> (u64, usize, usize) {
    let st = app.session.active().unwrap();
    (st.revision, st.history.past_len(), app.session.journal.len())
}

/// Group from Layers… and Load Selection… follow their menu-bar twins, which grey them for the Background
/// (#12866) and for a lone Background with no saved selection (#13293), as the reference does.
const NO_SELECTION_GREY: [&str; 7] =
    ["Blending Options…", "Group from Layers…", "Convert to Layers", "Rasterize Layer", "Rasterize Layer Style", "Free Transform", "Load Selection…"];

fn select(app: &mut PhotocraftApp) {
    app.run("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
}

#[test]
fn without_a_selection_the_layer_menu_greys_the_reference_rows() {
    let (mut app, ctx) = app();
    right_click(&mut app, 40.0, 30.0);
    assert_eq!(greyed(&mut app, &ctx), sorted(&NO_SELECTION_GREY));
}

#[test]
fn inside_a_selection_the_selection_menu_greys_the_reference_rows() {
    let (mut app, ctx) = app();
    select(&mut app);
    right_click(&mut app, 20.0, 20.0);
    assert_eq!(greyed(&mut app, &ctx), sorted(&["Last Filter", "Fade…"]));
}

#[test]
fn rows_grey_out_like_their_menu_bar_twins() {
    for selection in [false, true] {
        let (mut app, _ctx) = app();
        if selection {
            select(&mut app);
        }
        right_click(&mut app, 20.0, 20.0);
        let menu = app.ui.canvas_tool_menu.clone().unwrap();
        for (_, id) in rows(&menu).iter().flatten() {
            assert_eq!(entry_enabled(&app, &menu, id), crate::menus::is_enabled(&app, id), "{id} (selection: {selection})");
        }
    }
}

#[test]
fn the_right_click_corrects_no_red_eye_and_adds_no_history_state() {
    for selection in [false, true] {
        let (mut app, _ctx) = app();
        if selection {
            select(&mut app);
        }
        let before = state(&app);
        right_click(&mut app, 20.0, 20.0);
        assert!(app.ui.canvas_tool_menu.is_some(), "the menu is open (selection: {selection})");
        assert_eq!(state(&app), before, "nothing ran (selection: {selection})");
    }
}

#[test]
fn the_cases_not_measured_keep_opening_nothing() {
    type Setup = fn(&mut PhotocraftApp);
    let cases: [(&str, Setup); 5] = [
        ("outside the selection", |app| app.run("select.rect", json!({"x": 40, "y": 30, "width": 10, "height": 10})).map(drop).unwrap()),
        ("a second layer", |app| app.run("layer.new.layer", json!({"name": "Ink"})).map(drop).unwrap()),
        ("a smart object", |app| app.run("layer.smartObjects.convertToSmartObject", json!({})).map(drop).unwrap()),
        ("a type layer", |app| {
            let background = app.session.active().unwrap().doc.layers[0].id.0;
            app.run("type.create", json!({"text": "Eyes", "size": 12, "x": 4, "y": 20})).unwrap();
            app.run("layer.select", json!({"layer": background})).unwrap();
            app.run("layer.delete", json!({})).unwrap();
        }),
        ("Grayscale", |app| app.run("image.mode.grayscale", json!({})).map(drop).unwrap()),
    ];
    for (what, setup) in cases {
        let (mut app, ctx) = app();
        setup(&mut app);
        let before = state(&app);
        right_click(&mut app, 20.0, 20.0);
        assert!(app.ui.canvas_tool_menu.is_none(), "{what}: no menu");
        assert!(listed(&mut app, &ctx).is_empty(), "{what}");
        assert_eq!(state(&app), before, "{what}: nothing ran");
    }
}
