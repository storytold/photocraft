//! The Artboard tool's Add New Artboard placing mode (#11386): with the mode on, a click on empty
//! canvas puts a board of the bar's size with its top-left corner at the click.

use egui::Modifiers;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// The measured document: two boards, the 750 x 1334 one selected, the other one apart from it.
fn app_with_selected_board() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [2000, 0, 300, 300]})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [900, 0, 750, 1334]})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app.ui.tool = Tool::Artboard;
    app
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64) {
    let m = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x, y }, m);
}

fn boards(app: &PhotocraftApp) -> Vec<Rect> {
    app.session.active().unwrap().doc.artboards().iter().map(|b| b.2.rect).collect()
}

fn last_step(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.entries().last().cloned()
}

#[test]
fn a_click_with_the_mode_on_puts_a_bar_sized_board_at_the_click() {
    let mut app = app_with_selected_board();
    app.ui.artboard_tool.adding = true;
    let before = boards(&app);
    click(&mut app, 222.0, 600.0);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let after = boards(&app);
    assert_eq!(after.len(), before.len() + 1);
    // 750 x 1334, top-left at the click, over the selected board (it did not move aside).
    assert_eq!(after.last().copied(), Some(Rect::new(222, 600, 972, 1934)));
    assert_eq!(after.get(..before.len()), Some(before.as_slice()));
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers.last().map(|l| l.name.as_str()), Some("Artboard 3"));
    assert_eq!(st.active_layer, Some(st.doc.artboards().last().unwrap().0));
    assert_eq!(last_step(&app).as_deref(), Some("New Artboard"));
    // One undo step.
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(boards(&app), before);
}

#[test]
fn the_click_is_rounded_to_whole_pixels() {
    let mut app = app_with_selected_board();
    app.ui.artboard_tool.adding = true;
    click(&mut app, 222.4, 600.6);
    assert_eq!(boards(&app).last().copied(), Some(Rect::new(222, 601, 972, 1935)));
}

#[test]
fn the_mode_stays_on_after_a_board_is_placed() {
    let mut app = app_with_selected_board();
    app.ui.artboard_tool.adding = true;
    click(&mut app, 222.0, 600.0);
    assert!(app.ui.artboard_tool.adding);
    // The next click is another placement, at the new selected board's size.
    click(&mut app, 3000.0, 100.0);
    assert_eq!(boards(&app).last().copied(), Some(Rect::new(3000, 100, 3750, 1434)));
}

#[test]
fn a_click_with_the_mode_off_still_places_nothing() {
    let mut app = app_with_selected_board();
    let before = boards(&app);
    click(&mut app, 222.0, 600.0);
    assert_eq!(boards(&app), before);
}

#[test]
fn with_no_board_the_click_uses_the_canvas_size() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Artboard;
    app.ui.artboard_tool.adding = true;
    click(&mut app, 40.0, 50.0);
    assert_eq!(boards(&app), vec![Rect::new(40, 50, 840, 650)]);
}

#[test]
fn a_drag_with_the_mode_on_still_draws_the_dragged_board() {
    let mut app = app_with_selected_board();
    app.ui.artboard_tool.adding = true;
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 80.0, y: 70.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 120.0, y: 110.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Up { x: 120.0, y: 110.0 }, m);
    assert_eq!(boards(&app).last().copied(), Some(Rect::new(20, 30, 120, 110)));
}

#[test]
fn a_click_on_an_existing_board_selects_it_and_places_nothing() {
    let mut app = app_with_selected_board();
    app.ui.artboard_tool.adding = true;
    let before = boards(&app);
    click(&mut app, 2100.0, 100.0);
    assert_eq!(boards(&app), before);
    let st = app.session.active().unwrap();
    assert_eq!(st.active_layer, Some(st.doc.artboards()[0].0));
}

#[test]
fn another_tool_turns_the_mode_off() {
    let mut app = app_with_selected_board();
    app.ui.artboard_tool.adding = true;
    assert_eq!(serde_json::to_value(&app.ui.artboard_tool).unwrap()["adding"], json!(true));
    app.ui.tool = Tool::Move;
    click(&mut app, 3000.0, 100.0);
    assert!(!app.ui.artboard_tool.adding);
    app.ui.tool = Tool::Artboard;
    let before = boards(&app);
    click(&mut app, 3000.0, 100.0);
    assert_eq!(boards(&app), before);
}
