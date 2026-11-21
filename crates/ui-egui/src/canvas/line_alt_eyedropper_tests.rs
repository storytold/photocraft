//! #13245: with the Line tool, ⌥ (Alt) held at the press with no shape layer active picks the colour under the
//! pointer as the foreground and the tool's Fill instead of drawing a line. ⌥ pressed only after the drag started still
//! draws the line, and with a shape layer active ⌥ at the press does not sample. ⇧⌥ at the press was not measured for
//! the Line tool and keeps drawing.

use craft::keymap::Mods;
use egui::Modifiers;
use photocraft_doc::{Fill, LayerContent};
use serde_json::json;

use super::shape_alt_samples;
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

const RED: [u8; 3] = [220, 30, 30];
const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];
/// A point on the red layer, and one on the white Background only.
const ON_RED: [f64; 2] = [200.0, 175.0];
const ON_WHITE: [f64; 2] = [500.0, 350.0];
const ALT: Mods = Mods { alt: true, ..Mods::NONE };

/// The issue's test document: 800 × 600, white Background, a red (220,30,30) pixel layer at 100,100 to 300,250
/// (active), black foreground, the Line tool in Shape mode.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 150})).unwrap();
    app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Line;
    app
}

/// History states and layers: what a drawn line would change.
fn document_state(app: &PhotocraftApp) -> (usize, usize) {
    let st = app.session.active().unwrap();
    (st.history.past_len(), st.doc.layers.len())
}

/// The foreground colour in 8-bit RGB.
fn foreground(app: &PhotocraftApp) -> [u8; 3] {
    let c = app.session.tools.foreground;
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as u8)
}

/// The tool's Fill colour: a solid Fill follows the foreground (`vector_ui::shapes::tool_paint`).
fn tool_fill(app: &PhotocraftApp) -> [u8; 3] {
    let o = &app.ui.tool_options;
    assert!(o.shape_fill && o.shape_fill_gradient.is_none(), "a solid Fill");
    foreground(app)
}

/// Press at `from` with `press` held, move to `to` with `rest`, release there with `rest`.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], press: Modifiers, rest: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, press);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, rest);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, rest);
}

/// The active layer's Fill colour in 8-bit RGB, which must be a shape layer with a solid Fill.
fn active_shape_fill(app: &PhotocraftApp) -> [u8; 3] {
    let st = app.session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let LayerContent::Shape(shape) = &layer.content else { panic!("a shape layer, not {:?}", layer.name) };
    let Some(Fill::Solid(c)) = &shape.fill else { panic!("a solid Fill, not {:?}", shape.fill) };
    let [r, g, b, _] = c.to_rgba8();
    [r, g, b]
}

#[test]
fn alt_click_on_the_red_layer_picks_it_as_foreground_and_fill() {
    let mut app = app();
    let before = document_state(&app);
    drag(&mut app, ON_RED, ON_RED, Modifiers::ALT, Modifiers::ALT);
    assert_eq!(foreground(&app), RED, "foreground #DC1E1E");
    assert_eq!(tool_fill(&app), RED, "Fill 220,30,30");
    assert_eq!(document_state(&app), before, "no line and no History state");
    // The next line is drawn in that Fill.
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE, Modifiers::NONE);
    assert_eq!(active_shape_fill(&app), RED, "the next line's Fill");
}

#[test]
fn an_alt_drag_draws_no_line_and_keeps_the_colour_under_the_release_point() {
    let mut app = app();
    let before = document_state(&app);
    tool_event(&mut app, ToolEvent::Down { x: ON_RED[0], y: ON_RED[1], pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Move { x: 250.0, y: 200.0, pressure: 1.0 }, Modifiers::ALT);
    assert_eq!(foreground(&app), RED, "the drag samples as it goes");
    tool_event(&mut app, ToolEvent::Up { x: ON_WHITE[0], y: ON_WHITE[1] }, Modifiers::ALT);
    assert_eq!(foreground(&app), WHITE);
    assert_eq!(tool_fill(&app), WHITE);
    assert_eq!(document_state(&app), before, "no line and no History state");
}

#[test]
fn alt_pressed_after_the_drag_started_still_draws_the_line() {
    let mut app = app();
    let layers = document_state(&app).1;
    drag(&mut app, [400.0, 300.0], [500.0, 350.0], Modifiers::NONE, Modifiers::ALT);
    assert_eq!(foreground(&app), BLACK, "nothing sampled");
    assert_eq!(document_state(&app).1, layers + 1, "a line was drawn");
}

#[test]
fn a_shape_layer_active_does_not_sample() {
    let mut app = app();
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE, Modifiers::NONE);
    app.run("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    assert!(!shape_alt_samples(&mut app, ALT, ON_RED[0], ON_RED[1]), "⌥ is left to the shape layer");
    tool_event(&mut app, ToolEvent::Down { x: ON_RED[0], y: ON_RED[1], pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Up { x: ON_RED[0], y: ON_RED[1] }, Modifiers::ALT);
    assert_eq!(foreground(&app), BLACK);
}

/// ⇧⌥ at the press was not measured for the Line tool: it keeps drawing, as before.
#[test]
fn shift_alt_at_the_press_keeps_drawing() {
    let mut app = app();
    let layers = document_state(&app).1;
    assert!(!shape_alt_samples(&mut app, Mods { shift: true, ..ALT }, ON_RED[0], ON_RED[1]));
    drag(&mut app, ON_RED, [260.0, 175.0], Modifiers::SHIFT | Modifiers::ALT, Modifiers::SHIFT | Modifiers::ALT);
    assert_eq!(foreground(&app), BLACK, "nothing sampled");
    assert_eq!(document_state(&app).1, layers + 1, "a line was drawn");
}
