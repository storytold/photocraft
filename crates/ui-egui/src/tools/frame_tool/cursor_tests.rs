//! The Frame tool's cursor (#13883): the standard arrow over a frame, the tool's crosshair on
//! empty canvas, and the resize cursors on a selected frame's handles. Drives the real app.

use super::*;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Frame;
    app
}

/// A 100 x 80 frame at (20, 30), selected and active.
fn app_with_empty_frame() -> Box<PhotocraftApp> {
    let mut app = app();
    app.run("layer.new.frame", json!({"x": 20, "y": 30, "width": 100, "height": 80})).unwrap();
    app
}

/// Handle slop in document pixels (the canvas uses its screen slop at the current zoom).
const REACH: f64 = 8.0;

#[test]
fn the_cursor_is_the_arrow_over_a_frame_and_the_crosshair_on_empty_canvas() {
    let app = app_with_empty_frame();
    assert_eq!(cursor_icon(&app, [70., 70.], REACH), Some(CursorIcon::Default), "inside the frame");
    assert_eq!(cursor_icon(&app, [20., 30.], REACH), Some(CursorIcon::ResizeNwSe), "top-left handle keeps its resize cursor");
    assert_eq!(cursor_icon(&app, [150., 150.], REACH), None, "empty canvas: no icon, so the tool's crosshair shows");
}

#[test]
fn a_frame_with_content_also_shows_the_arrow() {
    let mut app = app();
    let photo = app.run("layer.new.layer", json!({"name": "Photo"})).unwrap()["layer"].as_u64().unwrap();
    app.run("edit.fill", json!({"contents": "black"})).unwrap();
    app.run("layer.new.frame", json!({"x": 20, "y": 30, "width": 100, "height": 80, "layer": photo})).unwrap();
    assert_eq!(cursor_icon(&app, [70., 70.], REACH), Some(CursorIcon::Default), "a frame holding an image");
}

#[test]
fn an_unselected_frame_still_shows_the_arrow() {
    let mut app = app_with_empty_frame();
    let background = {
        let st = app.session.active().unwrap();
        st.doc.layers.iter().find(|l| st.doc.is_background(l.id)).unwrap().id.0
    };
    app.run("layer.select", json!({"layer": background})).unwrap();
    assert_eq!(cursor_icon(&app, [70., 70.], REACH), Some(CursorIcon::Default), "the frame is not the active layer");
}

#[test]
fn a_quarter_turned_frame_uses_the_edges_on_screen_direction() {
    let mut app = app_with_empty_frame();
    let frame = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.frame.set", json!({"layer": frame.0, "transform": [0, 100, -80, 0, 100, 30]})).unwrap();
    assert_eq!(cursor_icon(&app, [100., 80.], REACH), Some(CursorIcon::ResizeHorizontal), "the quarter-turned frame's top edge runs vertically");
    assert_eq!(cursor_icon(&app, [100., 30.], REACH), Some(CursorIcon::ResizeNeSw), "the quarter-turned frame's corner uses the on-screen diagonal");
}

#[test]
fn hidden_and_locked_frames_do_not_show_resize_cursors() {
    let mut app = app_with_empty_frame();
    let frame = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.setProps", json!({"layer": frame.0, "visible": false})).unwrap();
    assert_eq!(cursor_icon(&app, [20., 30.], REACH), None, "a hidden frame's handles are gone");

    app.run("layer.setProps", json!({"layer": frame.0, "visible": true})).unwrap();
    app.session
        .edit("lock", |doc, _| {
            doc.layer_mut(frame).unwrap().locks.position = true;
            Ok(())
        })
        .unwrap();
    assert_eq!(cursor_icon(&app, [20., 30.], REACH), Some(CursorIcon::Default), "a locked frame shows the arrow, not a resize cursor");
}

#[test]
fn no_document_has_no_cursor_and_does_not_panic() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert_eq!(cursor_icon(&app, [10., 10.], REACH), None);
}
