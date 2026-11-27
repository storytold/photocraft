//! #12890: every Pen click in Path mode is its own History step and the Work Path exists from the first
//! click. The names and their counts are the issue's measured rows: Path mode, the Pen tool, a document with no
//! path selected (its 800 × 600 RGB 8-bit document, scaled to these 200 × 200 anchors). Cases the issue did not
//! measure keep today's behaviour and are pinned here as today's behaviour.

use super::*;
use crate::canvas::testing;
use craft::draw::ui::VectorMode;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT, Queryable};

/// A 200 by 200 document with the Pen in Path mode: the issue's measured configuration.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Pen;
    app.ui.tool_options.vector_mode = VectorMode::Path;
    app
}

fn click(app: &mut PhotocraftApp, at: [f64; 2]) {
    testing::click(app, at, Modifiers::NONE);
}

/// A press at `from`, a drag to `to` and the release there, with `mods` held throughout.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    testing::press(app, from, mods);
    testing::move_to(app, to, mods);
    testing::release(app, to, mods);
}

fn key(app: &mut PhotocraftApp, key: Key) {
    testing::key(app, key, Modifiers::NONE);
}

/// Edit › Undo and Redo as the menu, the shortcut and the control channel run them.
fn menu(app: &mut PhotocraftApp, id: &str) {
    let ctx = app.ctx().clone();
    crate::menus::invoke(app, &ctx, id, json!({})).unwrap();
}

/// The History step names, oldest first.
fn steps(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries()
}

/// The step names added since the `before` of [`steps`].
fn added(app: &PhotocraftApp, before: usize) -> Vec<String> {
    steps(app).split_off(before)
}

fn work(app: &PhotocraftApp) -> Option<Path> {
    app.session.active().unwrap().doc.work_path.clone()
}

/// Each subpath of `path`: whether it is closed and its anchor points.
fn subpaths(path: &Path) -> Vec<(bool, Vec<[f64; 2]>)> {
    path.subpaths.iter().map(|s| (s.closed, s.knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect())).collect()
}

fn draft_len(app: &PhotocraftApp) -> Option<usize> {
    app.ui.pen.as_ref().map(|p| p.draft.knots.len())
}

/// Three anchors, as the issue's Path-mode gesture: two corners and a third anchor.
const TRIANGLE: [[f64; 2]; 3] = [[20.0, 20.0], [120.0, 20.0], [120.0, 120.0]];

fn triangle(app: &mut PhotocraftApp) {
    for at in TRIANGLE {
        click(app, at);
    }
}

// ---------------------------------------------------------------------------------------------
// The measured rows

#[test]
fn the_first_click_makes_the_work_path_as_one_step_new_work_path() {
    let mut app = app();
    let before = steps(&app).len();
    click(&mut app, TRIANGLE[0]);
    assert_eq!(added(&app, before), ["New Work Path"], "the first click is its own step");
    assert_eq!(work(&app).map(|p| subpaths(&p)), Some(vec![(false, vec![[20.0, 20.0]])]), "the Work Path exists at once");
    assert_eq!(draft_len(&app), Some(1), "and the Pen is still drawing");
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

#[test]
fn every_further_click_is_its_own_new_anchor_point_step() {
    let mut app = app();
    let before = steps(&app).len();
    triangle(&mut app);
    assert_eq!(added(&app, before), ["New Work Path", "New Anchor Point", "New Anchor Point"], "one step per click");
    assert_eq!(subpaths(work(&app).as_ref().unwrap()), [(false, vec![[20.0, 20.0], [120.0, 20.0], [120.0, 120.0]])]);
    assert_eq!(draft_len(&app), Some(3), "the draft and the Work Path stay the same path");
}

#[test]
fn a_click_drag_pulls_its_handles_in_one_new_anchor_point_step() {
    let mut app = app();
    let before = steps(&app).len();
    click(&mut app, TRIANGLE[0]);
    drag(&mut app, TRIANGLE[1], [150.0, 60.0], Modifiers::NONE);
    assert_eq!(added(&app, before), ["New Work Path", "New Anchor Point"], "the drag's samples add no steps of their own");
    let k = work(&app).unwrap().subpaths[0].knots[1];
    assert!(k.smooth && k.out_ctrl != photocraft_geom::Point::new(120.0, 20.0), "its handles are pulled: {k:?}");
}

#[test]
fn one_undo_takes_back_exactly_one_click() {
    let mut app = app();
    let before = steps(&app).len();
    triangle(&mut app);
    menu(&mut app, "edit.undo");
    assert_eq!(steps(&app).len(), before + 2, "one step back");
    assert_eq!(subpaths(work(&app).as_ref().unwrap()), [(false, vec![[20.0, 20.0], [120.0, 20.0]])], "one anchor less");
    assert_eq!(draft_len(&app), Some(2), "the Pen carries on from the anchors that are left");
    click(&mut app, [60.0, 140.0]);
    assert_eq!(added(&app, before + 2), ["New Anchor Point"], "and drawing goes on in the same path");
    assert_eq!(subpaths(work(&app).as_ref().unwrap()), [(false, vec![[20.0, 20.0], [120.0, 20.0], [60.0, 140.0]])]);
}

#[test]
fn clicking_the_first_anchor_closes_the_path_as_one_step_close_path() {
    let mut app = app();
    let before = steps(&app).len();
    triangle(&mut app);
    click(&mut app, TRIANGLE[0]);
    assert_eq!(added(&app, before), ["New Work Path", "New Anchor Point", "New Anchor Point", "Close Path"]);
    assert_eq!(subpaths(work(&app).as_ref().unwrap()), [(true, vec![[20.0, 20.0], [120.0, 20.0], [120.0, 120.0]])]);
    assert!(app.ui.pen.is_none(), "the closing click ends the draft");
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

#[test]
fn the_next_click_after_a_close_is_one_step_new_path_component() {
    let mut app = app();
    let before = steps(&app).len();
    triangle(&mut app);
    click(&mut app, TRIANGLE[0]);
    click(&mut app, [60.0, 140.0]);
    assert_eq!(added(&app, before), ["New Work Path", "New Anchor Point", "New Anchor Point", "Close Path", "New Path Component"]);
    assert_eq!(
        subpaths(work(&app).as_ref().unwrap()),
        [(true, vec![[20.0, 20.0], [120.0, 20.0], [120.0, 120.0]]), (false, vec![[60.0, 140.0]])],
        "a second component in the same path"
    );
}

#[test]
fn a_click_on_an_open_endpoint_is_one_step_pick_up_path() {
    let mut app = app();
    let before = steps(&app).len();
    click(&mut app, TRIANGLE[0]);
    click(&mut app, TRIANGLE[1]);
    key(&mut app, Key::Escape);
    let picked_up = work(&app);
    click(&mut app, TRIANGLE[1]);
    assert_eq!(added(&app, before), ["New Work Path", "New Anchor Point", "Pick-up Path"], "the endpoint click is its own step, and Escape added none");
    assert!(app.ui.pen.as_ref().is_some_and(|p| p.resumes.is_some()), "the subpath is the draft");
    assert_eq!(work(&app), picked_up, "the pick-up changes nothing in the path");
    click(&mut app, [180.0, 120.0]);
    assert_eq!(added(&app, before + 3), ["New Anchor Point"], "and it carries on drawing there");
    assert_eq!(subpaths(work(&app).as_ref().unwrap()), [(false, vec![[20.0, 20.0], [120.0, 20.0], [180.0, 120.0]])], "in place, not a second component");
}

#[test]
fn alt_click_on_the_last_smooth_anchor_is_one_step_pick_up_path() {
    let mut app = app();
    let before = steps(&app).len();
    click(&mut app, TRIANGLE[0]);
    drag(&mut app, TRIANGLE[1], [150.0, 60.0], Modifiers::NONE);
    let pulled = work(&app).unwrap().subpaths[0].knots[1];
    assert!(pulled.smooth && pulled.out_ctrl != pulled.anchor, "a pulled, smooth anchor: {pulled:?}");
    testing::click(&mut app, TRIANGLE[1], Modifiers::ALT);
    assert_eq!(added(&app, before), ["New Work Path", "New Anchor Point", "Pick-up Path"]);
    let k = work(&app).unwrap().subpaths[0].knots[1];
    assert!(!k.smooth && k.out_ctrl == k.anchor, "its outgoing handle is gone: {k:?}");
}

#[test]
fn a_ctrl_drag_of_an_anchor_is_one_step_drag_anchor_point() {
    let mut app = app();
    let before = steps(&app).len();
    triangle(&mut app);
    drag(&mut app, TRIANGLE[1], [120.0, 60.0], Modifiers::COMMAND);
    assert_eq!(added(&app, before), ["New Work Path", "New Anchor Point", "New Anchor Point", "Drag Anchor Point"]);
    assert_eq!(subpaths(work(&app).as_ref().unwrap()), [(false, vec![[20.0, 20.0], [120.0, 60.0], [120.0, 120.0]])]);
    assert!(app.ui.pen.is_some(), "the path is still being drawn");
}

#[test]
fn enter_and_escape_add_no_step_and_name_the_next_click_as_measured() {
    // ↩ deselects the Work Path the finish left (#12892): the next click starts a new one.
    let mut app = app();
    let before = steps(&app).len();
    triangle(&mut app);
    key(&mut app, Key::Enter);
    assert!(app.ui.pen.is_none(), "Enter ends the path");
    assert_eq!(added(&app, before), ["New Work Path", "New Anchor Point", "New Anchor Point"], "Enter adds no step");
    click(&mut app, [60.0, 140.0]);
    assert_eq!(added(&app, before + 3), ["New Work Path"]);
    assert_eq!(subpaths(work(&app).as_ref().unwrap()), [(false, vec![[60.0, 140.0]])], "a new Work Path, not a component");

    // Escape keeps it selected: the next click adds a component to it.
    let mut app = self::app();
    let before = steps(&app).len();
    triangle(&mut app);
    key(&mut app, Key::Escape);
    assert_eq!(added(&app, before), ["New Work Path", "New Anchor Point", "New Anchor Point"], "Escape adds no step");
    click(&mut app, [60.0, 140.0]);
    assert_eq!(added(&app, before + 3), ["New Path Component"]);
    assert_eq!(subpaths(work(&app).as_ref().unwrap()).len(), 2);
}

/// The path that replaces the one Enter finished (#12892 deselects it) is drawn the same live way as a fresh one: the
/// replacing write leaves the drawn path the document's only one, so its subpath is the one at index zero and the
/// clicks after the first are steps of their own. Pinning only the first click let the draft go live nowhere.
#[test]
fn the_path_that_replaces_the_one_enter_finished_is_live_from_its_first_click() {
    let mut app = app();
    let before = steps(&app).len();
    triangle(&mut app);
    key(&mut app, Key::Enter);
    click(&mut app, [60.0, 140.0]);
    click(&mut app, [140.0, 60.0]);
    assert_eq!(added(&app, before + 3), ["New Work Path", "New Anchor Point"], "every click of the new path is its own step");
    assert_eq!(subpaths(work(&app).as_ref().unwrap()), [(false, vec![[60.0, 140.0], [140.0, 60.0]])], "the first path is replaced, not added to");
    assert_eq!(draft_len(&app), Some(2), "and the Pen is still drawing it");
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

/// The reviewer's input: three anchors, Undo to two, Redo back to the tip. The state Redo brings back is one of the
/// Pen's own clicks, so the draft follows the path forward too (as it does back) and drawing goes on in that path:
/// otherwise the next click starts a new Work Path over the one that was being drawn.
#[test]
fn redoing_to_the_tip_of_a_live_path_puts_its_click_back_on_the_pen() {
    let mut app = app();
    let before = steps(&app).len();
    triangle(&mut app);
    menu(&mut app, "edit.undo");
    assert_eq!(draft_len(&app), Some(2), "Undo takes one click back");
    menu(&mut app, "edit.redo");
    testing::move_to(&mut app, [180.0, 180.0], Modifiers::NONE);
    assert_eq!(draft_len(&app), Some(3), "Redo to the tip puts that click back on the Pen");
    assert!(!app.ui.status_error, "{}", app.ui.status);
    click(&mut app, [180.0, 40.0]);
    assert_eq!(added(&app, before + 3), ["New Anchor Point"], "and drawing goes on in the same path");
    assert_eq!(subpaths(work(&app).as_ref().unwrap()), [(false, vec![[20.0, 20.0], [120.0, 20.0], [120.0, 120.0], [180.0, 40.0]])], "not a new path over it");
}

/// A `path.set` from outside the Pen can carry any History label the command accepts, one of the Pen's
/// own step names included - the label is a name, not a proof of who wrote the state. So a control-channel
/// or scripted write that replaces the Work Path the Pen is drawing, named "New Anchor Point", must not
/// put that path back on the Pen: the draft is stale and is cancelled with the reason, the way every other
/// command's write makes it (#6593), instead of quietly going on over the other caller's anchors (#16686
/// review, second safeguard).
#[test]
fn another_caller_cannot_forge_a_pen_step_with_its_history_name() {
    let mut app = app();
    let before = steps(&app).len();
    triangle(&mut app);
    assert_eq!(draft_len(&app), Some(3), "the Pen is drawing a live Work Path");
    // Another caller replaces that same Work Path under one of the Pen's step names.
    app.run(
        "path.set",
        json!({"name": "work", "historyName": "New Anchor Point", "path": {"subpaths": [{"closed": false, "knots": [[10.0, 10.0], [10.0, 90.0]]}]}}),
    )
    .unwrap();
    assert_eq!(added(&app, before), ["New Work Path", "New Anchor Point", "New Anchor Point", "New Anchor Point"], "the foreign write carries a Pen step name");
    assert_eq!(subpaths(work(&app).as_ref().unwrap()), [(false, vec![[10.0, 10.0], [10.0, 90.0]])], "the other caller's anchors are the document's path now");
    assert!(
        app.ui.pen.is_none(),
        "the Pen does not take that step over: the draft is cancelled, not rebuilt from it: {:?}",
        app.ui.pen.as_ref().map(|p| p.draft.knots.len())
    );
    assert!(app.ui.status_error && app.ui.status.contains("cancelled"), "{}", app.ui.status);
}

#[test]
fn the_make_shape_button_names_its_step_make_shape_from_path() {
    let mut app = app();
    let before = steps(&app).len();
    triangle(&mut app);
    crate::vector_ui::make_shape(&mut app).unwrap();
    assert_eq!(added(&app, before + 3), ["Make shape from path"], "not the shape layer's own name");
    let st = app.session.active().unwrap();
    assert!(matches!(st.doc.layer(st.active_layer.unwrap()).unwrap().content, LayerContent::Shape(_)), "a shape layer was made");
    assert!(work(&app).is_none(), "the shape takes the Work Path");
}

#[test]
fn undo_and_redo_step_the_clicks_one_at_a_time() {
    let mut app = app();
    let before = steps(&app).len();
    triangle(&mut app);
    key(&mut app, Key::Escape);
    assert_eq!(steps(&app).len(), before + 3, "finishing with Escape adds no step");
    menu(&mut app, "edit.undo");
    assert_eq!(subpaths(work(&app).as_ref().unwrap()), [(false, vec![[20.0, 20.0], [120.0, 20.0]])]);
    menu(&mut app, "edit.redo");
    assert_eq!(subpaths(work(&app).as_ref().unwrap()), [(false, vec![[20.0, 20.0], [120.0, 20.0], [120.0, 120.0]])], "Redo puts the anchor back");
    assert_eq!(steps(&app).len(), before + 3, "the same three steps: nothing was added by finishing or by stepping back and forth");
}

// ---------------------------------------------------------------------------------------------
// The cases the issue did not measure keep today's behaviour

#[test]
fn the_shape_mode_pen_still_makes_its_layer_on_the_first_click() {
    let mut app = app();
    app.ui.tool_options.vector_mode = VectorMode::Shape;
    let before = steps(&app).len();
    triangle(&mut app);
    assert_eq!(added(&app, before), ["New Shape Layer", "Edit Shape", "Edit Shape"], "#12909 is untouched");
    assert!(work(&app).is_none(), "Shape mode leaves no Work Path");
}

#[test]
fn the_curvature_pen_keeps_today_one_step_at_the_finish() {
    let mut app = app();
    app.ui.tool = Tool::CurvaturePen;
    let before = steps(&app).len();
    triangle(&mut app);
    assert!(work(&app).is_none(), "not measured: still only a draft while drawing");
    assert_eq!(steps(&app).len(), before, "and no step yet");
    key(&mut app, Key::Enter);
    assert_eq!(added(&app, before), ["Work Path"], "one step, as today");
}

#[test]
fn a_selected_saved_path_still_becomes_one_step_at_the_finish() {
    let mut app = app();
    app.run("path.set", json!({"name": "Side", "path": {"subpaths": [{"closed": false, "knots": [[10.0, 10.0], [10.0, 90.0]]}]}})).unwrap();
    app.ui.selected_path = Some("Side".into());
    let before = steps(&app).len();
    let saved = |app: &PhotocraftApp| app.session.active().unwrap().doc.paths.iter().find(|p| p.name == "Side").map(|p| p.path.subpaths.len());
    triangle(&mut app);
    assert_eq!(saved(&app), Some(1), "not measured: the saved path is not written click by click");
    assert_eq!(steps(&app).len(), before, "and no step while drawing");
    key(&mut app, Key::Enter);
    assert_eq!(saved(&app), Some(2), "the finished path joins it");
    assert_eq!(added(&app, before), ["Save Path"], "one step for the finish, as today");
}

#[test]
fn another_tool_keeps_its_own_step_names_over_the_pen_steps() {
    let mut app = app();
    triangle(&mut app);
    key(&mut app, Key::Escape);
    let before = steps(&app).len();
    app.ui.tool = Tool::Rectangle;
    testing::frame(&mut app);
    drag(&mut app, [40.0, 140.0], [100.0, 180.0], Modifiers::NONE);
    assert_eq!(added(&app, before), ["New Shape Layer"], "the Rectangle tool is unaffected");
}

/// The options bar replays its last render while its version holds (`panels::region_versions::options_bar`), so each
/// thing that row works from has to move the version on its own. A Pen path drawn click by click (#12890) changes
/// those with no History step, so nothing else would redraw the bar. Here every ingredient moves alone, with no
/// document command between the passes: the Paths panel's selection (the version every region shares already holds it),
/// the Work Path the app hides (#12892) and the draft being drawn - which
/// `pen_enter_deselects_tests::enter_deselects_the_work_path_and_the_next_path_replaces_it` pins as well.
#[test]
fn the_options_bar_row_follows_each_path_ingredient_on_its_own() {
    let mut app = app();
    let two = |name: &str, at: [f64; 2]| json!({"name": name, "path": {"subpaths": [{"closed": false, "knots": [at, [at[0] + 60.0, at[1]]]}]}});
    app.run("path.set", two("Side", [10.0, 90.0])).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    let enabled =
        |h: &egui_kittest::Harness<'_, Box<PhotocraftApp>>| ["Selection…", "Shape"].map(|label| !h.get_by_label(label).accesskit_node().is_disabled());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 300.0)).with_context(app.ctx().clone()).build_ui_state(
        |ui, app| {
            // The bar in the shell's options slot, keyed by its version as in the app's window.
            let _ = craft::ui::shell::AppShell::embedded(craft::ui::runtime::env::WindowKey::new("options"))
                .options_bar(true, |app| Some(crate::panels::region_versions::options_bar(app)), |ui, _, app| crate::panels::options_bar(app, ui))
                .show(ui, app);
        },
        app,
    );
    h.run();
    assert_eq!(enabled(&h), [false, false], "nothing selected, no Work Path, no draft");

    // The Paths panel's selection, with no command of its own.
    h.state_mut().ui.selected_path = Some("Side".into());
    h.run();
    assert_eq!(enabled(&h), [true, true], "the selected saved path is the active path");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "selecting it is no step");

    // The Work Path's hide, with a path in the document and no draft: as Make Selection and ↩ leave it (#12892).
    h.state_mut().ui.selected_path = None;
    h.state_mut().run("path.set", two("work", [30.0, 30.0])).unwrap();
    h.run();
    assert_eq!(enabled(&h), [true, true], "a shown Work Path is the active path");
    super::super::path_ops::deselect_loaded_path(h.state_mut(), "work");
    h.run();
    assert_eq!(enabled(&h), [false, false], "hiding it moves no step and no draft: the bar follows the hide");

    // The draft on its own again, over the hidden Work Path.
    let owner = crate::vector_ui::capture_pen_owner(h.state());
    h.state_mut().ui.pen = Some(PenPath::corners(&[[30.0, 60.0], [90.0, 60.0]], owner));
    h.run();
    assert_eq!(enabled(&h), [true, true], "the path being drawn is the one the row works from");
}
