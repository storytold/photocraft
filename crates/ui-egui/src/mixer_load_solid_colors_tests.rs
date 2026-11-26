//! The Mixer Brush's Load Solid Colors Only (#11952). On, ⌥-click loads the one colour under the
//! hotspot (the first green pixel, 200,300 → 30,180,60), the swatch shows it and the next stroke paints
//! it; off, ⌥-click loads the tip image as before.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

const RED: [f32; 4] = [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0];
const GREEN: [f32; 4] = [30.0 / 255.0, 180.0 / 255.0, 60.0 / 255.0, 1.0];

/// An 800 × 600 document whose pixel layer is red left of x 200 and green from there on, with the
/// Mixer Brush active.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Split"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 600})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": RED})).unwrap();
    app.run("select.rect", json!({"x": 200, "y": 0, "width": 600, "height": 600})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": GREEN})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000", "background": "#ffffff"})).unwrap();
    app.ui.tool = Tool::MixerBrush;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app
}

fn alt_click(app: &mut PhotocraftApp, x: f64, y: f64) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::ALT);
    tool_event(app, ToolEvent::Up { x, y }, Modifiers::ALT);
}

fn level(c: [f32; 4]) -> [i32; 4] {
    c.map(|v| (v * 255.0).round() as i32)
}

#[test]
fn with_it_on_alt_click_loads_the_colour_under_the_hotspot_and_the_next_stroke_paints_it() {
    let mut app = app();
    app.run("tools.setBrush", json!({"brush": {"mixer": {"solidColorsOnly": true, "wet": 0.0}, "size": 20, "hardness": 1.0}})).unwrap();
    // The middle of the first green pixel: a click right on the edge can map back a hair short of it.
    alt_click(&mut app, 200.5, 300.0);
    let mixer = &app.session.tools.mixer;
    assert_eq!(mixer.reservoir.map(level), Some(level(GREEN)), "{mixer:?}");
    assert!(mixer.spatial.is_none(), "a plain colour, not the tip image");
    // The Current Brush Load swatch shows it.
    assert_eq!(mixer.next_load(app.session.tools.foreground).map(level), Some(level(GREEN)));
    // The next stroke, on an empty layer above, paints it.
    app.run("layer.new.layer", json!({"name": "Paint"})).unwrap();
    tool_event(&mut app, ToolEvent::Down { x: 400.0, y: 100.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 500.0, y: 100.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 500.0, y: 100.0 }, Modifiers::NONE);
    let st = app.session.active().unwrap();
    let px = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(450, 100);
    let [r, g, b, a] = level(px);
    assert!((r - 30).abs() <= 2 && (g - 180).abs() <= 2 && (b - 60).abs() <= 2 && a >= 250, "stroke painted {:?}", level(px));
}

#[test]
fn with_it_off_alt_click_loads_the_tip_image_as_before() {
    let mut app = app();
    alt_click(&mut app, 200.0, 300.0);
    assert!(app.session.tools.mixer.spatial.is_some(), "the sampled tip image");
}
