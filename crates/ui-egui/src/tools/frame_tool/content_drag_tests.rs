//! #13878: a drag inside a frame that holds an image moves only the image, one "Move" step on
//! release, with the image active afterwards; the frame stays where it is.
use super::*;
use crate::canvas::tool_event;
use egui::Modifiers;

/// The issue's document: 600 × 400, white Background, Red (100 × 80 at 50,50), Blue (120 × 60 at
/// 300,200) and an empty layer on top; Red put in a frame by drawing 60..140 × 60..120 over it.
fn app() -> (PhotocraftApp, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400, "background": "white"})).unwrap();
    for (name, x, y, w, h, color) in [("Blue", 300, 200, 120, 60, "#0000ff"), ("Red", 50, 50, 100, 80, "#ff0000")] {
        app.run("layer.new.layer", json!({"name": name})).unwrap();
        app.run("select.rect", json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
        app.run("edit.fill", json!({"color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
    }
    let red = app.session.active().unwrap().active_layer.unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Frame;
    drag(&mut app, [60.0, 60.0], [140.0, 120.0]);
    let frame = frame_id(&app);
    assert!(app.session.active().unwrap().doc.layer(frame).unwrap().children().is_some_and(|c| !c.is_empty()), "the frame took Red");
    let _ = red;
    (app, frame)
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let m = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    tool_event(app, ToolEvent::Move { x: mid[0], y: mid[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

fn frame_id(app: &PhotocraftApp) -> LayerId {
    let st = app.session.active().unwrap();
    st.doc.walk().into_iter().find(|(_, _, l)| l.frame.is_some()).map(|(_, _, l)| l.id).expect("a frame")
}

fn frame_box(app: &PhotocraftApp, frame: LayerId) -> [i32; 4] {
    let b = app.session.active().unwrap().doc.layer(frame).unwrap().frame.as_ref().unwrap().bounds();
    [b.x0, b.y0, b.x1, b.y1]
}

fn content(app: &PhotocraftApp, frame: LayerId) -> (LayerId, [i32; 4]) {
    let st = app.session.active().unwrap();
    let child = st.doc.layer(frame).unwrap().children().unwrap()[0].clone();
    let b = photocraft_engine::transform_cmds::transform_frame(&st.doc, &child);
    (child.id, [b.x0, b.y0, b.x1, b.y1])
}

fn entries(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries()
}

#[test]
fn a_drag_inside_moves_the_image_alone_in_one_move_step() {
    let (mut app, frame) = app();
    let (image, before) = content(&app, frame);
    assert_eq!(before, [50, 50, 150, 130], "Red where it was drawn");
    let history = entries(&app);
    drag(&mut app, [100.0, 90.0], [150.0, 120.0]);
    let (_, after) = content(&app, frame);
    assert_eq!(after, [100, 80, 200, 160], "Red moved +50,+30");
    assert_eq!(frame_box(&app, frame), [60, 60, 140, 120], "the frame stayed");
    let now = entries(&app);
    assert_eq!(now.len(), history.len() + 1, "one step");
    assert_eq!(now.last().map(String::as_str), Some("Move"));
    assert!(app.ui.transform.is_none(), "no transform left pending");
    assert_eq!(app.session.active().unwrap().active_layer, Some(image), "the image is active");
    // Undo puts Red back in one step.
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(content(&app, frame).1, before);
    assert_eq!(frame_box(&app, frame), [60, 60, 140, 120]);
}

#[test]
fn during_the_drag_the_image_shows_where_it_goes() {
    let (mut app, frame) = app();
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 90.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 150.0, y: 120.0, pressure: 1.0 }, m);
    let preview = app.frame_preview.as_ref().expect("a preview").doc.clone();
    let (image, _) = content(&app, frame);
    let child = preview.layer(image).unwrap().clone();
    let b = photocraft_engine::transform_cmds::transform_frame(&preview, &child);
    assert_eq!([b.x0, b.y0, b.x1, b.y1], [100, 80, 200, 160]);
    assert_eq!(content(&app, frame).1, [50, 50, 150, 130], "nothing committed yet");
    tool_event(&mut app, ToolEvent::Up { x: 150.0, y: 120.0 }, m);
    assert!(app.frame_preview.is_none());
}

#[test]
fn a_border_drag_still_moves_the_frame_and_an_empty_frame_still_moves() {
    let (mut app, frame) = app();
    let (_, image) = content(&app, frame);
    // On the frame's left border: the frame itself, as today.
    drag(&mut app, [60.0, 90.0], [70.0, 90.0]);
    assert_eq!(frame_box(&app, frame)[0], 70, "the frame moved");
    assert_eq!(entries(&app).last().map(String::as_str), Some("Frame Properties"));
    let _ = image;
    // An empty frame elsewhere moves as a whole, as today.
    app.run("layer.select", json!({"layer": app.session.active().unwrap().doc.layers[0].id.0})).unwrap();
    drag(&mut app, [400.0, 40.0], [500.0, 120.0]);
    let empty = app
        .session
        .active()
        .unwrap()
        .doc
        .walk()
        .into_iter()
        .filter(|(_, _, l)| l.frame.is_some() && l.children().is_none_or(|c| c.is_empty()))
        .map(|(_, _, l)| l.id)
        .next()
        .expect("the empty frame");
    drag(&mut app, [450.0, 80.0], [460.0, 85.0]);
    assert_eq!(frame_box(&app, empty), [410, 45, 510, 125]);
    assert_eq!(entries(&app).last().map(String::as_str), Some("Move"));
}
