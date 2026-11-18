//! #13350: the Triangle tool's drag fills its box, apex at the top centre, in either direction and
//! from the centre with Alt, and Create Triangle sends the same triangle. Shift keeps the
//! equilateral triangle of the constrained square's circle.

use egui::Modifiers;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app(w: u32, h: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": w, "height": h})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Triangle;
    app
}

/// A canvas drag from `from` to `to` with `mods` held from the first move, as the canvas sends it.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
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

#[test]
fn a_drag_fills_its_box_in_either_direction() {
    for (from, to) in [([400.0, 300.0], [600.0, 400.0]), ([600.0, 400.0], [400.0, 300.0])] {
        let mut app = app(800, 600);
        let n = steps(&app);
        drag(&mut app, from, to, Modifiers::NONE);
        let v = info(&mut app);
        assert_eq!(anchors(&v), [(500.0, 300.0), (600.0, 400.0), (400.0, 400.0)]);
        assert_eq!(v["live"]["kind"], "triangle");
        assert_eq!(steps(&app), n + 1);
    }
}

#[test]
fn alt_after_the_start_draws_from_the_centre() {
    let mut app = app(800, 600);
    drag(&mut app, [400.0, 300.0], [500.0, 350.0], Modifiers::ALT);
    assert_eq!(anchors(&info(&mut app)), [(400.0, 250.0), (500.0, 350.0), (300.0, 350.0)]);
}

#[test]
fn the_dialog_and_the_drag_send_the_same_triangle() {
    let mut dialog = app(800, 600);
    let id =
        crate::dialogs::shape_dialog::open(&mut dialog, craft::draw::ui::shape_drag::ShapeKind::Triangle, craft::behavior::kurbo::Point::new(400.0, 300.0))
            .unwrap();
    crate::dialogs::testing::confirm(&mut dialog, id).unwrap();
    let mut dragged = app(800, 600);
    drag(&mut dragged, [400.0, 300.0], [500.0, 400.0], Modifiers::NONE);
    let (a, b) = (info(&mut dialog), info(&mut dragged));
    assert_eq!(a["live"], b["live"]);
    assert_eq!(anchors(&a), [(450.0, 300.0), (500.0, 400.0), (400.0, 400.0)]);
}
