//! #12840: in an Indexed Color document the Freeform Pen keeps Path mode when Shape is chosen, its
//! strokes still make the Work Path, and Make: Shape refuses with Photoshop's "Could not fill" alert
//! and leaves the document unchanged. The Pen and other colour modes weren't measured and keep
//! today's behaviour.

use egui::Modifiers;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{ColorMode, LayerContent};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::options_bar_mode_wheel_tests::harness;
use crate::state::Tool;
use craft::draw::ui::VectorMode;

const ALERT: &str = "Could not fill because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";

type H = Harness<'static, Box<PhotocraftApp>>;

/// The full app with `tool`, its document converted to Indexed Color when `indexed`.
fn app_in(tool: Tool, indexed: bool) -> H {
    let mut h = harness(tool);
    if indexed {
        h.state_mut().run("image.mode.indexedColor", json!({"palette": "exact", "dither": "none", "forced": "none"})).unwrap();
        h.run_steps(4);
    }
    let mode = h.state().session.active().unwrap().doc.mode;
    assert_eq!(mode, if indexed { ColorMode::Indexed } else { ColorMode::Rgb });
    assert_eq!(h.state().ui.tool_options.vector_mode, VectorMode::Path);
    h
}

/// Picks `entry` in the options bar's Mode dropdown, which shows Path: the entry below the
/// dropdown, not the Make: Shape button that shares the label.
fn choose_mode(h: &mut H, entry: &str) {
    let combo = h.get_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().value().as_deref() == Some("Path")).expect("the Mode dropdown");
    let below = combo.rect().max.y;
    combo.click();
    h.run_steps(3);
    h.get_all_by_label(entry).find(|n| n.rect().min.y >= below - 0.5).expect("the open list's entry").click();
    h.run_steps(3);
}

/// The Make: Shape button of the options bar.
fn make_shape(h: &mut H) {
    h.get_all_by_label("Shape").find(|n| n.accesskit_node().role() == Role::Button).expect("Make: Shape").click();
    h.run_steps(4);
}

/// A freehand wave with the Freeform Pen through the canvas.
fn stroke(h: &mut H) {
    let app = h.state_mut();
    let none = Modifiers::NONE;
    let at = |k: u32| (10.0 + 1.5 * f64::from(k), 24.0 - 15.0 * (std::f64::consts::PI * f64::from(k) / 15.0).sin());
    tool_event(app, ToolEvent::Down { x: 10.0, y: 24.0, pressure: 1.0 }, none);
    for k in 1..=30 {
        let (x, y) = at(k);
        tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, none);
    }
    let (x, y) = at(30);
    tool_event(app, ToolEvent::Up { x, y }, none);
    h.run_steps(4);
}

fn states(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn shape_layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.walk().into_iter().filter(|(_, _, l)| matches!(l.content, LayerContent::Shape(_))).count()
}

fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_string)).collect()
}

#[test]
fn choosing_shape_keeps_the_freeform_pen_in_path_mode_in_indexed_color() {
    let mut h = app_in(Tool::FreeformPen, true);
    choose_mode(&mut h, "Shape");
    assert_eq!(h.state().ui.tool_options.vector_mode, VectorMode::Path);
    assert!(h.get_all_by_role(Role::ComboBox).any(|n| n.accesskit_node().value().as_deref() == Some("Path")), "the dropdown shows Path");
    assert!(alerts(h.state()).is_empty(), "no alert");
    // Not measured, today's behaviour: the Freeform Pen in RGB and the Curvature Pen in Indexed Color
    // (the Pen there is #12910's).
    for (tool, indexed) in [(Tool::FreeformPen, false), (Tool::CurvaturePen, true)] {
        let mut h = app_in(tool, indexed);
        choose_mode(&mut h, "Shape");
        assert_eq!(h.state().ui.tool_options.vector_mode, VectorMode::Shape, "{tool:?}, indexed: {indexed}");
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
    make_shape(&mut h);
    assert_eq!(alerts(h.state()), [ALERT]);
    let id = h.state().framework.dialogs.open_dialogs().first().map(|d| d.id).unwrap();
    let ctx = h.state().ctx().clone();
    crate::dialogs::confirm(h.state_mut(), &ctx, id).unwrap();
    h.run_steps(2);
    assert!(alerts(h.state()).is_empty(), "OK closes the alert");
    let st = h.state().session.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&doc, &st.doc), "the document is unchanged");
    assert_eq!((states(h.state()), shape_layers(h.state())), (n, 0), "no History step, no shape layer");
    assert!(!h.state().ui.status_error, "no status-bar error besides the alert");
}

/// The mode is shared by the pens: Shape chosen with the Curvature Pen turns to Path once the
/// Freeform Pen's bar shows, and its stroke makes the Work Path, no shape layer.
#[test]
fn a_shape_mode_chosen_with_another_pen_turns_to_path_on_the_freeform_pen() {
    let mut h = app_in(Tool::CurvaturePen, true);
    choose_mode(&mut h, "Shape");
    assert_eq!(h.state().ui.tool_options.vector_mode, VectorMode::Shape);
    h.state_mut().ui.tool = Tool::FreeformPen;
    h.run_steps(4);
    assert_eq!(h.state().ui.tool_options.vector_mode, VectorMode::Path);
    stroke(&mut h);
    assert!(h.state().session.active().unwrap().doc.work_path.is_some(), "the stroke makes the Work Path");
    assert_eq!(shape_layers(h.state()), 0);
}

/// Not refused: Make: Shape with the Freeform Pen in RGB still makes a shape layer, and the Pen in
/// Indexed Color does not get the Freeform Pen's alert (its own is #12910's).
#[test]
fn make_shape_with_the_freeform_pen_in_rgb_or_the_pen_in_indexed_is_not_refused() {
    let mut h = app_in(Tool::FreeformPen, false);
    stroke(&mut h);
    make_shape(&mut h);
    assert_eq!(shape_layers(h.state()), 1, "{}", h.state().ui.status);
    assert!(alerts(h.state()).is_empty());

    let mut h = app_in(Tool::Pen, true);
    for (x, y) in [(10.0, 10.0), (50.0, 10.0), (50.0, 40.0)] {
        crate::canvas::testing::click(h.state_mut(), [x, y], Modifiers::NONE);
    }
    h.run_steps(4);
    make_shape(&mut h);
    assert!(alerts(h.state()).iter().all(|a| a != ALERT), "the Pen has no Freeform Pen alert");
}
