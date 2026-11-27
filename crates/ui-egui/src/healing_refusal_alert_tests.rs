//! #12111: a Healing Brush press on a hidden target layer or a layer with Lock All opens the
//! reference application's alert with OK and paints nothing; an Alt-click still only sets the
//! source, a group keeps today's message (its own item, #12109) and the other tools keep theirs.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;
use crate::tools::brush_refusal::brush_refusal_alert_tests::{active, alerts, app, refused, steps, stroke};

/// An Alt-click: the clone source.
fn set_source(app: &mut PhotocraftApp, x: f64, y: f64) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::ALT);
    tool_event(app, ToolEvent::Up { x, y }, Modifiers::ALT);
}

#[test]
fn a_hidden_target_layer() {
    let mut app = app(Tool::Healing);
    set_source(&mut app, 50.0, 32.0);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    refused(&mut app, "Could not use the healing brush because the target layer is hidden.");
    assert!(app.ui.status.is_empty() || !app.ui.status_error, "no status-bar error as well: {:?}", app.ui.status);
}

#[test]
fn lock_all() {
    let mut app = app(Tool::Healing);
    set_source(&mut app, 50.0, 32.0);
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    refused(&mut app, "Could not use the healing brush because the layer is locked.");
}

#[test]
fn an_alt_click_on_a_hidden_or_locked_layer_only_sets_the_source() {
    for lock in [false, true] {
        let mut app = app(Tool::Healing);
        let id = active(&app);
        if lock {
            app.run("layer.lockLayers", json!({"all": true})).unwrap();
        } else {
            app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        }
        let before = steps(&app);
        set_source(&mut app, 50.0, 32.0);
        assert!(alerts(&app).is_empty(), "lock {lock}: {:?}", alerts(&app));
        assert_eq!(app.ui.clone_source, Some([50.0, 32.0]), "lock {lock}: the source is set");
        assert_eq!(steps(&app), before, "lock {lock}: no history step");
    }
}

#[test]
fn lock_image_pixels_and_lock_position_keep_todays_behaviour() {
    for lock in ["pixels", "position"] {
        let mut app = app(Tool::Healing);
        set_source(&mut app, 50.0, 32.0);
        app.run("layer.lockLayers", json!({ lock: true })).unwrap();
        stroke(&mut app);
        assert!(alerts(&app).is_empty(), "{lock}: {:?}", alerts(&app));
    }
}

#[test]
fn a_group_keeps_its_own_message() {
    let mut app = app(Tool::Healing);
    set_source(&mut app, 50.0, 32.0);
    app.run("layer.new.group", json!({})).unwrap();
    stroke(&mut app);
    assert_eq!(alerts(&app), ["Could not use the Healing Brush Tool because the target layer is a group."]);
}

#[test]
fn a_hidden_group_keeps_the_status_bar_error() {
    let mut app = app(Tool::Healing);
    set_source(&mut app, 50.0, 32.0);
    app.run("layer.new.group", json!({})).unwrap();
    let group = active(&app);
    app.run("layer.setProps", json!({"layer": group, "visible": false})).unwrap();
    stroke(&mut app);
    assert!(alerts(&app).is_empty(), "{:?}", alerts(&app));
    assert_eq!(app.ui.status, photocraft_engine::hidden_target::HIDDEN);
}

#[test]
fn an_unlocked_visible_layer_still_heals() {
    let mut app = app(Tool::Healing);
    set_source(&mut app, 50.0, 32.0);
    let before = steps(&app);
    stroke(&mut app);
    assert!(alerts(&app).is_empty(), "{:?}", alerts(&app));
    assert_eq!(steps(&app), before + 1, "one Healing Brush state");
}

#[test]
fn the_clone_stamp_keeps_the_status_bar() {
    let mut app = app(Tool::CloneStamp);
    set_source(&mut app, 50.0, 32.0);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    stroke(&mut app);
    assert!(alerts(&app).is_empty());
    assert_eq!(app.ui.status, photocraft_engine::hidden_target::HIDDEN);
}

/// Through the real canvas: a press reaches the tool through the host's preflight, which is where
/// the refusals run, so the Alt-click that sets the source must get past it.
mod canvas {
    use egui::Modifiers;
    use photocraft_engine::presets::PresetsExt;
    use serde_json::json;

    use crate::alt_click_tests::{click, harness, two_colours};
    use crate::state::Tool;
    use crate::tools::brush_refusal::brush_refusal_alert_tests::alerts;

    #[test]
    fn an_alt_click_on_a_hidden_or_locked_layer_sets_the_source_and_a_click_is_refused() {
        for lock in [false, true] {
            let mut h = harness(two_colours());
            h.state_mut().ui.tool = Tool::Healing;
            let id = h.state().session.active().unwrap().active_layer.unwrap().0;
            if lock {
                h.state_mut().run("layer.lockLayers", json!({"all": true})).unwrap();
            } else {
                h.state_mut().run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
            }
            click(&mut h, 150.0, 50.0, Modifiers::ALT);
            assert!(h.state().session.presets().clone.active().source.is_some(), "lock {lock}: the source is set");
            assert!(alerts(h.state()).is_empty(), "lock {lock}: no alert: {:?}", alerts(h.state()));
            // A plain click is still refused.
            click(&mut h, 50.0, 50.0, Modifiers::NONE);
            assert_eq!(alerts(h.state()).len(), 1, "lock {lock}: {:?}", alerts(h.state()));
        }
    }
}
