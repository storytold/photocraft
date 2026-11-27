//! #16875: an Option press can hand the Star tool's gesture to the eyedropper.

use craft::draw::ui::VectorMode;
use egui::Modifiers;
use photocraft_doc::{Color, Fill, LayerContent};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, testing, tool_event};
use crate::state::Tool;

const ON_RED: [f64; 2] = [200.0, 175.0];
const ON_WHITE: [f64; 2] = [500.0, 350.0];
const START_ON_WHITE: [f64; 2] = [400.0, 300.0];

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 150})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    app.ui.tool = Tool::Star;
    app.ui.tool_options.shape_mode = VectorMode::Shape;
    app.sync_views();
    app
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], press: Modifiers, rest: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, press);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, rest);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, rest);
}

fn layers_and_steps(app: &PhotocraftApp) -> (usize, usize) {
    let state = app.session.active().unwrap();
    (state.doc.layers.len(), state.history.past_len())
}

fn foreground(app: &PhotocraftApp) -> [u8; 3] {
    let c = app.session.tools.foreground;
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as u8)
}

#[test]
fn star_alt_drag_samples_the_release_pixel_for_the_foreground_and_fill() {
    for release in [Modifiers::ALT, Modifiers::NONE] {
        let mut app = app();
        app.ui.tool_options.shape_fill = false;
        let before = layers_and_steps(&app);

        tool_event(&mut app, ToolEvent::Down { x: ON_RED[0], y: ON_RED[1], pressure: 1.0 }, Modifiers::ALT);
        tool_event(&mut app, ToolEvent::Move { x: 250.0, y: 200.0, pressure: 1.0 }, Modifiers::ALT);
        assert_eq!(foreground(&app), [255, 0, 0], "red at the last move");
        tool_event(&mut app, ToolEvent::Up { x: ON_WHITE[0], y: ON_WHITE[1] }, release);

        assert_eq!(foreground(&app), [255, 255, 255], "white at release");
        assert!(app.ui.tool_options.shape_fill, "the Fill swatch is solid");
        assert_eq!(layers_and_steps(&app), before, "sampling leaves layers and history alone");
        assert!(!testing::in_gesture(&app));

        drag(&mut app, START_ON_WHITE, ON_WHITE, Modifiers::NONE, Modifiers::NONE);
        let state = app.session.active().unwrap();
        let layer = state.doc.layer(state.active_layer.unwrap()).unwrap();
        assert!(matches!(&layer.content, LayerContent::Shape(shape) if shape.fill == Some(Fill::Solid(Color::WHITE))));
    }
}

#[test]
fn star_alt_click_samples_without_opening_create_star() {
    let mut app = app();
    let before = layers_and_steps(&app);
    testing::click(&mut app, ON_RED, Modifiers::ALT);
    assert_eq!(foreground(&app), [255, 0, 0]);
    assert_eq!(layers_and_steps(&app), before);
    assert!(app.framework.dialogs.is_empty());
}

#[test]
fn star_alt_drag_with_a_star_layer_active_still_draws() {
    let mut app = app();
    drag(&mut app, START_ON_WHITE, ON_WHITE, Modifiers::NONE, Modifiers::NONE);
    let (layers, steps) = layers_and_steps(&app);
    drag(&mut app, ON_RED, [280.0, 230.0], Modifiers::ALT, Modifiers::ALT);
    assert_eq!(foreground(&app), [0, 0, 0]);
    assert_eq!(layers_and_steps(&app), (layers + 1, steps + 1));
}

#[test]
fn star_alt_pressed_after_the_drag_begins_still_draws() {
    let mut app = app();
    let (layers, steps) = layers_and_steps(&app);
    drag(&mut app, START_ON_WHITE, ON_WHITE, Modifiers::NONE, Modifiers::ALT);
    assert_eq!(foreground(&app), [0, 0, 0]);
    assert_eq!(layers_and_steps(&app), (layers + 1, steps + 1));
}

#[test]
fn star_alt_drag_in_path_and_pixels_modes_keeps_drawing() {
    for (mode, new_layers) in [(VectorMode::Path, 0), (VectorMode::Pixels, 1)] {
        let mut app = app();
        app.ui.tool_options.shape_mode = mode;
        let (layers, steps) = layers_and_steps(&app);
        drag(&mut app, START_ON_WHITE, ON_WHITE, Modifiers::ALT, Modifiers::ALT);
        assert_eq!(foreground(&app), [0, 0, 0], "{mode:?}");
        assert_eq!(layers_and_steps(&app), (layers + new_layers, steps + 1), "{mode:?}");
    }
}
