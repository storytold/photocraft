//! A Content-Aware Patch the engine refuses (a locked layer, Indexed Color, a hidden layer) shows the reference's
//! alert with OK, as the normal patch does, instead of only a status-bar line (CodeRabbit on #16178).

use serde_json::json;

use super::tests::app;
use super::*;
use crate::tools::brush_refusal::brush_refusal_alert_tests::alerts;

const LOCKED: &str = "Could not use the patch tool because the layer is locked.";

#[test]
fn a_content_aware_patch_on_a_locked_layer_shows_the_alert() {
    let mut app = app();
    app.run("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.setProps", json!({"layer": id.0, "locks": {"pixels": true}})).unwrap();
    app.ui.tool_options.patch_content_aware = true;
    finish_patch(&mut app, [12.0, 12.0], [40.0, 12.0]);
    assert_eq!(alerts(&app), [LOCKED]);
}
