//! #13216: a Direct Selection edit (anchor, handle or segment drag, nudge) of a live shape's path
//! asks the live-shape question first — "This operation will turn a live shape into a regular
//! path. Continue?", Yes (default) and No, and the "Don't show again" box shared with the Delete
//! Anchor Point question (#12768) — instead of converting silently. No leaves the document,
//! History and the live shape alone; Yes applies the edit as one History step and the shape is no
//! longer live. Saved paths, the work path and shapes that are not live are edited at once, and
//! the other tools and the path menu keep today's route.

use craft::ui::dialog::{DialogId, StackDialog as _, StandardPrompt};
use egui::Modifiers;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::dialogs::alerts;
use crate::dialogs::live_shape_prompt::LiveShapeAsk;
use crate::state::Tool;

/// The measured document's corner: the live rectangle 550,50–750,200 on 800 x 600, dragged
/// (750,50) by (+20,−20); after Yes the anchor sits at (770,30).
const CORNER: [f64; 2] = [750.0, 50.0];

/// [`CORNER`] after the measured drag.
const MOVED: [f64; 2] = [770.0, 30.0];

/// 800 x 600 with one live rectangle shape layer (550,50–750,200) as the active layer.
fn live(tool: Tool) -> (Box<PhotocraftApp>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    let shape = app.run("shape.create", json!({"kind": "rect", "rect": [550, 50, 200, 150], "fill": "#000000"})).unwrap()["layer"].as_u64().unwrap();
    (app, shape)
}

fn info(app: &mut PhotocraftApp, shape: u64) -> Value {
    app.run("shape.info", json!({"layer": shape})).unwrap()
}

/// The live shape's anchors, document points.
fn anchors(app: &mut PhotocraftApp, shape: u64) -> Vec<[f64; 2]> {
    info(app, shape)["path"]["subpaths"][0]["knots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| {
            let a = k["anchor"].as_array().unwrap();
            [a[0].as_f64().unwrap(), a[1].as_f64().unwrap()]
        })
        .collect()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

/// Whether any anchor of `shape` sits at `at`: a drag's delta carries the view's `f32` mapping of
/// a pointer to document pixels, so compare with the tolerance the other drag tests use.
#[track_caller]
fn has_anchor(app: &mut PhotocraftApp, shape: u64, at: [f64; 2]) -> bool {
    anchors(app, shape).iter().any(|a| (a[0] - at[0]).abs() < 1e-3 && (a[1] - at[1]).abs() < 1e-3)
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    testing::press(app, from, mods);
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    testing::move_to(app, mid, mods);
    testing::move_to(app, to, mods);
    testing::release(app, to, mods);
}

/// The measured drag: the rectangle's top-right corner by (+20, −20).
fn drag_corner(app: &mut PhotocraftApp) {
    drag(app, CORNER, MOVED, Modifiers::NONE);
}

/// The same drag with ⌘ held.
fn drag_corner_cmd(app: &mut PhotocraftApp) {
    drag(app, CORNER, MOVED, Modifiers::COMMAND);
}

/// The open live-shape question.
fn question(app: &PhotocraftApp) -> Option<DialogId> {
    alerts::prompt_tagged::<LiveShapeAsk>(app)
}

/// Answers the question with Yes, ticking "Don't show again" when `skip`.
fn answer_yes(app: &mut PhotocraftApp, id: DialogId, skip: bool) {
    let ctx = app.ctx().clone();
    if skip {
        app.framework.dialogs.set_json_field(&ctx, id, "skip", json!(true)).unwrap();
    }
    crate::dialogs::confirm(app, &ctx, id).unwrap();
}

/// Answers the question with No.
fn answer_no(app: &mut PhotocraftApp, id: DialogId) {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, "choice", json!("cancel")).unwrap();
    crate::dialogs::confirm(app, &ctx, id).unwrap();
}

#[test]
fn a_drag_of_a_live_shapes_anchor_asks_with_yes_no_and_an_unticked_box() {
    let (mut app, shape) = live(Tool::DirectSelection);
    let (before, held, n) = (info(&mut app, shape), anchors(&mut app, shape), steps(&app));
    drag_corner(&mut app);
    assert!(question(&app).is_some(), "the release asks instead of editing");
    assert_eq!(info(&mut app, shape), before, "nothing goes before the answer");
    assert_eq!(anchors(&mut app, shape), held, "the anchor is still at (750,50)");
    assert_eq!(steps(&app), n, "and no History step");
    let fields = app.framework.dialogs.get::<StandardPrompt>().and_then(|prompt| prompt.json_fields()).expect("the question lists its fields");
    assert_eq!(fields["skip"], json!(false), "the Don't show again box starts unticked");
}

#[test]
fn yes_moves_the_anchor_in_one_history_step_and_the_shape_is_no_longer_live() {
    let (mut app, shape) = live(Tool::DirectSelection);
    let n = steps(&app);
    let at = anchors(&mut app, shape).iter().position(|a| (a[0] - CORNER[0]).abs() < 1e-3 && (a[1] - CORNER[1]).abs() < 1e-3).expect("the corner is an anchor");
    drag_corner(&mut app);
    let id = question(&app).expect("the question");
    answer_yes(&mut app, id, false);
    assert!(question(&app).is_none());
    assert!(has_anchor(&mut app, shape, MOVED), "the anchor moved to (770,30): {:?}", anchors(&mut app, shape));
    let info = info(&mut app, shape);
    assert_eq!((&info["live"], &info["kind"]), (&Value::Null, &json!("path")), "the shape became a work path");
    assert_eq!(steps(&app), n + 1, "one History step for the answered drag");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Drag Anchor Point"));
    // Yes leaves the drag's own anchor selection behind, bound to the step it made, as a
    // question-free drag does.
    assert_eq!(
        (app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()),
        (Some(crate::direct_select::PathRef::Layer(shape)), vec![[0, at]]),
        "the dragged anchor stays selected after Yes"
    );
    assert!(app.session.undo(), "Undo takes the answered drag back");
    assert!(!has_anchor(&mut app, shape, MOVED));
}

#[test]
fn no_leaves_the_document_history_and_the_shape_live() {
    let (mut app, shape) = live(Tool::DirectSelection);
    let (before, n) = (info(&mut app, shape), steps(&app));
    drag_corner(&mut app);
    let id = question(&app).expect("the question");
    answer_no(&mut app, id);
    assert!(question(&app).is_none(), "No closes the question");
    assert_eq!(info(&mut app, shape), before, "the document is unchanged");
    assert_ne!(before["live"], Value::Null, "the answer was asked about a live shape");
    assert_eq!(steps(&app), n, "and History gained nothing");
    // Not recorded: the next drag asks again.
    drag_corner(&mut app);
    assert!(question(&app).is_some());
}

#[test]
fn a_segment_drag_of_a_live_shape_asks_first() {
    let (mut app, shape) = live(Tool::DirectSelection);
    let (before, n) = (info(&mut app, shape), steps(&app));
    // The top edge of the rectangle, its midpoint.
    drag(&mut app, [650.0, 50.0], [650.0, 30.0], Modifiers::NONE);
    assert!(question(&app).is_some(), "a segment drag asks too");
    assert_eq!(info(&mut app, shape), before);
    assert_eq!(steps(&app), n);
    let id = question(&app).unwrap();
    answer_yes(&mut app, id, false);
    assert_eq!(&info(&mut app, shape)["live"], &Value::Null, "Yes bends and converts");
    assert_eq!(steps(&app), n + 1);
}

/// A live ellipse's first knot is smooth with handles: dragging one of those asks as well, and
/// the click that only selects (no gesture, no move) never asks.
#[test]
fn a_handle_drag_of_a_live_shape_asks_first_and_a_plain_click_never_asks() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::DirectSelection;
    let shape = app.run("shape.create", json!({"kind": "ellipse", "rect": [550, 50, 200, 150], "fill": "#000000"})).unwrap()["layer"].as_u64().unwrap();
    let top = [650.0, 50.0];
    testing::click(&mut app, top, Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors.len(), 1, "the click selects the anchor");
    assert!(question(&app).is_none(), "selecting asks nothing");
    let shown = info(&mut app, shape);
    let knot = shown["path"]["subpaths"][0]["knots"].as_array().unwrap()[app.ui.direct_selection.anchors[0][1]].clone();
    let out = knot["out"].as_array().unwrap();
    let (hx, hy) = (out[0].as_f64().unwrap(), out[1].as_f64().unwrap());
    assert!((hx - top[0]).abs() + (hy - top[1]).abs() > 1.0, "the smooth anchor has a handle to drag");
    let (before, n) = (info(&mut app, shape), steps(&app));
    drag(&mut app, [hx, hy], [hx + 20.0, hy + 20.0], Modifiers::NONE);
    assert!(question(&app).is_some(), "a handle drag asks");
    assert_eq!(info(&mut app, shape), before);
    assert_eq!(steps(&app), n);
}

/// Direct Selection's ⌥-drag on a segment copies the subpath (#13211) — still an edit of the live
/// shape, so it asks.
#[test]
fn an_alt_segment_drag_of_a_live_shape_asks_and_yes_copies_in_one_step() {
    let (mut app, shape) = live(Tool::DirectSelection);
    let n = steps(&app);
    drag(&mut app, [650.0, 50.0], [650.0, 90.0], Modifiers::ALT);
    let id = question(&app).expect("the ⌥-drag asks");
    assert_eq!(steps(&app), n, "nothing ran while the question was open");
    answer_yes(&mut app, id, false);
    assert_eq!(&info(&mut app, shape)["live"], &Value::Null);
    assert_eq!(steps(&app), n + 1, "one History step for the answered copy");
}

/// The Insert key edits the path through `edits::insert`, not through `change_path` (the issue's
/// "Where"). The Done-when list does not name it, so it keeps today's route: the anchor is added
/// and the shape converts at once, without a question (guard test: it passes before and after
/// this change). Asked on #13216 alongside the ⌘/Ctrl question.
#[test]
fn the_insert_key_still_adds_an_anchor_to_a_live_shape_without_asking() {
    let (mut app, shape) = live(Tool::DirectSelection);
    testing::click(&mut app, CORNER, Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors.len(), 1, "an anchor is focused");
    let n = steps(&app);
    testing::key(&mut app, egui::Key::Insert, Modifiers::NONE);
    assert!(question(&app).is_none(), "the Insert key asks nothing, as today");
    assert_eq!(&info(&mut app, shape)["live"], &Value::Null, "and converts at once, as today");
    assert_eq!(anchors(&mut app, shape).len(), 5, "the anchor was added");
    assert_eq!(steps(&app), n + 1);
}

#[test]
fn an_arrow_key_nudge_of_a_live_shapes_selected_anchor_asks_first() {
    let (mut app, shape) = live(Tool::DirectSelection);
    testing::click(&mut app, CORNER, Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors.len(), 1, "the corner is selected");
    let (before, n) = (info(&mut app, shape), steps(&app));
    testing::key(&mut app, egui::Key::ArrowRight, Modifiers::NONE);
    assert!(question(&app).is_some(), "the nudge asks");
    assert_eq!(info(&mut app, shape), before);
    assert_eq!(steps(&app), n);
    let id = question(&app).unwrap();
    answer_yes(&mut app, id, false);
    // A nudge is one screen pixel: the corner went right, its row unchanged (the zoom scales it).
    assert!(
        anchors(&mut app, shape).iter().any(|a| a[0] > CORNER[0] + 0.5 && a[0] < CORNER[0] + 2.5 && (a[1] - CORNER[1]).abs() < 1e-3),
        "Yes nudged the corner right: {:?}",
        anchors(&mut app, shape)
    );
    assert_eq!(&info(&mut app, shape)["live"], &Value::Null);
    assert_eq!(steps(&app), n + 1);
}

#[test]
fn a_ticked_dont_show_again_skips_the_later_questions_too() {
    let (mut app, shape) = live(Tool::DirectSelection);
    drag_corner(&mut app);
    let id = question(&app).expect("the question");
    answer_yes(&mut app, id, true);
    assert_eq!(&info(&mut app, shape)["live"], &Value::Null, "Yes with the box applied the drag");
    // Another live shape: the answer was recorded, so the edit goes at once.
    let other = app.run("shape.create", json!({"kind": "rect", "rect": [100, 400, 100, 100], "fill": "#000000"})).unwrap()["layer"].as_u64().unwrap();
    let n = steps(&app);
    drag(&mut app, [200.0, 500.0], [220.0, 520.0], Modifiers::NONE);
    assert!(question(&app).is_none(), "the question does not come back");
    assert_eq!(&info(&mut app, other)["live"], &Value::Null, "the second shape was converted at once");
    assert_eq!(steps(&app), n + 1);
}

/// The skip is the Delete Anchor Point question's shared preference (#12768): remembered by that
/// tool, it also lets the Direct Selection drag through without a question.
#[test]
fn the_delete_anchor_tools_remembered_answer_also_lets_a_live_shape_drag_through() {
    let (mut app, shape) = live(Tool::DirectSelection);
    alerts::remember_skip(&mut app, crate::dialogs::live_shape_prompt::PREF);
    let n = steps(&app);
    drag_corner(&mut app);
    assert!(question(&app).is_none());
    assert!(has_anchor(&mut app, shape, MOVED));
    assert_eq!(steps(&app), n + 1);
}

#[test]
fn a_shape_that_is_not_live_a_saved_path_and_the_work_path_are_edited_at_once() {
    // A shape layer whose path is no longer live: its anchors move at once.
    let (mut app, shape) = live(Tool::DirectSelection);
    let path = info(&mut app, shape)["path"].clone();
    app.run("shape.edit", json!({"layer": shape, "path": path})).unwrap();
    assert_eq!(&info(&mut app, shape)["live"], &Value::Null);
    drag_corner(&mut app);
    assert!(question(&app).is_none(), "a converted shape layer asks nothing");
    assert!(has_anchor(&mut app, shape, MOVED));
    // A saved path selected in the Paths panel, and the work path.
    let square = json!({"subpaths": [{"closed": true, "knots": [[100, 100], [300, 100], [300, 300], [100, 300]]}]});
    app.run("path.set", json!({"name": "Path 1", "path": square})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    drag(&mut app, [300.0, 300.0], [320.0, 320.0], Modifiers::NONE);
    assert!(question(&app).is_none(), "a saved path asks nothing");
    app.run("path.set", json!({"name": "work", "path": square})).unwrap();
    app.ui.selected_path = Some("work".into());
    drag(&mut app, [300.0, 300.0], [340.0, 340.0], Modifiers::NONE);
    assert!(question(&app).is_none(), "the work path asks nothing");
    let end = app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].knots[2].anchor;
    assert!((end.x - 340.0).abs() < 1e-3 && (end.y - 340.0).abs() < 1e-3, "the work path moved: {end:?}");
}

/// Not measured for the other tools, so pinned at today's behaviour: a Path Selection press with ⌘
/// over a live shape gets no live-shape question — the ask is Direct Selection's tool alone (guard
/// test: it passes before and after this change; what the ⌘-gesture itself does today is another
/// ticket's subject and stays asserted nowhere here).
#[test]
fn path_selection_with_command_never_gets_the_live_shape_question() {
    let (mut app, _shape) = live(Tool::PathSelection);
    drag_corner_cmd(&mut app);
    assert!(question(&app).is_none(), "no question for this tool");
}

/// Not measured for other tools, so pinned at today's behaviour: the Convert Point tool still
/// converts a live shape with its click, without a question.
#[test]
fn the_convert_point_tool_still_converts_a_live_shape_without_asking() {
    let (mut app, shape) = live(Tool::ConvertPoint);
    let n = steps(&app);
    testing::click(&mut app, CORNER, Modifiers::NONE);
    assert!(question(&app).is_none());
    assert_eq!(&info(&mut app, shape)["live"], &Value::Null, "the click converted, as today");
    assert_eq!(steps(&app), n + 1);
}

/// Same for the canvas path menu the anchor tools open: its Delete Anchor Point entry runs the
/// command at once, question-free, as today.
#[test]
fn the_path_menu_still_deletes_a_live_shapes_anchor_without_asking() {
    use egui_kittest::kittest::Queryable;
    let (app, shape) = live(Tool::DeleteAnchorPoint);
    let mut h = crate::canvas::canvas_tool_menu::tests::harness(app);
    let p = testing::to_screen(h.state(), CORNER);
    crate::canvas::canvas_tool_menu::tests::right_click(&mut h, p, Modifiers::NONE);
    let before = steps(h.state());
    h.get_by_label("Delete Anchor Point").click();
    h.run_steps(2);
    assert!(question(h.state()).is_none(), "the menu asked nothing");
    let info = h.state_mut().run("shape.info", json!({"layer": shape})).unwrap();
    assert_eq!(info["path"]["subpaths"][0]["knots"].as_array().unwrap().len(), 3, "the anchor is gone");
    assert_eq!(&info["live"], &Value::Null, "the menu converted it, as today");
    assert_eq!(steps(h.state()), before + 1);
}

/// The answer belongs to the path as it was asked (the modal does not stop the control channel):
/// a path that changed meanwhile gets no edit, as the Delete Anchor Point question refuses.
#[test]
fn yes_after_the_path_changed_edits_nothing() {
    let (mut app, shape) = live(Tool::DirectSelection);
    drag_corner(&mut app);
    let id = question(&app).expect("the question");
    app.run("path.addAnchor", json!({"name": "layer", "layer": shape, "subpath": 0, "knot": 0, "t": 0.5})).unwrap();
    let (before, n) = (anchors(&mut app, shape), steps(&app));
    answer_yes(&mut app, id, false);
    assert_eq!(anchors(&mut app, shape), before, "the answered drag moved nothing");
    assert!(!before.contains(&MOVED));
    assert_eq!(steps(&app), n, "and made no step");
}
