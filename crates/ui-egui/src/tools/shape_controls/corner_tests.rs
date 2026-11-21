//! The corner radius widgets of live shapes, driven through whole canvas passes: a drag on a widget runs one
//! `shape.edit` per changed layer under one key, which is one History step.

use egui::Modifiers;
use photocraft_doc::LayerContent;
use photocraft_doc::vector::LiveShape;
use serde_json::json;

use super::corner_targets;
use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

const NONE: Modifiers = Modifiers::NONE;

/// A 200 × 260 document at 100 % with a live rectangle at (40, 40), 120 × 80, radii 20. The radius is above
/// the widgets' inset, so a widget sits at the centre of its corner's arc: (60, 60) for the top left.
fn app(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 260})).unwrap();
    app.sync_views();
    app.run("shape.create", json!({"kind": "roundedRect", "rect": [40, 40, 120, 80], "radii": 20})).unwrap();
    app.ui.tool = tool;
    app.run("view.actualPixels", json!({})).unwrap();
    testing::frame(&mut app);
    app
}

fn live_of(app: &PhotocraftApp, id: photocraft_doc::LayerId) -> LiveShape {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => sh.live.clone().expect("a live shape"),
        _ => panic!("a shape layer"),
    }
}

fn radii_of(app: &PhotocraftApp, id: photocraft_doc::LayerId) -> [f64; 4] {
    match live_of(app, id) {
        LiveShape::Rect { radii, .. } => radii,
        other => panic!("a live rect, not {other:?}"),
    }
}

fn corner_radius_of(app: &PhotocraftApp, id: photocraft_doc::LayerId) -> f64 {
    match live_of(app, id) {
        LiveShape::Polygon { corner_radius, .. } => corner_radius,
        other => panic!("a live polygon, not {other:?}"),
    }
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn active(app: &PhotocraftApp) -> photocraft_doc::LayerId {
    app.session.active().unwrap().active_layer.unwrap()
}

/// A press at `from`, two moves and a release at `to`, as a mouse does it.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    testing::press(app, from, NONE);
    testing::move_to(app, [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0], NONE);
    testing::move_to(app, to, NONE);
    testing::release(app, to, NONE);
}

/// Live rectangles, bottom first: (20, 20) 60 × 40 radii 15, (110, 20) 70 × 50 radii 15, and (20, 120) 80 × 60
/// radii 0, the last one left unselected. Radii of 15 are above the widgets' inset: the top-left widgets sit at
/// (35, 35) and (125, 35).
fn three(tool: Tool) -> (Box<PhotocraftApp>, Vec<photocraft_doc::LayerId>) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 260})).unwrap();
    app.sync_views();
    let mut ids = Vec::new();
    for (rect, r) in [([20, 20, 60, 40], 15), ([110, 20, 70, 50], 15), ([20, 120, 80, 60], 0)] {
        app.run("shape.create", json!({"kind": "rect", "rect": rect, "radii": r})).unwrap();
        ids.push(active(&app));
    }
    app.run("layer.select", json!({"layer": ids[0].0})).unwrap();
    app.run("layer.select", json!({"layer": ids[1].0, "mode": "add"})).unwrap();
    app.ui.tool = tool;
    app.run("view.actualPixels", json!({})).unwrap();
    testing::frame(&mut app);
    (app, ids)
}

#[test]
fn dragging_a_widget_rounds_every_corner_in_one_step() {
    let mut app = app(Tool::PathSelection);
    let id = active(&app);
    let n = steps(&app);
    // Dragging the top-left widget 15 px along the diagonal adds 15 to the radius, on every corner.
    drag(&mut app, [60.0, 60.0], [75.0, 75.0]);
    assert_eq!(radii_of(&app, id), [35.0; 4]);
    assert_eq!(steps(&app), n + 1, "one history step");
    assert!(app.session.undo());
    assert_eq!(radii_of(&app, id), [20.0; 4]);
}

#[test]
fn one_widget_rounds_every_selected_rectangle_alike_in_one_step() {
    let (mut app, ids) = three(Tool::PathSelection);
    let n = steps(&app);
    // The second rectangle's top-left widget, dragged 10 px in each direction: every selected rectangle takes
    // radius 25, the first one capped at half its short side (20).
    drag(&mut app, [125.0, 35.0], [135.0, 45.0]);
    assert_eq!(radii_of(&app, ids[0]), [20.0; 4]);
    assert_eq!(radii_of(&app, ids[1]), [25.0; 4]);
    assert_eq!(radii_of(&app, ids[2]), [0.0; 4], "the unselected one is untouched");
    assert_eq!(steps(&app), n + 1, "one history step for both layers");
    assert!(app.session.undo());
    assert_eq!((radii_of(&app, ids[0]), radii_of(&app, ids[1])), ([15.0; 4], [15.0; 4]));
}

#[test]
fn the_move_tool_shows_and_drags_the_widgets_with_its_transform_controls() {
    let (mut app, ids) = three(Tool::Move);
    assert!(corner_targets(&app).is_empty(), "no widgets without Show Transform Controls");
    app.ui.tool_options.move_show_transform = true;
    testing::frame(&mut app);
    assert_eq!(corner_targets(&app).len(), 2);
    let before = app.session.active().unwrap().selected_layers().to_vec();
    let n = steps(&app);
    drag(&mut app, [125.0, 35.0], [130.0, 40.0]);
    assert_eq!(radii_of(&app, ids[1]), [20.0; 4]);
    assert_eq!(app.session.active().unwrap().selected_layers(), before, "the press neither moved nor reselected layers");
    assert_eq!(steps(&app), n + 1, "the widget, not the Move tool, took the press: one step");
}

#[test]
fn rectangles_and_triangles_selected_together_take_the_same_radius() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 260})).unwrap();
    app.sync_views();
    app.run("shape.create", json!({"kind": "polygon", "rect": [40, 40, 120, 120], "sides": 3, "cornerRadius": 12})).unwrap();
    let tri = active(&app);
    app.run("shape.create", json!({"kind": "rect", "rect": [10, 200, 100, 40], "radii": 15})).unwrap();
    let rect = active(&app);
    app.run("layer.select", json!({"layer": tri.0, "mode": "add"})).unwrap();
    app.ui.tool = Tool::PathSelection;
    app.run("view.actualPixels", json!({})).unwrap();
    testing::frame(&mut app);
    let n = steps(&app);
    // The rectangle's top-left widget (25, 215), 3 px in each direction: radius 18 on both shapes, as the
    // host's two command shapes (`radii` for the rectangle, `cornerRadius` for the polygon), in one step.
    drag(&mut app, [25.0, 215.0], [28.0, 218.0]);
    assert_eq!(radii_of(&app, rect), [18.0; 4]);
    assert!((corner_radius_of(&app, tri) - 18.0).abs() < 1e-9, "{}", corner_radius_of(&app, tri));
    assert_eq!(steps(&app), n + 1, "one history step for both layers");
}

#[test]
fn a_star_shows_widgets_at_its_outer_points_only() {
    // #13308: a star's inner points stay sharp, so they have no widget.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.run("shape.create", json!({"kind": "star", "rect": [40, 40, 120, 100], "sides": 5, "starRatio": 0.5, "cornerRadius": 6})).unwrap();
    app.ui.tool = Tool::PathSelection;
    let targets = corner_targets(&app);
    assert_eq!(targets.len(), 1);
    let target = &targets[0];
    assert!(!target.individual, "a star has one radius for all its points");
    assert_eq!(target.corners.len(), 5);
    let sharp = &photocraft_vector::shapes::polygon(40.0, 40.0, 120.0, 100.0, 5, 0.5).subpaths[0].knots;
    for (i, corner) in target.corners.iter().enumerate() {
        let v = sharp[2 * i].anchor;
        assert_eq!((corner.vertex.x, corner.vertex.y), (v.x, v.y), "corner {i} is outer point {i}");
    }
}
