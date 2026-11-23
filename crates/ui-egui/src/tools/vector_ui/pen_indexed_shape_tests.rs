//! #12910: in an Indexed Color document the Pen keeps Path mode and Make: Shape is refused with the
//! reference application's alert ("Could not use the pen tool … Indexed Color images …") and OK, and
//! the document stays as it was. Path mode still draws a Work Path there. RGB, 16-bit CMYK and the
//! other colour modes keep making shape layers, and the other pens and tools keep today's Indexed
//! behaviour (not measured).

use egui::Modifiers;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{ColorMode, LayerContent, LayerId, Path};
use serde_json::{Value, json};

use craft::draw::ui::VectorMode;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;
use crate::tools::brush_refusal::brush_refusal_alert_tests::alerts;

/// The measured alert for the Pen (#12910). #12840 measured the Freeform Pen with different wording,
/// so the message belongs to the active tool, not to the Make: Shape path in general.
const INDEXED_ALERT: &str = "Could not use the pen tool because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";

/// The options bar of `tool` (the bar follows `app.ui.tool`, as in the app) on a document made of
/// `doc` params, with `tool` active.
fn bar(tool: Tool, doc: Value) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", doc).unwrap();
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 400.0)).with_context(app.ctx().clone()).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            let tool = app.ui.tool;
            ui.horizontal(|ui| super::options_bar(app, ui, tool));
        },
        app,
    );
    h.run_steps(4);
    h
}

/// The Pen options bar on a 200 × 200 8-bit RGB document.
fn pen_bar() -> Harness<'static, Box<PhotocraftApp>> {
    bar(Tool::Pen, json!({"width": 200, "height": 200}))
}

/// The same document converted to Indexed Color, as an opened Indexed GIF is.
fn to_indexed(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    h.state_mut().run("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none"})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().doc.mode, ColorMode::Indexed, "the document is Indexed Color");
}

/// `tool`'s options bar on an Indexed Color document.
fn indexed_bar(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = bar(tool, json!({"width": 200, "height": 200}));
    to_indexed(&mut h);
    h
}

/// The Pen-family mode dropdown: the one showing a Pen mode.
fn dropdown<'a>(h: &'a Harness<'static, Box<PhotocraftApp>>) -> egui_kittest::Node<'a> {
    let modes = ["Shape", "Path"];
    h.query_all_by_role(Role::ComboBox).find(|n| n.value().is_some_and(|v| modes.contains(&v.as_str()))).expect("the mode dropdown")
}

/// The mode the dropdown shows.
fn shown_mode(h: &Harness<'static, Box<PhotocraftApp>>) -> String {
    dropdown(h).value().expect("the mode dropdown shows its mode")
}

/// Clicks the mode dropdown and picks its `entry`, as a hand does.
fn pick_mode(h: &mut Harness<'static, Box<PhotocraftApp>>, entry: &str) {
    let at = dropdown(h).rect().center();
    crate::layers_panel_nodes::click(h, at, Modifiers::NONE);
    let item = h.get_by_role_and_label(Role::ListBoxOption, entry);
    assert!(!item.accesskit_node().is_disabled(), "{entry} is listed enabled in the mode dropdown");
    item.click();
    h.run_steps(2);
}

/// Whether the options bar's Make: Shape button is greyed.
fn shape_button_disabled(h: &Harness<'static, Box<PhotocraftApp>>) -> bool {
    h.get_by_role_and_label(Role::Button, "Shape").accesskit_node().is_disabled()
}

/// Clicks the options bar's Make: Shape button.
fn click_shape_button(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    h.get_by_role_and_label(Role::Button, "Shape").click();
}

/// Clicks a closed square with the Pen: four corners and the click back on the first anchor.
fn square(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    let app = h.state_mut();
    for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0), (20.0, 120.0)] {
        testing::click(app, [x, y], Modifiers::NONE);
    }
    testing::click(app, [20.5, 20.5], Modifiers::NONE); // close
    h.run_steps(2);
}

/// The Path-mode Work Path the last `square` made.
fn work_path(h: &Harness<'static, Box<PhotocraftApp>>) -> Path {
    h.state().session.active().unwrap().doc.work_path.clone().expect("a work path")
}

fn steps(h: &Harness<'static, Box<PhotocraftApp>>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

fn layers(h: &Harness<'static, Box<PhotocraftApp>>) -> usize {
    h.state().session.active().unwrap().doc.layers.len()
}

/// The document's shape layers, as (layer, path).
fn shapes(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<(LayerId, Path)> {
    let st = h.state().session.active().unwrap();
    st.doc
        .walk()
        .iter()
        .filter_map(|(_, _, l)| match &l.content {
            LayerContent::Shape(sh) => Some((l.id, sh.path.clone())),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Done when 1: choosing Shape in the mode dropdown keeps Path

/// #12910: the mode dropdown still lists Shape enabled, and picking it leaves the Pen in Path mode.
#[test]
fn choosing_shape_in_the_mode_dropdown_keeps_path_in_an_indexed_document() {
    let mut h = indexed_bar(Tool::Pen);
    assert_eq!(shown_mode(&h), "Path", "the Pen starts in Path mode");
    pick_mode(&mut h, "Shape");
    let app = h.state();
    assert_eq!(app.ui.tool_options.vector_mode, VectorMode::Path, "Shape cannot be chosen: the mode stays Path");
    assert_eq!(shown_mode(&h), "Path", "and the dropdown keeps showing Path");
}

/// A second pick changes nothing either, so the mode cannot slip through over two frames.
#[test]
fn picking_shape_twice_keeps_path_in_an_indexed_document() {
    let mut h = indexed_bar(Tool::Pen);
    pick_mode(&mut h, "Shape");
    pick_mode(&mut h, "Shape");
    h.run_steps(4);
    assert_eq!(h.state().ui.tool_options.vector_mode, VectorMode::Path);
    assert_eq!(shown_mode(&h), "Path");
}

/// Only the dropdown pick is refused: a Shape mode that never came from the dropdown (a tool preset,
/// `penMode`) keeps today's behaviour, which the issue leaves unmeasured.
#[test]
fn a_shape_mode_set_outside_the_dropdown_keeps_todays_behaviour() {
    let mut h = indexed_bar(Tool::Pen);
    h.state_mut().ui.tool_options.vector_mode = VectorMode::Shape;
    h.run_steps(2);
    assert_eq!(h.state().ui.tool_options.vector_mode, VectorMode::Shape, "not measured: today's behaviour");
    pick_mode(&mut h, "Path");
    assert_eq!(h.state().ui.tool_options.vector_mode, VectorMode::Path, "and Path stays Path");
}

// ---------------------------------------------------------------------------------------------
// Done when 2: Make Shape alerts with OK and changes nothing

/// #12910: the Make: Shape button stays enabled; clicking it opens the measured alert with OK and
/// leaves the Work Path, the layers and the History as they were.
#[test]
fn make_shape_with_the_pen_in_an_indexed_document_alerts_and_changes_nothing() {
    let mut h = indexed_bar(Tool::Pen);
    square(&mut h);
    let drawn = work_path(&h);
    let (before_layers, before_steps, doc) = (layers(&h), steps(&h), h.state().session.active().unwrap().doc.id);
    assert!(!shape_button_disabled(&h), "the Make: Shape button is enabled");
    click_shape_button(&mut h);
    h.run_steps(3);
    let app = h.state();
    assert_eq!(alerts(app), [INDEXED_ALERT], "the alert, once");
    assert_eq!(app.session.active().unwrap().doc.id, doc, "still the same document");
    assert_eq!(layers(&h), before_layers, "no new layer");
    assert_eq!(shapes(&h), Vec::new(), "and no shape layer");
    assert_eq!(steps(&h), before_steps, "no History step");
    assert_eq!(app.session.active().unwrap().doc.work_path.as_ref(), Some(&drawn), "the Work Path is unchanged");
    let id = app.framework.dialogs.open_dialogs().last().map(|d| d.id).expect("the alert");
    let ctx = testing::ctx(app);
    crate::dialogs::confirm(h.state_mut(), &ctx, id).unwrap();
    assert!(alerts(h.state()).is_empty(), "OK closes the alert");
    assert_eq!(layers(&h), before_layers, "and OK changed nothing");
    assert_eq!(steps(&h), before_steps, "no History step was added");
}

// ---------------------------------------------------------------------------------------------
// Done when 3: Path mode still works in an Indexed Color document

/// Drawing the path in Path mode is not refused: one Work Path, one History step, no alert.
#[test]
fn the_pen_still_draws_a_work_path_in_an_indexed_document() {
    let mut h = indexed_bar(Tool::Pen);
    let before = steps(&h);
    square(&mut h);
    let drawn = work_path(&h);
    assert_eq!(drawn.subpaths.len(), 1);
    assert_eq!(drawn.subpaths[0].knots.len(), 4);
    assert!(drawn.subpaths[0].closed, "the click back on the first anchor closed it");
    assert_eq!(steps(&h), before + 1, "one History step");
    assert!(alerts(h.state()).is_empty(), "no alert for drawing a path");
    assert_eq!(h.state().ui.tool_options.vector_mode, VectorMode::Path);
}

// ---------------------------------------------------------------------------------------------
// Done when 4: RGB and 16-bit CMYK keep making shape layers

/// Shape mode and the shape layer it makes are untouched outside Indexed Color: 8-bit RGB, 16-bit
/// CMYK and 8-bit Grayscale (Grayscale and the rest were not measured, so they keep today's behaviour).
#[test]
fn rgb_and_16_bit_cmyk_documents_keep_making_shape_layers_with_the_pen() {
    for (label, doc) in [
        ("RGB 8", json!({"width": 200, "height": 200})),
        ("CMYK 16", json!({"width": 200, "height": 200, "mode": "cmyk", "depth": 16, "background": "white"})),
        ("Gray 8", json!({"width": 200, "height": 200, "mode": "grayscale", "depth": 8, "background": "white"})),
    ] {
        let mut h = bar(Tool::Pen, doc);
        assert_eq!(shown_mode(&h), "Path", "{label}: the Pen starts in Path mode");
        pick_mode(&mut h, "Shape");
        assert_eq!(h.state().ui.tool_options.vector_mode, VectorMode::Shape, "{label}: Shape is chosen");
        assert_eq!(shown_mode(&h), "Shape", "{label}: the dropdown shows Shape");
        square(&mut h);
        assert!(h.state().session.active().unwrap().doc.work_path.is_none(), "{label}: Shape mode leaves no Work Path");
        let made = shapes(&h);
        assert_eq!(made.len(), 1, "{label}: one shape layer");
        let (id, path) = &made[0];
        assert_eq!(path.subpaths.len(), 1, "{label}");
        assert!(h.state().session.active().unwrap().doc.layer(*id).is_some(), "{label}");
        assert!(alerts(h.state()).is_empty(), "{label}: no alert");
    }
}

/// Make: Shape on a Work Path in an RGB document still makes the shape layer, one History step.
#[test]
fn make_shape_still_turns_a_work_path_into_a_shape_layer_in_rgb() {
    let mut h = pen_bar();
    square(&mut h);
    let drawn = work_path(&h);
    let before = steps(&h);
    assert!(!shape_button_disabled(&h), "the Work Path enables the button");
    click_shape_button(&mut h);
    h.run_steps(3);
    let made = shapes(&h);
    assert_eq!(made.len(), 1, "the shape layer");
    assert_eq!(made[0].1, drawn, "the shape takes the Work Path");
    assert_eq!(steps(&h), before + 1, "one History step");
    assert!(alerts(h.state()).is_empty(), "no alert");
}

// ---------------------------------------------------------------------------------------------
// Not measured cases keep today's behaviour: the other pens and the other tools

/// The Freeform Pen and the Curvature Pen in Indexed Color were not measured: Shape is still chosen
/// in the dropdown they share with the Pen.
#[test]
fn another_pen_still_chooses_shape_in_an_indexed_document() {
    for tool in [Tool::FreeformPen, Tool::CurvaturePen] {
        let mut h = indexed_bar(tool);
        pick_mode(&mut h, "Shape");
        assert_eq!(h.state().ui.tool_options.vector_mode, VectorMode::Shape, "{tool:?}: not measured, today's behaviour");
    }
}

/// Make: Shape with the Curvature Pen and with the Rectangle tool in an Indexed Color document still
/// makes the shape layer from the Work Path: only the Pen is refused.
#[test]
fn another_tool_still_makes_its_shape_layer_in_an_indexed_document() {
    for tool in [Tool::CurvaturePen, Tool::Rectangle] {
        let mut h = indexed_bar(Tool::Pen);
        square(&mut h);
        let before = steps(&h);
        let app = h.state_mut();
        app.ui.tool = tool;
        if tool == Tool::Rectangle {
            app.ui.tool_options.shape_mode = VectorMode::Path;
        }
        h.run_steps(2);
        assert!(!shape_button_disabled(&h), "{tool:?}: the Shape button is there and enabled");
        click_shape_button(&mut h);
        h.run_steps(3);
        let made = shapes(&h);
        assert_eq!(made.len(), 1, "{tool:?}: the shape layer");
        assert_eq!(made[0].1.subpaths[0].knots.len(), 4, "{tool:?}: the Work Path became the shape");
        assert_eq!(steps(&h), before + 1, "{tool:?}: one History step");
        assert!(alerts(h.state()).is_empty(), "{tool:?}: no alert");
    }
}
