//! #13298 through the tools: a Polygon-tool drag and Create Polygon make a polygon that fills its box, a ⇧-constrained
//! drag keeps a regular polygon inscribed in a circle, and the Star tool keeps its geometry. The geometry itself is
//! tested in `photocraft_vector` (`polygon_box_geometry_tests`).

use egui::Modifiers;
use photocraft_doc::vector::LiveShape;
use photocraft_doc::{LayerContent, ShapeLayer};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app(tool: Tool, sides: u32) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.run("prefs.set", json!({"path": "tools.snapVectorToolsAndTransformsToPixelGrid", "value": false})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app.ui.tool_options.polygon_sides = sides;
    app
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

fn active_shape(app: &PhotocraftApp) -> ShapeLayer {
    let st = app.session.active().unwrap();
    let LayerContent::Shape(s) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("a shape layer") };
    s.clone()
}

/// The tight box of the shape's anchors: [x0, y0, x1, y1].
fn tight(s: &ShapeLayer) -> [f64; 4] {
    let pts: Vec<[f64; 2]> = s.path.subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
    let f = |i: usize, min: bool| pts.iter().map(|p| p[i]).fold(if min { f64::MAX } else { f64::MIN }, |a, b| if min { a.min(b) } else { a.max(b) });
    [f(0, true), f(1, true), f(0, false), f(1, false)]
}

fn fills_box(s: &ShapeLayer) -> bool {
    matches!(s.live, Some(LiveShape::Polygon { fills_box: true, .. }))
}

fn near(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
}

#[test]
fn a_polygon_drag_fills_the_dragged_box_and_up_left_does_the_same() {
    for sides in [3, 5, 6, 8] {
        let mut down = app(Tool::Polygon, sides);
        drag(&mut down, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE);
        let s = active_shape(&down);
        assert!(fills_box(&s), "{sides} sides: {:?}", s.live);
        assert!(near(tight(&s), [400.0, 300.0, 600.0, 400.0]), "{sides} sides: {:?}", tight(&s));
        let mut up = app(Tool::Polygon, sides);
        drag(&mut up, [600.0, 400.0], [400.0, 300.0], Modifiers::NONE);
        assert_eq!(active_shape(&up).path, s.path, "{sides} sides: dragging up-left");
    }
}

#[test]
fn a_shift_drag_keeps_a_regular_polygon_inscribed_in_a_circle() {
    let mut app = app(Tool::Polygon, 5);
    drag(&mut app, [400.0, 300.0], [600.0, 500.0], Modifiers::SHIFT);
    let s = active_shape(&app);
    assert!(!fills_box(&s), "{:?}", s.live);
    let [x0, y0, x1, y1] = tight(&s);
    assert!(x1 - x0 > y1 - y0, "a regular pentagon in a square box is wider than tall: {:?}", tight(&s));
}

#[test]
fn the_star_tool_keeps_its_geometry() {
    let mut app = app(Tool::Star, 5);
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE);
    assert!(!fills_box(&active_shape(&app)));
}

#[test]
fn create_polygon_makes_the_same_shape_as_a_drag_of_the_same_box() {
    let mut drawn = app(Tool::Polygon, 5);
    drag(&mut drawn, [400.0, 300.0], [500.0, 400.0], Modifiers::NONE);
    let mut typed = app(Tool::Polygon, 5);
    let geometry =
        craft::draw::geometry::shape::ShapeGeometry::Polygon { rect: craft::behavior::kurbo::Rect::new(400.0, 300.0, 500.0, 400.0), sides: 5, star_ratio: 1.0 };
    super::create_geometry(&mut typed, &geometry, craft::draw::ui::shape_drag::ShapeKind::Polygon, None).unwrap();
    let (a, b) = (active_shape(&drawn), active_shape(&typed));
    assert!(fills_box(&b) && near(tight(&b), [400.0, 300.0, 500.0, 400.0]), "{:?}", tight(&b));
    assert_eq!(a.path, b.path);
}
