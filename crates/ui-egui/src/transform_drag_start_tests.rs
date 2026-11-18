//! Free Transform presses on an open box (#11103): clicks keep their rules. A click on the box
//! moves nothing, a jittery click outside it on the image leaves it as it was, a double-click on it
//! commits. The box's drag start and the preview's mip levels are the framework's (`TransformBox`,
//! `PreviewTextures`).

use egui::Modifiers;
use photocraft_doc::LayerId;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

/// An 800 × 600 document with a red block (x 100..300, y 100..300) on layer "Paint" and Free
/// Transform open on it.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Move;
    let paint = LayerId(app.run("layer.new.layer", json!({"name": "Paint"})).unwrap()["layer"].as_u64().unwrap());
    app.run("layer.select", json!({"layer": paint.0})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let app_ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &app_ctx, "edit.freeTransform", json!({})).unwrap();
    testing::frame(&mut app);
    assert!(app.ui.transform.is_some(), "Free Transform is open");
    app
}

fn quad(app: &PhotocraftApp) -> [[f64; 2]; 4] {
    app.ui.transform.as_ref().expect("the box is open").quad_array()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn a_click_on_the_box_moves_nothing_and_keeps_it_open() {
    let mut app = app();
    let (start, before) = (quad(&app), steps(&app));
    testing::click(&mut app, [200.0, 200.0], Modifiers::NONE);
    assert!(app.ui.transform.is_some());
    assert_eq!(quad(&app), start);
    assert_eq!(steps(&app), before);
}

#[test]
fn a_jittery_click_outside_the_box_on_the_image_leaves_it_as_it_was() {
    let mut app = app();
    let pivot = |app: &PhotocraftApp| app.ui.transform.as_ref().map(|t| t.session.pivot());
    let (start, at) = (quad(&app), pivot(&app));
    // Only the Background has pixels here (#11100): the box stays open.
    testing::press(&mut app, [600.0, 450.0], Modifiers::NONE);
    testing::move_to(&mut app, [601.0, 450.0], Modifiers::NONE);
    testing::release(&mut app, [601.0, 450.0], Modifiers::NONE);
    assert!(app.ui.transform.is_some(), "the box stays open");
    assert_eq!((quad(&app), pivot(&app)), (start, at), "the jitter's turn is put back");
}

#[test]
fn a_double_click_on_the_box_still_commits() {
    let mut app = app();
    // Moved 10 px first: committing an unchanged box adds no History step.
    app.ui.transform.as_mut().unwrap().edit_box(|b| *b = b.moved((10.0, 0.0).into()));
    testing::frame(&mut app);
    let before = steps(&app);
    testing::double_click(&mut app, [200.0, 200.0], Modifiers::NONE);
    testing::frame(&mut app);
    assert!(app.ui.transform.is_none(), "committed");
    assert_eq!(steps(&app), before + 1);
}
