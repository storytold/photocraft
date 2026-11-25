//! #12957: holding Alt while dragging with the Smudge tool turns Finger Painting the other way for
//! that one stroke. The options bar's checkbox keeps its value and the History state stays "Smudge
//! Tool". Without Alt the stroke follows the checkbox, and Alt with the other retouching tools
//! keeps its own meaning.
//!
//! The document is 200 × 100: a layer black from x 0 to 99 and white from x 100 on, the foreground
//! a green (0, 160, 0), a 20 px hard tip, Strength 100 %.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app(finger_painting: bool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#ffffff"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#00a000"})).unwrap();
    app.session.tools.brush.size = 20.0;
    app.session.tools.brush.hardness = 1.0;
    app.ui.tool_options.strength = 100.0;
    app.ui.tool_options.finger_painting = finger_painting;
    app.ui.tool = Tool::Smudge;
    app.sync_views();
    app
}

fn drag(app: &mut PhotocraftApp, from: f64, to: f64, mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from, y: 50.0, pressure: 1.0 }, mods);
    let mut x = from;
    while x < to {
        x = (x + 5.0).min(to);
        tool_event(app, ToolEvent::Move { x, y: 50.0, pressure: 1.0 }, mods);
    }
    tool_event(app, ToolEvent::Up { x: to, y: 50.0 }, mods);
}

fn pixel(app: &PhotocraftApp, x: i32) -> [u8; 3] {
    let st = app.session.active().unwrap();
    let c = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, 50);
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as u8)
}

fn greenish([r, g, b]: [u8; 3]) -> bool {
    g > 100 && r < 60 && b < 60
}

#[test]
fn alt_with_finger_painting_off_paints_the_foreground() {
    let mut app = app(false);
    drag(&mut app, 120.0, 170.0, Modifiers::ALT);
    assert!(greenish(pixel(&app, 150)), "the foreground along the stroke: {:?}", pixel(&app, 150));
    assert!(!app.ui.tool_options.finger_painting, "the checkbox keeps its value");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Smudge Tool"));
}

#[test]
fn alt_with_finger_painting_on_smudges_plainly() {
    let mut app = app(true);
    drag(&mut app, 80.0, 130.0, Modifiers::ALT);
    let carried = pixel(&app, 110);
    assert!(carried.iter().all(|v| *v < 128) && !greenish(carried), "black carried into the white, no green: {carried:?}");
    assert!(app.ui.tool_options.finger_painting, "the checkbox keeps its value");
}

#[test]
fn without_alt_the_checkbox_decides() {
    let mut app = app(false);
    drag(&mut app, 120.0, 170.0, Modifiers::NONE);
    assert_eq!(pixel(&app, 150), [255, 255, 255], "white smudged over white stays white");
}

#[test]
fn alt_flips_finger_painting_for_the_smudge_tool_only() {
    for tool in [Tool::Blur, Tool::Sharpen, Tool::Dodge, Tool::Burn] {
        let mut p = json!({"strength": 100.0, "fingerPainting": false});
        crate::retouch_ui::alt_finger_painting(tool, true, &mut p);
        assert_eq!(p["fingerPainting"], false, "{tool:?}");
    }
    let mut p = json!({"strength": 100.0, "fingerPainting": false});
    crate::retouch_ui::alt_finger_painting(Tool::Smudge, true, &mut p);
    assert_eq!(p["fingerPainting"], true);
}

#[test]
fn alt_below_full_strength_keeps_the_checkbox_value() {
    for on in [false, true] {
        let mut p = json!({"strength": 50.0, "fingerPainting": on});
        crate::retouch_ui::alt_finger_painting(Tool::Smudge, true, &mut p);
        assert_eq!(p["fingerPainting"], on, "Alt at Strength 50 % is not measured: the stroke follows the checkbox");
    }
}
