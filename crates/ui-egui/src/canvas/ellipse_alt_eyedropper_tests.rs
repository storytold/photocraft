//! #13224: with the Ellipse tool, ⌥ (Alt) held at the press with no shape layer active picks the
//! colour under the pointer as the foreground and the tool's Fill instead of drawing or opening
//! the Create dialog; ⇧⌥ does the same. ⌥ pressed only after the drag started still draws from the
//! centre, and with a shape layer active ⌥ at the press does not sample.

use craft::keymap::Mods;
use egui::Modifiers;
use photocraft_doc::{Color, Fill, LayerContent};
use serde_json::json;

use super::shape_alt_samples;
use crate::PhotocraftApp;
use crate::canvas::testing::in_gesture;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

const RED: [u8; 3] = [220, 30, 30];
const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];
/// A point on the red layer, and one on the white Background only.
const ON_RED: [f64; 2] = [200.0, 175.0];
const ON_WHITE: [f64; 2] = [500.0, 350.0];
const ALT: Mods = Mods { alt: true, ..Mods::NONE };

/// The issue's test document: 800 × 600, white Background, a red (220,30,30) pixel layer at
/// 100,100 to 300,250 (active), black foreground, the Ellipse tool in Shape mode.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 150})).unwrap();
    app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::EllipseShape;
    app
}

/// What a press could change: history states, layers, open dialogs and colour samplers.
fn document_state(app: &PhotocraftApp) -> (usize, usize, usize, usize) {
    let st = app.session.active().unwrap();
    (st.history.past_len(), st.doc.layers.len(), app.framework.dialogs.stack().ids().count(), st.color_samplers.len())
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
    for press in [Modifiers::ALT, Modifiers::SHIFT | Modifiers::ALT] {
        let mut app = app();
        let before = document_state(&app);
        drag(&mut app, ON_RED, ON_RED, press, press);
        assert_eq!(foreground(&app), RED, "{press:?}: foreground #DC1E1E");
        assert_eq!(tool_fill(&app), RED, "{press:?}: Fill 220,30,30");
        assert_eq!(document_state(&app), before, "{press:?}: no History state, layer, dialog or colour sampler");
        assert!(!in_gesture(&app), "{press:?}: the release ends the sampling");
        // The next shape is drawn in that Fill.
        drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE, Modifiers::NONE);
        assert_eq!(active_shape_fill(&app), RED, "{press:?}: the next ellipse's Fill");
    }
}

#[test]
fn an_alt_drag_keeps_the_colour_under_the_release_point() {
    for press in [Modifiers::ALT, Modifiers::SHIFT | Modifiers::ALT] {
        let mut app = app();
        let before = document_state(&app);
        // The last move is still on red; the release is on white.
        tool_event(&mut app, ToolEvent::Down { x: ON_RED[0], y: ON_RED[1], pressure: 1.0 }, press);
        tool_event(&mut app, ToolEvent::Move { x: 250.0, y: 200.0, pressure: 1.0 }, press);
        assert_eq!(foreground(&app), RED, "{press:?}: the drag samples as it goes");
        tool_event(&mut app, ToolEvent::Up { x: ON_WHITE[0], y: ON_WHITE[1] }, press);
        assert_eq!(foreground(&app), WHITE, "{press:?}: foreground");
        assert_eq!(tool_fill(&app), WHITE, "{press:?}: Fill");
        assert_eq!(document_state(&app), before, "{press:?}: no shape and no History state");
    }
}

#[test]
fn a_gradient_or_no_fill_becomes_the_sampled_solid_fill() {
    let gradient = Fill::Gradient {
        stops: vec![(0.0, Color::BLACK), (1.0, Color::WHITE)],
        angle: 90.0,
        scale: 1.0,
        style: Default::default(),
        reverse: false,
        opacity_stops: Vec::new(),
        midpoints: Vec::new(),
        offset: (0.0, 0.0),
        dither: false,
        align: true,
    };
    for (fill, fill_gradient) in [(true, Some(gradient)), (false, None)] {
        let mut app = app();
        app.ui.tool_options.shape_fill = fill;
        app.ui.tool_options.shape_fill_gradient = fill_gradient;
        drag(&mut app, ON_RED, ON_RED, Modifiers::ALT, Modifiers::ALT);
        assert_eq!(tool_fill(&app), RED, "fill {fill}");
    }
}

#[test]
fn alt_pressed_after_the_drag_started_still_draws_from_the_centre() {
    let mut app = app();
    let layers = document_state(&app).1;
    drag(&mut app, [400.0, 300.0], [500.0, 350.0], Modifiers::NONE, Modifiers::ALT);
    assert_eq!(foreground(&app), BLACK, "nothing sampled");
    assert_eq!(document_state(&app).1, layers + 1, "a ellipse was drawn");
    let st = app.session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let LayerContent::Shape(shape) = &layer.content else { panic!("a shape layer") };
    // The anchors' mean is the centre of a rectangle, an ellipse and a regular polygon.
    let knots = &shape.path.subpaths[0].knots;
    let n = knots.len() as f64;
    let (cx, cy) = (knots.iter().map(|k| k.anchor.x).sum::<f64>() / n, knots.iter().map(|k| k.anchor.y).sum::<f64>() / n);
    assert!((cx - 400.0).abs() < 1e-6 && (cy - 300.0).abs() < 1e-6, "centred on the press: ({cx}, {cy})");
}

#[test]
fn a_shape_layer_active_does_not_sample() {
    let mut app = app();
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE, Modifiers::NONE);
    app.run("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    assert!(!shape_alt_samples(&mut app, ALT, ON_RED[0], ON_RED[1]), "⌥ is left to the subtract");
    tool_event(&mut app, ToolEvent::Down { x: ON_RED[0], y: ON_RED[1], pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Up { x: ON_RED[0], y: ON_RED[1] }, Modifiers::ALT);
    assert_eq!(foreground(&app), BLACK);
}
