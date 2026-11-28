//! #13271: a Path Selection click on a shape layer locked with Lock All is a click on empty
//! canvas, as in the reference application: the active layer stays and no History step is added.
//! Unlocked shapes and shapes with only Lock Position are still picked.

use egui::Modifiers;
use photocraft_doc::LayerId;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// A rectangle shape at 100,100–300,200 and one at 400,100–600,200, each locked with `locks`
/// (`null` for none), and a pixel layer "Red" above them, active, with the Path Selection tool.
fn setup(left: serde_json::Value, right: serde_json::Value) -> (PhotocraftApp, LayerId, LayerId, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    let shape = |app: &mut PhotocraftApp, x: i32, locks: serde_json::Value| {
        app.run("shape.create", json!({"kind": "rect", "rect": [x, 100, 200, 100], "fill": "#2050c0"})).unwrap();
        if !locks.is_null() {
            app.run("layer.lockLayers", locks).unwrap();
        }
        app.session.active().unwrap().active_layer.unwrap()
    };
    let a = shape(&mut app, 100, left);
    let b = shape(&mut app, 400, right);
    let red = LayerId(app.run("layer.new.layer", json!({"name": "Red"})).unwrap()["layer"].as_u64().unwrap());
    app.ui.tool = Tool::PathSelection;
    (app, a, b, red)
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x, y }, Modifiers::NONE);
}

fn active(app: &PhotocraftApp) -> LayerId {
    app.session.active().unwrap().active_layer.unwrap()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn a_click_on_a_lock_all_shape_keeps_the_active_layer() {
    let (mut app, locked, _, red) = setup(json!({"all": true}), json!(null));
    let before = steps(&app);
    click(&mut app, 200.0, 150.0);
    assert_eq!(active(&app), red, "the locked shape is not picked");
    assert_ne!(active(&app), locked);
    assert_eq!(steps(&app), before, "no History step");
    // Its outline is passed over too.
    click(&mut app, 100.0, 150.0);
    assert_eq!(active(&app), red);
}

#[test]
fn unlocked_and_position_locked_shapes_are_still_picked() {
    let (mut app, open, _, _) = setup(json!(null), json!(null));
    click(&mut app, 200.0, 150.0);
    assert_eq!(active(&app), open);

    let (mut app, _, position, _) = setup(json!({"all": true}), json!({"position": true}));
    click(&mut app, 500.0, 150.0);
    assert_eq!(active(&app), position, "Lock Position alone keeps the pick");
}
