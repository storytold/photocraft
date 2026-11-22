//! #11733: the Color Sampler's right-click menu cannot act on a sampler that is gone.

use serde_json::json;

use crate::canvas_tool_menu::SAMPLER_DELETE;
use crate::{PhotocraftApp, Tool};

/// A 200 x 100 document with the Color Sampler and samplers at (20, 20) and (120, 60).
fn app() -> (Box<PhotocraftApp>, egui::Context) {
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

/// The mouse opens the same ordered menu the control channel sees, and only a hit can be deleted.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_sampler_menu_only_deletes_the_point_under_the_pointer() {
    use crate::canvas::testing::{control, key};

    let (mut app, _) = app();
    app.run("view.colorSamplers.add", json!({"x": 180, "y": 90})).unwrap();
    let before = samplers(&app);
    let expected_rows = [
        Some(SAMPLER_DELETE),
        None,
        Some("colorSampler.readout.actual"),
        Some("colorSampler.readout.proof"),
        None,
        Some("colorSampler.readout.gray"),
        Some("colorSampler.readout.rgb"),
        Some("colorSampler.readout.web"),
        Some("colorSampler.readout.hsb"),
        Some("colorSampler.readout.hsl"),
        Some("colorSampler.readout.cmyk"),
        Some("colorSampler.readout.lab"),
        None,
        Some("colorSampler.readout.gray16"),
        Some("colorSampler.readout.rgb16"),
        Some("colorSampler.readout.cmyk16"),
        Some("colorSampler.readout.lab16"),
        None,
        Some("colorSampler.readout.totalInk"),
    ];

    for ((x, y), can_delete) in [((70.0, 40.0), false), ((120.0, 60.0), true)] {
        right_click(&mut app, x, y);
        assert_eq!(samplers(&app), before, "a right-click never places a sampler");
        let inspected = control(&mut app, "ui.inspect", json!({}));
        let rows = inspected.pointer("/result/canvasToolMenu/entries").and_then(serde_json::Value::as_array).expect("the open sampler menu");
        let order: Vec<_> =
            rows.iter().map(|row| if row["separator"] == true { None } else { Some(row["id"].as_str().expect("expected a menu item id")) }).collect();
        assert_eq!(order, expected_rows);
        let enabled: Vec<_> = rows.iter().filter(|row| row["enabled"] == true).map(|row| row["id"].as_str().unwrap()).collect();
        let expected_enabled: &[&str] = if can_delete { &[SAMPLER_DELETE] } else { &[] };
        assert_eq!(enabled, expected_enabled, "right-click at ({x}, {y})");

        let reply = control(&mut app, "ui.context.choose", json!({"id": SAMPLER_DELETE}));
        assert_eq!(reply["ok"], can_delete, "right-click at ({x}, {y}): {reply}");
        if !can_delete {
            assert_eq!(samplers(&app), before, "an unavailable Delete changes nothing");
            key(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
        }
        assert!(app.ui.canvas_tool_menu.is_none(), "the menu closes");
    }

    assert_eq!(samplers(&app), [[20.5, 20.5], [180.5, 90.5]], "only the middle sampler is removed");
    let listed = app.run("view.colorSamplers.list", json!({})).unwrap();
    assert_eq!(listed["samplers"][1]["number"], 2, "the last sampler becomes number 2");
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
    let original_menu = app.ui.canvas_tool_menu.clone().unwrap();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    assert!(app.ui.canvas_tool_menu.is_none(), "switching documents closes the menu");
    for (x, y) in [(20, 20), (180, 90)] {
        app.run("view.colorSamplers.add", json!({"x": x, "y": y})).unwrap();
    }
    assert!(!crate::canvas_tool_menu::entry_enabled(&app, &original_menu, SAMPLER_DELETE), "the original menu cannot delete another document's sampler");
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
