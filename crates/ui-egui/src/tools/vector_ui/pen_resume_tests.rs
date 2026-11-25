//! Undo back to where a Pen path was finished gives the path back to the Pen, and Redo finishes
//! it again (`pen_resume`). The Pen in Path mode draws straight into the Work Path, one step per click (#12890), and
//! has no finish step to give back: the tests of the mechanism take the Curvature Pen, which still finishes a draft
//! in one step, except where the Pen's own steps are the subject.

use super::*;
use crate::canvas::testing;
use photocraft_engine::actions_cmds::ActionsExt;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app
}

/// Edit › Undo or Redo as the menu, the shortcut and the control channel run it.
fn menu(app: &mut PhotocraftApp, id: &str) {
    let app_ctx = app.ctx().clone();
    crate::menus::invoke(app, &app_ctx, id, json!({})).unwrap();
}

/// A click of the current tool at the document point `(x, y)`.
fn click(app: &mut PhotocraftApp, x: f64, y: f64) {
    testing::click(app, [x, y], egui::Modifiers::NONE);
}

/// A key press through the canvas pass, as the app handles it.
fn press(app: &mut PhotocraftApp, key: egui::Key, modifiers: egui::Modifiers) {
    testing::key(app, key, modifiers);
}

/// Three anchors clicked with the current tool.
fn three_anchors(app: &mut PhotocraftApp) {
    for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
        click(app, x, y);
    }
}

/// Switches to `tool` and runs the pass that ends the Pen's path.
fn switch_tool(app: &mut PhotocraftApp, tool: Tool) {
    crate::tool_memory::select(app, tool);
    testing::frame(app);
}

/// A triangle drawn with the Pen and closed by clicking its first anchor; the knots it had
/// before the close.
fn close_triangle(app: &mut PhotocraftApp) -> Vec<[[f64; 2]; 3]> {
    app.ui.tool = Tool::Pen;
    three_anchors(app);
    let knots = app.ui.pen.as_ref().unwrap().triples();
    click(app, 20.0, 20.0);
    assert!(app.ui.pen.is_none(), "clicking the first anchor commits the path");
    knots
}

/// [`close_triangle`] in Path mode: the closed triangle is the work path.
fn closed_triangle(app: &mut PhotocraftApp) -> Vec<[[f64; 2]; 3]> {
    let knots = close_triangle(app);
    assert!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].closed);
    knots
}

/// The Work Path's subpaths, each as its closing and its knot count; empty when there is no Work Path.
fn shape_of_work(app: &PhotocraftApp) -> Vec<(bool, usize)> {
    app.session.active().unwrap().doc.work_path.as_ref().map_or_else(Vec::new, |p| p.subpaths.iter().map(|s| (s.closed, s.knots.len())).collect())
}

#[test]
fn pen_undo_after_closing_takes_back_only_the_close() {
    let mut app = app();
    let knots = closed_triangle(&mut app);
    let steps = app.session.active().unwrap().history.past_len();

    // #12890: the closing click is the step "Close Path". Undo takes exactly that back: the path stays in the Work
    // Path, open again with the anchors it had.
    menu(&mut app, "edit.undo");
    assert_eq!(shape_of_work(&app), [(false, 3)], "open again, not deleted");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps - 1);
    // The close was a finish with a step, so the Pen takes the open path back and carries on (#6619).
    assert_eq!(app.ui.pen.as_ref().map(|p| p.draft.knots.len()), Some(3), "the open path is back on the Pen");
    let sub = &app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0];
    let anchors: Vec<_> = sub.knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
    assert_eq!(anchors, knots.iter().map(|k| k[0]).collect::<Vec<_>>(), "the anchors are as they were drawn");

    // The next Undo takes back the last click, one anchor at a time (#12890).
    menu(&mut app, "edit.undo");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.draft.knots.len()), Some(2));
    assert_eq!(shape_of_work(&app), [(false, 2)]);
}

#[test]
fn pen_undo_after_enter_or_esc_takes_back_one_click() {
    for key in [egui::Key::Enter, egui::Key::Escape] {
        let mut app = app();
        app.ui.tool = Tool::Pen;
        three_anchors(&mut app);
        press(&mut app, key, egui::Modifiers::NONE);
        assert!(app.ui.pen.is_none(), "{key:?} ends the path");
        assert_eq!(shape_of_work(&app), [(false, 3)], "{key:?} adds no step (#12890)");

        // Undo takes back exactly the last click; no draft comes back, because the finish made no step.
        press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(shape_of_work(&app), [(false, 2)], "{key:?}");
        assert!(app.ui.pen.is_none(), "{key:?}");
        press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(shape_of_work(&app), [(false, 1)]);
        press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(shape_of_work(&app), [], "the first click's step goes with the Work Path");

        // Redo puts the clicks back one at a time.
        for want in [(false, 1), (false, 2), (false, 3)] {
            menu(&mut app, "edit.redo");
            assert_eq!(shape_of_work(&app), [want], "{key:?}");
        }
    }
}

#[test]
fn pen_undo_after_a_close_drag_gives_back_the_first_anchor_unchanged() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    three_anchors(&mut app);
    // Dragging from the first anchor pulls out its handles in the closed path only.
    testing::press(&mut app, [20.0, 20.0], egui::Modifiers::NONE);
    testing::move_to(&mut app, [20.0, 60.0], egui::Modifiers::NONE);
    testing::release(&mut app, [20.0, 60.0], egui::Modifiers::NONE);
    let first = app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].knots[0];
    assert!(first.smooth && first.out_ctrl.y > 20.0, "the close-drag shaped the closed path: {first:?}");

    // Undo takes the "Close Path" step back: the path is open again with its first anchor untouched (#12890).
    menu(&mut app, "edit.undo");
    let first = &app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].knots[0];
    assert!(!first.smooth && first.in_ctrl == first.anchor && first.out_ctrl == first.anchor, "no pulled handles: {first:?}");
    assert_eq!(shape_of_work(&app), [(false, 3)]);
}

#[test]
fn curvature_pen_undo_after_closing_reopens_the_path() {
    let mut app = app();
    app.ui.tool = Tool::CurvaturePen;
    three_anchors(&mut app);
    let knots = app.ui.pen.as_ref().unwrap().triples();
    click(&mut app, 20.0, 20.0);
    assert!(app.ui.pen.is_none());
    assert!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].closed);

    menu(&mut app, "edit.undo");
    let pen = app.ui.pen.as_ref().unwrap();
    assert!(pen.curvature);
    assert_eq!(pen.triples(), knots);
    assert!(app.session.active().unwrap().doc.work_path.is_none());
}

#[test]
fn undo_and_redo_step_a_closed_pen_path_one_click_at_a_time() {
    let mut app = app();
    closed_triangle(&mut app);
    let closed = app.session.active().unwrap().doc.work_path.clone();
    // #12890: Undo takes back the closing click, Redo puts it back; the path is never deleted whole.
    for _ in 0..2 {
        menu(&mut app, "edit.undo");
        assert_eq!(shape_of_work(&app), [(false, 3)], "open again");
        menu(&mut app, "edit.redo");
        assert_eq!(app.session.active().unwrap().doc.work_path, closed, "Redo closes it again");
        assert!(app.ui.pen.is_none());
    }
}

#[test]
fn redo_after_editing_a_reopened_path_leaves_the_draft_alone() {
    let mut app = app();
    // The Curvature Pen still finishes its path in one step, which is what Redo would put back.
    app.ui.tool = Tool::CurvaturePen;
    three_anchors(&mut app);
    click(&mut app, 20.0, 20.0);
    assert!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].closed, "closed and committed");
    menu(&mut app, "edit.undo");
    click(&mut app, 60.0, 160.0);
    let draft = app.ui.pen.clone();
    // The draft changed since the reopen, so the closed path is not the one Redo brings back.
    menu(&mut app, "edit.redo");
    assert_eq!(app.ui.pen, draft);
}

#[test]
fn undo_after_a_later_step_takes_that_step_back_first() {
    let mut app = app();
    closed_triangle(&mut app);
    app.run("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 40], "fill": "#ff0000"})).unwrap();
    let after_close = app.session.active().unwrap().history.past_len() - 1;
    menu(&mut app, "edit.undo");
    assert!(app.ui.pen.is_none(), "Undo takes back the rectangle only");
    assert_eq!(app.session.active().unwrap().history.past_len(), after_close);
    assert!(app.session.active().unwrap().doc.work_path.is_some());
    // Back at the close, the next Undo takes the closing click back (#12890).
    menu(&mut app, "edit.undo");
    assert_eq!(shape_of_work(&app), [(false, 3)]);
}

#[test]
fn undo_walks_back_every_path_drawn_since_the_last_reset_one_click_at_a_time() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    // Three paths, each ended with Esc: it keeps the Work Path selected (#12892) so the next path joins it, and adds
    // no step of its own (#12890).
    for dy in [0.0, 50.0, 100.0] {
        for (x, y) in [(20.0, 20.0 + dy), (120.0, 20.0 + dy), (120.0, 60.0 + dy)] {
            click(&mut app, x, y);
        }
        press(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(app.ui.pen.is_none());
    }
    assert_eq!(shape_of_work(&app), vec![(false, 3), (false, 3), (false, 3)], "the paths build up in the work path");

    // #12890: Undo walks back through the clicks, one at a time, and a component goes with its first anchor.
    let wanted: [Vec<(bool, usize)>; 3] = [vec![(false, 3), (false, 3), (false, 2)], vec![(false, 3), (false, 3), (false, 1)], vec![(false, 3), (false, 3)]];
    for want in wanted {
        press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(shape_of_work(&app), want, "{}", app.ui.status);
        assert!(app.ui.pen.is_none(), "no finish step to give back");
    }
}

#[test]
fn undo_steps_back_from_make_selection_to_the_path_clicks() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    three_anchors(&mut app);
    // The canvas menu's Make Selection ends the path, then makes the selection from it.
    pen_commit(&mut app, false);
    app.run("path.toSelection", json!({"name": "work"})).unwrap();
    let doc = |app: &PhotocraftApp| app.session.active().unwrap().doc.clone();
    assert!(doc(&app).selection.is_some());

    menu(&mut app, "edit.undo");
    assert!(doc(&app).selection.is_none(), "Undo takes back the selection first");
    assert!(doc(&app).work_path.is_some() && app.ui.pen.is_none());
    // #12890: the clicks are the steps, so the next Undo takes one anchor back rather than a finish.
    menu(&mut app, "edit.undo");
    assert_eq!(shape_of_work(&app), [(false, 2)], "then the last click goes");

    // Redo walks forward again.
    menu(&mut app, "edit.redo");
    menu(&mut app, "edit.redo");
    assert!(doc(&app).selection.is_some() && app.ui.pen.is_none());
    assert_eq!(shape_of_work(&app), [(false, 3)]);
}

#[test]
fn undo_after_switching_tools_takes_the_last_click_or_the_finish() {
    for pen_tool in [Tool::Pen, Tool::CurvaturePen] {
        let mut app = app();
        app.ui.tool = pen_tool;
        three_anchors(&mut app);
        let knots = app.ui.pen.as_ref().unwrap().triples();
        // Choosing another tool ends the path in the tool host's next pass.
        switch_tool(&mut app, Tool::Move);
        assert!(app.session.active().unwrap().doc.work_path.is_some());

        press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
        if pen_tool == Tool::Pen {
            // #12890: the Pen's clicks are the steps and switching tools added none, so Undo takes a click back.
            assert_eq!(shape_of_work(&app), [(false, 2)], "one click back");
            assert!(app.ui.pen.is_none(), "no finish to give back");
        } else {
            assert_eq!(app.ui.tool, pen_tool, "the tool that drew the path comes back");
            assert_eq!(app.ui.pen.as_ref().map(|p| p.triples()), Some(knots));
            assert!(app.session.active().unwrap().doc.work_path.is_none());
        }
    }
}

#[test]
fn a_free_transform_box_keeps_its_own_undo_and_redo_over_a_finished_pen_path() {
    let mut app = app();
    let ctx = app.ctx().clone();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 50, "y": 50, "width": 60, "height": 60})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    // The Curvature Pen still finishes the path in one step, which the box's own Undo and Redo stand in front of.
    app.ui.tool = Tool::CurvaturePen;
    three_anchors(&mut app);
    let knots = app.ui.pen.as_ref().unwrap().triples();
    switch_tool(&mut app, Tool::Move);

    // Undo under the box steps the box: the path stays finished, and comes back once it closes.
    crate::transform_tool::begin(&mut app, &ctx).unwrap();
    menu(&mut app, "edit.undo");
    assert!(app.ui.transform.is_some() && app.ui.pen.is_none(), "the box took the Undo");
    assert!(app.session.active().unwrap().doc.work_path.is_some());
    crate::transform_tool::cancel(&mut app);
    menu(&mut app, "edit.undo");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.triples()), Some(knots));

    // Redo under the box steps the box too, leaving the Pen its path.
    crate::transform_tool::begin(&mut app, &ctx).unwrap();
    menu(&mut app, "edit.redo");
    assert!(app.ui.transform.is_some() && app.ui.pen.is_some(), "the box took the Redo");
    assert!(app.session.active().unwrap().doc.work_path.is_none());

    // Nor do ⌘/Ctrl+Z or Edit › Undo take an anchor off the draft under the box.
    let knots = app.ui.pen.as_ref().unwrap().triples();
    press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
    menu(&mut app, "edit.undo");
    assert!(app.ui.transform.is_some());
    assert_eq!(app.ui.pen.as_ref().unwrap().triples(), knots, "the box took the Undo");
    crate::transform_tool::cancel(&mut app);
    press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
    let anchors = |k: &Knots| k.iter().map(|t| t[0]).collect::<Vec<_>>();
    let now = app.ui.pen.as_ref().unwrap().triples();
    assert_eq!(anchors(&now), anchors(&knots[..knots.len() - 1].to_vec()), "then Undo takes the last anchor");
}

#[test]
fn redo_does_not_put_a_finished_path_beside_its_edited_draft() {
    let mut app = app();
    let ctx = app.ctx().clone();
    // The Curvature Pen still finishes a path in one History step; the Pen in Path mode draws it click by click
    // (#12890), so there is no finish step beside a draft to put back.
    app.ui.tool = Tool::CurvaturePen;
    three_anchors(&mut app);
    press(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
    menu(&mut app, "edit.undo");
    click(&mut app, 20.0, 120.0);
    let knots = app.ui.pen.as_ref().unwrap().triples();
    assert!(!crate::menus::is_enabled(&app, "edit.redo"), "Redo greys out over the edited draft");
    crate::menus::invoke(&mut app, &ctx, "edit.redo", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.work_path.is_none(), "no finished path beside the draft");
    assert_eq!(app.ui.pen.as_ref().unwrap().triples(), knots);

    // Back to what Undo gave back, Redo finishes it again.
    press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
    assert!(crate::menus::is_enabled(&app, "edit.redo"));
    menu(&mut app, "edit.redo");
    assert!(app.ui.pen.is_none());
    assert_eq!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].knots.len(), 3);
}

#[test]
fn redo_of_a_step_made_and_undone_over_a_resumed_path_does_not_finish_it() {
    let mut app = app();
    app.ui.tool = Tool::CurvaturePen;
    three_anchors(&mut app);
    press(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
    menu(&mut app, "edit.undo");
    // Another step over the resumed path, undone: it, not the finish, is next to redo.
    app.run("select.all", json!({})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    assert!(crate::menus::is_enabled(&app, "edit.redo"));
    menu(&mut app, "edit.redo");
    let st = app.session.active().unwrap();
    assert!(st.doc.selection.is_some(), "Redo made the selection again");
    assert!(st.doc.work_path.is_none(), "and didn't finish the path");
    // The steps since changed the document under the draft, which drops it (`PenOwner::valid`).
    assert!(app.ui.pen.is_none());
}

#[test]
fn a_refused_pen_redo_keeps_the_path_claimed_by_its_document() {
    let mut app = app();
    app.ui.tool = Tool::CurvaturePen;
    three_anchors(&mut app);
    press(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
    menu(&mut app, "edit.undo");
    let owner = app.ui.pen_doc;
    assert!(app.ui.pen.is_some() && owner.is_some());
    app.automation_input = true;
    app.services.automation_command = Some(Box::new(|id, _| if id == "edit.redo" { Err("denied".into()) } else { Ok(()) }));
    assert!(pen_finish_on_redo(&mut app).is_err());
    assert!(app.ui.pen.is_some());
    assert_eq!(app.ui.pen_doc, owner, "the path given back still belongs to its document");
}

#[test]
fn pen_undo_after_closing_a_shape_reopens_the_path() {
    let mut app = app();
    app.ui.tool_options.vector_mode = craft::draw::ui::shape::VectorMode::Shape;
    let layers = app.session.active().unwrap().doc.layers.len();
    let knots = close_triangle(&mut app);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1);
    menu(&mut app, "edit.undo");
    // The shape layer was made on the first anchor (#12909): Undo takes back only the closing.
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1, "the shape layer stays, open again");
    assert_eq!(app.ui.pen.as_ref().unwrap().triples(), knots);
}

fn work_path(app: &PhotocraftApp) -> Option<Path> {
    app.session.active().unwrap().doc.work_path.clone()
}

type Knots = Vec<[[f64; 2]; 3]>;

/// An open path of two anchors ended with Esc (which keeps it selected, #12892), then carried on from its last anchor (#6593) by one
/// more anchor and ended again. The open path and the carried-on draft's knots. The Curvature Pen, which still finishes
/// in one step (#12890), stands for the carry-on mechanism here.
fn carry_on_a_path(app: &mut PhotocraftApp) -> (Option<Path>, Knots, Knots) {
    app.ui.tool = Tool::CurvaturePen;
    for (x, y) in [(20.0, 20.0), (120.0, 20.0)] {
        click(app, x, y);
    }
    let first = app.ui.pen.as_ref().unwrap().triples();
    press(app, egui::Key::Escape, egui::Modifiers::NONE);
    let open = work_path(app);
    click(app, 120.0, 20.0);
    // The Curvature Pen's click on an endpoint only selects it; the next click carries the path on (#10726).
    click(app, 120.0, 120.0);
    assert!(
        app.ui.pen.as_ref().is_some_and(|p| p.resumes.is_some()),
        "the click after the endpoint carries the path on: pen {:?}, status {}",
        app.ui.pen.as_ref().map(|p| (p.draft.knots.len(), p.curvature)),
        app.ui.status
    );
    let carried = app.ui.pen.as_ref().unwrap().triples();
    assert_eq!(carried.len(), 3);
    press(app, egui::Key::Escape, egui::Modifiers::NONE);
    let work = work_path(app).unwrap();
    assert_eq!((work.subpaths.len(), work.subpaths[0].knots.len()), (1, 3), "the path grew in place");
    (open, first, carried)
}

/// #6413 review: a path carried on from its endpoint (#6593) comes back from Undo still carried
/// on. Undo then takes back the added anchor, then drops the draft and leaves the path as it was
/// (`remove_last`); one more Undo gives back the path's own first finish.
#[test]
fn undo_after_finishing_a_carried_on_path_gives_it_back_still_carried_on() {
    let mut app = app();
    let (open, _first, carried) = carry_on_a_path(&mut app);

    menu(&mut app, "edit.undo");
    let pen = app.ui.pen.as_ref().expect("the carried-on path is back on the Pen");
    assert_eq!(pen.triples(), carried);
    assert!(pen.resumes.is_some(), "still carrying the path on");
    // The anchors, not the Curvature Pen's handle reflow: which handles the path carried before the added anchor is
    // the curvature solver's own business, not this test's.
    let anchors = |p: &Option<Path>| p.as_ref().map(|p| p.subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect::<Vec<_>>());
    assert_eq!(anchors(&work_path(&app)), anchors(&open), "the path is as it was before the second finish");

    menu(&mut app, "edit.undo");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.draft.knots.len()), Some(2), "the added anchor goes");
    menu(&mut app, "edit.undo");
    assert!(app.ui.pen.is_none(), "past the added anchors the draft goes");
    assert_eq!(anchors(&work_path(&app)), anchors(&open), "and the path stays as it was");

    // No resume record is left to take back: in Path mode the Pen's clicks are the steps (#12890), so there is no
    // finish step whose path comes back to the Pen. The one-step finish is pinned for the unmeasured variants in
    // pen_variants_tests and freeform_history_name_tests.
    assert!(app.ui.pen.is_none(), "no draft comes back for the added anchors");
}

#[test]
fn redo_finishes_a_carried_on_path_again_in_place() {
    let mut app = app();
    let (_, _, carried) = carry_on_a_path(&mut app);
    let finished = work_path(&app);
    menu(&mut app, "edit.undo");
    menu(&mut app, "edit.redo");
    assert!(app.ui.pen.is_none());
    assert_eq!(work_path(&app), finished, "one subpath again, not a second one beside it");

    // With the draft dropped, Redo brings the finish back, and Undo gives the path back again.
    menu(&mut app, "edit.undo");
    menu(&mut app, "edit.undo");
    menu(&mut app, "edit.undo");
    assert!(app.ui.pen.is_none());
    menu(&mut app, "edit.redo");
    assert_eq!(work_path(&app), finished);
    menu(&mut app, "edit.undo");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.triples()), Some(carried));
}

/// The draft Undo gives back belongs to the document state Undo went back to: the next frame
/// doesn't take it for a stale draft and finish it, and ↩ commits it.
#[test]
fn a_path_given_back_by_undo_is_owned_by_the_current_document_state() {
    let mut app = app();
    // The Curvature Pen: its finish is one step, which Undo gives back as a draft.
    app.ui.tool = Tool::CurvaturePen;
    three_anchors(&mut app);
    let knots = app.ui.pen.as_ref().unwrap().triples();
    click(&mut app, 20.0, 20.0);
    assert!(work_path(&app).unwrap().subpaths[0].closed, "closed and committed");
    menu(&mut app, "edit.undo");
    let owner_valid = |app: &PhotocraftApp| app.ui.pen.as_ref().and_then(|p| p.owner.as_ref()).is_some_and(|o| o.valid(app));
    assert!(owner_valid(&app));
    assert!(!cancel_stale_pen(&mut app));
    testing::frame(&mut app);
    assert_eq!(app.ui.pen.as_ref().map(|p| p.triples()), Some(knots), "the draft is still being drawn");
    assert!(work_path(&app).is_none());
    press(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
    assert!(app.ui.pen.is_none() && !app.ui.status_error, "{}", app.ui.status);
    assert_eq!(work_path(&app).unwrap().subpaths[0].knots.len(), 3);
}

/// Three anchors drawn with the Curvature Pen and ended with ↩; their knots. Its finish is one History step, which the
/// Pen in Path mode no longer has (#12890).
fn enter_path(app: &mut PhotocraftApp) -> Knots {
    app.ui.tool = Tool::CurvaturePen;
    three_anchors(app);
    let knots = app.ui.pen.as_ref().unwrap().triples();
    press(app, egui::Key::Enter, egui::Modifiers::NONE);
    assert!(app.ui.pen.is_none() && work_path(app).is_some());
    knots
}

/// #6413 review: Edit › Step Backward and Step Forward (the History panel's too) step a Pen path
/// as Undo and Redo do, instead of deleting the finished path whole. #12890: the Pen's clicks are
/// its own steps, so Step Backward takes one click at a time.
#[test]
fn step_backward_and_forward_step_a_pen_path_as_undo_and_redo_do() {
    let mut app = app();
    closed_triangle(&mut app);
    menu(&mut app, "history.stepBackward");
    assert_eq!(shape_of_work(&app), [(false, 3)], "the closing click is taken back");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.draft.knots.len()), Some(3), "the open path is back on the Pen (#6619)");
    menu(&mut app, "history.stepForward");
    assert_eq!(shape_of_work(&app), [(true, 3)], "Step Forward closes it again");
    menu(&mut app, "history.stepBackward");
    menu(&mut app, "history.stepBackward");
    assert_eq!(shape_of_work(&app), [(false, 2)], "then the clicks go one at a time");

    // A first path's anchors step back with no History step to go back to.
    let mut app = self::app();
    app.ui.tool = Tool::Pen;
    click(&mut app, 20.0, 20.0);
    assert!(crate::menus::is_enabled(&app, "history.stepBackward"));
    menu(&mut app, "history.stepBackward");
    assert!(app.ui.pen.is_none() && work_path(&app).is_none(), "the draft goes with its Work Path");
}

/// #6413 review: a path finished in another document leaves this document's Redo of a path given
/// back by Undo alone; the next Undo gives the path back again rather than deleting it.
#[test]
fn a_path_finished_in_another_document_keeps_this_documents_pen_redo() {
    let mut app = app();
    let knots = enter_path(&mut app);
    let finished = work_path(&app);
    menu(&mut app, "edit.undo");
    for _ in 0..knots.len() {
        menu(&mut app, "edit.undo");
    }
    assert!(app.ui.pen.is_none() && work_path(&app).is_none(), "every anchor taken back");

    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    enter_path(&mut app);
    assert!(app.session.set_active(0));

    menu(&mut app, "edit.redo");
    assert_eq!(work_path(&app), finished);
    menu(&mut app, "edit.undo");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.triples()), Some(knots), "Undo gives the path back to the Pen");
    assert!(work_path(&app).is_none());
}

/// #6413 review: the same anchors redrawn in Shape mode after Undo took the path given back apart
/// are a new draft: Redo doesn't throw it away to bring back the finished path. The finish is one step
/// here: the Pen in Path mode draws its steps click by click (#12890), so this takes the selected
/// saved path, which still finishes in one step.
#[test]
fn redo_leaves_the_same_anchors_redrawn_in_shape_mode_alone() {
    let mut app = app();
    app.run("path.set", json!({"name": "Side", "path": {"subpaths": [{"closed": false, "knots": [[10.0, 10.0], [10.0, 90.0]]}]}})).unwrap();
    app.ui.selected_path = Some("Side".into());
    app.ui.tool = Tool::Pen;
    three_anchors(&mut app);
    let knots = app.ui.pen.as_ref().unwrap().triples();
    press(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
    let saved = |app: &PhotocraftApp| app.session.active().unwrap().doc.paths.iter().find(|p| p.name == "Side").map(|p| p.path.subpaths.len());
    assert!(app.ui.pen.is_none() && saved(&app) == Some(2), "the finished path went into the selected saved path");
    menu(&mut app, "edit.undo");
    for _ in 0..knots.len() {
        menu(&mut app, "edit.undo");
    }
    assert!(app.ui.pen.is_none());
    app.ui.tool_options.vector_mode = craft::draw::ui::shape::VectorMode::Shape;
    // Not through clicks: in Shape mode they make the shape layer at once (#12909), a History step that clears Redo.
    // This is a Shape draft that is not in the document yet.
    let corners: Vec<[f64; 2]> = knots.iter().map(|k| k[0]).collect();
    app.ui.pen = Some(PenPath::corners(&corners, capture_pen_owner(&app)));
    let redrawn = app.ui.pen.as_ref().unwrap().triples();
    assert_eq!(redrawn.len(), knots.len(), "the same anchors, redrawn");
    assert!(!crate::menus::is_enabled(&app, "edit.redo"), "Redo greys out over the new draft");
    menu(&mut app, "edit.redo");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.triples()), Some(redrawn), "the Shape draft is still being drawn");
    assert!(work_path(&app).is_none(), "no Work Path brought back beside it");
}

/// #6413 review: a draft the document refuses (a locked shape layer) and Esc keeps as the Work
/// Path comes back to the Pen on Undo too.
#[test]
fn undo_gives_back_a_refused_draft_esc_kept_as_the_work_path() {
    let mut app = app();
    app.ui.tool_options.vector_mode = craft::draw::ui::shape::VectorMode::Shape;
    close_triangle(&mut app);
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    app.ui.tool_options.pen_path_op = "subtract".into();
    for (x, y) in [(40.0, 40.0), (100.0, 40.0), (100.0, 100.0)] {
        click(&mut app, x, y);
    }
    let knots = app.ui.pen.as_ref().unwrap().triples();
    press(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
    assert!(app.ui.pen.is_none() && work_path(&app).is_some(), "kept as the Work Path: {}", app.ui.status);
    menu(&mut app, "edit.undo");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.triples()), Some(knots), "the draft is back on the Pen");
    assert!(work_path(&app).is_none());
}

/// Keep a real refused commit, with an older document edit and a library Undo to route to.
fn failed_library_draft(tool: Tool, shape: bool) -> Box<PhotocraftApp> {
    let mut app = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("actions.newSet", json!({"name": "Keep"})).unwrap();
    app.run("actions.clearAll", json!({"confirm": true})).unwrap();
    app.ui.tool = tool;
    app.ui.tool_options.vector_mode = if shape { craft::draw::ui::shape::VectorMode::Shape } else { craft::draw::ui::shape::VectorMode::Path };
    // A refused path is refused from the first anchor on: the Pen makes its shape layer (#12909) or its Work Path
    // (#12890) with the first click.
    if shape {
        app.session.authorize =
            Some(|id, _| if id == "shape.create" { Err(photocraft_engine::EngineError::Other("refused shape fixture".into())) } else { Ok(()) });
    } else {
        app.session.authorize = Some(|id, _| if id == "path.set" { Err(photocraft_engine::EngineError::Other("refused path fixture".into())) } else { Ok(()) });
    }
    for (x, y) in [(20.0, 20.0), (80.0, 60.0)] {
        click(&mut app, x, y);
    }
    assert!(!pen_commit(&mut app, false), "the fixture must actually refuse the commit");
    let draft = app.ui.pen.as_ref().unwrap();
    assert!(draft.commit_failed && draft.owner.as_ref().is_some_and(|owner| owner.valid(&app)));
    assert_eq!(draft.draft.knots.len(), 2);
    app
}

#[test]
fn failed_pen_draft_after_tool_switch_leaves_library_undo_to_each_entrypoint() {
    for tool in [Tool::Pen, Tool::CurvaturePen] {
        for entry in ["menu", "shortcut", "command"] {
            let mut app = failed_library_draft(tool, false);
            assert!(pen_can_undo(&app));
            assert!(crate::actions::library::edit_undo_label(&app).is_none());
            let draft = app.ui.pen.clone();
            let history = app.session.active().unwrap().history.past_len();
            switch_tool(&mut app, Tool::Brush);
            assert_eq!(app.ui.pen, draft, "a refused path waits for explicit retry");
            assert!(!pen_can_undo(&app));
            assert_eq!(crate::actions::library::edit_undo_label(&app), Some("Undo Clear All Actions"));
            match entry {
                "menu" => menu(&mut app, "edit.undo"),
                "shortcut" => press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND),
                _ => {
                    app.run("edit.undo", json!({})).unwrap();
                }
            }
            assert_eq!(app.session.actions().list.sets(), ["Keep"], "{tool:?}, {entry}: the library was restored");
            assert_eq!(app.ui.pen, draft, "{tool:?}, {entry}: hidden anchors are untouched");
            assert_eq!(app.session.active().unwrap().history.past_len(), history);
        }
    }
}

#[test]
fn active_failed_pen_drafts_keep_anchor_undo_and_library_pending() {
    for tool in [Tool::Pen, Tool::CurvaturePen] {
        for shortcut in [false, true] {
            let mut app = failed_library_draft(tool, false);
            let history = app.session.active().unwrap().history.past_len();
            assert!(pen_can_undo(&app));
            assert!(crate::actions::library::edit_undo_label(&app).is_none());
            if shortcut {
                press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
            } else {
                menu(&mut app, "edit.undo");
            }
            assert_eq!(app.ui.pen.as_ref().unwrap().draft.knots.len(), 1, "{tool:?}: the active editor owns Undo");
            assert!(app.session.actions().list.sets().is_empty(), "the library Undo is still pending");
            assert_eq!(app.session.active().unwrap().history.past_len(), history);
        }
    }
}

#[test]
fn hidden_failed_pen_drafts_do_not_take_document_history_steps() {
    for tool in [Tool::Pen, Tool::CurvaturePen] {
        let mut app = failed_library_draft(tool, false);
        let draft = app.ui.pen.clone();
        let history = app.session.active().unwrap().history.past_len();
        assert_eq!(history, 1, "the fixture has exactly one committed layer edit");
        switch_tool(&mut app, Tool::Brush);
        menu(&mut app, "history.stepBackward");
        assert_eq!(app.session.active().unwrap().history.past_len(), 0, "the document edit was undone");
        assert_eq!(app.ui.pen, draft, "the retained draft was not edited");
        assert!(!crate::menus::is_enabled(&app, "history.stepBackward"), "hidden anchors cannot enable another History step");
        assert!(app.session.actions().list.sets().is_empty(), "History does not consume the library Undo");
    }
}

#[test]
fn failed_shape_drafts_keep_the_work_path_fallback_on_tool_switch() {
    for tool in [Tool::Pen, Tool::CurvaturePen] {
        let mut app = failed_library_draft(tool, true);
        let draft = app.ui.pen.clone().unwrap();
        let history = app.session.active().unwrap().history.past_len();
        assert!(app.session.active().unwrap().doc.work_path.is_none());
        switch_tool(&mut app, Tool::Brush);
        assert!(app.ui.pen.is_none(), "a refused Shape is kept as the Work Path instead");
        let path = app.session.active().unwrap().doc.work_path.as_ref().unwrap();
        let knots: Vec<_> =
            path.subpaths[0].knots.iter().map(|k| [[k.anchor.x, k.anchor.y], [k.in_ctrl.x, k.in_ctrl.y], [k.out_ctrl.x, k.out_ctrl.y]]).collect();
        assert_eq!(knots, draft.triples(), "{tool:?}: the fallback retains all geometry");
        assert!(!path.subpaths[0].closed);
        assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
    }
}

/// A valid Freeform release which the document refuses, not an in-progress lasso gesture.
fn refused_freeform_library_draft() -> Box<PhotocraftApp> {
    let mut app = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("actions.newSet", json!({"name": "Keep"})).unwrap();
    app.run("actions.clearAll", json!({"confirm": true})).unwrap();
    app.ui.tool = Tool::FreeformPen;
    app.ui.tool_options.vector_mode = craft::draw::ui::shape::VectorMode::Path;
    let owner = capture_pen_owner(&app).unwrap();
    app.session.authorize = Some(|id, _| if id == "path.set" { Err(photocraft_engine::EngineError::Other("refused Freeform fixture".into())) } else { Ok(()) });
    freeform_finish_owned(&mut app, &[[20.0, 20.0, 1.0], [80.0, 20.0, 1.0], [80.0, 80.0, 1.0]], &[1, 2], owner);
    let draft = app.ui.pen.as_ref().expect("the refused fitted release must be retained");
    assert!(draft.commit_failed && draft.freeform_origin && draft.owner.as_ref().is_some_and(|owner| owner.valid(&app)));
    assert_eq!(draft.draft.knots.len(), 3, "all three straight vertices survived fitting");
    app
}

#[test]
fn failed_freeform_drafts_keep_same_tool_anchor_undo() {
    for entry in ["menu", "shortcut", "history"] {
        let mut app = refused_freeform_library_draft();
        let draft = app.ui.pen.as_ref().unwrap().clone();
        let history = app.session.active().unwrap().history.past_len();
        assert!(!draft.drawn_with(Tool::FreeformPen), "general drawing and finish policy is unchanged");
        assert!(pen_can_undo(&app));
        assert!(crate::actions::library::edit_undo_label(&app).is_none());
        match entry {
            "menu" => menu(&mut app, "edit.undo"),
            "shortcut" => press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND),
            _ => menu(&mut app, "history.stepBackward"),
        }
        assert_eq!(app.ui.pen.as_ref().unwrap().triples(), draft.triples()[..2].to_vec(), "{entry}: retain the old fitted-anchor Undo");
        assert!(app.session.actions().list.sets().is_empty(), "the library Undo stays pending");
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
        switch_tool(&mut app, Tool::Brush);
        let remaining = app.ui.pen.clone();
        assert_eq!(crate::actions::library::edit_undo_label(&app), Some("Undo Clear All Actions"));
        menu(&mut app, "edit.undo");
        assert_eq!(app.session.actions().list.sets(), ["Keep"]);
        assert_eq!(app.ui.pen, remaining, "another tool does not edit the failed Freeform draft");
    }
}

#[test]
fn freeform_selection_does_not_claim_a_failed_pen_or_curvature_draft() {
    for tool in [Tool::Pen, Tool::CurvaturePen] {
        for shortcut in [false, true] {
            let mut app = failed_library_draft(tool, false);
            let draft = app.ui.pen.clone();
            assert!(!draft.as_ref().unwrap().freeform_origin);
            let history = app.session.active().unwrap().history.past_len();
            switch_tool(&mut app, Tool::FreeformPen);
            assert!(!pen_can_undo(&app), "selecting Freeform cannot invent a Freeform release");
            assert_eq!(crate::actions::library::edit_undo_label(&app), Some("Undo Clear All Actions"));
            if shortcut {
                press(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
            } else {
                menu(&mut app, "edit.undo");
            }
            assert_eq!(app.session.actions().list.sets(), ["Keep"], "{tool:?}: undo the library, not foreign anchors");
            assert_eq!(app.ui.pen, draft);
            assert_eq!(app.session.active().unwrap().history.past_len(), history);
        }
    }
}

/// A refused Pen draft is kept (#15488): under another tool, Delete is that tool's (the active layer goes); back under
/// the Pen that drew it, Delete takes back one anchor at a time, as while drawing, and only the last anchor drops the
/// draft.
#[test]
fn a_kept_pen_draft_takes_delete_only_under_its_own_tool() {
    for tool in [Tool::Pen, Tool::CurvaturePen] {
        let mut app = app();
        app.run("layer.new.layer", json!({})).unwrap();
        app.ui.tool = tool;
        app.ui.tool_options.vector_mode = craft::draw::ui::shape::VectorMode::Path;
        // Refused from the first anchor on, as the Pen's Work Path write is (#12890).
        app.session.authorize = Some(|id, _| if id == "path.set" { Err(photocraft_engine::EngineError::Other("refused path fixture".into())) } else { Ok(()) });
        three_anchors(&mut app);
        assert!(!pen_commit(&mut app, false), "the fixture must actually refuse the commit");
        app.session.authorize = None;
        assert_eq!(app.ui.pen.as_ref().unwrap().draft.knots.len(), 3);
        let layers = app.session.active().unwrap().doc.layers.len();
        let kept = app.ui.pen.as_ref().unwrap().draft.clone();

        switch_tool(&mut app, Tool::AddAnchorPoint);
        press(&mut app, egui::Key::Delete, egui::Modifiers::NONE);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers - 1, "{tool:?}: Delete is the other tool's: the layer goes");
        assert_eq!(app.ui.pen.as_ref().map(|p| &p.draft), Some(&kept), "{tool:?}: the kept draft is untouched");

        switch_tool(&mut app, tool);
        for left in [2, 1] {
            press(&mut app, egui::Key::Delete, egui::Modifiers::NONE);
            assert_eq!(app.ui.pen.as_ref().map(|p| p.draft.knots.len()), Some(left), "{tool:?}: Delete takes back one anchor ({})", app.ui.status);
        }
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers - 1, "{tool:?}: no other layer went");
        press(&mut app, egui::Key::Delete, egui::Modifiers::NONE);
        assert!(app.ui.pen.is_none(), "{tool:?}: the last anchor drops the draft");
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers - 1, "{tool:?}: the last Delete is the drawing tool's too");
    }
}
