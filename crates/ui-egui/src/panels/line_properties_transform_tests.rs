//! #13265: the Properties panel's Transform section for a live line shows its length (W), its
//! thickness (H), the top-left corner of the box around it (X / Y), its angle and the Flip
//! Horizontal / Flip Vertical buttons, as the reference does (W 223.61, H 1, X 399.78, Y 299.56,
//! 26.57° for a 1 px line from 400,300 to 600,400). W and H scale the line in its own rotated
//! frame, like the Line tool's options-bar W / H (#13250): one "Scale Paths" step that keeps it a
//! live line. The angle field only shows the angle (typed angles were not measured). Other shape
//! layers and pixel layers keep their Transform section.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, LayerId, LiveShape};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The Properties panel body for the active layer, as the Properties panel shows it.
fn panel(app: Box<PhotocraftApp>) -> H {
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(420.0, 2400.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            let app: &mut PhotocraftApp = app;
            let layer = app.session.active().and_then(|s| s.active_layer.and_then(|id| s.doc.layer(id).cloned()));
            if let Some(layer) = layer {
                crate::layer_props_ui::properties(app, ui, &layer);
            }
        },
        app,
    );
    h.run_steps(4);
    h
}

/// An 800 x 600 document.
fn doc() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app
}

/// A line dragged with the Line tool from 400,300 to 600,400 (1 px, the tool's default weight).
fn line() -> (H, LayerId) {
    let mut app = doc();
    app.ui.tool = Tool::Line;
    let none = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 400.0, y: 300.0, pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Move { x: 500.0, y: 350.0, pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Move { x: 600.0, y: 400.0, pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Up { x: 600.0, y: 400.0 }, none);
    let id = app.session.active().unwrap().active_layer.unwrap();
    (panel(app), id)
}

/// The drawn line with the End arrowhead on, 15 px wide and 10 px long.
fn arrow() -> (H, LayerId) {
    let (mut h, id) = line();
    h.state_mut().run("shape.edit", json!({"layer": id.0, "arrowEnd": true, "arrowWidth": 15, "arrowLength": 10})).unwrap();
    h.run_steps(3);
    (h, id)
}

/// The SpinButton labelled `label`, if shown.
fn try_field<'h>(h: &'h H, label: &'h str) -> Option<egui_kittest::Node<'h>> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(label))
}

/// The number the field labelled `label` shows.
fn shown(h: &H, label: &str) -> f64 {
    let node = try_field(h, label).unwrap_or_else(|| panic!("the {label} field"));
    let text = node.value().unwrap_or_else(|| panic!("the {label} field shows a value"));
    // The decimal mark follows the locale the test runs in.
    let number = text.trim_end_matches("px").trim_end_matches('°').trim().replace(',', ".");
    number.parse().unwrap_or_else(|e| panic!("the {label} field shows a number, not {text:?} ({e})"))
}

/// Types `text` into the field labelled `label`, then Tabs out of it.
fn type_into(h: &mut H, label: &str, text: &str) {
    try_field(h, label).unwrap_or_else(|| panic!("the {label} field")).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

/// Whether the button labelled `label` is shown.
fn has_button(h: &H, label: &str) -> bool {
    h.query_all_by_role(Role::Button).any(|n| n.accesskit_node().label().as_deref() == Some(label))
}

fn click_button(h: &mut H, label: &str) {
    h.query_all_by_role(Role::Button).find(|n| n.accesskit_node().label().as_deref() == Some(label)).unwrap_or_else(|| panic!("the {label} button")).click();
    h.run_steps(3);
}

/// The live line of layer `id`: from, to, weight, arrowhead width, arrowhead length.
fn live(h: &H, id: LayerId) -> ([f64; 2], [f64; 2], f64, f64, f64) {
    match &h.state().session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => match &sh.live {
            Some(LiveShape::ArrowLine { from, to, weight, width, length, .. }) => (*from, *to, *weight, *width, *length),
            Some(LiveShape::Line { from, to, weight }) => (*from, *to, *weight, 0.0, 0.0),
            other => panic!("a live line, got {other:?}"),
        },
        _ => panic!("a shape layer"),
    }
}

fn steps(h: &H) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

fn undo_label(h: &H) -> Option<String> {
    h.state().session.active().unwrap().history.undo_label().map(str::to_owned)
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn assert_pt(got: [f64; 2], want: [f64; 2], what: &str) {
    assert!(near(got[0], want[0], 0.01) && near(got[1], want[1], 0.01), "{what}: {got:?}, want {want:?}");
}

#[test]
fn a_line_shows_its_length_thickness_angle_and_the_flip_buttons() {
    let (h, _) = line();
    // The pixel fields show whole pixels (the framework's pixel unit has no decimals): W 223.61
    // reads 224, X / Y (the box's top-left, 399.776 / 299.553) read 400 / 300. The exact values
    // are checked on the row in `the_line_row_holds_the_exact_length_thickness_and_angle`.
    assert_eq!(shown(&h, "Width"), 224.0, "W is the length");
    assert_eq!(shown(&h, "Height"), 1.0, "H is the thickness");
    assert_eq!((shown(&h, "X position"), shown(&h, "Y position")), (400.0, 300.0));
    assert!(near(shown(&h, "Angle"), 26.57, 0.006), "angle {}", shown(&h, "Angle"));
    assert!(has_button(&h, "Flip Horizontal") && has_button(&h, "Flip Vertical"), "both flip buttons");
}

#[test]
fn h_includes_a_wider_arrowhead() {
    let (h, _) = arrow();
    assert_eq!(shown(&h, "Width"), 224.0);
    assert_eq!(shown(&h, "Height"), 15.0, "H includes the 15 px arrowhead");
    assert!(near(shown(&h, "Angle"), 26.57, 0.006));
}

#[test]
fn w_scales_along_the_line_and_h_across_it_as_scale_paths_steps() {
    let (mut h, id) = arrow();
    let before_w = (live(&h, id), steps(&h));

    type_into(&mut h, "Width", "300");
    let (from, to, weight, width, length) = live(&h, id);
    assert_pt(from, [400.0, 300.0], "W keeps the start");
    assert_pt(to, [668.33, 434.16], "W moves the end along the line");
    assert!(near(length, 13.416, 0.001), "arrowhead length {length}");
    assert!(near(weight, 1.0, 1e-9) && near(width, 15.0, 1e-9), "weight {weight}, width {width}");
    assert_eq!(steps(&h), before_w.1 + 1, "one step");
    assert_eq!(undo_label(&h).as_deref(), Some("Scale Paths"));
    assert!(near(shown(&h, "Width"), 300.0, 0.006), "W {}", shown(&h, "Width"));
    assert!(near(shown(&h, "Height"), 15.0, 0.006), "H is unchanged: {}", shown(&h, "Height"));
    assert!(near(shown(&h, "Angle"), 26.57, 0.006), "the angle is kept");

    let before_h = (live(&h, id), steps(&h));
    type_into(&mut h, "Height", "20");
    let (from, to, weight, width, length) = live(&h, id);
    assert!(near(weight, 1.333, 0.001), "weight {weight}");
    assert!(near(width, 20.0, 1e-9), "arrowhead width {width}");
    assert!(near(length, 13.416, 0.001), "arrowhead length {length}");
    assert_pt(from, [398.88, 302.24], "H moves the start across the line");
    assert!(near((to[0] - from[0]).hypot(to[1] - from[1]), 300.0, 1e-6), "H keeps the length");
    assert_eq!(steps(&h), before_h.1 + 1, "one step");
    assert_eq!(undo_label(&h).as_deref(), Some("Scale Paths"));
    assert!(near(shown(&h, "Height"), 20.0, 0.006), "H {}", shown(&h, "Height"));

    // Each edit undoes in one step.
    h.state_mut().session.undo();
    h.run_steps(2);
    assert_eq!((live(&h, id), steps(&h)), before_h);
    h.state_mut().session.undo();
    h.run_steps(2);
    assert_eq!((live(&h, id), steps(&h)), before_w);
}

#[test]
fn w_of_a_plain_line_keeps_its_weight() {
    let (mut h, id) = line();
    type_into(&mut h, "Width", "300");
    let (from, to, weight, _, _) = live(&h, id);
    assert_pt(from, [400.0, 300.0], "the start");
    assert_pt(to, [668.33, 434.16], "the end");
    assert!(near(weight, 1.0, 1e-9), "weight {weight}");
    assert_eq!(undo_label(&h).as_deref(), Some("Scale Paths"));
}

#[test]
fn w_and_h_keep_a_diagonal_shape_create_line_live() {
    // `shape.create` without arrow keys (or an imported line) makes a plain `LiveShape::Line`.
    let mut app = doc();
    app.run("shape.create", json!({"kind": "line", "from": [0, 0], "to": [200, 100], "weight": 4})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let mut h = panel(app);
    assert!(matches!(
        &h.state().session.active().unwrap().doc.layer(id).unwrap().content,
        LayerContent::Shape(sh) if matches!(sh.live, Some(LiveShape::Line { .. }))
    ));
    type_into(&mut h, "Width", "300");
    let (from, to, weight, _, _) = live(&h, id);
    assert_pt(from, [0.0, 0.0], "the start");
    assert_pt(to, [268.33, 134.16], "the end");
    assert!(near(weight, 4.0, 1e-9), "weight {weight}");
    assert_eq!(undo_label(&h).as_deref(), Some("Scale Paths"));
    type_into(&mut h, "Height", "8");
    let (from, to, weight, _, _) = live(&h, id);
    assert!(near(weight, 8.0, 1e-9), "weight {weight}");
    assert!(near((to[0] - from[0]).hypot(to[1] - from[1]), 300.0, 1e-6), "H keeps the length");
    assert_eq!((shown(&h, "Width"), shown(&h, "Height")), (300.0, 8.0));
}

#[test]
fn the_flip_buttons_mirror_the_line_and_keep_it_live() {
    let (mut h, id) = arrow();
    let (from, to, weight, width, length) = live(&h, id);
    let n = steps(&h);

    click_button(&mut h, "Flip Horizontal");
    let (f, t, w, wd, l) = live(&h, id);
    assert_eq!(steps(&h), n + 1, "one step");
    assert!(near(t[0] - f[0], -(to[0] - from[0]), 1e-6) && near(t[1] - f[1], to[1] - from[1], 1e-6), "mirrored left to right: {f:?} {t:?}");
    assert!(near(w, weight, 1e-9) && near(wd, width, 1e-9) && near(l, length, 1e-9), "the weight and arrowhead are kept");
    assert!(near(shown(&h, "Angle"), 153.43, 0.006), "angle {}", shown(&h, "Angle"));
    assert_eq!(shown(&h, "Width"), 224.0);

    click_button(&mut h, "Flip Vertical");
    let (f2, t2, ..) = live(&h, id);
    assert_eq!(steps(&h), n + 2, "one step");
    assert!(near(t2[0] - f2[0], t[0] - f[0], 1e-6) && near(t2[1] - f2[1], -(t[1] - f[1]), 1e-6), "mirrored top to bottom: {f2:?} {t2:?}");
    assert!(near(shown(&h, "Angle"), -153.43, 0.006), "angle {}", shown(&h, "Angle"));
}

#[test]
fn a_typed_angle_changes_nothing() {
    let (mut h, id) = line();
    let before = (live(&h, id), steps(&h));
    type_into(&mut h, "Angle", "45");
    assert_eq!((live(&h, id), steps(&h)), before);
    assert!(near(shown(&h, "Angle"), 26.57, 0.006), "the field shows the line's angle again");
}

/// Focusing W or H and leaving it, which re-reads the field's rounded text (224 for 223.61), is
/// no edit.
#[test]
fn focusing_and_leaving_a_field_changes_nothing() {
    let (mut h, id) = line();
    let before = (live(&h, id), steps(&h));
    for label in ["Width", "Height"] {
        try_field(&h, label).unwrap_or_else(|| panic!("the {label} field")).click();
        h.run_steps(1);
        h.key_press(egui::Key::Tab);
        h.run_steps(3);
        assert_eq!((live(&h, id), steps(&h)), before, "{label}");
    }
}

#[test]
fn zero_negative_or_no_number_changes_nothing() {
    let (mut h, id) = arrow();
    let before = (live(&h, id), steps(&h));
    for typed in ["0", "-5", "abc", "NaN", "inf"] {
        for label in ["Width", "Height"] {
            type_into(&mut h, label, typed);
            assert_eq!((live(&h, id), steps(&h)), before, "{label} {typed}");
        }
    }
}

#[test]
fn a_rectangle_keeps_its_transform_section() {
    let mut app = doc();
    app.run("shape.create", json!({"kind": "rect", "rect": [200, 150, 400, 300], "fill": "#3366cc"})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let mut h = panel(app);
    assert_eq!((shown(&h, "Width"), shown(&h, "Height"), shown(&h, "X position"), shown(&h, "Y position")), (400.0, 300.0, 200.0, 150.0));
    assert!(try_field(&h, "Angle").is_none(), "no angle field");
    assert!(!has_button(&h, "Flip Horizontal") && !has_button(&h, "Flip Vertical"), "no flip buttons");
    // W still scales the box from its top-left, linked.
    type_into(&mut h, "Width", "200");
    let LayerContent::Shape(sh) = &h.state().session.active().unwrap().doc.layer(id).unwrap().content else { panic!("a shape") };
    let c = sh.path.control_bounds().unwrap();
    assert_eq!((c.0, c.1, c.2 - c.0, c.3 - c.1), (200.0, 150.0, 200.0, 150.0));
}

#[test]
fn a_pixel_layer_keeps_its_transform_section() {
    let mut app = doc();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 10, "y": 20, "width": 100, "height": 60})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let h = panel(app);
    assert_eq!((shown(&h, "Width"), shown(&h, "Height"), shown(&h, "X position"), shown(&h, "Y position")), (100.0, 60.0, 10.0, 20.0));
    assert!(try_field(&h, "Angle").is_none(), "no angle field");
    assert!(!has_button(&h, "Flip Horizontal") && !has_button(&h, "Flip Vertical"), "no flip buttons");
}

/// The exact numbers behind the rounded pixel fields: the row reads W 223.61, H 1 and 26.57°,
/// and X / Y are the top-left of the box around the line, 399.78 / 299.55.
#[test]
fn the_line_row_holds_the_exact_length_thickness_and_angle() {
    let (h, id) = line();
    let app = h.state();
    let layer = app.session.active().unwrap().doc.layer(id).unwrap().clone();
    let LayerContent::Shape(sh) = &layer.content else { panic!("a shape") };
    let c = sh.path.control_bounds().unwrap();
    assert!(near(c.0, 399.78, 0.005) && near(c.1, 299.55, 0.005), "the box's top-left {c:?}");
    let (rect, row, _) = super::line_row(&layer, [c.0, c.1]).expect("a line row");
    let parts = row.state.parts(rect, 0.0);
    assert!(near(parts.w * rect.width(), 223.607, 0.001), "W {}", parts.w * rect.width());
    assert!(near(parts.h * rect.height(), 1.0, 1e-9), "H {}", parts.h * rect.height());
    assert!(near(parts.angle, 26.565, 0.001), "angle {}", parts.angle);
    assert_eq!((row.state.pivot.x, row.state.pivot.y), (c.0, c.1), "X / Y are the box's top-left");
}

/// Other shapes and pixel layers get no line row; a non-finite corner gives none either.
#[test]
fn only_a_live_line_has_a_line_row() {
    let (h, id) = line();
    let layer = h.state().session.active().unwrap().doc.layer(id).unwrap().clone();
    assert!(super::line_row(&layer, [f64::NAN, 0.0]).is_none());
    let mut app = doc();
    app.run("shape.create", json!({"kind": "rect", "rect": [200, 150, 400, 300], "fill": "#3366cc"})).unwrap();
    let rect = app.session.active().unwrap().active_layer.unwrap();
    let pixel = LayerId(app.run("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    let st = app.session.active().unwrap();
    assert!(super::line_row(st.doc.layer(rect).unwrap(), [200.0, 150.0]).is_none());
    assert!(super::line_row(st.doc.layer(pixel).unwrap(), [0.0, 0.0]).is_none());
}

/// X and Y still move a line, keeping it live.
#[test]
fn x_moves_the_line() {
    let (mut h, id) = line();
    type_into(&mut h, "X position", "410");
    let (from, to, ..) = live(&h, id);
    assert_pt(from, [410.0, 300.0], "the start");
    assert_pt(to, [610.0, 400.0], "the end");
}
