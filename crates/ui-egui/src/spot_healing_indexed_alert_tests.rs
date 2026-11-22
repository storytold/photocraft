//! #12582: in an Indexed Color document a click or a drag with the Spot Healing Brush opens the reference
//! application's alert with OK and heals nothing; the options bar offers only Create Texture and greys Sample All
//! Layers. An RGB document heals and keeps its options as they are.

use egui::accesskit::{Role, Toggled};
use egui::{Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::tests::app;
use super::*;
use crate::canvas::{ToolEvent, tool_event};
use crate::tools::brush_refusal::brush_refusal_alert_tests::alerts;

const MESSAGE: &str = "Could not use the spot healing brush because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";

/// The test document with a black spot, the Spot Healing Brush chosen; Indexed Color when `indexed`.
fn spotted(indexed: bool) -> Box<PhotocraftApp> {
    let mut app = app();
    app.run("select.rect", json!({"x": 44, "y": 26, "width": 8, "height": 8})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#000000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    if indexed {
        app.run("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none"})).unwrap();
    }
    app.ui.tool = Tool::SpotHealing;
    app
}

fn state(app: &PhotocraftApp) -> (Vec<f32>, usize) {
    let st = app.session.active().unwrap();
    (st.doc.layers[0].surface().unwrap().read_region(st.doc.bounds()), st.history.past_len())
}

fn stroke(app: &mut PhotocraftApp, to: f64) {
    tool_event(app, ToolEvent::Down { x: 48.0, y: 30.0, pressure: 1.0 }, Modifiers::NONE);
    if to != 48.0 {
        tool_event(app, ToolEvent::Move { x: to, y: 30.0, pressure: 1.0 }, Modifiers::NONE);
    }
    tool_event(app, ToolEvent::Up { x: to, y: 30.0 }, Modifiers::NONE);
}

#[test]
fn a_click_and_a_drag_in_indexed_color_alert_and_heal_nothing() {
    for to in [48.0, 52.0] {
        let mut app = spotted(true);
        let before = state(&app);
        stroke(&mut app, to);
        assert_eq!(alerts(&app), [MESSAGE], "to {to}");
        assert!(state(&app) == before, "to {to}: nothing healed, no history state");
    }
}

/// The tool can change while the button is down: the stroke that started with the Spot Healing Brush still gets the
/// alert, not the engine's refusal in the status bar.
#[test]
fn a_stroke_started_with_spot_healing_alerts_even_if_the_tool_changed_before_the_release() {
    let mut app = spotted(true);
    let before = state(&app);
    app.ui.tool = Tool::Brush;
    assert!(crate::retouch_ui::finish_stroke(&mut app, Tool::SpotHealing, &[[48.0, 30.0, 1.0]], Mods::NONE, false));
    assert_eq!(alerts(&app), [MESSAGE]);
    assert!(state(&app) == before, "nothing healed, no history state");
}

#[test]
fn an_rgb_document_heals_without_an_alert() {
    let mut app = spotted(false);
    let before = state(&app);
    stroke(&mut app, 48.0);
    assert!(alerts(&app).is_empty());
    assert_eq!(state(&app).1, before.1 + 1, "one Spot Healing state");
}

/// The settled options bar of `app`.
fn bar(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1600.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    h
}

/// (disabled, checked) of the control named `label` with `role`.
fn control(h: &Harness<'static, Box<PhotocraftApp>>, role: Role, label: &str) -> (bool, bool) {
    let node = h.query_all_by_role(role).find(|n| n.accesskit_node().label().as_deref() == Some(label)).unwrap_or_else(|| panic!("missing {role:?} {label:?}"));
    let n = node.accesskit_node();
    (n.is_disabled(), n.toggled() == Some(Toggled::True))
}

#[test]
fn the_indexed_options_bar_offers_only_create_texture() {
    let h = bar(spotted(true));
    assert_eq!(control(&h, Role::RadioButton, "Content-Aware"), (true, false));
    assert_eq!(control(&h, Role::RadioButton, "Proximity Match"), (true, false));
    assert_eq!(control(&h, Role::RadioButton, "Create Texture"), (false, true));
    assert!(control(&h, Role::CheckBox, "Sample All Layers").0, "Sample All Layers is greyed");
    assert_eq!(h.state().ui.tool_options.spot_type, "contentAware", "the stored Type stays for other documents");
    let h = bar(spotted(false));
    assert_eq!(control(&h, Role::RadioButton, "Content-Aware"), (false, true), "RGB keeps its Type");
    assert!(!control(&h, Role::CheckBox, "Sample All Layers").0);
}
