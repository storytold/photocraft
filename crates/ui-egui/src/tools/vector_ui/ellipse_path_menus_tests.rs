//! #13235: the shape tools' options bar has Path operations, Path alignment and Path arrangement.
//! The operation is the shape tools' own setting: with one other than New Layer a drag becomes a
//! component of the active shape layer; Reset Tool puts it back to New Layer. Path alignment's
//! entries move the active shape layer's components (#13456).

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::vector::PathOp;
use photocraft_doc::{LayerContent, ShapeLayer};
use serde_json::json;

use super::pen::{PATH_ALIGN_ENTRIES, PATH_ARRANGE_ENTRIES, PATH_DISTRIBUTE_ENTRIES, PATH_SPACING_ENTRIES, shape_path_op};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{Tool, ToolOptions};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::EllipseShape;
    app
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let mods = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

fn layer_count(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layer_count()
}

fn active_shape(app: &PhotocraftApp) -> ShapeLayer {
    let st = app.session.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        LayerContent::Shape(s) => s.clone(),
        _ => panic!("the active layer is not a shape layer"),
    }
}

#[test]
fn defaults_are_new_layer_in_shape_mode_and_exclude_in_path_mode() {
    let mut o = ToolOptions::default();
    assert_eq!(o.shape_path_op, "newLayer");
    assert_eq!(shape_path_op(&o), None);
    o.shape_mode = "path".into();
    assert_eq!(shape_path_op(&o), Some("exclude"));
}

#[test]
fn an_operation_adds_the_next_drag_to_the_active_shape_layer() {
    for (op, want) in [("combine", PathOp::Combine), ("subtract", PathOp::Subtract), ("intersect", PathOp::Intersect), ("exclude", PathOp::Exclude)] {
        let mut app = app();
        drag(&mut app, [50.0, 50.0], [200.0, 200.0]);
        let layers = layer_count(&app);
        app.ui.tool_options.shape_path_op = op.into();
        drag(&mut app, [100.0, 100.0], [250.0, 250.0]);
        assert_eq!(layer_count(&app), layers, "{op}: no new layer");
        let shape = active_shape(&app);
        assert_eq!(shape.path.subpaths.len(), 2, "{op}: a second component");
        assert_eq!(shape.path.subpaths[1].op, want, "{op}");
    }
}

#[test]
fn new_layer_makes_a_new_shape_layer() {
    let mut app = app();
    drag(&mut app, [50.0, 50.0], [200.0, 200.0]);
    let layers = layer_count(&app);
    drag(&mut app, [100.0, 100.0], [250.0, 250.0]);
    assert_eq!(layer_count(&app), layers + 1);
}

#[test]
fn an_operation_with_a_pixel_layer_active_still_makes_a_shape_layer() {
    let mut app = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool_options.shape_path_op = "combine".into();
    let layers = layer_count(&app);
    drag(&mut app, [50.0, 50.0], [200.0, 200.0]);
    assert_eq!(layer_count(&app), layers + 1);
    assert!(matches!(app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().content, LayerContent::Shape(_)));
}

#[test]
fn set_op_writes_the_active_tools_own_setting() {
    let mut app = app();
    crate::path_components::set_op(&mut app, "subtract").unwrap();
    assert_eq!(app.ui.tool_options.shape_path_op, "subtract");
    assert_eq!(app.ui.tool_options.pen_path_op, "newLayer");
    app.ui.tool = Tool::Pen;
    crate::path_components::set_op(&mut app, "intersect").unwrap();
    assert_eq!(app.ui.tool_options.pen_path_op, "intersect");
    assert_eq!(app.ui.tool_options.shape_path_op, "subtract");
}

#[test]
fn reset_tool_restores_new_layer() {
    let mut app = app();
    app.ui.tool_options.shape_path_op = "exclude".into();
    crate::menus::invoke(&mut app, &egui::Context::default(), crate::tool_reset::RESET, json!({})).unwrap();
    assert_eq!(app.ui.tool_options.shape_path_op, "newLayer");
}

fn labels(e: &[(&'static str, &'static str)]) -> Vec<&'static str> {
    e.iter().map(|(l, _)| *l).collect()
}

#[test]
fn entries_follow_the_reference_order() {
    assert_eq!(
        labels(&PATH_ALIGN_ENTRIES),
        ["Align Left Edges", "Align Horizontal Centers", "Align Right Edges", "Align Top Edges", "Align Vertical Centers", "Align Bottom Edges"]
    );
    assert_eq!(
        labels(&PATH_DISTRIBUTE_ENTRIES),
        [
            "Distribute Left Edges",
            "Distribute Horizontal Centers",
            "Distribute Right Edges",
            "Distribute Top Edges",
            "Distribute Vertical Centers",
            "Distribute Bottom Edges"
        ]
    );
    assert_eq!(labels(&PATH_SPACING_ENTRIES), ["Distribute Horizontally", "Distribute Vertically"]);
    assert_eq!(PATH_ARRANGE_ENTRIES, ["Bring Shape To Front", "Bring Shape Forward", "Send Shape Backward", "Send Shape To Back"]);
}

/// The Ellipse tool's options bar on `app`, its fonts loaded.
fn bar(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1600.0, 500.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.horizontal(|ui| super::options_bar(app, ui, Tool::EllipseShape));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    h
}

/// Opens the Path alignment popup.
fn open_alignment(h: &mut Harness<'_, PhotocraftApp>) {
    h.get_by_label("Path alignment").click();
    h.run_steps(3);
}

#[test]
fn options_bar_shows_the_three_buttons_and_greys_the_entries_without_a_shape() {
    let mut h = bar(app());
    let x = |h: &Harness<'_, PhotocraftApp>, l: &str| h.get_by_label(l).rect().min.x;
    let order = [x(&h, "Path operations"), x(&h, "Path alignment"), x(&h, "Path arrangement")];
    assert!(order.is_sorted_by(|a, b| a < b), "left to right: {order:?}");
    h.get_by_label("Path operations").click();
    h.run_steps(3);
    assert!(h.get_by_label("Merge Shape Components").accesskit_node().is_disabled());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    open_alignment(&mut h);
    for label in alignment_labels() {
        assert!(!enabled(&h, label), "{label}");
    }
}

/// Every Path alignment entry, in the popup's order.
fn alignment_labels() -> Vec<&'static str> {
    [labels(&PATH_ALIGN_ENTRIES), labels(&PATH_DISTRIBUTE_ENTRIES), labels(&PATH_SPACING_ENTRIES)].concat()
}

/// Is the entry `label` enabled?
fn enabled(h: &Harness<'_, PhotocraftApp>, label: &str) -> bool {
    !h.get_by_label(label).accesskit_node().is_disabled()
}

/// The rectangles `[x0, y0, x1, y1]` as one shape layer's components, made the active layer.
fn shape_of(app: &mut PhotocraftApp, rects: &[[f64; 4]]) {
    let subpaths: Vec<_> = rects.iter().map(|[x0, y0, x1, y1]| json!({"closed": true, "knots": [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]})).collect();
    app.run("shape.create", json!({"kind": "path", "path": {"subpaths": subpaths}, "fill": "#ff0000"})).unwrap();
}

/// The first corner of each component of the active shape layer.
fn corners(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    active_shape(app).path.subpaths.iter().map(|s| [s.knots[0].anchor.x, s.knots[0].anchor.y]).collect()
}

/// Three components: A 10..30 × 20..60, B 60..100 × 40..50, C 120..150 × 100..140.
const THREE: [[f64; 4]; 3] = [[10.0, 20.0, 30.0, 60.0], [60.0, 40.0, 100.0, 50.0], [120.0, 100.0, 150.0, 140.0]];

/// #13456: with a shape layer active the Path alignment entries are enabled (Align with two
/// components, Distribute and Distribute Spacing with three) and greyed below that; Align To
/// Canvas aligns even one.
#[test]
fn path_alignment_entries_are_enabled_by_the_shape_layers_components() {
    let labels = alignment_labels();
    let (align, rest) = labels.split_at(PATH_ALIGN_ENTRIES.len());
    for (n, want_align, want_rest) in [(1, false, false), (2, true, false), (3, true, true)] {
        let mut app = app();
        shape_of(&mut app, &THREE[..n]);
        let mut h = bar(app);
        open_alignment(&mut h);
        for label in align {
            assert_eq!(enabled(&h, label), want_align, "{n} components: {label}");
        }
        for label in rest {
            assert_eq!(enabled(&h, label), want_rest, "{n} components: {label}");
        }
    }
    // One component aligns to the canvas.
    let mut app = app();
    shape_of(&mut app, &THREE[..1]);
    let mut h = bar(app);
    h.ctx.data_mut(|d| d.insert_temp(egui::Id::unique("path-align-to"), true));
    open_alignment(&mut h);
    assert!(enabled(&h, "Align Right Edges"));
    assert!(!enabled(&h, "Distribute Right Edges"));
    h.get_by_label("Align Right Edges").click();
    h.run_steps(2);
    assert_eq!(corners(h.state()), [[380.0, 20.0]], "right edge on the 400 px canvas");
}

/// #13456: each entry moves the shape layer's components as `path.alignComponents` and
/// `path.distributeComponents` do, in one undoable History step, and the popup stays open.
#[test]
fn each_path_alignment_entry_moves_the_components() {
    let want: [(&str, [[f64; 2]; 3]); 14] = [
        ("Align Left Edges", [[10.0, 20.0], [10.0, 40.0], [10.0, 100.0]]),
        ("Align Horizontal Centers", [[70.0, 20.0], [60.0, 40.0], [65.0, 100.0]]),
        ("Align Right Edges", [[130.0, 20.0], [110.0, 40.0], [120.0, 100.0]]),
        ("Align Top Edges", [[10.0, 20.0], [60.0, 20.0], [120.0, 20.0]]),
        ("Align Vertical Centers", [[10.0, 60.0], [60.0, 75.0], [120.0, 60.0]]),
        ("Align Bottom Edges", [[10.0, 100.0], [60.0, 130.0], [120.0, 100.0]]),
        ("Distribute Left Edges", [[10.0, 20.0], [65.0, 40.0], [120.0, 100.0]]),
        ("Distribute Horizontal Centers", [[10.0, 20.0], [57.5, 40.0], [120.0, 100.0]]),
        ("Distribute Right Edges", [[10.0, 20.0], [50.0, 40.0], [120.0, 100.0]]),
        ("Distribute Top Edges", [[10.0, 20.0], [60.0, 60.0], [120.0, 100.0]]),
        ("Distribute Vertical Centers", [[10.0, 20.0], [60.0, 75.0], [120.0, 100.0]]),
        ("Distribute Bottom Edges", [[10.0, 55.0], [60.0, 40.0], [120.0, 100.0]]),
        ("Distribute Horizontally", [[10.0, 20.0], [55.0, 40.0], [120.0, 100.0]]),
        ("Distribute Vertically", [[10.0, 20.0], [60.0, 75.0], [120.0, 100.0]]),
    ];
    assert_eq!(want.map(|(l, _)| l).to_vec(), alignment_labels(), "every entry, in order");
    let mut app = app();
    shape_of(&mut app, &THREE);
    let before = active_shape(&app).path;
    let mut h = bar(app);
    open_alignment(&mut h);
    for (label, after) in want {
        let steps = h.state().session.active().unwrap().history.entries().len();
        // The popup is still open from the previous click.
        h.get_by_label(label).click();
        h.run_steps(2);
        assert_eq!(corners(h.state()), after, "{label}");
        let st = h.state().session.active().unwrap();
        assert_eq!(st.history.entries().len(), steps + 1, "{label}: one step");
        let name = if label.starts_with("Align") { "Align Path Components" } else { "Distribute Path Components" };
        assert_eq!(st.history.undo_label(), Some(name), "{label}");
        assert!(h.state_mut().session.undo(), "{label}: undo");
        assert_eq!(active_shape(h.state()).path, before, "{label}: undone");
        assert!(h.state_mut().session.redo(), "{label}: redo");
        assert_eq!(corners(h.state()), after, "{label}: redone");
        assert!(h.state_mut().session.undo());
        h.run_steps(1);
    }
}

/// #13456: in Path mode the entries act on the Work Path, the path a Shape tool draws into.
#[test]
fn path_mode_aligns_the_work_path() {
    let mut app = app();
    app.ui.tool_options.shape_mode = "path".into();
    let path = json!({"subpaths": [{"knots": [[10, 10], [30, 10], [30, 30]]}, {"knots": [[50, 50], [90, 50], [90, 80]]}]});
    app.run("path.set", json!({"path": path})).unwrap();
    let mut h = bar(app);
    open_alignment(&mut h);
    h.get_by_label("Align Bottom Edges").click();
    h.run_steps(2);
    let work = h.state().session.active().unwrap().doc.work_path.clone().unwrap();
    let first: Vec<[f64; 2]> = work.subpaths.iter().map(|s| [s.knots[0].anchor.x, s.knots[0].anchor.y]).collect();
    assert_eq!(first, [[10.0, 60.0], [50.0, 50.0]], "bottoms at 80");
}

/// #13442 review: the Custom Shape tool also adds to the active shape layer, as one component
/// whose hole stays joined to it.
#[test]
fn a_custom_shape_with_an_operation_joins_the_active_shape_layer() {
    let mut app = app();
    drag(&mut app, [50.0, 50.0], [200.0, 200.0]);
    let layers = layer_count(&app);
    app.ui.tool = Tool::CustomShape;
    app.ui.presets_ui.custom_shape = "Ring".into();
    app.ui.tool_options.shape_path_op = "subtract".into();
    drag(&mut app, [100.0, 100.0], [250.0, 250.0]);
    assert_eq!(layer_count(&app), layers, "no new layer");
    let ops: Vec<PathOp> = active_shape(&app).path.subpaths.iter().map(|s| s.op).collect();
    assert_eq!(ops, [PathOp::Combine, PathOp::Subtract, PathOp::Join], "the ellipse, the ring and its hole");
}

/// #13442 review: a custom shape that can't be made one component (its parts exclude each other)
/// is not added with an operation, which would change its parts' operations: the layer stays as
/// it was and the status says why.
#[test]
fn a_custom_shape_that_is_not_one_component_is_refused() {
    use photocraft_doc::vector::{Path, Subpath};
    let mut app = app();
    drag(&mut app, [50.0, 50.0], [200.0, 200.0]);
    let before = active_shape(&app).path;
    let xor = Path::new(vec![
        Subpath::polygon(&[(0.0, 0.0), (60.0, 0.0), (60.0, 60.0), (0.0, 60.0)]),
        Subpath::polygon(&[(40.0, 40.0), (100.0, 40.0), (100.0, 100.0), (40.0, 100.0)]).with_op(PathOp::Exclude),
    ]);
    let r = app.run("shape.presets.new", json!({"name": "Xor", "path": photocraft_engine::vector_cmds::path_json(&xor)})).unwrap();
    app.ui.tool = Tool::CustomShape;
    app.ui.presets_ui.custom_shape = r["name"].as_str().unwrap().into();
    app.ui.tool_options.shape_path_op = "combine".into();
    let layers = layer_count(&app);
    drag(&mut app, [100.0, 100.0], [250.0, 250.0]);
    assert_eq!(layer_count(&app), layers);
    assert_eq!(active_shape(&app).path, before, "the layer is unchanged");
    assert!(app.ui.status_error, "{}", app.ui.status);
}
