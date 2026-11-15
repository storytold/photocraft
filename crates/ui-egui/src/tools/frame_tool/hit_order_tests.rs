//! #13882: a Frame tool drag that starts on a shape lying above an empty frame moves the shape, one
//! "Move" step, and the frame stays; elsewhere in the frame the drag moves the frame, as before.
use super::*;
use crate::canvas::tool_event;
use egui::Modifiers;

/// The issue's document: 600 × 400 white, Red and Blue pixel layers, an empty frame at 430..520 ×
/// 100..200, and a solid shape 450..550 × 50..150 drawn above it, covering part of it.
fn app() -> (PhotocraftApp, LayerId, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400, "background": "white"})).unwrap();
    for (name, x, y, w, h, color) in [("Red", 50, 50, 100, 80, "#ff0000"), ("Blue", 300, 200, 120, 60, "#0000ff")] {
        app.run("layer.new.layer", json!({"name": name})).unwrap();
        app.run("select.rect", json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
        app.run("edit.fill", json!({"color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
    }
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Frame;
    drag(&mut app, [430.0, 100.0], [520.0, 200.0]);
    let frame = layer_where(&app, |l| l.frame.is_some());
    assert!(app.session.active().unwrap().doc.layer(frame).unwrap().children().is_none_or(<[Layer]>::is_empty), "an empty frame");
    app.run("shape.create", json!({"rect": [450, 50, 100, 100], "fill": "#22aa44"})).unwrap();
    let shape = layer_where(&app, |l| matches!(l.content, LayerContent::Shape(_)));
    (app, frame, shape)
}

fn layer_where(app: &PhotocraftApp, f: impl Fn(&Layer) -> bool) -> LayerId {
    app.session.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| f(l)).map(|(_, _, l)| l.id).expect("the layer")
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let m = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

fn frame_box(app: &PhotocraftApp, frame: LayerId) -> [i32; 4] {
    let b = app.session.active().unwrap().doc.layer(frame).unwrap().frame.as_ref().unwrap().bounds();
    [b.x0, b.y0, b.x1, b.y1]
}

fn shape_box(app: &PhotocraftApp, shape: LayerId) -> [f64; 4] {
    let LayerContent::Shape(sh) = &app.session.active().unwrap().doc.layer(shape).unwrap().content else { panic!("a shape") };
    let (x0, y0, x1, y1) = sh.path.control_bounds().unwrap();
    [x0, y0, x1, y1]
}

fn entries(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries()
}

#[test]
fn a_drag_on_the_shape_above_the_frame_moves_the_shape() {
    let (mut app, frame, shape) = app();
    assert_eq!(shape_box(&app, shape), [450.0, 50.0, 550.0, 150.0]);
    let history = entries(&app);
    // From a point inside the frame that the shape covers.
    drag(&mut app, [480.0, 120.0], [520.0, 140.0]);
    assert_eq!(shape_box(&app, shape), [490.0, 70.0, 590.0, 170.0], "+40,+20");
    assert_eq!(frame_box(&app, frame), [430, 100, 520, 200], "the frame stayed");
    let now = entries(&app);
    assert_eq!(now.len(), history.len() + 1);
    assert_eq!(now.last().map(String::as_str), Some("Move"));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(shape_box(&app, shape), [450.0, 50.0, 550.0, 150.0], "Undo, one step");
    assert_eq!(entries(&app), history);
}

#[test]
fn elsewhere_in_the_frame_the_drag_moves_the_frame() {
    let (mut app, frame, shape) = app();
    // Inside the frame, below the shape's bottom edge.
    drag(&mut app, [470.0, 180.0], [480.0, 185.0]);
    assert_eq!(frame_box(&app, frame), [440, 105, 530, 205]);
    assert_eq!(shape_box(&app, shape), [450.0, 50.0, 550.0, 150.0], "the shape stayed");
    assert_eq!(entries(&app).last().map(String::as_str), Some("Move"));
}

/// CodeRabbit on #15174: with Snap on, a drag that starts on the shape above an empty frame moves the shape by
/// the pointer's offset. The frame's bounds are not a snap gesture for it: dragging 8 px left would bring the
/// frame's left edge (430 - 8 = 422) onto Blue's right edge (420), and the shape would have moved 10.
#[test]
fn the_shape_above_the_frame_moves_by_the_drag_offset_with_snap_on() {
    let (mut app, frame, shape) = app();
    app.ui.views[0].zoom = 1.0;
    app.ui.extras.snap = true;
    drag(&mut app, [480.0, 120.0], [472.0, 120.0]);
    assert_eq!(shape_box(&app, shape), [442.0, 50.0, 542.0, 150.0], "moved -8, not snapped to -10");
    assert_eq!(frame_box(&app, frame), [430, 100, 520, 200], "the frame stayed");
}
