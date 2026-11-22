//! #13314: a live polygon shows one corner radius widget, below its top vertex (for an even side count, at the
//! first vertex of the flat top edge, inside the shape), and its box shows the centre mark. Measured against the
//! reference application: a 5-sided polygon dragged from 400,300 to 600,400 has one widget below the top vertex
//! and a centre mark; a rectangle keeps a widget in each of its four corners.

use egui::Modifiers;
use photocraft_doc::vector::LiveShape;
use photocraft_doc::{LayerContent, LayerId};
use photocraft_vector::shapes;
use serde_json::json;

use super::{centre_mark, corner_targets};
use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

const NONE: Modifiers = Modifiers::NONE;

/// A 300 × 200 document at 100 % with one live polygon of `sides` sides filling the box (50, 40), 200 × 100,
/// corner radius `radius` (the Polygon tool's geometry, `fillsBox`), and the Polygon tool active.
fn app(sides: u32, radius: f64) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.sync_views();
    app.run("shape.create", json!({"kind": "polygon", "rect": [50, 40, 200, 100], "sides": sides, "cornerRadius": radius, "fillsBox": true})).unwrap();
    app.ui.tool = Tool::Polygon;
    app.run("view.actualPixels", json!({})).unwrap();
    testing::frame(&mut app);
    app
}

fn active(app: &PhotocraftApp) -> LayerId {
    app.session.active().unwrap().active_layer.unwrap()
}

fn live_of(app: &PhotocraftApp, id: LayerId) -> LiveShape {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => sh.live.clone().expect("a live shape"),
        _ => panic!("a shape layer"),
    }
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

/// The sharp vertices of the layer's live polygon, clockwise from the first one.
fn vertices(app: &PhotocraftApp, id: LayerId) -> Vec<[f64; 2]> {
    let LiveShape::Polygon { rect, sides, star_ratio, fills_box, .. } = live_of(app, id) else { panic!("a live polygon") };
    let sharp = shapes::regular_polygon(rect[0], rect[1], rect[2], rect[3], sides, star_ratio, fills_box);
    sharp.subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect()
}

/// The one widget corner of the only target: its vertex, and the arc centre for the target's radius (where the
/// widget sits once the radius is above the widgets' inset).
fn the_widget(app: &PhotocraftApp) -> ([f64; 2], [f64; 2]) {
    let targets = corner_targets(app);
    assert_eq!(targets.len(), 1, "one live shape selected");
    let corners = &targets[0].corners;
    assert_eq!(corners.len(), 1, "one widget, not one per vertex: {corners:?}");
    let c = &corners[0];
    let f = shapes::fillet([c.prev.x, c.prev.y], [c.vertex.x, c.vertex.y], [c.next.x, c.next.y], c.radius).expect("a rounded vertex");
    ([c.vertex.x, c.vertex.y], f.centre)
}

#[test]
fn a_five_sided_polygon_has_one_widget_below_its_top_vertex() {
    let app = app(5, 20.0);
    let (vertex, at) = the_widget(&app);
    assert_eq!(vertex, [150.0, 40.0], "the top vertex, at the top centre of the box");
    assert_eq!(vertex, vertices(&app, active(&app))[0]);
    assert!((at[0] - 150.0).abs() < 1e-9 && at[1] > 40.0, "the widget sits straight below the top vertex: {at:?}");
    assert!(!targets_individual(&app), "one radius for every vertex");
}

#[test]
fn even_side_counts_have_one_widget_inside_near_the_top() {
    for sides in [4, 6, 8] {
        let app = app(sides, 20.0);
        let (vertex, at) = the_widget(&app);
        assert_eq!(vertex, vertices(&app, active(&app))[0], "{sides} sides");
        assert!((vertex[1] - 40.0).abs() < 1e-9, "{sides} sides: a vertex of the flat top edge, {vertex:?}");
        assert!(at[1] > 40.0 && at[1] < 90.0, "{sides} sides: inside the shape, in its upper half: {at:?}");
        assert!(at[0] > 50.0 && at[0] < 250.0, "{sides} sides: {at:?}");
    }
}

#[test]
fn dragging_the_one_widget_rounds_every_corner_in_one_step() {
    let mut app = app(5, 20.0);
    let id = active(&app);
    let n = steps(&app);
    let knots = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => sh.path.subpaths[0].knots.len(),
        _ => panic!("a shape layer"),
    };
    assert_eq!(knots(&app), 10, "every vertex rounded: two knots each");
    let (_, at) = the_widget(&app);
    // Straight down, along the top vertex's bisector: the radius grows.
    testing::press(&mut app, at, NONE);
    testing::move_to(&mut app, [at[0], at[1] + 4.0], NONE);
    testing::move_to(&mut app, [at[0], at[1] + 8.0], NONE);
    testing::release(&mut app, [at[0], at[1] + 8.0], NONE);
    let LiveShape::Polygon { corner_radius, .. } = live_of(&app, id) else { panic!("a live polygon") };
    assert!(corner_radius > 20.0, "the radius grew: {corner_radius}");
    assert_eq!(knots(&app), 10, "every vertex still rounded, with the one shared radius");
    assert_eq!(steps(&app), n + 1, "one history step");
    assert!(app.session.undo());
    let LiveShape::Polygon { corner_radius, .. } = live_of(&app, id) else { panic!("a live polygon") };
    assert_eq!(corner_radius, 20.0);
}

#[test]
fn a_rectangle_keeps_its_four_widgets() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.sync_views();
    app.run("shape.create", json!({"kind": "rect", "rect": [50, 40, 200, 100], "radii": 10})).unwrap();
    app.ui.tool = Tool::Rectangle;
    let targets = corner_targets(&app);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].corners.len(), 4);
    assert!(targets[0].individual);
}

#[test]
fn the_polygon_tool_marks_the_box_centre() {
    assert!(centre_mark(Tool::Polygon), "the Polygon tool's box shows its centre");
    assert!(centre_mark(Tool::EllipseShape), "the Ellipse tool's still does");
    for tool in [Tool::Rectangle, Tool::PathSelection, Tool::Triangle, Tool::Star, Tool::Move] {
        assert!(!centre_mark(tool), "{tool:?} keeps its box without a centre mark");
    }
}

fn targets_individual(app: &PhotocraftApp) -> bool {
    corner_targets(app).iter().any(|t| t.individual)
}
