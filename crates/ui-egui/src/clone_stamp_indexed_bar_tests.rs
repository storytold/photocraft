//! In an Indexed Color document the Clone Stamp's Sample is greyed at Current Layer and a stroke
//! samples the current layer, whatever Sample held; the Normal Mode reads "Threshold". Opacity,
//! Flow and Aligned stay enabled, and RGB documents keep the list, the label and the Sample choice.

use egui::Modifiers;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, Node};
use photocraft_doc::BlendMode;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::options_bar_mode_wheel_tests::harness;
use crate::state::Tool;

/// The Clone Stamp in a new document, Sample set to All Layers; `indexed` converts it first.
fn bar(indexed: bool) -> Harness<'static, PhotocraftApp> {
    let mut h = harness(Tool::CloneStamp);
    let app = h.state_mut();
    if indexed {
        app.run("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none"})).unwrap();
    }
    app.ui.tool_options.clone_sample = "all".into();
    h.run_steps(4);
    h
}

/// The Sample dropdown: the one showing one of its choices.
fn sample<'a>(h: &'a Harness<'_, PhotocraftApp>) -> Node<'a> {
    let choices = ["Current Layer", "Current & Below", "All Layers"];
    h.query_all_by_role(Role::ComboBox).find(|n| n.value().is_some_and(|v| choices.contains(&v.as_str()))).expect("the Sample dropdown")
}

/// The options bar's field or checkbox named `name` ("Opacity:" in the Photoshop-look themes).
fn control<'a>(h: &'a Harness<'_, PhotocraftApp>, role: Role, name: &str) -> Node<'a> {
    h.query_all_by_role(role).find(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == name)).unwrap_or_else(|| panic!("{name}"))
}

/// An Alt-click for the source, then a drag, through the canvas: the committed stroke's params.
fn clone_stroke(h: &mut Harness<'_, PhotocraftApp>) -> Value {
    let app = h.state_mut();
    tool_event(app, ToolEvent::Down { x: 40.0, y: 20.0, pressure: 1.0 }, Modifiers::ALT);
    tool_event(app, ToolEvent::Up { x: 40.0, y: 20.0 }, Modifiers::ALT);
    tool_event(app, ToolEvent::Down { x: 10.0, y: 30.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: 30.0, y: 30.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 30.0, y: 30.0 }, Modifiers::NONE);
    let (id, params) = app.session.journal.last().unwrap();
    assert_eq!(id, "paint.cloneStamp", "{}", app.ui.status);
    params.clone()
}

#[test]
fn in_an_indexed_document_sample_is_greyed_at_current_layer() {
    let h = bar(true);
    let node = sample(&h);
    assert_eq!(node.value().as_deref(), Some("Current Layer"));
    assert!(node.accesskit_node().is_disabled());
    assert_eq!(h.state().ui.tool_options.clone_sample, "all", "the stored choice is kept");
}

#[test]
fn a_stroke_in_an_indexed_document_samples_the_current_layer() {
    let mut h = bar(true);
    let steps = h.state().session.active().unwrap().history.past_len();
    assert_eq!(clone_stroke(&mut h)["sampleLayer"], "current");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "one Clone Stamp state");
}

fn shows(h: &Harness<'_, PhotocraftApp>, value: &str) -> bool {
    h.query_all_by_role(Role::ComboBox).any(|n| n.value().as_deref() == Some(value))
}

/// Other modes in Indexed Color weren't measured: they keep today's list.
#[test]
fn in_an_indexed_document_another_mode_keeps_the_list() {
    let mut h = bar(true);
    // A brush setter notes the change (`brush_changed`), which the options bar's content version holds.
    let tools = &mut h.state_mut().session.tools;
    tools.brush.mode = BlendMode::Dissolve;
    tools.brush_changed();
    h.run_steps(4);
    assert!(shows(&h, "Dissolve"));
    assert!(!shows(&h, "Threshold"));
}

#[test]
fn back_in_rgb_sample_shows_and_sends_the_stored_choice() {
    let mut h = bar(true);
    h.state_mut().run("image.mode.rgb", json!({})).unwrap();
    h.run_steps(4);
    let node = sample(&h);
    assert_eq!(node.value().as_deref(), Some("All Layers"));
    assert!(!node.accesskit_node().is_disabled());
    assert_eq!(clone_stroke(&mut h)["sampleLayer"], "all");
}

#[test]
fn opacity_flow_and_aligned_stay_enabled_in_an_indexed_document() {
    let h = bar(true);
    for (role, name) in [(Role::SpinButton, "Opacity"), (Role::SpinButton, "Flow"), (Role::CheckBox, "Aligned")] {
        assert!(!control(&h, role, name).accesskit_node().is_disabled(), "{name}");
    }
}
