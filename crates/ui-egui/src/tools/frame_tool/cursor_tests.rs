//! The Frame tool's cursor (#13883): the standard arrow over a frame, the tool's crosshair on
//! empty canvas, and the resize cursors on a selected frame's handles. Drives the real app.

use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app
}

/// A 100 x 80 frame at (20, 30), selected and active.
fn app_with_empty_frame() -> PhotocraftApp {
    let mut app = app();
    app.run("layer.new.frame", json!({"x": 20, "y": 30, "width": 100, "height": 80})).unwrap();
    app
}

#[test]
fn the_cursor_is_the_arrow_over_a_frame_and_the_crosshair_on_empty_canvas() {
    let app = app_with_empty_frame();
    assert_eq!(cursor(&app, [70.0, 70.0]), Some(CursorIcon::Default), "inside the frame");
    assert_eq!(cursor(&app, [20.0, 30.0]), Some(CursorIcon::ResizeNwSe), "top-left handle keeps its resize cursor");
    assert_eq!(cursor(&app, [300.0, 250.0]), None, "empty canvas: no icon, so the tool's crosshair shows");
}

#[test]
fn a_frame_with_content_also_shows_the_arrow() {
    let mut app = app();
    let photo = app.run("layer.new.layer", json!({"name": "Photo"})).unwrap()["layer"].as_u64().unwrap();
    app.run("edit.fill", json!({"contents": "black"})).unwrap();
    app.run("layer.new.frame", json!({"x": 20, "y": 30, "width": 100, "height": 80, "layer": photo})).unwrap();
    assert_eq!(cursor(&app, [70.0, 70.0]), Some(CursorIcon::Default), "a frame holding an image");
}

#[test]
fn an_unselected_frame_still_shows_the_arrow() {
    let mut app = app_with_empty_frame();
    let background = {
        let st = app.session.active().unwrap();
        st.doc.layers.iter().find(|l| st.doc.is_background(l.id)).unwrap().id.0
    };
    app.run("layer.select", json!({"layer": background})).unwrap();
    assert_eq!(cursor(&app, [70.0, 70.0]), Some(CursorIcon::Default), "the frame is not the active layer");
}

#[test]
fn no_document_has_no_cursor_and_does_not_panic() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert_eq!(cursor(&app, [10.0, 10.0]), None);
}
