//! #12907: the Pen (and Freeform / Curvature Pen) Shape-mode options bar shows Fill and Stroke
//! swatches with the shape tools' popups, stroke width and type, W / H, and Align Edges, in the
//! reference's order: Fill: swatch, Stroke: swatch, width, type, W:, H:, Align Edges (on).
//! W and H read 0 px with no shape and the live size of the shape being drawn (200 x 200 for a
//! 200 x 200 triangle); a one-anchor or a flat (line) draft keeps 0 px on its flat axis, and a
//! Freeform Pen trace still under the pointer is measured from that tool's own outline points. The
//! not-measured cases (the stroke default, typing into W / H) keep today's behaviour.

use egui::Modifiers;
use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_doc::{LayerContent, ShapeLayer};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

/// The options bar of `tool` (the bar follows `app.ui.tool`) on a 400 x 300 RGB document,
/// with the dialogs the chip popups open drawn as the app draws them.
fn bar(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.tool = tool;
    let ctx = app.ctx().clone();
    let mut h = Harness::builder().with_size(vec2(1800.0, 700.0)).with_step_dt(0.25).with_context(ctx).build_ui_state(
        move |ui, app: &mut Box<PhotocraftApp>| {
            crate::screen_picker::tick(app, ui.ctx());
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            let tool = app.ui.tool;
            ui.horizontal(|ui| super::options_bar(app, ui, tool));
            crate::dialogs::run(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    h
}

/// The Pen options bar switched to Shape mode through its mode dropdown.
fn shape_bar() -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = bar(Tool::Pen);
    pick_mode(&mut h, "Shape");
    h
}

/// The Pen-family mode dropdown: the one showing a Pen mode.
fn dropdown<'a>(h: &'a Harness<'static, Box<PhotocraftApp>>) -> egui_kittest::Node<'a> {
    let modes = ["Shape", "Path"];
    h.query_all_by_role(Role::ComboBox).find(|n| n.value().is_some_and(|v| modes.contains(&v.as_str()))).expect("the mode dropdown")
}

/// Clicks the mode dropdown and picks its `entry`, as a hand does.
fn pick_mode(h: &mut Harness<'static, Box<PhotocraftApp>>, entry: &str) {
    let at = dropdown(h).rect().center();
    crate::layers_panel_nodes::click(h, at, Modifiers::NONE);
    h.get_by_role_and_label(Role::ListBoxOption, entry).click();
    h.run_steps(2);
}

/// The chip of the paint named `label` (a colour well, as in the shape tools' bar).
fn chip<'h>(h: &'h Harness<'static, Box<PhotocraftApp>>, label: &'h str) -> egui_kittest::Node<'h> {
    h.get_by_role_and_label(Role::ColorWell, label)
}

/// Opens the chip `label` and picks its kind button `kind` ("No Color", "Solid").
fn pick_kind(h: &mut Harness<'static, Box<PhotocraftApp>>, label: &str, kind: &str) {
    chip(h, label).click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::RadioButton, kind).click();
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
}

/// Clicks the popover's swatch named `name` (a list item "<name>, <hex>" in the Swatches list).
/// The list is the Solid paint's, and a group's rows show once its header is open.
fn click_swatch(h: &mut Harness<'static, Box<PhotocraftApp>>, name: &str) {
    let prefix = format!("{name}, #");
    let found = |h: &Harness<'static, Box<PhotocraftApp>>| {
        h.query_all_by_role(Role::ListItem).any(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.starts_with(&prefix)))
    };
    if !found(h) {
        let group = name.split(' ').next().unwrap_or(name);
        if h.query_by_role_and_label(Role::Button, group).is_none() {
            h.get_by_role_and_label(Role::RadioButton, "Solid").click();
            h.run_steps(2);
        }
        if !found(h) {
            h.get_by_role_and_label(Role::Button, group).click();
            h.run_steps(2);
        }
    }
    h.query_all_by_role(Role::ListItem)
        .find(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.starts_with(&prefix)))
        .unwrap_or_else(|| panic!("the swatch {name}"))
        .click();
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
}

/// The SpinButton field labelled `label`.
fn field<'h>(h: &'h Harness<'static, Box<PhotocraftApp>>, label: &'h str) -> egui_kittest::Node<'h> {
    h.get_by_role_and_label(Role::SpinButton, label)
}

/// The number the field labelled `label` shows: its accessible value without the unit after it ("200 px" reads
/// "200").
fn field_text(h: &Harness<'static, Box<PhotocraftApp>>, label: &str) -> String {
    let value = field(h, label).value().unwrap_or_else(|| panic!("the {label} field shows a value"));
    value.split(' ').next().unwrap_or_default().to_owned()
}

/// The number the view field labelled `label` shows, as a `f64` for the cases whose size comes
/// back through the document's screen mapping and is not a round integer. The parse is deliberately
/// dot-based: the harness runs with the built-in (en) locale, so the field shows `0.5`; a locale
/// switch in the framework would show `0,5` and this would need a locale pin, not a new parser.
fn field_number(h: &Harness<'static, Box<PhotocraftApp>>, label: &str) -> f64 {
    let shown = field_text(h, label);
    shown.parse().unwrap_or_else(|e| panic!("the {label} field shows a number, not {shown:?} ({e})"))
}

/// Clicks a closed square with the Pen: four corners and the click back on the first anchor.
fn square(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    let app = h.state_mut();
    for (x, y) in [(20.0, 20.0), (220.0, 20.0), (220.0, 220.0), (20.0, 220.0)] {
        testing::click(app, [x, y], Modifiers::NONE);
    }
    testing::click(app, [20.5, 20.5], Modifiers::NONE); // close
    h.run_steps(3);
}

/// The document's only shape layer; panics unless there is exactly one.
fn the_shape(h: &Harness<'static, Box<PhotocraftApp>>) -> ShapeLayer {
    let st = h.state().session.active().unwrap();
    let shapes: Vec<&ShapeLayer> = st
        .doc
        .walk()
        .iter()
        .filter_map(|(_, _, l)| match &l.content {
            LayerContent::Shape(sh) => Some(sh),
            _ => None,
        })
        .collect();
    assert_eq!(shapes.len(), 1, "exactly one shape layer");
    shapes[0].clone()
}

fn color_of(f: &photocraft_doc::Fill) -> [f32; 4] {
    let photocraft_doc::Fill::Solid(c) = f else { panic!("a solid fill") };
    let [r, g, b] = c.to_rgb();
    [r, g, b, c.alpha]
}

// ---------------------------------------------------------------------------------------------
// Done when 1, 2 and 3 together: the measured row of the Shape-mode bar

/// The measured row: Fill swatch, Stroke swatch, stroke width, stroke type, W, H, Align Edges —
/// all present and left to right in that order.
#[test]
fn shape_mode_bar_shows_fill_stroke_wh_and_align_edges_in_measured_order() {
    let h = shape_bar();
    let fill = chip(&h, "Fill:").rect();
    let stroke = chip(&h, "Stroke:").rect();
    let width = field(&h, "Stroke width").rect();
    let style = h.get_by_role_and_label(Role::Button, "Stroke style").rect();
    let w = field(&h, "W:").rect();
    let hh = field(&h, "H:").rect();
    let align = h.get_by_label("Align Edges");
    assert!(!align.accesskit_node().is_disabled(), "Align Edges is enabled in Shape mode");
    assert_eq!(align.accesskit_node().toggled(), Some(Toggled::True), "Align Edges is on in Shape mode");
    let left = [fill, stroke, width, style, w, hh, align.rect()];
    for pair in left.windows(2) {
        assert!(pair[0].min.x < pair[1].min.x, "the row order puts {:?} left of {:?}", pair[0], pair[1]);
    }
}

// ---------------------------------------------------------------------------------------------
// Done when 1: a fill picked in the Pen bar paints the next Pen shape, whatever the foreground

/// The reference (#12907): with the foreground set to #3366CC, a fill picked in the Pen bar's Fill
/// swatch fills the next Pen shape; the foreground colour does not win.
#[test]
fn a_fill_picked_in_the_pen_bar_paints_the_next_pen_shape() {
    let mut h = shape_bar();
    h.state_mut().run("tools.setColors", json!({"foreground": [0.2, 0.4, 0.8, 1.0]})).unwrap();
    h.run_steps(2);
    chip(&h, "Fill:").click();
    h.run_steps(4);
    click_swatch(&mut h, "RGB Green");
    square(&mut h);
    let sh = the_shape(&h);
    assert_eq!(sh.fill.as_ref().map(color_of), Some([0.0, 1.0, 0.0, 1.0]), "the picked swatch fills the Pen shape");
}

/// No Color in the Fill swatch leaves the next Pen shape unfilled; Solid brings the fill back
/// (the popup works both ways on the Pen bar, not only on the shape tools').
#[test]
fn no_colour_in_the_pen_fill_swatch_leaves_the_next_pen_shape_unfilled() {
    let mut h = shape_bar();
    pick_kind(&mut h, "Fill:", "No Color");
    square(&mut h);
    assert!(the_shape(&h).fill.is_none(), "No Color: the Pen shape has no fill");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(2);
    pick_kind(&mut h, "Fill:", "Solid");
    square(&mut h);
    assert!(the_shape(&h).fill.is_some(), "Solid: the next Pen shape is filled again");
}

/// A Pen-bar pick edits the tool defaults, never the shape layer that happens to be active: the
/// layer keeps its own paint and no History step is added (the Pen row is the next shape's, as
/// the Shape tools' bar without a selection; Properties remains the way to edit a layer).
#[test]
fn a_pen_bar_pick_leaves_the_active_shape_layer_alone() {
    let mut h = shape_bar();
    h.state_mut().run("shape.create", json!({"kind": "rect", "rect": [20, 20, 60, 40], "fill": "#336699"})).unwrap();
    h.run_steps(2);
    let before = h.state().session.active().unwrap().history.past_len();
    chip(&h, "Fill:").click();
    h.run_steps(4);
    click_swatch(&mut h, "RGB Green");
    let app = h.state();
    assert_eq!(color_of(the_shape(&h).fill.as_ref().expect("a fill")), [0.2, 0.4, 0.6, 1.0], "the active layer keeps its own fill");
    assert_eq!(app.session.active().unwrap().history.past_len(), before, "no document edit");
    assert_eq!(app.session.tools.foreground, [0.0, 1.0, 0.0, 1.0], "the pick reached the tool, as the shape tools' bar does");
}

// ---------------------------------------------------------------------------------------------
// Done when 2: W and H show 0 px with no shape and the live size while drawing

/// The measured 200 x 200: W and H read 0 before the first click and the triangle's live size
/// while it is being drawn.
#[test]
fn w_and_h_show_zero_for_no_shape_and_the_live_size_while_drawing() {
    let mut h = shape_bar();
    assert_eq!(field_text(&h, "W:"), "0", "no shape: 0 px");
    assert_eq!(field_text(&h, "H:"), "0", "no shape: 0 px");
    let app = h.state_mut();
    for (x, y) in [(20.0, 20.0), (220.0, 20.0), (120.0, 220.0)] {
        testing::click(app, [x, y], Modifiers::NONE);
    }
    h.run_steps(3);
    assert_eq!(field_text(&h, "W:"), "200", "the 200 x 200 triangle: W");
    assert_eq!(field_text(&h, "H:"), "200", "the 200 x 200 triangle: H");
}

/// Degenerate drafts stay 0 px rather than a negative or missing number: one anchor shows 0 x 0,
/// and a straight line (two clicks at the same y) shows 200 x 0. `pen_shape_size` guards the empty
/// draft and `path_bounds` the flat box; the pin holds before and after this PR's change, since
/// the guard is part of the code the PR adds.
#[test]
fn degenerate_drafts_show_zero_on_the_flat_axis() {
    let mut h = shape_bar();
    testing::click(h.state_mut(), [20.0, 120.0], Modifiers::NONE);
    h.run_steps(2);
    assert_eq!(field_text(&h, "W:"), "0", "one anchor: 0 px wide");
    assert_eq!(field_text(&h, "H:"), "0", "one anchor: 0 px tall");
    testing::click(h.state_mut(), [220.0, 120.0], Modifiers::NONE);
    h.run_steps(2);
    assert_eq!(field_text(&h, "W:"), "200", "the line draft: 200 px wide");
    assert_eq!(field_text(&h, "H:"), "0", "the line draft: 0 px tall, not a negative height");
}

/// A Freeform Pen trace that is still under the pointer (#12907 review): while a trace is being
/// dragged its points live in the Freeform tool's own outline draft, and the shared pen draft holds
/// nothing until the pass ends, so the row measures the trace being drawn. The number itself is not
/// a measured case (the issue's measured live size is the click-drawn 200 x 200 triangle above, and
/// a freehand run has no reference number), so this pins the one thing the row must do here: follow
/// the trace instead of reading 0 px while it is still under the pointer.
#[test]
fn w_and_h_follow_a_freeform_trace_still_under_the_pointer() {
    let mut h = bar(Tool::FreeformPen);
    pick_mode(&mut h, "Shape");
    assert_eq!(field_number(&h, "W:"), 0.0, "no trace: 0 px");
    assert_eq!(field_number(&h, "H:"), 0.0, "no trace: 0 px");
    let app = h.state_mut();
    testing::press(app, [20.0, 20.0], Modifiers::NONE);
    for at in [[120.0, 20.0], [220.0, 20.0], [220.0, 120.0], [220.0, 220.0]] {
        testing::move_to(app, at, Modifiers::NONE);
    }
    assert!(app.canvas_tools.vector.freeform.draft.is_open(), "the trace is still being dragged");
    assert!(app.ui.pen.is_none(), "and the shared pen draft holds nothing yet");
    h.run_steps(2);
    assert!((field_number(&h, "W:") - 200.0).abs() < 1.0, "the trace in progress is 200 px wide, not 0");
    assert!((field_number(&h, "H:") - 200.0).abs() < 1.0, "the trace in progress is 200 px tall, not 0");
}

/// Not measured: what typing into W / H does. It keeps today's behaviour — the field shows the
/// live size and an edit changes neither the tool options nor the document.
#[test]
fn typing_into_w_and_h_changes_nothing() {
    let mut h = shape_bar();
    let before = h.state().session.active().unwrap().history.past_len();
    let w_before = h.state().ui.tool_options.clone();
    field(&h, "W:").click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("123".into()));
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    let app = h.state();
    assert_eq!(app.ui.tool_options, w_before, "not measured: the fields are read-only views today");
    assert_eq!(app.session.active().unwrap().history.past_len(), before, "no edit");
    assert_eq!(field_text(&h, "W:"), "0", "the field keeps showing the live size");
}

// ---------------------------------------------------------------------------------------------
// Done when 3: Align Edges on in Shape mode, greyed in Path mode

/// The checkbox is on and toggles in Shape mode; Path mode has no Fill / Stroke swatch, no W / H,
/// and its Align Edges is greyed rather than hidden (main's #11756 model; the issue allows
/// "absent or greyed").
#[test]
fn align_edges_is_on_in_shape_mode_and_greyed_in_path_mode() {
    let mut h = shape_bar();
    h.get_by_label("Align Edges").click();
    h.run_steps(3);
    assert!(!h.state().ui.tool_options.align_edges, "clicking toggles the option");
    assert_eq!(h.get_by_label("Align Edges").accesskit_node().toggled(), Some(Toggled::False));
    pick_mode(&mut h, "Path");
    assert!(h.query_by_label("Align Edges").is_some_and(|n| n.accesskit_node().is_disabled()), "Path mode: Align Edges greyed");
    assert!(h.query_by_role_and_label(Role::ColorWell, "Fill:").is_none(), "Path mode: no Fill swatch");
    assert!(h.query_by_role_and_label(Role::ColorWell, "Stroke:").is_none(), "Path mode: no Stroke swatch");
    assert!(h.query_by_label("W:").is_none(), "Path mode: no W");
    assert!(h.query_by_label("H:").is_none(), "Path mode: no H");
}

// ---------------------------------------------------------------------------------------------
// Done when 4: the stroke stays as it is today (not measured until the default is settled)

/// The stroke width field still starts at 0 px and its edit still reaches the tool options; the
/// Stroke style button is still there. The reference's 1 px default is not taken.
#[test]
fn the_stroke_width_and_type_stay_as_they_are_today() {
    assert_eq!(crate::state::ToolOptions::default().stroke_width, 0.0, "today's default");
    let mut h = shape_bar();
    assert_eq!(h.state().ui.tool_options.stroke_width, 0.0, "the bar opens without changing the width");
    assert_eq!(field_text(&h, "Stroke width"), "0", "the width field reads 0 px");
    field(&h, "Stroke width").click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("5".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_options.stroke_width, 5.0, "the width field still edits the tool option");
    assert!(h.query_by_role_and_label(Role::Button, "Stroke style").is_some(), "the stroke type button stays");
}

// ---------------------------------------------------------------------------------------------
// Neighbour cases: the shape tools' bar and the Pen's other modes

/// The Shape tools' bar is unchanged: Rectangle has Fill and Stroke chips and, since main merged
/// #11756, its one Align Edges (not a second one from this PR); no W / H fields (#2496); the
/// Triangle keeps its checkbox.
#[test]
fn the_shape_tools_keep_their_own_bars() {
    let h = bar(Tool::Rectangle);
    assert!(h.query_by_role_and_label(Role::ColorWell, "Fill color").is_some(), "the Rectangle bar keeps its Fill swatch");
    assert!(h.query_by_role_and_label(Role::ColorWell, "Stroke").is_some(), "the Rectangle bar keeps its Stroke swatch");
    assert!(h.query_by_label("W:").is_none(), "the Rectangle bar still has no W field (#2496)");
    assert!(h.query_by_label("H:").is_none(), "the Rectangle bar still has no H field (#2496)");
    let _ = h.get_by_label("Align Edges"); // exactly one, main's #11756 checkbox, not a duplicate of ours
    let h = bar(Tool::Triangle);
    assert!(h.query_by_label("Align Edges").is_some(), "the Triangle keeps its Align Edges (#13370)");
}

/// The Freeform Pen's and the Curvature Pen's Shape-mode branch of the same bar gains the same
/// row (the issue's Where covers all three pens); Path mode keeps its Make: row and gains nothing.
#[test]
fn the_freeform_pen_shape_bar_gains_the_same_row() {
    for tool in [Tool::FreeformPen, Tool::CurvaturePen] {
        let mut h = bar(tool);
        pick_mode(&mut h, "Shape");
        assert!(h.query_by_role_and_label(Role::ColorWell, "Fill:").is_some(), "{tool:?} Shape mode: the Fill swatch");
        assert!(h.query_by_role_and_label(Role::ColorWell, "Stroke:").is_some(), "{tool:?} Shape mode: the Stroke swatch");
        assert!(h.query_by_label("W:").is_some(), "{tool:?} Shape mode: W");
        assert!(h.query_by_label("H:").is_some(), "{tool:?} Shape mode: H");
        assert!(h.query_by_label("Align Edges").is_some(), "{tool:?} Shape mode: Align Edges");
        pick_mode(&mut h, "Path");
        let align = h.query_by_label("Align Edges");
        assert!(align.is_none() || align.unwrap().accesskit_node().is_disabled(), "{tool:?} Path mode: Align Edges greyed or absent");
        assert!(h.query_by_role_and_label(Role::Button, "Shape").is_some(), "{tool:?} Path mode: Make: Shape stays");
    }
}
