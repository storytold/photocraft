//! #12840: in an Indexed Color document the Freeform Pen keeps Path mode when Shape is chosen, its
//! strokes still make the Work Path, and Make: Shape refuses with Photoshop's "Could not fill" alert
//! and leaves the document unchanged. The Pen and other colour modes weren't measured and keep
//! today's behaviour.

use egui::Modifiers;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{ColorMode, LayerContent};
use serde_json::json;

use super::add_anchor_options_bar_tests::harness;
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{DialogKind, Tool};

const ALERT: &str = "Could not fill because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";

/// The full app with `tool`, its document converted to Indexed Color when `indexed`.
fn app_in(tool: Tool, indexed: bool) -> Harness<'static, PhotocraftApp> {
    let mut h = harness(tool);
    // The Pen's options bar with Make: Selection…, Mask and Shape runs past 1280 px, which put
    // the Shape button's centre off screen on CI's fonts.
    h.set_size(egui::vec2(1400.0, 800.0));
    h.run_steps(2);
    if indexed {
        h.state_mut().run("image.mode.indexedColor", json!({"palette": "exact", "dither": "none", "forced": "none"})).unwrap();
        h.run_steps(4);
    }
    let mode = h.state().session.active().unwrap().doc.mode;
    assert_eq!(mode, if indexed { ColorMode::Indexed } else { ColorMode::Rgb });
    assert_eq!(h.state().ui.tool_options.vector_mode, "path");
    h
}

/// Picks `entry` in the options bar's Mode dropdown, which shows Path. The list's entries are
/// toggles, unlike the Make: Shape button that shares the label.
fn choose_mode(h: &mut Harness<'static, PhotocraftApp>, entry: &str) {
    h.get_all_by_role(egui::accesskit::Role::ComboBox).find(|n| n.accesskit_node().value().as_deref() == Some("Path")).expect("the Mode dropdown").click();
    h.run_steps(3);
    h.get_all_by_label(entry).find(|n| n.accesskit_node().toggled().is_some()).expect("the open list's entry").click();
    h.run_steps(3);
}

/// A freehand wave with the Freeform Pen through the canvas.
fn stroke(h: &mut Harness<'static, PhotocraftApp>) {
    let app = h.state_mut();
    let none = Modifiers::NONE;
    let at = |k: u32| (20.0 + 5.0 * f64::from(k), 50.0 - 30.0 * (std::f64::consts::PI * f64::from(k) / 15.0).sin());
    tool_event(app, ToolEvent::Down { x: 20.0, y: 50.0, pressure: 1.0 }, none);
    for k in 1..=30 {
        let (x, y) = at(k);
        tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, none);
    }
    let (x, y) = at(30);
    tool_event(app, ToolEvent::Up { x, y }, none);
    h.run_steps(4);
}

fn states(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn shape_layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.walk().into_iter().filter(|(_, _, l)| matches!(l.content, LayerContent::Shape(_))).count()
}

fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message").and_then(|m| m.as_str()).map(str::to_string)).collect()
}

#[test]
fn choosing_shape_keeps_the_freeform_pen_in_path_mode_in_indexed_color() {
    let mut h = app_in(Tool::FreeformPen, true);
    choose_mode(&mut h, "Shape");
    assert_eq!(h.state().ui.tool_options.vector_mode, "path");
    assert!(h.get_all_by_role(egui::accesskit::Role::ComboBox).any(|n| n.accesskit_node().value().as_deref() == Some("Path")), "the dropdown shows Path");
    assert!(alerts(h.state()).is_empty(), "no alert");
    // Not measured, today's behaviour: the Freeform Pen in RGB and the Pen in Indexed Color.
    for (tool, indexed) in [(Tool::FreeformPen, false), (Tool::Pen, true)] {
        let mut h = app_in(tool, indexed);
        choose_mode(&mut h, "Shape");
        assert_eq!(h.state().ui.tool_options.vector_mode, "shape", "{tool:?}, indexed: {indexed}");
    }
}

#[test]
fn make_shape_after_a_freeform_stroke_in_indexed_color_alerts_and_changes_nothing() {
    let mut h = app_in(Tool::FreeformPen, true);
    let before = states(h.state());
    stroke(&mut h);
    assert!(h.state().session.active().unwrap().doc.work_path.is_some(), "the stroke makes the Work Path");
    assert_eq!(states(h.state()), before + 1);
    let (doc, n) = (h.state().session.active().unwrap().doc.clone(), states(h.state()));
    h.get_by_label("Shape").click();
    h.run_steps(4);
    assert_eq!(alerts(h.state()), [ALERT]);
    assert!(h.query_by_label(ALERT).is_some(), "the alert is shown");
    h.get_by_label("OK").click();
    h.run_steps(4);
    assert!(alerts(h.state()).is_empty(), "OK closes the alert");
    let st = h.state().session.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&doc, &st.doc), "the document is unchanged");
    assert_eq!((states(h.state()), shape_layers(h.state())), (n, 0), "no History step, no shape layer");
    assert!(!h.state().ui.status_error, "no status-bar error besides the alert");
}

/// The mode is shared by the pens: Shape chosen with the Pen turns to Path once Shift+P switches
/// to the Freeform Pen, and its stroke makes the Work Path, no shape layer.
#[test]
fn a_shape_mode_chosen_with_the_pen_turns_to_path_on_the_freeform_pen() {
    let mut h = app_in(Tool::Pen, true);
    choose_mode(&mut h, "Shape");
    assert_eq!(h.state().ui.tool_options.vector_mode, "shape");
    for _ in 0..3 {
        if h.state().ui.tool == Tool::FreeformPen {
            break;
        }
        h.key_press_modifiers(Modifiers::SHIFT, egui::Key::P);
        h.run_steps(2);
    }
    assert_eq!(h.state().ui.tool, Tool::FreeformPen);
    h.run_steps(2);
    assert_eq!(h.state().ui.tool_options.vector_mode, "path");
    stroke(&mut h);
    assert!(h.state().session.active().unwrap().doc.work_path.is_some(), "the stroke makes the Work Path");
    assert_eq!(shape_layers(h.state()), 0);
}

/// Not measured or not refused, today's behaviour: Make: Shape with the Pen in Indexed Color and
/// with the Freeform Pen in RGB makes a shape layer.
#[test]
fn make_shape_with_the_pen_in_indexed_or_the_freeform_pen_in_rgb_still_makes_a_shape_layer() {
    for (tool, indexed) in [(Tool::Pen, true), (Tool::FreeformPen, false)] {
        let mut h = app_in(tool, indexed);
        if tool == Tool::Pen {
            for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 80.0)] {
                let app = h.state_mut();
                tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
                tool_event(app, ToolEvent::Up { x, y }, Modifiers::NONE);
            }
            h.run_steps(4);
        } else {
            stroke(&mut h);
        }
        h.get_by_label("Shape").click();
        h.run_steps(4);
        assert_eq!(shape_layers(h.state()), 1, "{tool:?}, indexed: {indexed}");
        assert!(alerts(h.state()).is_empty());
    }
}
