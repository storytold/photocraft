//! #12909: the Pen in Shape mode with New Layer makes its shape layer on the first click, and the
//! layer follows the path anchor by anchor, so the fill shows before the path is closed.
use super::*;

/// An 800 by 600 document with a pixel layer above the Background, the Pen in Shape mode.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Pen;
    app.ui.tool_options.vector_mode = "shape".into();
    app
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64) {
    pen_down(app, x, y, false);
    pen_up(app);
}

/// Edit › Undo and Redo as the menu, the shortcut and the control channel run them.
fn menu(app: &mut PhotocraftApp, id: &str) {
    crate::menus::invoke(app, &egui::Context::default(), id, json!({})).unwrap();
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn undo_label(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.undo_label().map(str::to_string)
}

/// Every shape layer of the document, bottom to top: its name and shape.
fn shapes(app: &PhotocraftApp) -> Vec<(String, photocraft_doc::ShapeLayer)> {
    let doc = &app.session.active().unwrap().doc;
    doc.layers
        .iter()
        .filter_map(|l| match &l.content {
            LayerContent::Shape(sh) => Some((l.name.clone(), sh.clone())),
            _ => None,
        })
        .collect()
}

/// The anchors of each subpath of a shape and whether it is closed.
fn subpaths(sh: &photocraft_doc::ShapeLayer) -> Vec<(bool, Vec<[f64; 2]>)> {
    sh.path.subpaths.iter().map(|s| (s.closed, s.knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect())).collect()
}

fn draft_len(app: &PhotocraftApp) -> Option<usize> {
    app.ui.pen.as_ref().map(|p| p.knots.len())
}

fn covered(path: &Path, x: i32, y: i32) -> f32 {
    photocraft_vector::fill_rasterizer(path, photocraft_vector::DEFAULT_TOLERANCE).render(photocraft_geom::Rect::from_xywh(x, y, 1, 1))[0]
}

#[test]
fn the_first_click_makes_the_shape_layer_right_away() {
    let mut app = app();
    let (before, below) = (steps(&app), app.session.active().unwrap().active_layer.unwrap());
    click(&mut app, 500.0, 100.0);
    let list = shapes(&app);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].0, "Shape 1");
    assert_eq!(subpaths(&list[0].1), [(false, vec![[500.0, 100.0]])]);
    assert_eq!(steps(&app), before + 1);
    assert_eq!(undo_label(&app).as_deref(), Some("New Shape Layer"));
    assert!(app.ui.pen.as_ref().is_some_and(|p| p.live_layer), "the path is still being drawn");
    let st = app.session.active().unwrap();
    let at = |id| st.doc.layers.iter().position(|l| l.id == id);
    assert_eq!(st.active_layer.and_then(at), at(below).map(|i| i + 1), "above the layer that was active");
}

#[test]
fn the_layer_fills_the_open_path_anchor_by_anchor() {
    let mut app = app();
    let before = steps(&app);
    for (x, y) in [(500.0, 100.0), (700.0, 100.0), (700.0, 300.0)] {
        click(&mut app, x, y);
    }
    let list = shapes(&app);
    assert_eq!(list.len(), 1, "one layer, not one per anchor");
    let sh = &list[0].1;
    assert_eq!(subpaths(sh), [(false, vec![[500.0, 100.0], [700.0, 100.0], [700.0, 300.0]])], "an open path, not closed yet");
    assert!(sh.fill.is_some(), "it has the options bar's fill");
    assert_eq!(covered(&sh.path, 650, 150), 1.0, "the triangle is filled before it is closed");
    assert_eq!(covered(&sh.path, 550, 250), 0.0, "and only the triangle");
    assert_eq!(steps(&app), before + 3, "the layer, then one step per further anchor");
    assert_eq!(undo_label(&app).as_deref(), Some("Edit Shape"));
    assert_eq!(draft_len(&app), Some(3));
}

#[test]
fn one_click_then_enter_leaves_a_one_anchor_shape_layer() {
    let mut app = app();
    click(&mut app, 500.0, 100.0);
    let before = steps(&app);
    pen_enter(&mut app);
    assert!(app.ui.pen.is_none());
    let list = shapes(&app);
    assert_eq!(list.len(), 1);
    assert_eq!(subpaths(&list[0].1), [(false, vec![[500.0, 100.0]])]);
    assert_eq!(steps(&app), before, "finishing adds no step");
}

#[test]
fn finishing_with_enter_or_esc_then_clicking_elsewhere_starts_a_new_layer() {
    let mut app = app();
    click(&mut app, 500.0, 100.0);
    click(&mut app, 700.0, 100.0);
    pen_enter(&mut app);
    click(&mut app, 100.0, 400.0);
    assert_eq!(shapes(&app).iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["Shape 1", "Shape 2"]);
    click(&mut app, 300.0, 400.0);
    pen_end(&mut app);
    click(&mut app, 600.0, 500.0);
    let list = shapes(&app);
    assert_eq!(list.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["Shape 1", "Shape 2", "Shape 3"]);
    let anchors: Vec<_> = list.iter().map(|s| subpaths(&s.1)[0].1.len()).collect();
    assert_eq!(anchors, [2, 2, 1]);
}

#[test]
fn clicking_the_first_anchor_closes_the_same_layer_and_keeps_its_fill() {
    let mut app = app();
    for (x, y) in [(500.0, 100.0), (700.0, 100.0), (700.0, 300.0)] {
        click(&mut app, x, y);
    }
    click(&mut app, 500.0, 100.0);
    assert!(app.ui.pen.is_none(), "closing commits");
    let list = shapes(&app);
    assert_eq!(list.len(), 1, "no second layer");
    let sh = &list[0].1;
    assert_eq!(subpaths(sh), [(true, vec![[500.0, 100.0], [700.0, 100.0], [700.0, 300.0]])]);
    assert!(sh.fill.is_some());
    assert_eq!(covered(&sh.path, 650, 150), 1.0);
    let st = app.session.active().unwrap();
    assert_eq!(st.active_layer.and_then(|id| st.doc.layer(id)).map(|l| l.name.as_str()), Some("Shape 1"), "the layer is the active one");
}

#[test]
fn undo_while_drawing_takes_back_anchors_then_the_layer() {
    let mut app = app();
    let layers = app.session.active().unwrap().doc.layers.len();
    for (x, y) in [(500.0, 100.0), (700.0, 100.0), (700.0, 300.0)] {
        click(&mut app, x, y);
    }
    menu(&mut app, "edit.undo");
    assert_eq!(draft_len(&app), Some(2), "the draft lost its last anchor");
    assert_eq!(subpaths(&shapes(&app)[0].1), [(false, vec![[500.0, 100.0], [700.0, 100.0]])], "and so did the layer");
    // Drawing carries on from the draft.
    click(&mut app, 700.0, 400.0);
    assert_eq!(subpaths(&shapes(&app)[0].1), [(false, vec![[500.0, 100.0], [700.0, 100.0], [700.0, 400.0]])]);
    menu(&mut app, "edit.undo");
    menu(&mut app, "edit.undo");
    assert_eq!(draft_len(&app), Some(1));
    assert_eq!(subpaths(&shapes(&app)[0].1), [(false, vec![[500.0, 100.0]])]);
    menu(&mut app, "edit.undo");
    assert!(app.ui.pen.is_none(), "the draft goes with its layer");
    assert!(shapes(&app).is_empty());
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers);
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

#[test]
fn redo_while_drawing_brings_the_anchor_back_and_the_draft_follows() {
    let mut app = app();
    for (x, y) in [(500.0, 100.0), (700.0, 100.0), (700.0, 300.0)] {
        click(&mut app, x, y);
    }
    menu(&mut app, "edit.undo");
    assert_eq!(draft_len(&app), Some(2));
    menu(&mut app, "edit.redo");
    // The next frame, or the next click, reconciles the draft with the document.
    crate::vector_ui::cancel_stale(&mut app);
    assert_eq!(draft_len(&app), Some(3));
    assert_eq!(subpaths(&shapes(&app)[0].1), [(false, vec![[500.0, 100.0], [700.0, 100.0], [700.0, 300.0]])]);
    // And it is a live draft again: the next anchor goes into the same layer.
    click(&mut app, 500.0, 300.0);
    assert_eq!(shapes(&app).len(), 1);
    assert_eq!(subpaths(&shapes(&app)[0].1)[0].1.len(), 4);
}

#[test]
fn another_history_state_drops_a_draft_whose_layer_is_gone() {
    let mut app = app();
    click(&mut app, 500.0, 100.0);
    click(&mut app, 700.0, 100.0);
    // Something else removes the layer (a History step back past it).
    app.session.undo();
    app.session.undo();
    crate::vector_ui::cancel_stale(&mut app);
    assert!(app.ui.pen.is_none());
    assert!(shapes(&app).is_empty());
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

#[test]
fn path_mode_other_operations_and_the_other_pens_make_no_layer_on_the_first_click() {
    let mut app = app();
    let before = steps(&app);
    // Path mode.
    app.ui.tool_options.vector_mode = "path".into();
    click(&mut app, 500.0, 100.0);
    assert!(shapes(&app).is_empty() && steps(&app) == before);
    assert!(app.ui.pen.as_ref().is_some_and(|p| !p.live_layer));
    app.ui.pen = None;
    // A Path Operation other than New Layer adds to a shape instead.
    app.ui.tool_options.vector_mode = "shape".into();
    app.ui.tool_options.pen_path_op = "combine".into();
    click(&mut app, 500.0, 100.0);
    assert!(shapes(&app).is_empty() && steps(&app) == before);
    app.ui.pen = None;
    app.ui.tool_options.pen_path_op = "newLayer".into();
    // The Curvature Pen.
    app.ui.tool = Tool::CurvaturePen;
    crate::vector_ui::curvature_down(&mut app, 500.0, 100.0);
    crate::vector_ui::curvature_up(&mut app);
    assert!(shapes(&app).is_empty() && steps(&app) == before);
    assert!(app.ui.pen.as_ref().is_some_and(|p| !p.live_layer));
}

#[test]
fn a_path_carried_on_from_its_endpoint_stays_one_step() {
    let mut app = app();
    click(&mut app, 500.0, 100.0);
    click(&mut app, 700.0, 100.0);
    pen_enter(&mut app);
    let before = steps(&app);
    click(&mut app, 700.0, 100.0);
    assert!(app.ui.pen.as_ref().is_some_and(|p| p.resumes.is_some() && !p.live_layer), "a press on the endpoint carries the path on");
    click(&mut app, 700.0, 300.0);
    pen_enter(&mut app);
    assert_eq!(shapes(&app).len(), 1, "no new layer");
    assert_eq!(subpaths(&shapes(&app)[0].1)[0].1.len(), 3);
    assert_eq!(steps(&app), before + 1);
}

/// Refuse `shape.edit`, so the next anchor stays the draft's alone.
fn refuse_edits(app: &mut PhotocraftApp) {
    app.session.authorize = Some(|id, _| if id == "shape.edit" { Err(photocraft_engine::EngineError::Other("refused edit fixture".into())) } else { Ok(()) });
}

#[test]
fn undo_takes_back_an_anchor_the_layer_refused_before_any_document_step() {
    let mut app = app();
    click(&mut app, 500.0, 100.0);
    click(&mut app, 700.0, 100.0);
    refuse_edits(&mut app);
    click(&mut app, 700.0, 300.0);
    assert_eq!(draft_len(&app), Some(3), "the refused anchor stays in the draft");
    app.session.authorize = None;
    let before = steps(&app);
    menu(&mut app, "edit.undo");
    assert_eq!(draft_len(&app), Some(2), "only the refused anchor is taken back");
    assert_eq!(subpaths(&shapes(&app)[0].1), [(false, vec![[500.0, 100.0], [700.0, 100.0]])], "the layer keeps the anchors it took");
    assert_eq!(steps(&app), before, "no document step is undone");
    menu(&mut app, "edit.undo");
    assert_eq!(draft_len(&app), Some(1));
    assert_eq!(subpaths(&shapes(&app)[0].1), [(false, vec![[500.0, 100.0]])]);
}

#[test]
fn an_edit_elsewhere_keeps_the_anchors_the_layer_has_not_taken() {
    let mut app = app();
    let below = app.session.active().unwrap().active_layer.unwrap().0;
    click(&mut app, 500.0, 100.0);
    click(&mut app, 700.0, 100.0);
    refuse_edits(&mut app);
    click(&mut app, 700.0, 300.0);
    app.session.authorize = None;
    app.run("layer.setProps", json!({"layer": below, "opacity": 0.5})).unwrap();
    crate::vector_ui::cancel_stale(&mut app);
    assert_eq!(draft_len(&app), Some(3), "the pending anchor is kept");
    // The next anchor takes the pending one into the layer with it.
    click(&mut app, 500.0, 300.0);
    assert_eq!(subpaths(&shapes(&app)[0].1)[0].1.len(), 4);
}

#[test]
fn closing_a_carried_on_open_shape_keeps_a_fill_set_by_hand() {
    let mut app = app();
    click(&mut app, 500.0, 100.0);
    click(&mut app, 700.0, 100.0);
    click(&mut app, 700.0, 300.0);
    pen_enter(&mut app);
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    let options_fill = shapes(&app)[0].1.fill.clone();
    let mut path = None;
    if let LayerContent::Shape(sh) = &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
        path = Some(photocraft_engine::vector_cmds::path_json(&sh.path));
    }
    app.run("shape.edit", json!({"layer": id, "path": path.unwrap(), "fill": [0.25, 0.5, 0.75, 1.0]})).unwrap();
    let own = shapes(&app)[0].1.fill.clone();
    assert!(own.is_some() && own != options_fill, "the layer has a fill of its own");
    click(&mut app, 700.0, 300.0); // its last anchor: carry on
    click(&mut app, 500.0, 300.0);
    click(&mut app, 500.0, 100.0); // its first anchor: close
    let sh = &shapes(&app)[0].1;
    assert!(sh.path.subpaths[0].closed);
    assert_eq!(sh.fill, own, "a fill set by hand is not replaced by the options bar's");
}
