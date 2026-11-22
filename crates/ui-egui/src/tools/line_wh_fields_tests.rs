//! #13250: the Line tool's options bar shows W, a link button and H after the stroke controls. With
//! no line active they read 0 px and the link is off; with a line active W is its length and H its
//! thickness (the weight, or the arrowhead width when wider). Typing W or H resizes the line in its
//! own rotated frame as one "Scale Paths" step, keeping it a live line; the tool's own Weight and
//! arrowhead options for the next line do not change.
//!
//! H follows the measured numbers: the start of a 15 px thick line moves from 400,300 to
//! 398.88,302.24 for H 20, a scale about the edge of the line's box on the start's side, not its
//! centre line (a scale about the centre line would keep the start at 400,300).

use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, LayerId, LiveShape};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

/// An 800 x 600 document with the Line tool picked, its options bar in a harness; `drawn` drags a
/// line from 400,300 to 600,400 with the tool first.
fn bar(drawn: bool) -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Line;
    if drawn {
        let none = egui::Modifiers::NONE;
        tool_event(&mut app, ToolEvent::Down { x: 400.0, y: 300.0, pressure: 1.0 }, none);
        tool_event(&mut app, ToolEvent::Move { x: 500.0, y: 350.0, pressure: 1.0 }, none);
        tool_event(&mut app, ToolEvent::Move { x: 600.0, y: 400.0, pressure: 1.0 }, none);
        tool_event(&mut app, ToolEvent::Up { x: 600.0, y: 400.0 }, none);
    }
    let mut h =
        Harness::builder().with_context(app.ctx().clone()).with_size(vec2(2400.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    h.run_steps(4);
    h
}

/// The drawn line with the End arrowhead on, 15 px wide and 10 px long, set as the Properties panel sets it.
fn arrow_bar() -> (H, LayerId) {
    let mut h = bar(true);
    let id = active(&h);
    h.state_mut().run("shape.edit", json!({"layer": id.0, "arrowEnd": true, "arrowWidth": 15, "arrowLength": 10})).unwrap();
    h.run_steps(3);
    (h, id)
}

fn active(h: &H) -> LayerId {
    h.state().session.active().unwrap().active_layer.unwrap()
}

/// The SpinButton labelled `label`.
fn field<'h>(h: &'h H, label: &'h str) -> egui_kittest::Node<'h> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(label)).unwrap_or_else(|| panic!("the {label} field"))
}

/// The number the field labelled `label` shows.
fn shown(h: &H, label: &str) -> f64 {
    let text = field(h, label).value().unwrap_or_else(|| panic!("the {label} field shows a value"));
    // The decimal mark follows the locale the test runs in.
    let number = text.trim_end_matches("px").trim().replace(',', ".");
    number.parse().unwrap_or_else(|e| panic!("the {label} field shows a number, not {text:?} ({e})"))
}

/// Types `text` into the field labelled `label`, then Tabs out of it.
fn type_into(h: &mut H, label: &str, text: &str) {
    field(h, label).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

/// Whether the link button is on.
fn link_on(h: &H) -> bool {
    h.get_by_role_and_label(Role::Button, "Maintain aspect ratio").accesskit_node().toggled() == Some(Toggled::True)
}

/// The live arrow line of layer `id`: from, to, weight, width, length.
fn arrow(h: &H, id: LayerId) -> ([f64; 2], [f64; 2], f64, f64, f64) {
    match &h.state().session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => match &sh.live {
            Some(LiveShape::ArrowLine { from, to, weight, width, length, end: true, .. }) => (*from, *to, *weight, *width, *length),
            other => panic!("a live line with its End arrowhead, got {other:?}"),
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

/// The tool's own settings the next line is drawn with.
fn tool_settings(h: &H) -> (f32, bool, bool, f32, f32) {
    let o = &h.state().ui.tool_options;
    (o.line_weight, o.arrow_start, o.arrow_end, o.arrow_width, o.arrow_length)
}

#[test]
fn no_line_reads_zero_and_the_link_is_off() {
    let h = bar(false);
    assert_eq!(shown(&h, "W:"), 0.0);
    assert_eq!(shown(&h, "H:"), 0.0);
    assert!(!link_on(&h), "the link is off");
}

#[test]
fn a_drawn_line_reads_its_length_and_thickness() {
    let h = bar(true);
    assert!(near(shown(&h, "W:"), 223.61, 0.006), "W {}", shown(&h, "W:"));
    assert_eq!(shown(&h, "H:"), 1.0);
    assert!(!link_on(&h), "the link is off");
}

#[test]
fn w_then_h_scale_the_line_in_its_own_frame_as_scale_paths_steps() {
    let (mut h, id) = arrow_bar();
    let tool = tool_settings(&h);
    assert_eq!(shown(&h, "H:"), 15.0, "the thickness includes the 15 px arrowhead");
    let before_w = (arrow(&h, id), steps(&h));

    type_into(&mut h, "W:", "300");
    let (from, to, weight, width, length) = arrow(&h, id);
    assert_pt(from, [400.0, 300.0], "W keeps the start");
    assert_pt(to, [668.33, 434.16], "W moves the end along the line");
    assert!(near(length, 13.416, 0.001), "arrowhead length {length}");
    assert!(near(weight, 1.0, 1e-9) && near(width, 15.0, 1e-9), "weight {weight}, width {width}");
    assert_eq!(steps(&h), before_w.1 + 1, "one step");
    assert_eq!(undo_label(&h).as_deref(), Some("Scale Paths"));
    assert!(near(shown(&h, "W:"), 300.0, 0.006));

    let before_h = (arrow(&h, id), steps(&h));
    type_into(&mut h, "H:", "20");
    let (from, to, weight, width, length) = arrow(&h, id);
    assert!(near(weight, 1.333, 0.001), "weight {weight}");
    assert!(near(width, 20.0, 1e-9), "arrowhead width {width}");
    assert!(near(length, 13.416, 0.001), "arrowhead length {length}");
    assert_pt(from, [398.88, 302.24], "H moves the start across the line");
    assert!(near((to[0] - from[0]).hypot(to[1] - from[1]), 300.0, 1e-6), "H keeps the length");
    assert_eq!(steps(&h), before_h.1 + 1, "one step");
    assert_eq!(undo_label(&h).as_deref(), Some("Scale Paths"));
    assert_eq!(shown(&h, "H:"), 20.0);

    assert_eq!(tool_settings(&h), tool, "the tool's Weight and arrowhead options are unchanged");

    // Each edit undoes in one step.
    h.state_mut().session.undo();
    h.run_steps(2);
    assert_eq!((arrow(&h, id), steps(&h)), before_h);
    h.state_mut().session.undo();
    h.run_steps(2);
    assert_eq!((arrow(&h, id), steps(&h)), before_w);
}

#[test]
fn zero_negative_or_no_number_changes_nothing() {
    let (mut h, id) = arrow_bar();
    let before = (arrow(&h, id), steps(&h));
    for typed in ["0", "-5", "abc", "NaN", "inf"] {
        for label in ["W:", "H:"] {
            type_into(&mut h, label, typed);
            assert_eq!((arrow(&h, id), steps(&h)), before, "{label} {typed}");
        }
    }
}

#[test]
fn typing_without_a_line_edits_nothing() {
    let mut h = bar(false);
    let n = h.state().session.active().unwrap().doc.layers.len();
    type_into(&mut h, "W:", "300");
    type_into(&mut h, "H:", "20");
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), n);
    assert_eq!((shown(&h, "W:"), shown(&h, "H:")), (0.0, 0.0));
}

#[test]
fn the_frame_scale_helper_refuses_sizes_that_are_not_positive_and_finite() {
    use photocraft_engine::vector_cmds::{line_frame_scale, line_frame_size};
    let line =
        LiveShape::ArrowLine { from: [400.0, 300.0], to: [600.0, 400.0], weight: 1.0, start: false, end: true, width: 15.0, length: 10.0, concavity: 0.0 };
    let (len, thick) = line_frame_size(&line).unwrap();
    assert!(near(len, 223.607, 0.001) && thick == 15.0);
    for (l, t) in [(0.0, 15.0), (300.0, 0.0), (f64::NAN, 15.0), (300.0, f64::INFINITY), (-1.0, 15.0)] {
        assert!(line_frame_scale(&line, l, t).is_none(), "{l} x {t}");
    }
    let flat = LiveShape::ArrowLine { from: [1.0, 1.0], to: [1.0, 1.0], weight: 1.0, start: false, end: false, width: 15.0, length: 10.0, concavity: 0.0 };
    assert!(line_frame_size(&flat).is_none() && line_frame_scale(&flat, 300.0, 20.0).is_none());
    let rect = LiveShape::Rect { rect: [0.0, 0.0, 10.0, 10.0], radii: [0.0; 4] };
    assert!(line_frame_size(&rect).is_none() && line_frame_scale(&rect, 300.0, 20.0).is_none());
    // A plain line's thickness is its weight, whatever the (hidden) arrowhead width.
    let plain =
        LiveShape::ArrowLine { from: [400.0, 300.0], to: [600.0, 400.0], weight: 1.0, start: false, end: false, width: 15.0, length: 10.0, concavity: 0.0 };
    assert_eq!(line_frame_size(&plain).map(|s| s.1), Some(1.0));
}

#[test]
fn another_transform_of_a_line_keeps_its_history_name() {
    let (mut h, id) = arrow_bar();
    // A rotation is no scale in the line's frame.
    let (s, c) = 0.3f64.sin_cos();
    h.state_mut().run("shape.edit", json!({"layer": id.0, "transform": [c, s, -s, c, 0.0, 0.0]})).unwrap();
    assert_ne!(undo_label(&h).as_deref(), Some("Scale Paths"));
    // A move is none either.
    h.state_mut().run("shape.edit", json!({"layer": id.0, "transform": [1.0, 0.0, 0.0, 1.0, 5.0, 5.0]})).unwrap();
    assert_ne!(undo_label(&h).as_deref(), Some("Scale Paths"));
}

/// The live plain line (no arrow parameters) of layer `id`: from, to, weight.
fn plain(h: &H, id: LayerId) -> ([f64; 2], [f64; 2], f64) {
    match &h.state().session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => match &sh.live {
            Some(LiveShape::Line { from, to, weight }) => (*from, *to, *weight),
            other => panic!("a live plain line, got {other:?}"),
        },
        _ => panic!("a shape layer"),
    }
}

#[test]
fn w_then_h_keep_a_diagonal_plain_line_live() {
    // `shape.create` without arrow keys (or an imported line) makes a plain `LiveShape::Line`.
    let mut h = bar(false);
    h.state_mut().run("shape.create", json!({"kind": "line", "from": [0, 0], "to": [200, 100], "weight": 4})).unwrap();
    h.run_steps(3);
    let id = active(&h);
    plain(&h, id);
    assert!(near(shown(&h, "W:"), 223.61, 0.006), "W {}", shown(&h, "W:"));
    assert_eq!(shown(&h, "H:"), 4.0);

    type_into(&mut h, "W:", "300");
    let (from, to, weight) = plain(&h, id);
    assert_pt(from, [0.0, 0.0], "W keeps the start");
    assert_pt(to, [268.33, 134.16], "W moves the end along the line");
    assert!(near(weight, 4.0, 1e-9), "weight {weight}");
    assert_eq!(undo_label(&h).as_deref(), Some("Scale Paths"));
    assert!(near(shown(&h, "W:"), 300.0, 0.006));

    type_into(&mut h, "H:", "8");
    let (from, to, weight) = plain(&h, id);
    assert!(near(weight, 8.0, 1e-9), "weight {weight}");
    assert!(near((to[0] - from[0]).hypot(to[1] - from[1]), 300.0, 1e-6), "H keeps the length");
    // The pivot is the box edge 2 px off the start; doubling the thickness moves the start 2 px
    // along the normal (-0.447, 0.894).
    assert_pt(from, [-0.894, 1.789], "H moves the start across the line");
    assert_eq!(undo_label(&h).as_deref(), Some("Scale Paths"));
    assert_eq!(shown(&h, "H:"), 8.0);
}
