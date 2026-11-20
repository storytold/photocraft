//! #13351: Shift with the Triangle tool draws an equilateral triangle in the circle of the square
//! on the drag's shorter side, apex up; Shift+Alt centres the circle on the press. The preview
//! shows the same triangle (the framework tool's preview), and a plain drag still fills its box (#13350).

use egui::Modifiers;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 1000, "height": 800})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Triangle;
    app
}

/// `shape.info` of the active layer.
fn info(app: &mut PhotocraftApp) -> Value {
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    app.session.execute("shape.info", json!({"layer": id})).unwrap()
}

fn anchors(v: &Value) -> Vec<(f64, f64)> {
    let knots = v["path"]["subpaths"][0]["knots"].as_array().unwrap();
    knots.iter().map(|k| (k["anchor"][0].as_f64().unwrap(), k["anchor"][1].as_f64().unwrap())).collect()
}

fn close(got: &[(f64, f64)], want: &[(f64, f64)]) {
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g.0 - w.0).abs() <= 0.05 && (g.1 - w.1).abs() <= 0.05, "{got:?} vs {want:?}");
    }
}

fn drag(start: [f64; 2], end: [f64; 2], mods: Modifiers) -> Vec<(f64, f64)> {
    let mut app = app();
    tool_event(&mut app, ToolEvent::Down { x: start[0], y: start[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: (start[0] + end[0]) / 2.0, y: (start[1] + end[1]) / 2.0, pressure: 1.0 }, mods);
    tool_event(&mut app, ToolEvent::Move { x: end[0], y: end[1], pressure: 1.0 }, mods);
    tool_event(&mut app, ToolEvent::Up { x: end[0], y: end[1] }, mods);
    anchors(&info(&mut app))
}

#[test]
fn a_plain_drag_still_fills_its_box() {
    let got = drag([400.0, 300.0], [600.0, 400.0], Modifiers::NONE);
    close(&got, &[(500.0, 300.0), (600.0, 400.0), (400.0, 400.0)]);
}
