use craft::draw::geometry::ShapeGeometry;
use craft::draw::ui::VectorMode;
use craft::draw::ui::paths::{PathKind, PathsCommands, PathsEvent, PathsFooterButton};
use craft::draw::ui::pen::{AnchorsShown, EditPath};
use craft::draw::ui::shape_drag::ShapeKind;
use craft::keymap::Mods;
use egui::Modifiers;

use super::host;
use super::overlay::overlay_paths;
use super::paths_panel::{apply, command_params};
use super::shapes::{finish_drag, shape_preview};
use super::*;
use crate::canvas::testing;
use crate::canvas::{ToolEvent, tool_event};
use crate::direct_select::PathRef;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app
}

/// A drag of a shape tool from `from` to `to`, through the tool's own event path, without modifiers.
fn shape_drag(app: &mut PhotocraftApp, tool: Tool, from: [f64; 2], to: [f64; 2]) {
    app.ui.tool = tool;
    let none = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, none);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, none);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, none);
}

#[test]
fn line_drag_with_a_path_operation_records_one_line_tool_step() {
    let mut app = app();
    app.ui.tool_options.shape_mode = VectorMode::Shape;
    shape_drag(&mut app, Tool::Line, [20.0, 20.0], [120.0, 100.0]);
    let initial = app.run("shape.info", json!({})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();

    for op in ["combine", "subtract", "intersect"] {
        app.ui.tool_options.shape_path_op = op.into();
        shape_drag(&mut app, Tool::Line, [30.0, 30.0], [120.0, 85.0]);
        let result = app.run("shape.info", json!({})).unwrap();
        assert_eq!(result["layer"], initial["layer"], "{op}");
        assert_eq!(result["path"]["subpaths"][1]["op"], op);
        let history = &app.session.active().unwrap().history;
        assert_eq!(history.past_len(), steps + 1, "{op}");
        assert_eq!(history.undo_label(), Some("Line Tool"), "{op}");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.run("shape.info", json!({})).unwrap(), initial, "{op}");
    }
}

/// A plain click at each point, with the current tool.
fn clicks(app: &mut PhotocraftApp, points: &[[f64; 2]]) {
    for at in points {
        testing::click(app, *at, Modifiers::NONE);
    }
}

/// The paths the canvas draws for the active document.
fn overlay(app: &PhotocraftApp) -> Vec<(PathRef, EditPath, AnchorsShown)> {
    overlay_paths(app, app.session.active().unwrap().doc.id)
}

/// How the first overlay entry `which` accepts is drawn; `None` when the overlay holds none.
fn anchors_of(app: &PhotocraftApp, which: impl Fn(PathRef) -> bool) -> Option<AnchorsShown> {
    overlay(app).into_iter().find(|(r, ..)| which(*r)).map(|(.., a)| a)
}

fn work_anchors(app: &PhotocraftApp) -> Option<AnchorsShown> {
    anchors_of(app, |r| r == PathRef::Work)
}

fn saved_anchors(app: &PhotocraftApp) -> Option<AnchorsShown> {
    anchors_of(app, |r| matches!(r, PathRef::Saved(_)))
}

/// Whether the draft changes what the overlay holds for document `index`.
fn draft_drawn(app: &mut PhotocraftApp, index: usize) -> bool {
    let doc = app.session.documents()[index].doc.id;
    let with = overlay_paths(app, doc);
    let pen = app.ui.pen.take();
    let without = overlay_paths(app, doc);
    app.ui.pen = pen;
    with != without
}

#[test]
fn all_shape_tools_and_open_pen_use_the_complete_stroke_defaults() {
    for tool in [Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Line, Tool::CustomShape] {
        let mut a = app();
        a.ui.tool_options.stroke_width = 5.0;
        a.ui.tool_options.shape_stroke = crate::shape_stroke_ui::StrokeOptions {
            cap: photocraft_doc::LineCap::Square,
            join: photocraft_doc::LineJoin::Bevel,
            dashes: vec![4.0, 2.0, 1.0, 3.0],
            dash_offset: -0.5,
            opacity: 0.4,
            ..Default::default()
        };
        let history = a.session.active().unwrap().history.past_len();
        shape_drag(&mut a, tool, [20.0, 20.0], [120.0, 100.0]);
        let info = a.run("shape.info", json!({})).unwrap();
        assert_eq!(info["stroke"]["dashes"], json!([4.0, 2.0, 1.0, 3.0]));
        assert_eq!(info["stroke"]["cap"], "square");
        assert_eq!(info["stroke"]["width"], 5.0);
        assert_eq!(a.session.active().unwrap().history.past_len(), history + 1);
        a.run("edit.undo", json!({})).unwrap();
        a.run("edit.redo", json!({})).unwrap();
        assert_eq!(a.run("shape.info", json!({})).unwrap(), info);
        a.ui.tool_options.vector_mode = VectorMode::Shape;
        a.ui.pen = Some(PenPath::corners(&[[30.0, 40.0], [80.0, 40.0]], capture_pen_owner(&a)));
        pen_commit(&mut a, false);
        let open = a.run("shape.info", json!({})).unwrap();
        assert_eq!(open["stroke"]["dashes"], info["stroke"]["dashes"]);
        assert_eq!(open["path"]["subpaths"][0]["closed"], false);
    }
}

#[test]
fn freeform_straight_segments_keep_the_captured_saved_path_and_reject_a_foreign_document() {
    let mut app = app();
    app.ui.tool = Tool::FreeformPen;
    let seed = json!({"subpaths":[{"knots":[[20,20],[120,20]]}]});
    for name in ["work", "Saved drawing"] {
        app.run("path.set", json!({"name": name, "path": seed})).unwrap();
    }
    app.ui.selected_path = Some("Saved drawing".into());
    let owner = capture_pen_owner(&app).unwrap();
    let original = app.session.active().unwrap().doc.clone();
    let n = app.session.active().unwrap().history.past_len();
    let points = [[20.0, 60.0, 1.0], [60.0, 60.0, 1.0], [100.0, 80.0, 1.0], [140.0, 80.0, 1.0]];
    app.ui.selected_path = Some("work".into());
    app.ui.tool_options.vector_mode = VectorMode::Shape;
    freeform_finish_owned(&mut app, &points, &[1, 3], owner);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.work_path, original.work_path);
    let saved = &st.doc.paths.iter().find(|p| p.name == "Saved drawing").unwrap().path;
    assert_eq!(saved.subpaths[0], original.paths[0].path.subpaths[0]);
    assert_eq!(saved.subpaths.len(), 2);
    let knots = &saved.subpaths[1].knots;
    assert_eq!(knots.len(), 4);
    for (a, b) in [(0, 1), (2, 3)] {
        assert_eq!(knots[a].out_ctrl, knots[a].anchor, "straight segment starts at its anchor");
        assert_eq!(knots[b].in_ctrl, knots[b].anchor, "straight segment ends at its anchor");
    }
    assert_eq!(st.history.past_len(), n + 1);
    assert!(app.ui.pen.is_none());
    app.ui.tool_options.vector_mode = VectorMode::Path;
    let owner = capture_pen_owner(&app).unwrap();
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let (source, n) = (app.session.active().unwrap().doc.clone(), app.session.active().unwrap().history.past_len());
    freeform_finish_owned(&mut app, &points, &[1, 3], owner);
    assert!(Arc::ptr_eq(&source, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), n);
}

#[test]
fn paths_context_menu_uses_the_clicked_row_and_dispatches_commands() {
    let mut app = app();
    let path = json!({"subpaths": [{"closed": true, "knots": [
        {"anchor": [10, 10], "in": [10, 10], "out": [10, 10]},
        {"anchor": [80, 10], "in": [80, 10], "out": [80, 10]},
        {"anchor": [80, 80], "in": [80, 80], "out": [80, 80]}
    ]}]});
    app.run("path.set", json!({"name": "First", "path": path})).unwrap();
    app.run("path.set", json!({"name": "Second", "path": path})).unwrap();
    let (id, params) = command_params(&app, PathsCommands::MakeSelection, &PathRef::Saved(0)).expect("Make Selection applies to a saved path");
    assert_eq!(id, "path.toSelection");
    assert_eq!(params["name"], "First");
    app.run(id, params).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_some());
    assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("path.toSelection"));

    app.run("path.set", json!({"name": "First copy 1", "path": path})).unwrap();
    let (id, duplicate) = command_params(&app, PathsCommands::DuplicatePath, &PathRef::Saved(0)).expect("Duplicate applies to a saved path");
    assert_eq!(id, "path.set");
    assert_eq!(duplicate["name"], "First copy 2", "duplicate must not overwrite an existing path");
}

#[test]
fn failed_freeform_straight_segment_commit_keeps_its_owned_fitted_draft() {
    let mut app = app();
    app.ui.tool = Tool::FreeformPen;
    let owner = capture_pen_owner(&app).unwrap();
    let (source, n) = (app.session.active().unwrap().doc.clone(), app.session.active().unwrap().history.past_len());
    let points = [[20.0, 60.0, 1.0], [1e40, 60.0, 1.0], [140.0, 80.0, 1.0]];
    freeform_finish_owned(&mut app, &points, &[1, 2], owner.clone());
    let pen = app.ui.pen.as_ref().expect("invalid fitted geometry must remain available after its failed commit");
    assert!(pen.commit_failed);
    assert_eq!(pen.owner.as_ref(), Some(&owner));
    assert!(owner.valid(&app));
    assert_eq!(pen.draft.knots.len(), 3);
    assert_eq!(pen.triples()[1], [[1e40, 60.0]; 3]);
    assert!(Arc::ptr_eq(&source, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), n);
}

/// #6552: a shape layer's "<Layer> Shape Path" row offers Delete Path, and it deletes the
/// shape layer, as Photoshop's Delete Path does (measured: one "Delete Layer" step).
#[test]
fn shape_path_row_delete_path_deletes_the_shape_layer() {
    let mut app = app();
    app.run("shape.create", json!({"kind": "ellipse", "rect": [20, 20, 100, 70]})).unwrap();
    let st = app.session.active().unwrap();
    let (layers, shape) = (st.doc.layers.len(), st.active_layer.unwrap());
    let (id, params) = command_params(&app, PathsCommands::DeletePath, &PathRef::Layer(shape.0)).expect("Delete Path is offered");
    assert_eq!(id, "layer.delete");
    app.run(id, params).unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.layers.len(), layers - 1);
    assert!(doc.layer(shape).is_none(), "the shape layer is gone");
    app.run("edit.undo", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.layer(shape).is_some(), "one undo step brings it back");
}

/// #6238: renaming a saved path renames it in place; renaming the Work Path saves it under the name typed.
#[test]
fn double_clicking_a_path_name_renames_it_in_place() {
    let mut app = app();
    let path = json!({"subpaths": [{"closed": true, "knots": [
        {"anchor": [10, 10], "in": [10, 10], "out": [10, 10]},
        {"anchor": [80, 10], "in": [80, 10], "out": [80, 10]},
        {"anchor": [80, 80], "in": [80, 80], "out": [80, 80]}
    ]}]});
    app.run("path.set", json!({"name": "First", "path": path})).unwrap();
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    let names = |app: &PhotocraftApp| app.session.active().unwrap().doc.paths.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
    apply(&mut app, PathsEvent::rename(PathRef::Saved(0), "Outline"));
    assert_eq!(names(&app), ["Outline"]);
    assert!(app.session.active().unwrap().doc.work_path.is_some(), "renaming a saved path leaves the work path alone");
    apply(&mut app, PathsEvent::rename(PathRef::Work, "Circle"));
    assert_eq!(names(&app), ["Outline", "Circle"]);
    assert!(app.session.active().unwrap().doc.work_path.is_none());
    assert_eq!(app.ui.selected_path.as_deref(), Some("Circle"));
}

#[test]
fn pen_keeps_anchors_of_a_finished_path() {
    // #6884: a path finished with Enter, or by closing it, lost its anchor squares while the
    // Pen stayed active, so it looked gone and could not be edited.
    let mut app = app();
    app.ui.tool = Tool::Pen;
    app.ui.pen = Some(PenPath::corners(&[[10.0, 10.0], [80.0, 10.0], [80.0, 80.0]], capture_pen_owner(&app)));
    pen_commit(&mut app, true);
    assert!(app.ui.pen.is_none());
    assert!(app.session.active().unwrap().doc.work_path.is_some());
    for tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen, Tool::PathSelection] {
        app.ui.tool = tool;
        assert_eq!(work_anchors(&app), Some(AnchorsShown::All), "{tool:?}");
    }
    for tool in [Tool::Brush, Tool::Move] {
        app.ui.tool = tool;
        assert_eq!(work_anchors(&app), Some(AnchorsShown::Hidden), "{tool:?}: the outline is drawn with any tool, without anchors");
    }
}

#[test]
fn an_unselected_work_path_is_not_drawn_over_the_canvas() {
    // #6551: Ctrl-click Load as Selection leaves the Paths panel's selection alone, yet the Work
    // Path stayed drawn over the new selection's edge. Photoshop draws it only while selected
    // in the Paths panel (or a path tool is active).
    let mut app = app();
    let path = json!({"subpaths": [{"closed": true, "knots": [
        {"anchor": [20, 20], "in": [20, 20], "out": [20, 20]},
        {"anchor": [90, 20], "in": [90, 20], "out": [90, 20]},
        {"anchor": [90, 90], "in": [90, 90], "out": [90, 90]}
    ]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    app.run("path.toSelection", json!({"name": "work", "mode": "replace"})).unwrap();
    assert!(app.ui.selected_path.is_none());
    for tool in [Tool::Move, Tool::Brush, Tool::Rectangle, Tool::Zoom] {
        app.ui.tool = tool;
        assert_eq!(work_anchors(&app), None, "{tool:?}: an unselected Work Path stays off the canvas");
    }
    // Selected in the Paths panel: drawn with any tool.
    app.ui.selected_path = Some("work".into());
    for tool in [Tool::Move, Tool::Brush] {
        app.ui.tool = tool;
        assert!(work_anchors(&app).is_some(), "{tool:?}: the selected Work Path is drawn");
    }
    // Another path selected: the Work Path is not the target path.
    app.ui.selected_path = Some("Saved".into());
    app.ui.tool = Tool::Move;
    assert_eq!(work_anchors(&app), None);
    // A path tool shows the Work Path it edits even when the panel selection was cleared.
    app.ui.selected_path = None;
    for tool in [Tool::Pen, Tool::PathSelection] {
        app.ui.tool = tool;
        assert!(work_anchors(&app).is_some(), "{tool:?}: path tools draw the Work Path");
    }
}

/// #9114: the active type layer's Type Path shows with the Pen, whose Auto Add/Delete edits it.
#[test]
fn the_type_path_of_type_on_a_path_is_drawn_with_the_pen() {
    let mut app = app();
    app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[100, 20], [100, 180]]}]}})).unwrap();
    app.run("type.create", json!({"x": 20, "y": 70, "text": "On a path", "size": 24})).unwrap();
    app.run("type.attachToPath", json!({})).unwrap();
    app.run("path.delete", json!({"name": "work"})).unwrap();
    app.ui.tool = Tool::Pen;
    assert!(overlay(&app).iter().any(|(r, ..)| matches!(r, PathRef::TypePath(_))), "the Type Path is drawn");
}

#[test]
fn show_target_path_and_extras_hide_the_path_outlines() {
    // #1119: View › Show › Target Path only flipped its checkmark; the canvas kept drawing the
    // work path and the active shape's path.
    let mut app = app();
    let ctx = app.ctx().clone();
    let path = json!({"subpaths": [{"closed": true, "knots": [
        {"anchor": [10, 10], "in": [10, 10], "out": [10, 10]},
        {"anchor": [80, 10], "in": [80, 10], "out": [80, 10]},
        {"anchor": [80, 80], "in": [80, 80], "out": [80, 80]}
    ]}]});
    app.run("path.set", json!({"path": path})).unwrap();
    assert!(app.session.active().unwrap().doc.work_path.is_some());
    shape_drag(&mut app, Tool::Rectangle, [100.0, 100.0], [160.0, 150.0]);
    let toggle = |app: &mut PhotocraftApp, id: &str| crate::menus::invoke(app, &ctx, id, Value::Null).unwrap();
    for tool in [Tool::Pen, Tool::PathSelection, Tool::DirectSelection, Tool::Rectangle] {
        app.ui.tool = tool;
        assert!(app.ui.view.show.target_path && app.ui.view.extras);
        assert!(!overlay(&app).is_empty(), "{tool:?}: the paths are drawn by default");
        toggle(&mut app, "view.show.targetPath");
        assert!(!app.ui.view.show.target_path);
        assert!(overlay(&app).is_empty(), "{tool:?}: Target Path off");
        toggle(&mut app, "view.show.targetPath");
        toggle(&mut app, "view.extras");
        assert!(!app.ui.view.extras);
        assert!(overlay(&app).is_empty(), "{tool:?}: Extras off");
        toggle(&mut app, "view.extras");
        assert!(!overlay(&app).is_empty(), "{tool:?}: shown again");
    }
    // The Pen's path in progress is a gesture, not the target path: it stays visible.
    app.ui.tool = Tool::Pen;
    app.ui.pen = Some(PenPath::corners(&[[20.0, 20.0], [60.0, 40.0]], capture_pen_owner(&app)));
    toggle(&mut app, "view.show.targetPath");
    assert!(draft_drawn(&mut app, 0), "the pen path in progress is still drawn");
}

#[test]
fn making_a_selection_from_the_pen_path_deselects_and_hides_it() {
    // #6079: after ⌘↩ the work path's outline stayed on the canvas; Photoshop deselects it.
    let mut app = app();
    app.ui.tool = Tool::Pen;
    app.ui.pen = Some(PenPath::corners(&[[20.0, 20.0], [120.0, 20.0], [120.0, 80.0]], capture_pen_owner(&app)));
    path_to_selection(&mut app, json!({})).unwrap();
    let st = app.session.active().unwrap();
    assert!(st.doc.selection.is_some(), "the selection is made");
    assert!(st.doc.work_path.is_some(), "the work path is kept");
    assert_eq!(app.ui.selected_path, None, "and deselected in the Paths panel");
    assert!(overlay(&app).is_empty(), "so the canvas no longer draws it");
    assert!(!delete_key_deletes_work_path(&app), "nor deletes it");
    // Undo removes the selection; the path stays.
    app.run("edit.undo", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.work_path.is_some());
    assert!(app.session.active().unwrap().doc.selection.is_none());
    assert!(overlay(&app).is_empty(), "still hidden");
    // Auto Add/Delete makes the selected saved path the only target (#12762), so the plain Pen overlay is the one that shows
    // both paths.
    app.ui.tool_options.pen_auto_add_delete = false;
    // Selecting a saved path in the Paths panel doesn't show it; only the Work Path does.
    let saved = json!({"subpaths": [{"closed": true, "knots": [{"anchor": [60, 60]}, {"anchor": [90, 60]}, {"anchor": [90, 90]}]}]});
    app.run("path.set", json!({"name": "Saved", "path": saved})).unwrap();
    app.ui.selected_path = Some("Saved".into());
    // The selected saved path is drawn (#6550), the hidden Work Path isn't.
    assert!(!work_path_shown(&app) && saved_anchors(&app).is_some(), "a saved path keeps the Work Path hidden");
    assert_eq!(work_anchors(&app), None);
    let hidden = app.ui.hidden_work_path.take();
    assert!(work_anchors(&app).is_some(), "the Work Path adds its own outline when shown");
    app.ui.hidden_work_path = hidden;
    app.ui.selected_path = None;
    // Another Work Path made any other way (Make Work Path, `path.set` over the control
    // channel…) is a different path: it shows without being selected.
    let other = json!({"subpaths": [{"closed": true, "knots": [{"anchor": [5, 5]}, {"anchor": [50, 5]}, {"anchor": [50, 50]}]}]});
    app.run("path.set", json!({"name": "work", "path": other})).unwrap();
    assert_eq!(app.ui.selected_path, None);
    assert!(work_path_shown(&app) && work_anchors(&app).is_some(), "a changed Work Path is shown");
    // Selecting it in the Paths panel shows it again.
    app.ui.selected_path = Some("work".into());
    assert!(work_anchors(&app).is_some());
    // A new path drawn after the deselect replaces the work path and is shown.
    path_to_selection(&mut app, json!({})).unwrap();
    assert!(overlay(&app).is_empty());
    app.ui.pen = Some(PenPath::corners(&[[10.0, 10.0], [40.0, 10.0]], capture_pen_owner(&app)));
    pen_commit(&mut app, false);
    assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
    assert_eq!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths.len(), 1, "replaced, not combined");
    assert!(work_anchors(&app).is_some());
    // A shape layer's path stays the active path: layer targeting is unchanged.
    app.ui.tool_options.vector_mode = VectorMode::Shape;
    app.ui.pen = Some(PenPath::corners(&[[20.0, 20.0], [120.0, 20.0], [120.0, 80.0]], capture_pen_owner(&app)));
    path_to_selection(&mut app, json!({})).unwrap();
    assert_eq!(app.ui.selected_path.as_deref(), Some("layer"));
    assert!(active_shape_path(&app).is_some());
}

#[test]
fn the_saved_path_selected_in_the_paths_panel_is_drawn() {
    // #6550: selecting a saved path in the Paths panel drew nothing on the canvas.
    let mut app = app();
    let tri = |o: f64| {
        json!({"subpaths": [{"closed": true, "knots": [
            {"anchor": [20.0 + o, 20], "in": [20.0 + o, 20], "out": [20.0 + o, 20]},
            {"anchor": [90.0 + o, 20], "in": [90.0 + o, 20], "out": [90.0 + o, 20]},
            {"anchor": [90.0 + o, 90], "in": [90.0 + o, 90], "out": [90.0 + o, 90]}
        ]}]})
    };
    app.run("path.set", json!({"name": "Path 1", "path": tri(0.0)})).unwrap();
    app.run("path.set", json!({"name": "Path 2", "path": tri(10.0)})).unwrap();
    assert!(app.session.active().unwrap().doc.work_path.is_none());
    for tool in [Tool::Move, Tool::Brush, Tool::Pen, Tool::PathSelection, Tool::DirectSelection] {
        app.ui.tool = tool;
        app.ui.selected_path = None;
        assert!(overlay(&app).is_empty(), "{tool:?}: nothing selected, nothing drawn");
        app.ui.selected_path = Some("Path 1".into());
        assert_eq!(selected_saved_path(&app).map(|p| p.subpaths[0].knots[0].anchor.x), Some(20.0));
        assert!(saved_anchors(&app).is_some(), "{tool:?}: the selected saved path is drawn");
        app.ui.selected_path = Some("Path 2".into());
        assert_eq!(selected_saved_path(&app).map(|p| p.subpaths[0].knots[0].anchor.x), Some(30.0));
        app.ui.selected_path = Some("Gone".into());
        assert!(overlay(&app).is_empty(), "{tool:?}: a missing path draws nothing");
        app.ui.selected_path = Some("Path 1".into());
        app.ui.view.show.target_path = false;
        assert!(overlay(&app).is_empty(), "{tool:?}: View > Show > Target Path off");
        app.ui.view.show.target_path = true;
    }
    // Path Selection (and the Pen tools, as for the work path) add anchor squares.
    app.ui.selected_path = Some("Path 1".into());
    app.ui.tool = Tool::Move;
    assert_eq!(saved_anchors(&app), Some(AnchorsShown::Hidden));
    for tool in [Tool::PathSelection, Tool::Pen] {
        app.ui.tool = tool;
        assert_eq!(saved_anchors(&app), Some(AnchorsShown::All), "{tool:?}");
    }
}

/// #9453: in Path mode a Shape-tool drag adds its outline to the work path, one subpath per
/// drag, and makes no layer, as Photoshop's Path tool mode does. Shape stays the default.
#[test]
fn path_mode_shape_drags_add_to_the_work_path() {
    let mut app = app();
    assert_eq!(app.ui.tool_options.shape_mode, VectorMode::Shape, "Shape by default, as in Photoshop");
    let layers = app.session.active().unwrap().doc.layers.len();
    app.ui.tool_options.shape_mode = VectorMode::Path;
    shape_drag(&mut app, Tool::EllipseShape, [20.0, 20.0], [70.0, 50.0]);
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.layers.len(), layers, "no layer");
    let work = doc.work_path.as_ref().expect("a work path");
    assert_eq!(work.subpaths.len(), 1);
    assert!(work.subpaths[0].closed);
    let b = work.control_bounds().unwrap();
    assert!((b.0 - 20.0).abs() < 0.5 && (b.1 - 20.0).abs() < 0.5 && (b.2 - 70.0).abs() < 0.5 && (b.3 - 50.0).abs() < 0.5, "{b:?}");
    assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
    shape_drag(&mut app, Tool::Rectangle, [100.0, 100.0], [140.0, 130.0]);
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.work_path.as_ref().unwrap().subpaths.len(), 2, "a second drag adds a subpath");
    assert_eq!(doc.layers.len(), layers);
    // Back in Shape mode a drag makes a shape layer again.
    app.ui.tool_options.shape_mode = VectorMode::Shape;
    shape_drag(&mut app, Tool::Rectangle, [10.0, 150.0], [60.0, 180.0]);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1);
}

/// CodeRabbit on #9456: Path mode adds a Custom Shape to the work path as one shape component,
/// its hole joined to it, as Photoshop does (the Ring's hole no longer turns into a combined
/// subpath, nor stays a subtracted one that would cut the work path under it).
#[test]
fn path_mode_custom_shape_joins_its_hole_to_it() {
    use photocraft_doc::PathOp;
    let mut app = app();
    app.ui.tool_options.shape_mode = VectorMode::Path;
    shape_drag(&mut app, Tool::Rectangle, [100.0, 100.0], [140.0, 130.0]);
    app.ui.presets_ui.custom_shape = "Ring".into();
    shape_drag(&mut app, Tool::CustomShape, [20.0, 20.0], [70.0, 70.0]);
    let ops: Vec<PathOp> = app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths.iter().map(|s| s.op).collect();
    // The ring takes Path mode's default operation, Exclude Overlapping Shapes (#13235).
    assert_eq!(ops, [PathOp::Combine, PathOp::Exclude, PathOp::Join], "the rectangle, the ring and its hole");
    assert_eq!(work_covered(&app, 45, 45), 0.0, "the ring's hole");
    assert_eq!(work_covered(&app, 22, 45), 1.0, "the ring");
}

/// CodeRabbit on #9456, checked in Photoshop 2025: a Ring drawn in Path mode over a rectangle
/// cuts its hole in the ring only; the rectangle stays whole where the hole overlaps it.
#[test]
fn path_mode_custom_shape_hole_leaves_the_work_path_under_it() {
    let mut app = app();
    app.ui.tool_options.shape_mode = VectorMode::Path;
    shape_drag(&mut app, Tool::Rectangle, [20.0, 20.0], [120.0, 120.0]);
    app.ui.presets_ui.custom_shape = "Ring".into();
    shape_drag(&mut app, Tool::CustomShape, [80.0, 80.0], [180.0, 180.0]);
    assert_eq!(work_covered(&app, 50, 50), 1.0, "the rectangle");
    assert_eq!(work_covered(&app, 110, 110), 1.0, "the rectangle under the ring's hole");
    assert_eq!(work_covered(&app, 130, 130), 0.0, "the ring's hole");
    assert_eq!(work_covered(&app, 170, 130), 1.0, "the ring");
}

/// Every built-in custom shape drawn alone in Path mode covers what the shape does, joined into
/// one component or, where that would change its area (the Crescent Moon's cut reaches past
/// the disc), kept as it is.
#[test]
fn path_mode_custom_shapes_keep_their_area() {
    let mut app = app();
    app.ui.tool_options.shape_mode = VectorMode::Path;
    let rect = photocraft_geom::Rect::from_xywh(0, 0, 200, 200);
    for g in photocraft_engine::presets::shapes::builtin() {
        for shape in g.items {
            app.run("path.delete", json!({"name": "work"})).ok();
            app.ui.presets_ui.custom_shape = shape.name.clone();
            shape_drag(&mut app, Tool::CustomShape, [10.0, 10.0], [190.0, 190.0]);
            let work = app.session.active().unwrap().doc.work_path.clone().expect("a work path");
            let want = photocraft_engine::presets::shapes::fit(&shape.path, [10.0, 10.0, 180.0, 180.0], false);
            let (a, b) = (photocraft_vector::path_coverage(&work, rect), photocraft_vector::path_coverage(&want, rect));
            let off = a.iter().zip(&b).filter(|(a, b)| (*a - *b).abs() > 0.5).count();
            assert_eq!(off, 0, "{}: {off} pixels differ", shape.name);
        }
    }
}

/// Coverage of the work path at pixel `(x, y)`.
fn work_covered(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    let path = app.session.active().unwrap().doc.work_path.clone().expect("a work path");
    photocraft_vector::fill_rasterizer(&path, photocraft_vector::DEFAULT_TOLERANCE).render(photocraft_geom::Rect::from_xywh(x, y, 1, 1))[0]
}

/// CodeRabbit on #9456: a custom shape that starts the work path brings its fill rule, so an
/// even-odd star keeps its empty centre.
#[test]
fn path_mode_custom_shape_starts_the_work_path_with_its_fill_rule() {
    let mut app = app();
    app.ui.tool_options.shape_mode = VectorMode::Path;
    // A pentagram drawn in one stroke: even-odd leaves the pentagon in the middle empty.
    let star: Vec<[f64; 2]> = (0..5)
        .map(|k| {
            let a = std::f64::consts::TAU * (k * 2) as f64 / 5.0 - std::f64::consts::FRAC_PI_2;
            [50.0 + 50.0 * a.cos(), 50.0 + 50.0 * a.sin()]
        })
        .collect();
    let r = app.run("shape.presets.new", json!({"name": "Even-Odd Star", "path": {"fillRule": "evenodd", "subpaths": [{"knots": star}]}}));
    assert!(r.is_ok(), "{r:?}");
    app.ui.presets_ui.custom_shape = "Even-Odd Star".into();
    shape_drag(&mut app, Tool::CustomShape, [20.0, 20.0], [120.0, 120.0]);
    let work = app.session.active().unwrap().doc.work_path.clone().expect("a work path");
    assert_eq!(work.fill_rule, photocraft_doc::vector::FillRule::EvenOdd);
    let b = work.control_bounds().unwrap();
    let (cx, cy) = (((b.0 + b.2) / 2.0) as i32, ((b.1 + b.3) / 2.0) as i32);
    assert_eq!(work_covered(&app, cx, cy), 0.0, "the star's centre stays empty");
}

/// #9486: a 1 px Line-tool drag fills one pixel row or column, opaque, as Photoshop's Align
/// Edges draws it (a drag at y 23.2 or x 33.4 rounds its ends to 23 or 33 and fills row 22 or
/// column 32), instead of half of two.
#[test]
fn one_pixel_line_fills_one_pixel_row() {
    for ((x0, y0), (x1, y1), want) in
        [((10.4, 23.2), (70.3, 23.2), photocraft_geom::Rect::new(10, 22, 70, 23)), ((33.4, 8.6), (33.4, 50.2), photocraft_geom::Rect::new(32, 9, 33, 50))]
    {
        let mut app = app();
        app.ui.tool = Tool::Line;
        app.ui.tool_options.line_weight = 1.0;
        let none = egui::Modifiers::NONE;
        tool_event(&mut app, ToolEvent::Down { x: x0, y: y0, pressure: 1.0 }, none);
        tool_event(&mut app, ToolEvent::Move { x: x1, y: y1, pressure: 1.0 }, none);
        tool_event(&mut app, ToolEvent::Up { x: x1, y: y1 }, none);
        let st = app.session.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
        assert!(matches!(l.content, LayerContent::Shape(_)), "a shape layer");
        let s = l.surface().unwrap();
        assert_eq!(s.content_bounds(), want, "one pixel across");
        let mid = s.read_region(photocraft_geom::Rect::new(
            (want.x0 + want.x1) / 2,
            (want.y0 + want.y1) / 2,
            (want.x0 + want.x1) / 2 + 1,
            (want.y0 + want.y1) / 2 + 1,
        ));
        assert!(mid[3] > 0.999, "opaque: {mid:?}");
    }
}

/// CodeRabbit on #9506: a straight line's width changes smoothly with its weight under Align
/// Edges: the weight snaps to the nearest whole pixel (1.99 and 2.01 both fill 2 rows, not 1
/// and 3), and its sides stay on pixel edges.
#[test]
fn align_edges_snaps_a_fractional_weight_to_the_nearest_pixel() {
    for (weight, want) in [(1.0, 1.0), (1.4, 1.0), (1.99, 2.0), (2.0, 2.0), (2.01, 2.0), (2.6, 3.0), (3.0, 3.0)] {
        for (a, b, i) in [([10.0, 23.0], [70.0, 23.0], 1), ([33.0, 9.0], [33.0, 50.0], 0)] {
            let (from, to, w) = super::shapes::align_line_edges(a, b, weight, false).expect("a line");
            assert_eq!(w, want, "weight {weight}");
            let (lo, hi) = (from[i] - w / 2.0, from[i] + w / 2.0);
            assert!(lo.fract() == 0.0 && hi.fract() == 0.0 && from[i] == to[i], "weight {weight}: sides {lo}..{hi}");
        }
    }
}

/// CodeRabbit on #9506: a ⇧-drag line (a 45° geometry) stays at 45° when its ends snap to whole
/// pixels, even a short one (rounding each end on its own turned 10.1,10.49 → 11.3,11.69 into a 1 × 2 step).
#[test]
fn a_short_shift_drag_line_keeps_its_45_degrees() {
    let line = |app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], shift: bool| {
        app.ui.tool = Tool::Line;
        let geometry = ShapeGeometry::Line { from: (from[0], from[1]).into(), to: (to[0], to[1]).into(), weight: 1.0 };
        finish_drag(app, ShapeKind::Line, &geometry, Mods { shift, ..Mods::default() });
        let info = app.run("shape.info", json!({})).unwrap();
        let p = |k: &str| [info["live"][k][0].as_f64().unwrap(), info["live"][k][1].as_f64().unwrap()];
        (p("from"), p("to"))
    };
    let cases: [([f64; 2], [f64; 2]); 4] =
        [([10.1, 10.49], [11.3, 11.69]), ([20.4, 5.6], [17.1, 8.9]), ([3.3, 40.2], [9.8, 33.1]), ([50.5, 50.5], [44.2, 44.0])];
    for (start, end) in cases {
        // The tool's ⇧ snap puts the end on the 45° ray through the start at the mean extent.
        let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
        let side = (dx.abs() + dy.abs()) / 2.0;
        let snapped = [start[0] + side * dx.signum(), start[1] + side * dy.signum()];
        let mut a = app();
        let (from, to) = line(&mut a, start, snapped, true);
        assert!(from.iter().chain(&to).all(|v| v.fract() == 0.0), "whole pixels: {from:?} {to:?}");
        let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
        assert!(dx != 0.0 && dx.abs() == dy.abs(), "{start:?} → {end:?}: still 45°: {from:?} {to:?}");
    }
    // Without the snap each end still rounds on its own.
    let mut a = app();
    let (from, to) = line(&mut a, [10.1, 10.49], [11.3, 11.89], false);
    assert_eq!((from, to), ([10.0, 10.0], [11.0, 12.0]));
}

#[test]
fn shape_preview_is_the_committed_path() {
    let mut app = app();
    app.ui.tool_options.corner_radius = 8.0;
    let none = Modifiers::NONE;
    // The geometry the tool reports while the drag runs, as the canvas previews it.
    let running = |app: &PhotocraftApp| host::shape_preview(app).map(|(kind, geometry)| (kind, geometry.clone()));
    for (row, tool) in [Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Line].into_iter().enumerate() {
        app.ui.tool = tool;
        // Each shape is drawn on its own row: the scale box of the shape before it, still selected, takes a press on its handles.
        let y = 10.0 + 38.0 * row as f64;
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y, pressure: 1.0 }, none);
        tool_event(&mut app, ToolEvent::Move { x: 60.0, y: y + 20.0, pressure: 1.0 }, none);
        let (kind, geometry) = running(&app).unwrap_or_else(|| panic!("{tool:?}: the drag reports a shape"));
        let shown = shape_preview(&app, kind, &geometry).unwrap_or_else(|| panic!("{tool:?}: a preview")).path;
        tool_event(&mut app, ToolEvent::Up { x: 60.0, y: y + 20.0 }, none);
        let st = app.session.active().unwrap();
        let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("{tool:?}: no shape layer") };
        assert_eq!(sh.path, shown, "{tool:?}");
    }
    app.ui.tool = Tool::Rectangle;
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Move { x: 10.2, y: 40.0, pressure: 1.0 }, none);
    if let Some((kind, geometry)) = running(&app) {
        assert!(shape_preview(&app, kind, &geometry).is_none(), "too thin to draw");
    }
    tool_event(&mut app, ToolEvent::Up { x: 10.2, y: 40.0 }, none);
}

#[test]
fn pen_builds_and_closes_a_work_path() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    clicks(&mut app, &[[20.0, 20.0], [120.0, 20.0]]);
    let none = Modifiers::NONE;
    testing::press(&mut app, [120.0, 120.0], none);
    testing::move_to(&mut app, [140.0, 140.0], none); // smooth knot
    testing::release(&mut app, [140.0, 140.0], none);
    clicks(&mut app, &[[20.5, 20.5]]); // click the first anchor: close
    assert!(app.ui.pen.is_none());
    let wp = app.session.active().unwrap().doc.work_path.clone().unwrap();
    assert!(wp.subpaths[0].closed && wp.subpaths[0].knots.len() == 3);
    assert!(wp.subpaths[0].knots[2].smooth);
    // Path Selection moves it.
    path_selection_finish(&mut app, [50.0, 50.0], [60.0, 55.0]);
    let wp = app.session.active().unwrap().doc.work_path.clone().unwrap();
    assert_eq!((wp.subpaths[0].knots[0].anchor.x, wp.subpaths[0].knots[0].anchor.y), (30.0, 25.0));
}

/// #5373: an unfinished path stays with its own document through same-frame switches.
#[test]
fn an_unclosed_path_stays_with_its_document() {
    // Both pens claim the document at their first anchor.
    let open_path = |app: &mut PhotocraftApp| clicks(app, &[[20.0, 20.0], [120.0, 60.0]]);
    let work = |app: &PhotocraftApp, i: usize| app.session.documents()[i].doc.work_path.clone();
    // File › New commits the path to its original document.
    for tool in [Tool::Pen, Tool::CurvaturePen] {
        let mut app = app();
        app.ui.tool = tool;
        open_path(&mut app);
        assert!(app.ui.pen.is_some() && app.ui.pen_doc.is_some(), "{tool:?}: claimed by its document");
        app.run("file.new", json!({"width": 300, "height": 150})).unwrap();
        assert!(app.ui.pen.is_none() && app.ui.pen_doc.is_none(), "{tool:?}: the first document's path is gone");
        assert!(work(&app, 0).is_some(), "{tool:?}: the original keeps its Work Path");
        assert_eq!(work(&app, 1), None, "{tool:?}: the new document is blank");
    }
    // Two documents; the path below is drawn on the second (active) one.
    let mut app = app();
    app.ui.tool = Tool::Pen;
    app.run("file.new", json!({"width": 300, "height": 150})).unwrap();
    open_path(&mut app);
    assert!(app.session.set_active(0));
    pen_commit(&mut app, false);
    assert!(app.ui.pen.is_none());
    assert!(work(&app, 0).is_none(), "the active document stays untouched");
    assert!(work(&app, 1).is_some(), "the owner keeps its Work Path");
    // Staying on its own document, the path still commits there.
    open_path(&mut app);
    pen_commit(&mut app, false);
    assert!(work(&app, 0).is_some(), "it commits to the document it was drawn on");
    assert!(work(&app, 1).is_some());
    for act in [0, 1, 2] {
        open_path(&mut app);
        assert!(app.session.set_active(1));
        match act {
            0 | 1 => {
                app.ui.tool = if act == 0 { Tool::Pen } else { Tool::CurvaturePen };
                // Away from any anchor: an earlier act's first click left a one-anchor Work Path in this document
                // (#12890), and a press on its anchor would be the anchor tools' delete.
                clicks(&mut app, &[[150.0 + 20.0 * act as f64, 120.0]]);
            }
            _ => assert!(!remove_last(&mut app), "nothing of this document's to undo"),
        }
        let knots = app.ui.pen.as_ref().map_or(0, |p| p.draft.knots.len());
        assert!(knots <= 1, "act {act}: a new path, not the other document's (knots {knots})");
        // The two drawing entry points start a path of their own here; undo has none to take.
        let expected = if act == 2 { None } else { app.session.active().map(|st| st.doc.id) };
        assert_eq!(
            app.ui.pen_doc,
            expected,
            "act {act}: pen {:?}, status {}",
            app.ui.pen.as_ref().map(|p| (p.draft.knots.len(), p.curvature, p.live_path)),
            app.ui.status
        );
        app.ui.pen = None;
        app.ui.pen_doc = None;
        app.ui.tool = Tool::Pen;
        assert!(app.session.set_active(0));
    }
    open_path(&mut app);
    app.ui.pen = None;
    app.ui.pen_doc = None;
    open_path(&mut app);
    app.ui.direct_selection.target = Some(crate::direct_select::PathRef::Pen);
    app.ui.direct_selection.anchors = vec![[0, 1]];
    app.run("file.new", json!({"width": 120, "height": 90})).unwrap();
    assert_eq!(app.ui.direct_selection, Default::default(), "the anchors went with the path");
}

#[test]
fn pen_overlay_uses_the_view_document_even_during_direct_selection() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    clicks(&mut app, &[[20.0, 20.0], [80.0, 60.0]]);
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    // The draft's document is the first one, the new document the second. A frame with the second one active finishes the
    // stale draft (`cancel_stale_pen`), so the modifiers are set in a frame while the first is active and the second
    // is made active only for the reads.
    for modifiers in [Modifiers::NONE, Modifiers::COMMAND] {
        assert!(app.session.set_active(0));
        testing::modifiers(&mut app, modifiers);
        for active in [0, 1] {
            assert!(app.session.set_active(active));
            for tool in [Tool::Pen, Tool::DirectSelection] {
                app.ui.tool = tool;
                for (index, visible) in [(0, true), (1, false)] {
                    assert_eq!(draft_drawn(&mut app, index), visible, "active {active}, {tool:?}, {modifiers:?}, document {index}");
                }
            }
        }
    }
    assert!(app.session.set_active(0));
    testing::modifiers(&mut app, Modifiers::NONE);
}

#[test]
fn failed_stale_pen_commit_preserves_the_draft_until_retry() {
    for action in ["pen", "curvature", "undo", "commit", "command"] {
        let mut app = app();
        app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[5, 5], [10, 10]]}]}})).unwrap();
        let original = app.session.active().unwrap().doc.work_path.clone().unwrap();
        app.ui.selected_path = Some("work".into());
        app.ui.tool = Tool::Pen;
        // The Pen writes the Work Path as it draws (#12890), so the document has to refuse the path from the first
        // anchor on for this to be the state where every commit attempt is refused.
        app.session.authorize = Some(|id, _| if id == "path.set" { Err(photocraft_engine::EngineError::Other("test rejection".into())) } else { Ok(()) });
        clicks(&mut app, &[[20.0, 20.0], [80.0, 60.0]]);
        let mut draft = app.ui.pen.clone();
        draft.as_mut().unwrap().commit_failed = true;
        let owner = app.ui.pen_doc;
        app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("test rejection".into())));
        // The new document is the one the Paths panel selection belongs to once a pass has seen it. The pass finds the
        // old draft stale and its commit refused, which is the state the actions below meet.
        app.sync_views();
        app.ui.tool_options.vector_mode = VectorMode::Shape;
        app.ui.selected_path = Some("foreign named path".into());
        match action {
            "pen" => testing::press(&mut app, [150.0, 150.0], Modifiers::NONE),
            "curvature" => {
                app.ui.tool = Tool::CurvaturePen;
                testing::press(&mut app, [150.0, 150.0], Modifiers::NONE);
            }
            "undo" => assert!(!remove_last(&mut app)),
            "commit" => {
                pen_commit(&mut app, true);
            }
            _ => {
                assert!(app.run("file.close", json!({})).is_err());
            }
        }
        assert_eq!(app.ui.pen, draft, "{action}: failed save retains all anchors");
        assert_eq!(app.ui.pen_doc, owner);
        assert_eq!(app.session.active_index(), Some(1));
        assert_eq!(app.session.documents()[0].doc.work_path.as_ref(), Some(&original));
        assert!(app.session.documents()[1].doc.work_path.is_none());
        assert!(app.ui.status_error);
        assert!(!crate::direct_select::candidates(&app).iter().any(|(target, _)| *target == crate::direct_select::PathRef::Pen));
        app.session.authorize = None;
        app.sync_views();
        assert_eq!(app.ui.pen, draft, "passive frames do not retry a refused commit");
        assert_eq!(app.session.documents()[0].doc.work_path.as_ref(), Some(&original));
        assert!(pen_commit(&mut app, false), "the user explicitly retries the valid captured target");
        assert!(app.ui.pen.is_none());
        assert_eq!(app.session.active_index(), Some(1));
        assert_eq!(app.ui.selected_path.as_deref(), Some("foreign named path"));
        let saved = app.session.documents()[0].doc.work_path.clone().unwrap();
        assert_eq!(saved.subpaths.len(), original.subpaths.len() + 1);
        assert_eq!(saved.subpaths[0], original.subpaths[0]);
        app.sync_views();
        assert_eq!(app.session.documents()[0].doc.work_path.as_ref(), Some(&saved), "retry never duplicates the path");
        assert!(app.session.set_active(0));
        assert!(app.session.undo());
        assert_eq!(app.session.active().unwrap().doc.work_path.as_ref(), Some(&original));
    }
}

/// A draft under two anchors is no path, so a new document takes none of it along. The Pen's first anchor is a step of
/// its own in the document it was drawn on (#12890); the Curvature Pen keeps its draft until the finish.
#[test]
fn new_document_drops_a_one_anchor_draft() {
    for tool in [Tool::Pen, Tool::CurvaturePen] {
        let mut app = app();
        app.ui.tool = tool;
        clicks(&mut app, &[[20.0, 30.0]]);
        assert!(app.ui.pen.is_some(), "{tool:?}: the anchor starts a draft");
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        assert!(app.ui.pen.is_none(), "{tool:?}");
        let one_anchor = app.session.documents()[0].doc.work_path.as_ref().map(|p| p.subpaths[0].knots.len());
        assert_eq!(one_anchor, (tool == Tool::Pen).then_some(1), "{tool:?}");
        assert!(app.session.active().unwrap().doc.work_path.is_none());
    }
}

#[test]
fn stale_pen_commits_to_its_owner_across_document_changes() {
    use photocraft_doc::{ColorMode, SampleType, Size};
    for tool in [Tool::Pen, Tool::CurvaturePen] {
        for action in ["switch", "open", "closeOther", "pen", "curvature", "undo", "commit"] {
            let mut app = app();
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            app.ui.tool = tool;
            clicks(&mut app, &[[20.0, 20.0], [80.0, 60.0], [120.0, 30.0]]);
            let owner = app.ui.pen_doc.unwrap();
            let expected = pen_path(app.ui.pen.as_ref().unwrap());
            app.ui.direct_selection.target = Some(crate::direct_select::PathRef::Pen);
            app.ui.direct_selection.anchors = vec![[0, 1]];
            assert!(app.session.set_active(0));
            match action {
                "open" => {
                    app.services.import = Some(Box::new(|name, _, _| Ok((Document::new(name, Size::new(4, 4), ColorMode::Rgb, SampleType::U8), Vec::new()))));
                    app.open_file("synthetic.png", &[]).unwrap();
                }
                "closeOther" => {
                    app.run("file.close", json!({"document": 0})).unwrap();
                }
                "pen" => {
                    app.ui.tool = Tool::Pen;
                    testing::press(&mut app, [150.0, 150.0], Modifiers::NONE);
                }
                "curvature" => {
                    app.ui.tool = Tool::CurvaturePen;
                    testing::press(&mut app, [150.0, 150.0], Modifiers::NONE);
                }
                "undo" => assert!(!remove_last(&mut app)),
                "commit" => {
                    pen_commit(&mut app, true);
                }
                _ => app.sync_views(),
            }
            let source = app.session.documents().iter().find(|d| d.doc.id == owner).unwrap();
            let path = source.doc.work_path.as_ref().expect("the owner keeps the unfinished path");
            assert_eq!(path, &expected, "{tool:?} {action}");
            let others = |app: &PhotocraftApp| -> Vec<usize> {
                app.session
                    .documents()
                    .iter()
                    .filter(|d| d.doc.id != owner)
                    .map(|d| d.doc.work_path.as_ref().map_or(0, |p| p.subpaths.iter().map(|s| s.knots.len()).sum::<usize>()))
                    .collect()
            };
            // #12890: a press in another document writes its Work Path there with its first anchor, so that document
            // holds exactly that one anchor; every other document stays empty.
            let counts = others(&app);
            if action == "pen" {
                assert_eq!(counts.as_slice(), &[1], "{tool:?} {action}: the receiving document has the press's one-anchor Work Path");
            } else {
                assert!(counts.iter().all(|n| *n == 0), "{tool:?} {action}: no other document takes the draft: {counts:?}");
            }
            if action != "closeOther" {
                assert_ne!(app.session.active().unwrap().doc.id, owner, "the active document is restored");
            }
            assert_eq!(app.ui.direct_selection, Default::default());
            let index = app.session.documents().iter().position(|d| d.doc.id == owner).unwrap();
            assert!(app.session.set_active(index));
            assert!(app.session.undo());
            // #12890: the Pen's clicks were steps of its own, so one Undo leaves the path with the anchors before the
            // last; the Curvature Pen still commits the whole path in one step, which Undo takes back entire.
            let anchors = app.session.active().map_or(0, |st| st.doc.work_path.as_ref().map_or(0, |p| p.subpaths.iter().map(|s| s.knots.len()).sum::<usize>()));
            assert_eq!(anchors, usize::from(tool == Tool::Pen) * 2, "{tool:?} {action}");
            assert!(app.session.redo());
            assert!(app.session.active().unwrap().doc.work_path.is_some());
        }
    }
}

/// #4721: a second Pen path joins the Work Path it was drawn after (still selected), as a new
/// subpath, instead of replacing it; a saved path that is selected takes it instead; with no
/// path selected, the new path replaces the Work Path, as in Photoshop.
#[test]
fn a_new_pen_path_joins_the_selected_path() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    let draw = |app: &mut PhotocraftApp, x: f64| {
        clicks(app, &[[x, 20.0], [x + 40.0, 20.0], [x + 40.0, 60.0]]);
        clicks(app, &[[x + 0.5, 20.5]]); // close
    };
    let work = |app: &PhotocraftApp| app.session.active().unwrap().doc.work_path.clone().unwrap();
    draw(&mut app, 20.0);
    assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
    draw(&mut app, 120.0);
    let wp = work(&app);
    assert_eq!(wp.subpaths.len(), 2, "the first path stays");
    assert!(wp.subpaths.iter().all(|s| s.closed && s.knots.len() == 3));
    assert_eq!(wp.subpaths[0].knots[0].anchor.x, 20.0);
    assert_eq!(wp.subpaths[1].knots[0].anchor.x, 120.0);
    // #12890: the second path went into the Work Path click by click, one step each: its component, its two further
    // anchors and its close. Undo takes them back one at a time and leaves the first path alone.
    let steps_of_the_second_path = 4;
    for _ in 0..steps_of_the_second_path {
        app.session.undo();
    }
    assert_eq!(work(&app).subpaths.len(), 1, "undo took the second path back anchor by anchor");
    app.session.redo();
    assert_eq!(work(&app).subpaths.len(), 2, "and Redo puts its first anchor back");
    // A selected saved path takes the next one.
    app.run("path.set", json!({"name": "Saved", "path": {"subpaths": [{"knots": [[2, 2], [12, 2], [12, 8]]}]}})).unwrap();
    app.ui.selected_path = Some("Saved".into());
    draw(&mut app, 220.0);
    let saved = app.session.active().unwrap().doc.paths.iter().find(|p| p.name == "Saved").unwrap().path.clone();
    assert_eq!(saved.subpaths.len(), 2);
    assert_eq!(work(&app).subpaths.len(), 2, "the Work Path is left alone");
    assert_eq!(app.ui.selected_path.as_deref(), Some("Saved"));
    // No path selected: a new Work Path replaces the old one.
    app.ui.selected_path = None;
    draw(&mut app, 320.0);
    let wp = work(&app);
    assert_eq!(wp.subpaths.len(), 1);
    assert_eq!(wp.subpaths[0].knots[0].anchor.x, 320.0);
    assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
}

#[test]
fn pen_point_undo_preserves_earlier_knots_and_document_history() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    app.run("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 40], "fill": "#ff0000"})).unwrap();
    let old_history = app.session.active().unwrap().history.past_len();
    let none = Modifiers::NONE;

    clicks(&mut app, &[[20.0, 20.0]]);
    testing::press(&mut app, [120.0, 20.0], none);
    testing::move_to(&mut app, [120.0, 60.0], none);
    testing::release(&mut app, [120.0, 60.0], none);
    clicks(&mut app, &[[120.0, 120.0]]);

    let original_first_two = app.ui.pen.as_ref().unwrap().draft.knots[..2].to_vec();
    assert!(remove_last(&mut app));
    let pen = app.ui.pen.as_ref().unwrap();
    assert_eq!(pen.draft.knots, original_first_two);
    assert!(!testing::in_gesture(&app), "undo leaves no Pen drag running");
    // #12890: each click is a step of its own, so Undo takes that one step back and leaves the shape layer's step
    // before them alone. The Work Path is in the document while the path is still being drawn.
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), old_history + 2, "three clicks made three steps, Undo took one back");
    let work = st.doc.work_path.as_ref().expect("the Work Path exists while drawing");
    assert_eq!(work.subpaths[0].knots.len(), 2);
    assert!(work.subpaths[0].knots[1].smooth, "retained smooth handles are unchanged");
    let steps = app.session.active().unwrap().history.past_len();
    pen_commit(&mut app, false);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "finishing adds no step");
    let work = app.session.active().unwrap().doc.work_path.as_ref().unwrap();
    assert_eq!(work.subpaths[0].knots.len(), 2);
    assert!(work.subpaths[0].knots[1].smooth, "retained smooth handles are unchanged");
}

/// #1419: Solid Color (and every new fill or adjustment layer) made while a path is selected
/// in the Paths panel takes it as its vector mask; with the path deselected, the selection
/// is its layer mask again.
#[test]
fn new_fill_layer_takes_the_selected_path_as_its_vector_mask() {
    let mut app = app();
    let layer = |app: &PhotocraftApp, r: Result<Value, String>| {
        let id = photocraft_doc::LayerId(r.unwrap()["layer"].as_u64().unwrap());
        app.session.active().unwrap().doc.layer(id).unwrap().clone()
    };
    // Drawing a work path with the Pen selects it.
    app.ui.tool = Tool::Pen;
    clicks(&mut app, &[[20.0, 20.0], [120.0, 20.0], [120.0, 120.0], [20.5, 20.5]]);
    assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
    let wp = app.session.active().unwrap().doc.work_path.clone().unwrap();
    app.run("select.rect", json!({"x": 150, "y": 150, "width": 20, "height": 20})).unwrap();
    for id in ["layer.newFillLayer.solidColor", "layer.newFillLayer.gradient", "layer.newAdjustmentLayer.invert"] {
        app.ui.selected_path = Some("work".into());
        let r = app.run(id, json!({}));
        let l = layer(&app, r);
        assert_eq!(l.vector_mask.as_ref().map(|v| &v.path), Some(&wp), "{id}: the work path is the vector mask");
        assert!(l.mask.is_none(), "{id}: the path wins over the selection");
        // The new layer's vector mask is now the active path, not the work path.
        assert_eq!(app.ui.selected_path.as_deref(), Some("layer"), "{id}");
    }
    // So the next fill layer isn't masked by the work path again.
    let r = app.run("layer.newFillLayer.solidColor", json!({}));
    assert!(layer(&app, r).vector_mask.is_none());
    // A caller can opt out.
    let r = app.run("layer.newFillLayer.solidColor", json!({"path": null}));
    assert!(layer(&app, r).vector_mask.is_none());
    // A selected saved path is used.
    app.run("path.set", json!({"name": "Saved", "path": {"subpaths": [{"closed": true, "knots": [[0, 0], [9, 0], [9, 9]]}]}})).unwrap();
    app.ui.selected_path = Some("Saved".into());
    let saved = app.session.active().unwrap().doc.paths[0].path.clone();
    let r = app.run("layer.newFillLayer.solidColor", json!({}));
    assert_eq!(layer(&app, r).vector_mask.map(|v| v.path), Some(saved));
    // Deselected (or a stale name): the selection is the layer mask, no vector mask.
    for sel in [None, Some("Gone".to_string())] {
        app.ui.selected_path = sel;
        // The selection goes into the new layer's mask, so each fill layer needs its own (#8801).
        app.run("select.rect", json!({"x": 150, "y": 150, "width": 20, "height": 20})).unwrap();
        let r = app.run("layer.newFillLayer.solidColor", json!({}));
        let l = layer(&app, r);
        assert!(l.vector_mask.is_none() && l.mask.as_ref().is_some_and(|m| m.value(160, 160) == 1.0 && m.value(5, 5) == 0.0));
    }
    // Make Work Path selects the new work path.
    app.run("select.rect", json!({"x": 150, "y": 150, "width": 20, "height": 20})).unwrap();
    app.run("select.toWorkPath", json!({"tolerance": 2.0})).unwrap();
    assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
}

/// #1520: changing one corner through the Properties command must keep the other three
/// independent, remain a live rectangle, and undo/redo as one geometry edit.
#[test]
fn live_rectangle_corner_radius_edit_preserves_other_corners_and_history() {
    let mut app = app();
    let created = app.run("shape.create", json!({"kind": "roundedRect", "rect": [10, 10, 150, 100], "radii": [5, 10, 15, 20]})).unwrap();
    let id = created["layer"].as_u64().unwrap();
    let get_radii = |app: &PhotocraftApp| {
        let st = app.session.active().unwrap();
        let shape = st.doc.layer(photocraft_doc::LayerId(id)).unwrap();
        match &shape.content {
            LayerContent::Shape(sh) => match &sh.live {
                Some(photocraft_doc::vector::LiveShape::Rect { radii, .. }) => *radii,
                _ => panic!("rectangle must remain a live shape"),
            },
            _ => panic!("expected shape layer"),
        }
    };
    assert_eq!(get_radii(&app), [5.0, 10.0, 15.0, 20.0]);
    let mut patch = corner_radii_patch(get_radii(&app), 1, 30.0);
    patch["layer"] = json!(id);
    app.run("shape.edit", patch).unwrap();
    assert_eq!(get_radii(&app), [5.0, 30.0, 15.0, 20.0]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(get_radii(&app), [5.0, 10.0, 15.0, 20.0]);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(get_radii(&app), [5.0, 30.0, 15.0, 20.0]);
    for corner in 0..4 {
        let before = get_radii(&app);
        let radius = 35.0 + corner as f32;
        let mut expected = before;
        expected[corner] = f64::from(radius);
        let mut patch = corner_radii_patch(before, corner, radius);
        patch["layer"] = json!(id);
        app.run("shape.edit", patch).unwrap();
        assert_eq!(get_radii(&app), expected);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(get_radii(&app), before);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(get_radii(&app), expected);
    }
    app.run("shape.edit", json!({"layer": id, "radii": 12})).unwrap();
    assert_eq!(get_radii(&app), [12.0; 4]);
}

/// The options bar of `tool` edits the selected shape's fill and stroke, one history step per edit.
fn shape_bar_harness(tool: Tool) -> egui_kittest::Harness<'static, Box<PhotocraftApp>> {
    let mut app = app();
    app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
    shape_drag(&mut app, Tool::Rectangle, [20.0, 20.0], [120.0, 100.0]);
    let mut h = egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1100.0, 48.0)).build_ui_state(
        move |ui, app: &mut Box<PhotocraftApp>| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.horizontal_centered(|ui| options_bar(app, ui, tool));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::studio());
    h.run_steps(4);
    h
}

/// The fill and stroke of the active shape layer.
fn active_shape_paint(app: &PhotocraftApp) -> (Option<photocraft_doc::Fill>, Option<photocraft_doc::vector::ShapeStroke>) {
    let st = app.session.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        LayerContent::Shape(sh) => (sh.fill.clone(), sh.stroke.clone()),
        _ => panic!("not a shape layer"),
    }
}

/// Clicks the options bar's Fill check box.
fn click_bar_fill(h: &mut egui_kittest::Harness<'_, Box<PhotocraftApp>>) {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    h.get_by_role_and_label(Role::CheckBox, "Fill").click();
    h.run_steps(2);
}

/// Whether the options bar shows its Fill check box.
fn bar_shows_fill(h: &egui_kittest::Harness<'_, Box<PhotocraftApp>>) -> bool {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    h.query_by_role_and_label(Role::CheckBox, "Fill").is_some()
}

fn type_stroke_width(h: &mut egui_kittest::Harness<'_, Box<PhotocraftApp>>) {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    h.get_all_by_role(Role::SpinButton).next().expect("the stroke width").click();
    h.run();
    for c in "6".chars() {
        h.event(egui::Event::Text(c.to_string()));
        h.run();
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
}

#[test]
fn shape_options_bar_edits_the_selected_shape() {
    let mut h = shape_bar_harness(Tool::Rectangle);
    assert!(active_shape_paint(h.state()).0.is_some() && active_shape_paint(h.state()).1.is_none());
    type_stroke_width(&mut h);
    let stroke = active_shape_paint(h.state()).1.expect("the width field stroked the shape");
    assert_eq!(stroke.width, 6.0);
    assert_eq!(stroke.paint, photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(0.0, 0.0, 1.0, 1.0)));
    // Fill off: one step.
    let before = h.state().session.active().unwrap().history.past_len();
    click_bar_fill(&mut h);
    assert_eq!(active_shape_paint(h.state()).0, None, "unchecking Fill removed the shape's fill");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), before + 1, "one step for the edit");
    assert!(!h.state().ui.tool_options.shape_fill, "and the next shape has none either");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(active_shape_paint(h.state()).0.is_some());
    // No shape selected: no edit.
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    let history = h.state().session.active().unwrap().history.past_len();
    h.run_steps(2);
    click_bar_fill(&mut h);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
}

/// #3915: Path Selection and Direct Selection options bars show Fill, Stroke, and Radius
/// controls when a shape is selected, and editing them edits the selected shape.
#[test]
fn path_selection_options_bar_edits_the_selected_shape() {
    let mut h = shape_bar_harness(Tool::PathSelection);
    assert!(active_shape_paint(h.state()).0.is_some() && active_shape_paint(h.state()).1.is_none());
    // Stroke width field is available on Path Selection bar too.
    type_stroke_width(&mut h);
    let stroke = active_shape_paint(h.state()).1.expect("the width field stroked the shape");
    assert_eq!(stroke.width, 6.0);
    assert_eq!(stroke.paint, photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(0.0, 0.0, 1.0, 1.0)));
    // Fill toggle: one step.
    let before = h.state().session.active().unwrap().history.past_len();
    click_bar_fill(&mut h);
    assert_eq!(active_shape_paint(h.state()).0, None, "unchecking Fill removed the shape's fill");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), before + 1, "one step for the edit");
    // No shape selected: renders hint text instead.
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.run_steps(2);
    assert!(!bar_shows_fill(&h));
}

/// #6553: a Paths row dropped on the footer's trash is deleted, whichever row is selected,
/// and a shape layer's path row does not start a drag.
#[test]
fn path_rows_dragged_onto_the_trash_are_deleted() {
    let mut app = app();
    let path = json!({"subpaths": [{"closed": true, "knots": [
        {"anchor": [10, 10], "in": [10, 10], "out": [10, 10]},
        {"anchor": [80, 10], "in": [80, 10], "out": [80, 10]},
        {"anchor": [80, 80], "in": [80, 80], "out": [80, 80]}
    ]}]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    app.ui.selected_path = Some("work".into());
    // The saved path, while the work path is selected: only the dropped row goes.
    apply(&mut app, PathsEvent::footer_drop(PathsFooterButton::Delete, vec![PathRef::Saved(0)]));
    let doc = &app.session.active().unwrap().doc;
    assert!(doc.paths.is_empty(), "the dropped saved path is deleted");
    assert!(doc.work_path.is_some(), "the selected work path is kept");
    // The work path.
    apply(&mut app, PathsEvent::footer_drop(PathsFooterButton::Delete, vec![PathRef::Work]));
    assert!(app.session.active().unwrap().doc.work_path.is_none(), "the dropped work path is deleted");
    // A shape layer's path row is not offered as a drag (its deletion is #6552's).
    assert!(!PathKind::ShapePath.drags());
    assert!(PathKind::Saved.drags() && PathKind::Work.drags());
}
