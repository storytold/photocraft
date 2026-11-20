//! Free Transform commit rules for clicks (#11100): a click outside the box on the image keeps the
//! box open, a click on the pasteboard commits.

use egui::Modifiers;
use photocraft_doc::LayerId;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};

/// Moves the open box 10 px right, reference point and corners together.
fn shift_right(app: &mut PhotocraftApp) {
    app.ui.transform.as_mut().expect("a box is open").edit_box(|b| *b = b.moved((10.0, 0.0).into()));
}

fn quad(app: &PhotocraftApp) -> [[f64; 2]; 4] {
    app.ui.transform.as_ref().expect("a box is open").session.quad().map(|p| [p.x, p.y])
}

/// A 200 × 200 document with a red block (x 20..60, y 20..60) on layer "Paint", Free Transform
/// open on it, and the box moved 10 px to the right as in the measurement.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    let paint = LayerId(app.run("layer.new.layer", json!({"name": "Paint"})).unwrap()["layer"].as_u64().unwrap());
    app.run("layer.select", json!({"layer": paint.0})).unwrap();
    app.run("select.rect", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let app_ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &app_ctx, "edit.freeTransform", json!({})).unwrap();
    shift_right(&mut app);
    app
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn click(app: &mut PhotocraftApp, p: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: p[0], y: p[1] }, Modifiers::NONE);
}

#[test]
fn a_click_outside_the_box_on_the_image_keeps_it_open() {
    let mut app = app();
    let (before, q) = (steps(&app), quad(&app));
    click(&mut app, [150.0, 150.0]);
    assert!(app.ui.transform.is_some(), "the box stays open");
    assert_eq!(quad(&app), q, "and unchanged");
    assert_eq!(steps(&app), before);
}

#[test]
fn a_click_on_the_pasteboard_commits_one_free_transform_step() {
    for p in [[-30.0, 100.0], [100.0, -30.0], [230.0, 100.0], [100.0, 230.0]] {
        let mut app = app();
        // This oracle addresses the canvas at these same four document points.
        // The floating task bar can cover them; use the real Window command to
        // expose the pasteboard without changing the click or bypassing its route.
        let app_ctx = app.ctx().clone();
        crate::menus::invoke(&mut app, &app_ctx, "window.panel.contextualTaskBar", json!({"on": false})).unwrap();
        assert!(!app.ui.panels.context_task_bar);
        crate::canvas::testing::shown(&mut app);
        let view = crate::canvas::testing::view(&app).expect("the canvas shows the original click point");
        let point = crate::canvas::testing::to_screen(&app, p);
        assert!(view.rect.contains(point), "the unchanged pasteboard point reaches the real canvas: {p:?}");
        assert!(crate::transform_task_bar::shown_rect(&app, &app_ctx).is_none(), "the bar cannot own this canvas click");
        let before = steps(&app);
        click(&mut app, p);
        assert!(app.ui.transform.is_none(), "{p:?}");
        assert_eq!(steps(&app), before + 1, "{p:?}");
        assert_eq!(app.session.active().unwrap().history.entries().last().map(ToString::to_string).as_deref(), Some("Free Transform"));
    }
}

/// #11100, measured in the reference application: a click outside the box on another layer's
/// pixels commits one Free Transform step, and the transformed layer stays the only selected one
/// (the click doesn't pick the layer under it).
#[test]
fn a_click_on_another_layers_pixels_commits_and_keeps_the_selection() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    // "Other" has a blue block at x 120..160, y 120..160; "Paint" a red one at x 20..60, y 20..60.
    for (name, at, color) in [("Other", 120, "#0000ff"), ("Paint", 20, "#ff0000")] {
        let id = app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap();
        app.run("layer.select", json!({"layer": id})).unwrap();
        app.run("select.rect", json!({"x": at, "y": at, "width": 40, "height": 40})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
    }
    let paint = app.session.active().unwrap().active_layer.unwrap();
    let app_ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &app_ctx, "edit.freeTransform", json!({})).unwrap();
    shift_right(&mut app);
    let before = steps(&app);
    click(&mut app, [140.0, 140.0]);
    assert!(app.ui.transform.is_none(), "committed");
    assert_eq!(steps(&app), before + 1);
    assert_eq!(app.session.active().unwrap().history.entries().last().map(ToString::to_string).as_deref(), Some("Free Transform"));
    assert_eq!(app.session.active().unwrap().selected_layers(), vec![paint], "the selection doesn't change");
}

/// A layer linked to the transformed one after the box opened moves with the commit
/// (`edit.transform` takes the link group along), so a click on its pixels is not a click on
/// another layer: the box stays open.
#[test]
fn a_click_on_a_layer_linked_after_the_box_opened_keeps_it_open() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    let mut ids = Vec::new();
    for (name, at, color) in [("Other", 120, "#0000ff"), ("Paint", 20, "#ff0000")] {
        let id = app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap();
        app.run("layer.select", json!({"layer": id})).unwrap();
        app.run("select.rect", json!({"x": at, "y": at, "width": 40, "height": 40})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        ids.push(LayerId(id));
    }
    let app_ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &app_ctx, "edit.freeTransform", json!({})).unwrap();
    shift_right(&mut app);
    app.session
        .edit("Link Layers", |doc, _| {
            for id in &ids {
                doc.layer_mut(*id).unwrap().link_group = Some(7);
            }
            Ok(())
        })
        .unwrap();
    click(&mut app, [140.0, 140.0]);
    assert!(app.ui.transform.is_some(), "the linked layer moves with the box: the click doesn't commit");
}

/// Only the frontmost layer under the click counts: where a grouped layer (not measured, so the
/// box stays open there) covers a top-level layer's pixels, the click doesn't commit through the
/// covered layer.
#[test]
fn a_click_on_a_grouped_layer_covering_another_layer_keeps_the_box_open() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    // "Paint" has a red block at x 20..60, y 20..60; "Other" a blue one at x 120..160, y 120..160,
    // covered by a green one on "Cover", which then goes into a group.
    let mut ids = Vec::new();
    for (name, at, color) in [("Paint", 20, "#ff0000"), ("Other", 120, "#0000ff"), ("Cover", 120, "#00ff00")] {
        let id = app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap();
        app.run("layer.select", json!({"layer": id})).unwrap();
        app.run("select.rect", json!({"x": at, "y": at, "width": 40, "height": 40})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        ids.push(id);
    }
    app.run("layer.groupLayers", json!({})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert!(doc.layers.iter().all(|l| l.id.0 != ids[2]), "Cover is grouped");
    assert!(doc.layers.iter().any(|l| l.id.0 == ids[1]), "Other is top-level");
    app.run("layer.select", json!({"layer": ids[0]})).unwrap();
    let app_ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &app_ctx, "edit.freeTransform", json!({})).unwrap();
    shift_right(&mut app);
    let before = steps(&app);
    click(&mut app, [140.0, 140.0]);
    assert!(app.ui.transform.is_some(), "the box stays open");
    assert_eq!(steps(&app), before);
}
