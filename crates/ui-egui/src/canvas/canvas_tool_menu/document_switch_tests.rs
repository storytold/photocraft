//! #12359: a canvas right-click menu belongs to the document it was opened on. When another
//! document becomes active it closes (`show`), `ui.context.choose` refuses its rows with an error
//! and closes it, and neither document changes. With the same document active it works as before.

use serde_json::{Value, json};

use super::count_tests::{call, right_click};
use super::*;

/// Document A (64 × 48, a pixel layer "Ink" with a selection) and document B (the same, active
/// after it opens), then A active again, with `tool`.
fn two_documents(tool: Tool) -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for _ in 0..2 {
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({"name": "Ink"})).unwrap();
        app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
        app.run("select.rect", json!({"x": 8, "y": 8, "width": 20, "height": 16})).unwrap();
    }
    activate(&mut app, 0);
    app.sync_views();
    app.ui.tool = tool;
    (app, egui::Context::default())
}

fn activate(app: &mut PhotocraftApp, index: usize) {
    app.run("document.activate", json!({"document": index})).unwrap();
}

/// Each document's layer count and history length, by tab.
fn state(app: &PhotocraftApp) -> Vec<(usize, usize)> {
    app.session.documents().iter().map(|d| (d.doc.layers.len(), d.history.past_len())).collect()
}

fn choose(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str) -> Value {
    call(app, ctx, "ui.context.choose", json!({"id": id}))
}

#[test]
fn a_selection_row_chosen_after_a_document_switch_runs_nowhere() {
    let (mut app, ctx) = two_documents(Tool::RectMarquee);
    right_click(&mut app, &ctx, 16.0, 16.0);
    assert!(app.ui.canvas_tool_menu.is_some(), "the menu opens on A");
    let before = state(&app);
    // Document B becomes active over the control channel, as in the issue.
    let r = call(&mut app, &ctx, "engine.execute", json!({"command": "document.activate", "params": {"document": 1}}));
    assert_eq!(r["ok"], true, "{r}");
    let r = choose(&mut app, &ctx, "layer.new.layerViaCut");
    assert_eq!(r["ok"], false, "the row is refused: {r}");
    assert!(app.ui.canvas_tool_menu.is_none(), "and the menu closes");
    assert_eq!(state(&app), before, "neither document changed");
}

#[test]
fn a_layer_row_chosen_after_a_document_switch_runs_nowhere() {
    let (mut app, ctx) = two_documents(Tool::Count);
    right_click(&mut app, &ctx, 40.0, 40.0);
    assert!(app.ui.canvas_tool_menu.is_some(), "the Count tool's layer menu opens on A");
    let before = state(&app);
    activate(&mut app, 1);
    let r = choose(&mut app, &ctx, "layer.delete");
    assert_eq!(r["ok"], false, "{r}");
    assert!(app.ui.canvas_tool_menu.is_none());
    assert_eq!(state(&app), before, "B's layer is still there, and A's");
}

#[test]
fn the_menu_closes_when_another_document_becomes_active() {
    let (mut app, ctx) = two_documents(Tool::RectMarquee);
    right_click(&mut app, &ctx, 16.0, 16.0);
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(menu.doc.is_some(), "the menu records its document");
    assert!(available(&app, &menu, "layer.new.layerViaCut"));
    activate(&mut app, 1);
    assert!(!available(&app, &menu, "layer.new.layerViaCut"), "no row is available on another document");
    let mut out = ctx.run_ui(Default::default(), |ui| show(&mut app, ui.ctx()));
    out.textures_delta.clear();
    assert!(app.ui.canvas_tool_menu.is_none(), "a frame closes it");
}

#[test]
fn the_menu_still_works_on_its_own_document() {
    let (mut app, ctx) = two_documents(Tool::RectMarquee);
    right_click(&mut app, &ctx, 16.0, 16.0);
    let layers = app.session.active().unwrap().doc.layers.len();
    // A switch away and back: the menu's document is active again.
    activate(&mut app, 1);
    activate(&mut app, 0);
    let r = choose(&mut app, &ctx, "layer.new.layerViaCut");
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1, "Layer Via Cut ran on A");
}
