//! #12606: the Clone Stamp's options bar shows the Ignore adjustment layers toggle right after
//! Sample, off by default and dimmed while Sample is Current Layer; it is sent with the stroke as
//! `ignoreAdjustments`, and Reset Tool turns it off. The Healing Brush's bar doesn't get it.

use egui::Modifiers;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, Node};
use serde_json::Value;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::options_bar_mode_wheel_tests::harness;
use crate::state::Tool;

const LABEL: &str = "Ignore adjustment layers";

/// `tool`'s options bar with Sample set to `sample`.
fn bar(tool: Tool, sample: &str) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = harness(tool);
    h.state_mut().ui.tool_options.clone_sample = sample.into();
    h.run_steps(4);
    h
}

fn toggle<'a>(h: &'a Harness<'_, Box<PhotocraftApp>>) -> Node<'a> {
    h.get_by_label(LABEL)
}

fn on(h: &Harness<'_, Box<PhotocraftApp>>) -> bool {
    h.state().ui.tool_options.clone_ignore_adjustments
}

/// An Alt-click for the source, then a click: the committed stroke's params.
fn clone_stroke(h: &mut Harness<'_, Box<PhotocraftApp>>) -> Value {
    let app = h.state_mut();
    tool_event(app, ToolEvent::Down { x: 40.0, y: 20.0, pressure: 1.0 }, Modifiers::ALT);
    tool_event(app, ToolEvent::Up { x: 40.0, y: 20.0 }, Modifiers::ALT);
    tool_event(app, ToolEvent::Down { x: 10.0, y: 30.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 10.0, y: 30.0 }, Modifiers::NONE);
    let (id, params) = app.session.journal.last().unwrap();
    assert_eq!(id, "paint.cloneStamp", "{}", app.ui.status);
    params.clone()
}

#[test]
fn the_toggle_follows_sample_and_is_off_by_default() {
    let h = bar(Tool::CloneStamp, "all");
    assert!(!on(&h), "off by default");
    let toggle = toggle(&h);
    let sample = h.query_all_by_role(Role::ComboBox).find(|n| n.value().as_deref() == Some("All Layers")).expect("the Sample dropdown");
    assert!(toggle.rect().left() >= sample.rect().right(), "right after Sample");
}

#[test]
fn dimmed_at_current_layer_enabled_with_current_and_below_and_all_layers() {
    for (sample, enabled) in [("current", false), ("currentAndBelow", true), ("all", true)] {
        let mut h = bar(Tool::CloneStamp, sample);
        assert_eq!(!toggle(&h).accesskit_node().is_disabled(), enabled, "{sample}");
        toggle(&h).click();
        h.run_steps(4);
        assert_eq!(on(&h), enabled, "{sample}: a click turns it on only when enabled");
    }
}

#[test]
fn the_stroke_sends_it_and_reset_tool_turns_it_off() {
    let mut off = bar(Tool::CloneStamp, "all");
    assert_eq!(clone_stroke(&mut off)["ignoreAdjustments"], false);
    let mut h = bar(Tool::CloneStamp, "all");
    toggle(&h).click();
    h.run_steps(4);
    assert!(on(&h));
    assert_eq!(clone_stroke(&mut h)["ignoreAdjustments"], true);
    crate::tool_reset::reset_options(&mut h.state_mut().ui.tool_options, Tool::CloneStamp);
    assert!(!on(&h), "Reset Tool turns it off");
}

#[test]
fn the_healing_brush_has_no_toggle() {
    let h = bar(Tool::Healing, "all");
    assert!(h.query_by_label(LABEL).is_none());
}
