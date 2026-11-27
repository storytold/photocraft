//! #12537: when the Paint Bucket can't fill (a hidden target layer, Lock All or Lock Image Pixels,
//! a group active), a click opens the reference application's alert with OK and fills nothing: no
//! History step, the target layer stays selected. Lock transparent pixels and Lock position still
//! fill; a pixel layer inside a group fills normally; a group's targeted layer mask is not refused
//! and takes today's step-without-fill. The Brush's alerts (Lock all only) and the other tools
//! stay as they are.

use egui::Modifiers;
use serde_json::json;

use super::brush_refusal_alert_tests::{active, alerts, app, steps, stroke};
use super::*;
use crate::canvas::{ToolEvent, tool_event};
use crate::control::{ControlRequest, Reply, handle};

/// A Paint Bucket click on the painted layer.
fn click(app: &mut PhotocraftApp) {
    tool_event(app, ToolEvent::Down { x: 30.0, y: 30.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 30.0, y: 30.0 }, Modifiers::NONE);
}

/// The active layer's pixel at the click point; a group has no surface, hence `None`.
fn pixel(app: &PhotocraftApp) -> Option<[f32; 4]> {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap())?.surface().map(|s| s.rgba(30, 30))
}

/// A named layer's pixel at the click point - the byte check when the active layer is a group.
fn child_rgba(app: &PhotocraftApp, id: photocraft_doc::LayerId) -> [f32; 4] {
    let st = app.session.active().unwrap();
    st.doc.layer(id).and_then(|l| l.surface()).map(|s| s.rgba(30, 30)).unwrap_or([0.0; 4])
}

/// The click opens one alert with `want` and changes nothing: the same pixels, no History step, the
/// same active layer. OK closes the alert.
fn refused(app: &mut PhotocraftApp, want: &str) {
    let before = (steps(app), pixel(app), active(app));
    click(app);
    assert_eq!(alerts(app), [want], "the alert, once");
    assert_eq!((steps(app), pixel(app), active(app)), before, "nothing filled, no History step, the target stays selected");
    let id = app.framework.dialogs.open_dialogs().first().map(|d| d.id).unwrap();
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(app, &ctx, id).unwrap();
    assert!(alerts(app).is_empty(), "OK closes the alert");
}

#[test]
fn a_hidden_target_layer() {
    let mut app = app(Tool::PaintBucket);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    refused(&mut app, "Could not use the paint bucket because the target layer is hidden.");
    assert!(app.ui.status.is_empty() || !app.ui.status_error, "no status-bar error as well: {:?}", app.ui.status);
    assert_eq!(
        photocraft_engine::hidden_target::HIDDEN,
        "Could not complete your request because the target layer is hidden.",
        "the engine's shared text stays as it is"
    );
}

#[test]
fn lock_all_and_lock_image_pixels() {
    // Both are the one "the layer is locked." alert in the reference application.
    for lock in ["all", "pixels"] {
        let mut bucket = app(Tool::PaintBucket);
        bucket.run("layer.lockLayers", json!({ lock: true })).unwrap();
        refused(&mut bucket, "Could not use the paint bucket because the layer is locked.");
    }
}

#[test]
fn a_group_is_active() {
    // The group holds the painted child layer, so its pixels are the byte check: a group has no
    // surface of its own and the group's own check would be vacuous.
    let mut app = app(Tool::PaintBucket);
    let child = app.session.active().unwrap().active_layer.unwrap();
    let red = child_rgba(&app, child);
    assert!(red[0] > 0.8 && red[3] > 0.99, "the child starts painted red: {red:?}");
    app.run("layer.groupLayers", json!({})).unwrap();
    // Once, with the measured wording: `press_refused` answers before `refuse_group` can add its
    // own alert.
    refused(&mut app, "Could not use the paint bucket because the pixels in a group cannot be edited without merging the group.");
    assert_eq!(child_rgba(&app, child), red, "the child layer's pixels inside the group stay as they were");
}

#[test]
fn a_group_inside_a_group_is_refused_once() {
    // Neighbour (pattern 3): the active inner group sits inside the outer group, which holds a
    // painted child; neither group has pixels of its own at the click, so the byte check is the
    // child's.
    let mut app = app(Tool::PaintBucket);
    app.run("layer.new.group", json!({})).unwrap();
    app.run("layer.new.group", json!({})).unwrap();
    let inner = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let child = app.session.active().unwrap().active_layer.unwrap();
    app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    app.run("layer.select", json!({"layer": inner.0})).unwrap();
    let st = app.session.active().unwrap();
    let path = st.doc.path_of(st.active_layer.unwrap()).unwrap();
    assert_eq!(path.len(), 2, "the active group sits inside the outer group: {path:?}");
    let red = child_rgba(&app, child);
    assert!(red[0] > 0.8 && red[3] > 0.99, "the child inside the groups starts painted red: {red:?}");
    refused(&mut app, "Could not use the paint bucket because the pixels in a group cannot be edited without merging the group.");
    assert_eq!(child_rgba(&app, child), red, "the child layer's pixels inside the groups stay as they were");
}

#[test]
fn the_alert_is_listed_and_confirmed_over_the_control_channel() {
    let ctx = egui::Context::default();
    let mut app = app(Tool::PaintBucket);
    app.run("layer.lockLayers", json!({"pixels": true})).unwrap();
    click(&mut app);
    let mut call = |method: &str, params: serde_json::Value| {
        let (req, _rx) = ControlRequest::new(method, params);
        match handle(&mut app, &ctx, &req) {
            Reply::Done(v) => v,
            _ => panic!("{method}: expected an immediate reply"),
        }
    };
    let inspect = call("ui.inspect", json!({}));
    let dialog = &inspect["result"]["dialogs"][0];
    assert_eq!(dialog["fields"]["message"], "Could not use the paint bucket because the layer is locked.", "{inspect}");
    assert_eq!(call("ui.dialog.confirm", json!({"dialog": dialog["id"]}))["ok"], true);
    assert!(alerts(&app).is_empty());
}

#[test]
fn a_click_on_a_visible_unlocked_layer_still_fills() {
    // The guard for the whole change: the painting path is untouched.
    let mut app = app(Tool::PaintBucket);
    let before = steps(&app);
    click(&mut app);
    assert!(alerts(&app).is_empty(), "{:?}", alerts(&app));
    assert_eq!(steps(&app), before + 1, "the fill was applied");
    let st = app.session.active().unwrap();
    let surface = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    for (x, y) in [(30, 30), (2, 2), (61, 61)] {
        let [r, g, b, a] = surface.rgba(x, y);
        assert!(a > 0.99 && r < 0.01 && g < 0.01 && b < 0.01, "the black foreground at ({x}, {y}): {:?}", [r, g, b, a]);
    }
}

#[test]
fn lock_transparent_pixels_still_fills_the_opaque_pixels() {
    let mut app = app(Tool::PaintBucket);
    app.run("layer.lockLayers", json!({"transparency": true})).unwrap();
    let before = steps(&app);
    click(&mut app);
    assert!(alerts(&app).is_empty(), "{:?}", alerts(&app));
    assert_eq!(steps(&app), before + 1);
    let [r, g, b, a] = pixel(&app).unwrap();
    assert!(a > 0.99 && r < 0.01 && g < 0.01 && b < 0.01, "the opaque region filled black: {:?}", [r, g, b, a]);
}

#[test]
fn lock_position_still_fills() {
    let mut app = app(Tool::PaintBucket);
    app.run("layer.lockLayers", json!({"position": true})).unwrap();
    let before = steps(&app);
    click(&mut app);
    assert!(alerts(&app).is_empty(), "{:?}", alerts(&app));
    assert_eq!(steps(&app), before + 1);
    let [r, g, b, a] = pixel(&app).unwrap();
    assert!(a > 0.99 && r < 0.01 && g < 0.01 && b < 0.01, "filled black: {:?}", [r, g, b, a]);
}

#[test]
fn a_pixel_layer_inside_a_group_fills_normally() {
    let mut app = app(Tool::PaintBucket);
    app.run("layer.new.group", json!({})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let st = app.session.active().unwrap();
    let id = st.active_layer.unwrap();
    let path = st.doc.path_of(id).unwrap();
    assert_eq!(path.len(), 2, "the new layer is a child of the group: {path:?}");
    assert!(st.doc.layer_at(&path[..1]).is_some_and(|l| l.is_group()), "its parent is a group");
    let before = steps(&app);
    click(&mut app);
    assert!(alerts(&app).is_empty(), "{:?}", alerts(&app));
    assert_eq!(steps(&app), before + 1, "the click filled the layer inside the group");
    let [r, g, b, a] = pixel(&app).unwrap();
    assert!(a > 0.99 && r < 0.01 && g < 0.01 && b < 0.01, "filled black: {:?}", [r, g, b, a]);
}

#[test]
fn a_group_s_targeted_mask_takes_a_step_that_fills_nothing() {
    // Not measured: keeps today's behaviour as a whole. Measured on `main` before and after this
    // change: the click opens no alert (the alert path only speaks for pixel targets), the engine
    // adds one "Paint Bucket" state, and its command answers `filled: true` while leaving the
    // all-white mask's samples as they are - all pinned here, so the name says exactly what today
    // is: a step, and no fill.
    let mut app = app(Tool::PaintBucket);
    app.run("layer.new.group", json!({})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.ui.mask_target = true;
    app.sync_mask_targets();
    assert_eq!(crate::canvas::paint_target(&app), json!("mask"), "the group's mask is targeted");
    let before = steps(&app);
    click(&mut app);
    assert!(alerts(&app).is_empty(), "no refusal alert on a mask target: {:?}", alerts(&app));
    assert_eq!(steps(&app), before + 1, "the engine's own step lands as today");
    let labels = app.run("document.inspect", json!({})).unwrap()["history"].as_array().cloned().unwrap_or_default();
    assert_eq!(labels.last(), Some(&json!("Paint Bucket")), "the step the click added carries the engine's own label");
    // The command's own answer, pinned once through the engine: it claims a fill and writes nothing.
    let reply = app.run("paint.bucket", json!({"x": 30, "y": 30, "target": "mask"})).unwrap();
    assert_eq!(reply["filled"], json!(true), "the engine reports filled: true, as on main");
    assert_eq!(steps(&app), before + 2, "and that call adds its step too");
    let st = app.session.active().unwrap();
    let mask = st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap();
    for (x, y) in [(30, 30), (2, 2), (61, 61)] {
        assert_eq!(mask.surface.sample_channel(x, y, 0), 1.0, "the mask samples stay white at ({x}, {y}) as on main");
    }
}

#[test]
fn the_brush_alerts_stay_as_they_are_and_the_other_tools_stay_quiet() {
    // The Brush (#11898): Lock all only, never Lock Image Pixels.
    let mut brush = app(Tool::Brush);
    brush.run("layer.lockLayers", json!({"all": true})).unwrap();
    stroke(&mut brush);
    assert_eq!(alerts(&brush), ["Could not use the brush tool because the layer is locked."], "the Brush's Lock all alert is unchanged");
    let mut brush = app(Tool::Brush);
    brush.run("layer.lockLayers", json!({"pixels": true})).unwrap();
    stroke(&mut brush);
    assert!(alerts(&brush).is_empty(), "the Brush keeps no alert for Lock Image Pixels: {:?}", alerts(&brush));
    // The Dodge tool has no layer alerts: a hidden target stays in the status bar.
    let mut dodge = app(Tool::Dodge);
    let id = active(&dodge);
    dodge.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    stroke(&mut dodge);
    assert!(alerts(&dodge).is_empty());
    assert_eq!(dodge.ui.status, photocraft_engine::hidden_target::HIDDEN);
}
