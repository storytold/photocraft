//! #11805: a right-click on the image during a Perspective Crop session opens a menu with Crop
//! and Cancel. Crop commits as Enter does, Cancel clears the quad as Esc does; with no quad the
//! right-click keeps today's behaviour.

use serde_json::{Value, json};

use super::*;
use crate::control::{ControlRequest, Outcome, handle};

const QUAD: [[f64; 2]; 4] = [[20.0, 20.0], [180.0, 20.0], [180.0, 130.0], [20.0, 130.0]];

/// A 200 × 150 document with the Perspective Crop tool.
fn app() -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PerspectiveCrop;
    (app, egui::Context::default())
}

fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (req, _rx) = ControlRequest::new(method, params);
    match handle(app, ctx, &req) {
        Outcome::Done(v) => v,
        _ => panic!("{method}: expected an immediate reply"),
    }
}

fn right_click(app: &mut PhotocraftApp, ctx: &egui::Context, x: f64, y: f64) {
    let r = call(app, ctx, "ui.pointer", json!({"button": "secondary", "events": [{"kind": "down", "x": x, "y": y}, {"kind": "up", "x": x, "y": y}]}));
    assert_eq!(r["ok"], true, "{r}");
}

fn listed(app: &mut PhotocraftApp, ctx: &egui::Context) -> Option<Vec<(String, bool)>> {
    let inspected = call(app, ctx, "ui.inspect", json!({}));
    let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array)?.clone();
    Some(entries.iter().map(|e| (e["id"].as_str().unwrap_or("―").to_string(), e["enabled"].as_bool().unwrap_or(false))).collect())
}

fn size(app: &PhotocraftApp) -> (u32, u32) {
    let s = app.session.active().unwrap().doc.size;
    (s.width, s.height)
}

#[test]
fn with_a_pending_quad_the_menu_is_crop_then_cancel() {
    let (mut app, ctx) = app();
    app.ui.perspective_crop = Some(QUAD);
    right_click(&mut app, &ctx, 100.0, 70.0);
    assert_eq!(listed(&mut app, &ctx), Some(vec![(PCROP_COMMIT.to_string(), true), (PCROP_CANCEL.to_string(), true)]), "exactly two entries, no separator");
    let labels: Vec<&str> = rows(app.ui.canvas_tool_menu.as_ref().unwrap()).iter().flatten().map(|(l, _)| *l).collect();
    assert_eq!(labels, ["Crop", "Cancel"]);
}

#[test]
fn crop_commits_as_enter_does() {
    // The same quad committed with Enter's `commit` and with the menu's Crop.
    let (mut by_enter, _) = app();
    by_enter.ui.perspective_crop = Some(QUAD);
    crate::perspective_crop_ui::commit(&mut by_enter);
    let (mut app, ctx) = app();
    app.ui.perspective_crop = Some(QUAD);
    let steps = app.session.active().unwrap().history.past_len();
    right_click(&mut app, &ctx, 100.0, 70.0);
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": PCROP_COMMIT}))["ok"], true);
    assert_eq!(size(&app), size(&by_enter), "the same crop as Enter");
    assert_ne!(size(&app), (200, 150), "it cropped");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "one undo step");
    assert!(app.ui.perspective_crop.is_none(), "the session ended");
}

#[test]
fn cancel_clears_the_quad_as_esc_does() {
    let (mut app, ctx) = app();
    app.ui.perspective_crop = Some(QUAD);
    let steps = app.session.active().unwrap().history.past_len();
    right_click(&mut app, &ctx, 100.0, 70.0);
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": PCROP_CANCEL}))["ok"], true);
    assert!(app.ui.perspective_crop.is_none());
    assert_eq!(size(&app), (200, 150), "document unchanged");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}

#[test]
fn without_a_quad_the_right_click_keeps_todays_behaviour() {
    let (mut app, ctx) = app();
    right_click(&mut app, &ctx, 100.0, 70.0);
    assert_eq!(listed(&mut app, &ctx), None, "no menu with no quad");
    // Corners still being clicked (not measured): no menu either.
    app.pcrop.clicks = vec![[10.0, 10.0], [90.0, 12.0]];
    right_click(&mut app, &ctx, 100.0, 70.0);
    assert_eq!(listed(&mut app, &ctx), None);
    assert!(!open_perspective_crop(&mut app, &ctx, Tool::Crop, [10.0, 10.0]), "only the Perspective Crop tool");
}

#[test]
fn crop_after_the_document_changed_crops_nothing() {
    let (mut app, ctx) = app();
    app.ui.perspective_crop = Some(QUAD);
    right_click(&mut app, &ctx, 100.0, 70.0);
    // Another document becomes active before the pick: its stale quad is dropped, nothing cropped.
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    choose(&mut app, &ctx, PCROP_COMMIT);
    assert_eq!(size(&app), (64, 48));
    assert!(app.ui.perspective_crop.is_none(), "the stale quad is dropped");
}

#[test]
fn crop_with_no_document_left_is_no_panic() {
    let (mut empty, ctx2) = app();
    empty.ui.perspective_crop = Some(QUAD);
    assert!(open_perspective_crop(&mut empty, &ctx2, Tool::PerspectiveCrop, [10.0, 10.0]));
    empty.run("file.close", json!({})).unwrap();
    choose(&mut empty, &ctx2, PCROP_COMMIT);
}
