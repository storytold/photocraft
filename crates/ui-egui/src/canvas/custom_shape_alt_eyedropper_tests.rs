//! #13467: with the Custom Shape tool, Alt held at the press (no shape layer active) picks the
//! colour under the pointer as the foreground and the tool's Fill instead of drawing. Alt pressed
//! only after the drag started still draws from the centre, and with a shape layer active Alt at
//! the press does not sample.

use craft::draw::ui::VectorMode;
use craft::keymap::Mods;
use egui::Modifiers;
use photocraft_doc::{LayerContent, vector::PathOp};
use serde_json::json;

use super::shape_alt_samples;
use crate::PhotocraftApp;
use crate::canvas::testing::click;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// 800 × 600, white Background, a red (220,30,30) pixel layer at 100,100 to 300,250 (active),
/// black foreground, the Custom Shape tool in Shape mode.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 150})).unwrap();
    app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    app.ui.tool = Tool::CustomShape;
    app.ui.tool_options.shape_mode = VectorMode::Shape;
    app.sync_views();
    app
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

fn foreground(app: &PhotocraftApp) -> [u8; 3] {
    let c = app.session.tools.foreground;
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as u8)
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], press: Modifiers, rest: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, press);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, rest);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, rest);
}

/// The tool's Fill colour: a solid fill follows the foreground (`vector_ui::shapes::tool_paint`).
fn tool_fill(app: &PhotocraftApp) -> [u8; 3] {
    let o = &app.ui.tool_options;
    assert!(o.shape_fill && o.shape_fill_gradient.is_none(), "a solid Fill");
    foreground(app)
}

#[test]
fn alt_click_on_the_red_layer_picks_it_as_foreground_and_fill() {
    let mut app = app();
    // A gradient or no Fill becomes the sampled solid colour.
    app.ui.tool_options.shape_fill = false;
    app.ui.tool_options.shape_fill_gradient = Some(photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(0.0, 0.0, 1.0, 1.0)));
    let (before, count) = (steps(&app), layers(&app));
    click(&mut app, [200.0, 175.0], Modifiers::ALT);
    assert_eq!(foreground(&app), [220, 30, 30], "#DC1E1E");
    assert_eq!(tool_fill(&app), [220, 30, 30], "Fill 220,30,30");
    assert_eq!(layers(&app), count, "no new layer");
    assert_eq!(steps(&app), before, "no History state");
}

#[test]
fn alt_drag_ending_on_white_picks_white() {
    let mut app = app();
    let (before, count) = (steps(&app), layers(&app));
    drag(&mut app, [400.0, 300.0], [500.0, 350.0], Modifiers::ALT, Modifiers::ALT);
    assert_eq!(foreground(&app), [255, 255, 255]);
    assert_eq!(tool_fill(&app), [255, 255, 255], "Fill white");
    assert_eq!(layers(&app), count, "no new layer");
    assert_eq!(steps(&app), before, "no History state");
}

#[test]
fn the_release_point_colour_is_the_one_kept() {
    let mut app = app();
    // The last move is on the red layer, the release on the white Background.
    tool_event(&mut app, ToolEvent::Down { x: 200.0, y: 175.0, pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Move { x: 250.0, y: 200.0, pressure: 1.0 }, Modifiers::ALT);
    assert_eq!(foreground(&app), [220, 30, 30], "red while over the layer");
    tool_event(&mut app, ToolEvent::Up { x: 500.0, y: 350.0 }, Modifiers::ALT);
    assert_eq!(foreground(&app), [255, 255, 255], "white where released");
    assert_eq!(tool_fill(&app), [255, 255, 255], "Fill white");
}

#[test]
fn alt_pressed_after_the_drag_started_draws_from_the_centre() {
    let mut app = app();
    let count = layers(&app);
    drag(&mut app, [400.0, 300.0], [500.0, 350.0], Modifiers::NONE, Modifiers::ALT);
    assert_eq!(foreground(&app), [0, 0, 0], "nothing sampled");
    assert_eq!(layers(&app), count + 1, "a shape layer");
    let st = app.session.active().unwrap();
    let LayerContent::Shape(shape) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("a shape layer") };
    // In the 300,250 to 500,350 box round the press, centred on it (a corner drag centres on 450,325).
    let (x0, y0, x1, y1) = shape.path.control_bounds().unwrap();
    assert!(x0 >= 299.5 && y0 >= 249.5 && x1 <= 500.5 && y1 <= 350.5, "{x0},{y0} to {x1},{y1}");
    assert!(((x0 + x1) / 2.0 - 400.0).abs() < 1.0 && ((y0 + y1) / 2.0 - 300.0).abs() < 1.0, "drawn from the centre: {x0},{y0} to {x1},{y1}");
}

#[test]
fn alt_drag_with_subtract_selected_edits_the_active_shape_without_sampling() {
    let mut app = app();
    app.run("shape.create", json!({"rect": [100, 100, 200, 150], "fill": "#3270c0", "stroke": null})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    click(&mut app, [200.0, 175.0], Modifiers::ALT);
    assert_eq!(foreground(&app), [0, 0, 0], "an active shape prevents sampling");
    app.ui.tool_options.shape_path_op = "subtract".into();
    let count = layers(&app);
    drag(&mut app, [150.0, 125.0], [250.0, 225.0], Modifiers::ALT, Modifiers::ALT);
    assert_eq!(foreground(&app), [0, 0, 0], "nothing sampled");
    assert_eq!(layers(&app), count, "the component was added to the active shape layer");
    let st = app.session.active().unwrap();
    let LayerContent::Shape(shape) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("a shape layer") };
    assert!(shape.path.subpaths.iter().any(|subpath| subpath.op == PathOp::Subtract), "Subtract adds a component to the active shape");
}

#[test]
fn path_and_pixels_mode_and_ctrl_alt_do_not_sample() {
    let mut app = app();
    const ALT: Mods = Mods { alt: true, ..Mods::NONE };
    for mode in [VectorMode::Path, VectorMode::Pixels] {
        app.ui.tool_options.shape_mode = mode;
        assert!(!shape_alt_samples(&mut app, ALT, 200.0, 175.0), "{mode:?} mode keeps drawing");
    }
    app.ui.tool_options.shape_mode = VectorMode::Shape;
    assert!(!shape_alt_samples(&mut app, Mods { ctrl: true, ..ALT }, 200.0, 175.0), "Ctrl+Alt does not sample");
    assert!(!shape_alt_samples(&mut app, Mods::NONE, 200.0, 175.0));
}
