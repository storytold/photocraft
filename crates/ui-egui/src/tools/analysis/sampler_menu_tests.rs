//! #11733: the Color Sampler's right-click menu cannot act on a sampler that is gone.

use serde_json::json;

use crate::canvas_tool_menu::SAMPLER_DELETE;
use crate::{PhotocraftApp, Tool};

/// A 200 x 100 document with the Color Sampler and samplers at (20, 20) and (120, 60).
fn app() -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("view.colorSamplers.add", json!({"x": 20, "y": 20})).unwrap();
    app.run("view.colorSamplers.add", json!({"x": 120, "y": 60})).unwrap();
    app.ui.tool = Tool::ColorSampler;
    let ctx = app.ctx().clone();
    (app, ctx)
}

/// A right-click at document point (x, y) through the window host, as a mouse sends it.
fn right_click(app: &mut PhotocraftApp, x: f64, y: f64) {
    crate::canvas::testing::secondary_click(app, [x, y], egui::Modifiers::NONE);
}

fn samplers(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    app.session.active().unwrap().color_samplers.clone()
}

/// Delete never removes a different sampler when the list changes while the menu is open: one
/// deleted meanwhile, or another document with a sampler at the same index and spot.
#[test]
fn delete_never_removes_another_sampler_after_the_list_changes_under_the_menu() {
    let (mut app, ctx) = app();
    app.run("view.colorSamplers.add", json!({"x": 180, "y": 90})).unwrap();
    right_click(&mut app, 120.0, 60.0);
    assert_eq!(app.ui.canvas_tool_menu.as_ref().and_then(|m| m.sampler), Some(1));
    // An agent deletes #1 while the menu is open: index 1 now names the old #3.
    app.run("view.colorSamplers.delete", json!({"index": 0})).unwrap();
    crate::canvas_tool_menu::close_stale(&mut app);
    assert!(app.ui.canvas_tool_menu.is_none(), "the menu's sampler moved, so the menu closes");
    crate::canvas_tool_menu::choose(&mut app, &ctx, SAMPLER_DELETE);
    assert_eq!(samplers(&app), [[120.5, 60.5], [180.5, 90.5]], "no other sampler is deleted");

    // Another document with a sampler at the same index and spot: still not this menu's sampler.
    right_click(&mut app, 180.0, 90.0);
    assert_eq!(app.ui.canvas_tool_menu.as_ref().and_then(|m| m.sampler), Some(1));
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    for (x, y) in [(20, 20), (180, 90)] {
        app.run("view.colorSamplers.add", json!({"x": x, "y": y})).unwrap();
    }
    crate::canvas_tool_menu::close_stale(&mut app);
    crate::canvas_tool_menu::choose(&mut app, &ctx, SAMPLER_DELETE);
    assert_eq!(samplers(&app).len(), 2, "another document's sampler is not deleted");
}

/// A stale index, a non-finite point, or no document at all: `sampler_at` answers `None`, and a
/// Delete then changes nothing and never panics.
#[test]
fn a_stale_sampler_or_odd_point_fails_gracefully() {
    let (mut app, ctx) = app();
    app.run("view.colorSamplers.clear", json!({})).unwrap();
    app.run("view.colorSamplers.add", json!({"x": 5, "y": 5})).unwrap();
    assert_eq!(super::sampler_at(&app, [120.0, 60.0]), None, "the sampler at that spot is gone");
    for p in [[f64::NAN, 5.0], [f64::INFINITY, 5.0], [5.0, f64::NEG_INFINITY]] {
        assert_eq!(super::sampler_at(&app, p), None, "{p:?}");
    }
    crate::canvas_tool_menu::choose(&mut app, &ctx, SAMPLER_DELETE);
    assert_eq!(samplers(&app), [[5.5, 5.5]], "Delete with no open menu removes nothing");

    let mut empty = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    empty.ui.tool = Tool::ColorSampler;
    assert_eq!(super::sampler_at(&empty, [1.0, 1.0]), None);
    if crate::canvas_tool_menu::open_at(&mut empty, Tool::ColorSampler, Default::default(), egui::pos2(10.0, 10.0), [1.0, 1.0], None) {
        crate::canvas_tool_menu::choose(&mut empty, &ctx, SAMPLER_DELETE);
    }
    assert!(empty.session.active().is_none());
}
