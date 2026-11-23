//! #13472: the Custom Shape tool shows the just-placed shape's bounding box with its eight handles, as the
//! Rectangle tool does (#11107): the press lands on a handle (the tool draws no new shape), a drag scales the
//! shape with one History step named "Transform Path" and no Free Transform session opens, and the box follows
//! the shape the tool just placed. Pen and Make Shape path layers keep today's no-box look with this tool
//! (not measured), and the box over the other tools' selections is the shape controls' own tests' concern.

use craft::draw::ui::VectorMode;
use egui::Modifiers;
use photocraft_doc::LayerContent;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

const NONE: Modifiers = Modifiers::NONE;

/// The box every test places the preset in: a square, so a corner handle (which scales by the drag's
/// projection on the box diagonal) follows the drag exactly.
const SQUARE: [f64; 4] = [400.0, 300.0, 600.0, 500.0];

/// A 1000 × 700 document, the Custom Shape tool active with the built-in Star picked.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 1000, "height": 700, "background": "white"})).unwrap();
    app.ui.presets_ui.custom_shape = "Star".into();
    app.ui.tool = Tool::CustomShape;
    app.run("view.fitOnScreen", json!({})).unwrap();
    testing::frame(&mut app);
    app
}

/// The Custom Shape tool's own drag from `[x0, y0]` to `[x1, y1]`: it places the preset as a path shape layer
/// and makes it the active layer. The id of that layer comes back.
fn place(app: &mut PhotocraftApp, [x0, y0, x1, y1]: [f64; 4]) -> u64 {
    let before = layer_count(app);
    testing::press(app, [x0, y0], NONE);
    testing::move_to(app, [(x0 + x1) / 2.0, (y0 + y1) / 2.0], NONE);
    testing::release(app, [x1, y1], NONE);
    let id = app.session.active().unwrap().active_layer.expect("the placed shape is the active layer").0;
    assert_eq!(layer_count(app), before + 1, "the drag placed one shape layer");
    let st = app.session.active().unwrap();
    assert!(matches!(st.doc.layer(photocraft_doc::LayerId(id)).unwrap().content, LayerContent::Shape(_)), "a shape layer");
    id
}

fn layer_count(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

fn bounds(app: &PhotocraftApp, id: u64) -> (f64, f64, f64, f64) {
    let st = app.session.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(photocraft_doc::LayerId(id)).unwrap().content else { panic!("a shape layer") };
    sh.path.control_bounds().unwrap()
}

fn near(got: (f64, f64, f64, f64), want: [f64; 4]) {
    let got = [got.0, got.1, got.2, got.3];
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1.0), "{got:?} vs {want:?}");
}

fn past(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn last_step(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.entries().last().cloned()
}

/// A press at `from`, two moves and a release at `to`, as a mouse does it.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    testing::press(app, from, NONE);
    testing::move_to(app, [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0], NONE);
    testing::move_to(app, to, NONE);
    testing::release(app, to, NONE);
}

#[test]
fn the_placed_shape_shows_a_box_whose_corner_handle_scales_it_in_one_step() {
    let mut app = app();
    let id = place(&mut app, SQUARE);
    let (x0, y0, x1, y1) = bounds(&app, id);
    let steps = past(&app);
    let layers = layer_count(&app);
    // The bottom-right handle, pressed 3 px outside the box as in the box controls' tests, dragged 30 px in.
    // Before the fix the press falls through to the tool: a second Star is placed, not a scale.
    drag(&mut app, [x1 + 3.0, y1 + 3.0], [x1 - 30.0, y1 - 30.0]);
    assert_eq!(past(&app), steps + 1, "one History step");
    assert_eq!(last_step(&app).as_deref(), Some("Transform Path"), "the handle scaled the shape");
    assert_eq!(layer_count(&app), layers, "the press took the handle, not the canvas: no new placement");
    assert!(app.ui.transform.is_none(), "no Free Transform session opens");
    near(bounds(&app, id), [x0, y0, x1 - 30.0, y1 - 30.0]);
}

/// The eight handles, clockwise from the top left: each pressed 3 px outside the box along its outside
/// normal and dragged 10 px in, scaling with one "Transform Path" step and undoing back to the placed box.
const HANDLE_PRESS: [[f64; 2]; 8] = [[-3.0, -3.0], [0.0, -3.0], [3.0, -3.0], [3.0, 0.0], [3.0, 3.0], [0.0, 3.0], [-3.0, 3.0], [-3.0, 0.0]];
const HANDLE_DRAG: [[f64; 2]; 8] = [[10.0, 10.0], [0.0, 10.0], [-10.0, 10.0], [-10.0, 0.0], [-10.0, -10.0], [0.0, -10.0], [10.0, -10.0], [10.0, 0.0]];

#[test]
fn the_box_has_eight_handles_and_every_one_scales_the_shape() {
    let mut app = app();
    let id = place(&mut app, SQUARE);
    let layers = layer_count(&app);
    for n in 0..8 {
        // The handle that sits on the placed box: corners on both edges, sides at their middle.
        let (x0, y0, x1, y1) = bounds(&app, id);
        let (bx, by) = ([x0, (x0 + x1) / 2.0, x1, x1, x1, (x0 + x1) / 2.0, x0, x0][n], [y0, y0, y0, (y0 + y1) / 2.0, y1, y1, y1, (y0 + y1) / 2.0][n]);
        let steps = past(&app);
        drag(&mut app, [bx + HANDLE_PRESS[n][0], by + HANDLE_PRESS[n][1]], [bx + HANDLE_DRAG[n][0], by + HANDLE_DRAG[n][1]]);
        assert_eq!(past(&app), steps + 1, "handle {n}: one History step");
        assert_eq!(last_step(&app).as_deref(), Some("Transform Path"), "handle {n}: the handle scaled");
        assert_eq!(layer_count(&app), layers, "handle {n}: no new placement, the press took the handle");
        near(bounds(&app, id), expected_box([x0, y0, x1, y1], n));
        // Back to the placed box for the next handle.
        app.run("edit.undo", json!({})).unwrap();
        near(bounds(&app, id), [x0, y0, x1, y1]);
    }
}

/// The box after handle `n` (clockwise from the top left) moved 10 px in, on a square box where the
/// corner's projection rule leaves the corner exactly at the drag's end.
fn expected_box(mut r: [f64; 4], n: usize) -> [f64; 4] {
    const IN: f64 = 10.0;
    if matches!(n, 0 | 6..=7) {
        r[0] += IN;
    }
    if matches!(n, 2..=4) {
        r[2] -= IN;
    }
    if matches!(n, 0..=2) {
        r[1] += IN;
    }
    if matches!(n, 4..=6) {
        r[3] -= IN;
    }
    r
}

/// The whole app in a kittest window on its own context, as the guide-cursor tests drive one: the input
/// above ran on that same context, and the pointer now reaches the canvas controls through window events.
fn harness(app: Box<PhotocraftApp>) -> egui_kittest::Harness<'static, Box<PhotocraftApp>> {
    let ctx = app.ctx().clone();
    let mut h = egui_kittest::Harness::builder().with_context(ctx).with_size(egui::vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |_cc| app);
    h.run_steps(4);
    h
}

/// The window's cursor icon after resting the pointer at document point `[x, y]`.
fn hover_cursor(h: &mut egui_kittest::Harness<'static, Box<PhotocraftApp>>, x: f64, y: f64) -> egui::CursorIcon {
    let p = testing::to_screen(h.state(), [x, y]);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(2);
    h.output().platform_output.cursor_icon
}

const HANDLE_ARROW: [egui::CursorIcon; 2] = [egui::CursorIcon::ResizeNwSe, egui::CursorIcon::ResizeNeSw];

/// The Background layer: `layer.select` never leaves the selection empty, so a test leaves the placed shape
/// by selecting this one.
fn background(app: &PhotocraftApp) -> u64 {
    app.session.active().unwrap().doc.layers.iter().find(|l| !matches!(l.content, LayerContent::Shape(_))).expect("the Background layer").id.0
}

#[test]
fn the_handle_cursor_shows_on_the_placed_shape_and_clears_on_deselect() {
    let mut app = app();
    let id = place(&mut app, SQUARE);
    let bg = background(&app);
    let (_x0, _y0, x1, y1) = bounds(&app, id);
    let mut h = harness(app);
    // The bottom-right handle, 3 px outside the box: the box's diagonal resize arrow. Before the fix
    // there is no box: the tool's own cursor shows.
    assert!(HANDLE_ARROW.contains(&hover_cursor(&mut h, x1 + 3.0, y1 + 3.0)), "the box's handle arrow on the handle");
    // The placed layer leaves the selection: the box is gone and the spot shows the tool's cursor again.
    h.state_mut().run("layer.select", json!({"layer": bg})).unwrap();
    h.run_steps(2);
    let icon = hover_cursor(&mut h, x1 + 3.0, y1 + 3.0);
    assert!(!HANDLE_ARROW.contains(&icon), "left the selection: no handle cursor ({icon:?})");
    // Selected again: the handle arrow is back.
    h.state_mut().run("layer.select", json!({"layer": id})).unwrap();
    h.run_steps(2);
    assert!(HANDLE_ARROW.contains(&hover_cursor(&mut h, x1 + 3.0, y1 + 3.0)), "selected again: the handle arrow");
}

#[test]
fn the_box_returns_when_the_just_placed_layer_is_selected_again() {
    let mut app = app();
    let id = place(&mut app, SQUARE);
    let bg = background(&app);
    let (x0, y0, x1, y1) = bounds(&app, id);
    // Select away from the shape the tool just placed, and back, with no other placement between: its box
    // shows, and its handle scales it.
    app.run("layer.select", json!({"layer": bg})).unwrap();
    testing::frame(&mut app);
    app.run("layer.select", json!({"layer": id})).unwrap();
    testing::frame(&mut app);
    let steps = past(&app);
    drag(&mut app, [x1 + 3.0, y1 + 3.0], [x1 - 30.0, y1 - 30.0]);
    assert_eq!(last_step(&app).as_deref(), Some("Transform Path"), "selected again: the handle scaled");
    assert_eq!(past(&app), steps + 1, "one History step");
    near(bounds(&app, id), [x0, y0, x1 - 30.0, y1 - 30.0]);
}

#[test]
fn a_new_shape_moves_the_box_and_the_older_shape_keeps_todays_look() {
    let mut app = app();
    let _first = place(&mut app, [100.0, 100.0, 300.0, 300.0]);
    let second = place(&mut app, [500.0, 300.0, 700.0, 500.0]);
    // The just-placed shape shows its box: its handle scales it. Before the fix the press places instead.
    let sb = bounds(&app, second);
    let steps = past(&app);
    let layers = layer_count(&app);
    drag(&mut app, [sb.2 + 3.0, sb.3 + 3.0], [sb.2 - 30.0, sb.3 - 30.0]);
    assert_eq!(last_step(&app).as_deref(), Some("Transform Path"), "the new shape's handle scaled");
    assert_eq!(past(&app), steps + 1, "one History step");
    assert_eq!(layer_count(&app), layers, "no new placement");
    near(bounds(&app, second), [sb.0, sb.1, sb.2 - 30.0, sb.3 - 30.0]);
    // A third placement takes the box: its handle scales.
    let third = place(&mut app, [100.0, 400.0, 300.0, 600.0]);
    let tb = bounds(&app, third);
    let steps = past(&app);
    drag(&mut app, [tb.2 + 3.0, tb.3 + 3.0], [tb.2 - 30.0, tb.3 - 30.0]);
    assert_eq!(last_step(&app).as_deref(), Some("Transform Path"), "the newest shape's handle scaled");
    assert_eq!(past(&app), steps + 1, "one History step");
    // The scaled second shape is an older custom shape layer now: with this tool it keeps today's look —
    // its corner is the tool's again, and its bounds stay.
    let sb = bounds(&app, second);
    let layers = layer_count(&app);
    drag(&mut app, [sb.2 + 3.0, sb.3 + 3.0], [sb.2 - 20.0, sb.3 - 20.0]);
    assert_eq!(layer_count(&app), layers + 1, "the older shape shows no box: its corner places a new shape");
    near(bounds(&app, second), [sb.0, sb.1, sb.2, sb.3]);
}

#[test]
fn undoing_the_placement_ends_the_box() {
    let mut app = app();
    let id = place(&mut app, SQUARE);
    app.run("edit.undo", json!({})).unwrap();
    testing::frame(&mut app);
    assert!(app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).is_none(), "the layer is gone");
    // Redo brings the layer and its record back: the box is active again and the handle scales it.
    app.run("edit.redo", json!({})).unwrap();
    testing::frame(&mut app);
    let (x0, y0, x1, y1) = bounds(&app, id);
    let steps = past(&app);
    let layers = layer_count(&app);
    drag(&mut app, [x1 + 3.0, y1 + 3.0], [x1 - 30.0, y1 - 30.0]);
    assert_eq!(last_step(&app).as_deref(), Some("Transform Path"), "redone: the handle scaled");
    assert_eq!(past(&app), steps + 1, "one History step");
    assert_eq!(layer_count(&app), layers, "no new placement");
    near(bounds(&app, id), [x0, y0, x1 - 30.0, y1 - 30.0]);
    // Undo the scale and the placement: where the handle was the tool draws again, and no "Transform Path".
    app.run("edit.undo", json!({})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    testing::frame(&mut app);
    let steps = past(&app);
    let layers = layer_count(&app);
    drag(&mut app, [SQUARE[2] + 3.0, SQUARE[3] + 3.0], [SQUARE[2] - 30.0, SQUARE[3] - 30.0]);
    assert_eq!(layer_count(&app), layers + 1, "the press placed a new shape");
    assert_ne!(last_step(&app).as_deref(), Some("Transform Path"), "nothing scaled");
    assert_eq!(past(&app), steps + 1, "one step: the placement");
}

/// The record names one layer: with the placed shape selected AMONG several layers the box is inactive, and
/// the press at its box's bottom-right corner reaches the tool. Guard test, passes before and after (with
/// this tool no selection showed a box before the fix).
#[test]
fn selecting_the_shape_among_several_keeps_the_box_inactive() {
    let mut app = app();
    let id = place(&mut app, SQUARE);
    let bg = background(&app);
    app.run("layer.select", json!({"layer": bg, "mode": "add"})).unwrap();
    testing::frame(&mut app);
    assert_eq!(app.session.active().unwrap().selected_layers().len(), 2, "both layers are selected");
    let (x0, y0, x1, y1) = bounds(&app, id);
    let steps = past(&app);
    let layers = layer_count(&app);
    drag(&mut app, [x1 + 3.0, y1 + 3.0], [x1 - 30.0, y1 - 30.0]);
    assert_eq!(layer_count(&app), layers + 1, "the press placed a new shape: the multi-layer selection shows no box");
    assert_ne!(last_step(&app).as_deref(), Some("Transform Path"), "nothing scaled");
    assert_eq!(past(&app), steps + 1, "one step: the placement");
    near(bounds(&app, id), [x0, y0, x1, y1]);
}

/// The placement record is UI state only: it is not saved with the panels, and a restored state carries no
/// record (the rule of `clone_cursor`, same shape of assertion). Does not compile without the change.
#[test]
fn the_placement_record_is_not_saved() {
    let mut app = app();
    place(&mut app, SQUARE);
    assert!(app.ui.presets_ui.placed_custom_shape.is_some(), "the placement left a record");
    let saved = serde_json::to_value(&app.ui.presets_ui).unwrap();
    assert!(saved.get("placed_custom_shape").is_none(), "the key is not in the saved JSON");
    let restored: crate::preset_panels::PresetUi = serde_json::from_value(saved).unwrap();
    assert!(restored.placed_custom_shape.is_none(), "the restored state carries no record");
}

/// A Pen path committed to a shape layer (the Pen tool in Shape mode): with the Custom Shape tool it keeps
/// today's look (not measured) — a press at its box's corner draws a new custom shape rather than scaling it.
#[test]
fn a_pen_shape_layer_keeps_todays_look_with_the_custom_shape_tool() {
    let mut app = app();
    app.ui.tool_options.vector_mode = VectorMode::Shape;
    app.ui.tool = Tool::Pen;
    for at in [[200.0, 200.0], [300.0, 200.0], [300.0, 300.0]] {
        testing::click(&mut app, at, NONE);
    }
    assert!(crate::vector_ui::pen_commit(&mut app, true), "the Pen path committed");
    let id = app.session.active().unwrap().active_layer.expect("the committed layer is active").0;
    let LayerContent::Shape(sh) = &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content else { panic!("a shape layer") };
    assert!(sh.live.is_none(), "the Pen layer is a plain path shape");
    app.ui.tool = Tool::CustomShape;
    testing::frame(&mut app);
    assert_no_box_with_custom_shape_tool(&mut app, id);
}

/// Make Shape from the work path: the same pin for the Pen options bar / paths panel route.
#[test]
fn a_make_shape_layer_keeps_todays_look_with_the_custom_shape_tool() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    for at in [[200.0, 200.0], [300.0, 200.0], [300.0, 300.0]] {
        testing::click(&mut app, at, NONE);
    }
    assert!(crate::vector_ui::make_shape(&mut app).is_ok(), "Make Shape made the layer");
    let id = app.session.active().unwrap().active_layer.expect("the new layer is active").0;
    app.ui.tool = Tool::CustomShape;
    testing::frame(&mut app);
    assert_no_box_with_custom_shape_tool(&mut app, id);
}

fn assert_no_box_with_custom_shape_tool(app: &mut PhotocraftApp, id: u64) {
    let (x0, y0, x1, y1) = bounds(app, id);
    let steps = past(app);
    let layers = layer_count(app);
    drag(app, [x1 + 3.0, y1 + 3.0], [x1 - 20.0, y1 - 20.0]);
    near(bounds(app, id), [x0, y0, x1, y1]);
    assert_eq!(past(app), steps + 1, "one step: the placement, not a scale");
    assert_eq!(layer_count(app), layers + 1, "the press placed a new shape: the path layer shows no box");
    assert_ne!(last_step(app).as_deref(), Some("Transform Path"), "nothing scaled");
}

/// A preset placed without the tool's drag (the Shapes panel's click and drop run `shape.presets.place`
/// themselves): with the Custom Shape tool it keeps today's look — the box follows the tool's own placement,
/// which is what the issue measured.
#[test]
fn a_shape_placed_by_the_panel_keeps_todays_look() {
    let mut app = app();
    app.run("shape.presets.place", json!({"preset": "Star", "rect": [400.0, 300.0, 200.0, 200.0], "keepAspect": false})).unwrap();
    let id = app.session.active().unwrap().active_layer.expect("the placed layer is active").0;
    assert!(matches!(app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content, LayerContent::Shape(_)), "a shape layer");
    testing::frame(&mut app);
    assert_no_box_with_custom_shape_tool(&mut app, id);
}

/// Path Selection and the Rectangle tool take the box over a custom shape layer as they do today: the fix
/// only adds the Custom Shape tool itself.
#[test]
fn the_other_shape_tools_box_the_custom_shape_layer_as_ever() {
    for tool in [Tool::PathSelection, Tool::Rectangle] {
        let mut app = app();
        let id = place(&mut app, SQUARE);
        let (x0, y0, x1, y1) = bounds(&app, id);
        app.ui.tool = tool;
        testing::frame(&mut app);
        let steps = past(&app);
        let layers = layer_count(&app);
        drag(&mut app, [x1 + 3.0, y1 + 3.0], [x1 - 30.0, y1 - 30.0]);
        assert_eq!(last_step(&app).as_deref(), Some("Transform Path"), "{tool:?}: the handle scaled");
        assert_eq!(past(&app), steps + 1, "{tool:?}: one History step");
        assert_eq!(layer_count(&app), layers, "{tool:?}: no new layer");
        near(bounds(&app, id), [x0, y0, x1 - 30.0, y1 - 30.0]);
    }
}
