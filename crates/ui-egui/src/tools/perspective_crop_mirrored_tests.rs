//! #12489: a Perspective Crop box flipped on one axis (⌥ dragging a corner past the opposite one
//! along x only) is wound counter-clockwise. It commits mirrored, each corner
//! mapped as it lies, as in the reference application; `image.perspectiveCrop` takes the same
//! corners. Clicked corners still commit upright, a box turned 180° still commits upside down, and
//! self-crossing and degenerate quads stay invalid.

use std::sync::Arc;

use craft::behavior::kurbo::Point;
use egui::Modifiers;
use serde_json::json;

use super::*;
use crate::canvas::testing::{ToolEvent, tool_event};

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const YELLOW: [f32; 4] = [1.0, 1.0, 0.0, 1.0];

/// The measured test document: 800 x 500, red (top left), green (top right), blue (bottom left)
/// and yellow (bottom right) quadrants with a black square in the middle; the Perspective Crop tool.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 500})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PerspectiveCrop;
    let surf = Arc::make_mut(&mut app.session.active_mut().unwrap().doc).layers[0].surface_mut().unwrap();
    for (x, y, c) in [(0, 0, RED), (400, 0, GREEN), (0, 250, BLUE), (400, 250, YELLOW)] {
        surf.fill_rect(photocraft_geom::Rect::from_xywh(x, y, 400, 250), &c);
    }
    surf.fill_rect(photocraft_geom::Rect::from_xywh(395, 245, 10, 10), &[0.0, 0.0, 0.0, 1.0]);
    app
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    tool_event(app, ToolEvent::Move { x: mid[0], y: mid[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

fn set_quad(app: &mut PhotocraftApp, q: [[f64; 2]; 4]) {
    app.canvas_tools.perspective.tool.set(Some(q.map(|[x, y]| Point::new(x, y))));
}

/// The committed canvas's size and the colours at its four corners (TL, TR, BL, BR).
fn corners(app: &PhotocraftApp) -> ((u32, u32), [[f32; 4]; 4]) {
    let doc = &app.session.active().unwrap().doc;
    let (w, h) = (doc.size.width as i32, doc.size.height as i32);
    let surf = doc.layers[0].surface().unwrap();
    ((doc.size.width, doc.size.height), [surf.rgba(3, 3), surf.rgba(w - 4, 3), surf.rgba(3, h - 4), surf.rgba(w - 4, h - 4)])
}

#[test]
fn the_command_takes_counter_clockwise_corners_and_mirrors() {
    let mut app = app();
    let r = app.run("image.perspectiveCrop", json!({"corners": [[450, 220], [350, 220], [350, 300], [450, 300]]})).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(100), Some(80)));
    assert_eq!(corners(&app).1, [GREEN, RED, YELLOW, BLUE]);
    // Mirrored top to bottom the other way round.
    let mut app = self::app();
    app.run("image.perspectiveCrop", json!({"corners": [[350, 300], [450, 300], [450, 220], [350, 220]]})).unwrap();
    assert_eq!(corners(&app).1, [BLUE, YELLOW, RED, GREEN]);
}

#[test]
fn a_box_turned_180_degrees_commits_upside_down() {
    let mut app = app();
    drag(&mut app, [450.0, 300.0], [550.0, 400.0], Modifiers::NONE);
    drag(&mut app, [550.0, 400.0], [350.0, 200.0], Modifiers::ALT);
    // The drag maps through the view, which leaves the corners a fraction of a pixel off the whole numbers.
    let turned = quad(&app).expect("the box is up");
    let expected = [[450.0, 300.0], [350.0, 300.0], [350.0, 200.0], [450.0, 200.0]];
    assert!(turned.iter().flatten().zip(expected.iter().flatten()).all(|(got, want)| (got - want).abs() < 1e-3), "{turned:?}");
    apply(&mut app);
    assert_eq!(corners(&app), ((100, 100), [YELLOW, BLUE, GREEN, RED]));
}

#[test]
fn self_crossing_and_degenerate_quads_stay_invalid() {
    for q in [
        // Self-crossing (a bow tie), each way round.
        [[350.0, 220.0], [450.0, 220.0], [350.0, 300.0], [450.0, 300.0]],
        [[450.0, 220.0], [350.0, 220.0], [450.0, 300.0], [350.0, 300.0]],
        // Three corners on a line, and no area.
        [[350.0, 220.0], [400.0, 220.0], [450.0, 220.0], [350.0, 300.0]],
        [[400.0, 250.0]; 4],
    ] {
        let mut app = app();
        set_quad(&mut app, q);
        assert!(app.run("image.perspectiveCrop", json!({ "corners": q })).is_err(), "{q:?}");
        apply(&mut app);
        assert_eq!(quad(&app), Some(q), "{q:?} stays for fixing");
        assert_eq!(app.session.active().unwrap().doc.size.width, 800);
    }
    let mut app = app();
    assert!(app.run("image.perspectiveCrop", json!({"corners": [[350, 220], [f64::NAN, 220], [450, 300], [350, 300]]})).is_err());
}
