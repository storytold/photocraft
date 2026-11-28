//! #13456: the Shape tools' Path alignment entries move the components of the path the tool draws
//! into (the active shape layer in Shape mode, the Work Path in Path mode), one History step each.

use craft::draw::ui::VectorMode;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::LayerContent;
use photocraft_doc::vector::Path;
use serde_json::json;

use super::path_menus::{PATH_ALIGN_ENTRIES, PATH_DISTRIBUTE_ENTRIES, PATH_SPACING_ENTRIES};
use crate::PhotocraftApp;
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

/// A 400 × 400 document with the Ellipse tool active.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app.ui.tool = Tool::EllipseShape;
    app
}

/// The Ellipse tool's options bar on `app`, its fonts loaded.
fn bar(app: Box<PhotocraftApp>) -> H {
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1600.0, 700.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.horizontal(|ui| super::options_bar(app, ui, Tool::EllipseShape));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::studio());
    h.run_steps(4);
    h
}

/// Opens the Path alignment popover unless it is already open.
fn open_alignment(h: &mut H) {
    if h.query_by_label("Align Left Edges").is_none() {
        h.get_by_label("Path alignment").click();
        h.run_steps(3);
    }
}

/// Every Path alignment entry, in the popover's order.
fn alignment_labels() -> Vec<&'static str> {
    PATH_ALIGN_ENTRIES.iter().chain(&PATH_DISTRIBUTE_ENTRIES).chain(&PATH_SPACING_ENTRIES).map(|(l, _)| *l).collect()
}

/// Is the entry `label` enabled?
fn enabled(h: &H, label: &str) -> bool {
    !h.get_by_label(label).accesskit_node().is_disabled()
}

/// The rectangles `[x0, y0, x1, y1]` as one shape layer's components, made the active layer.
fn shape_of(app: &mut PhotocraftApp, rects: &[[f64; 4]]) {
    let subpaths: Vec<_> = rects.iter().map(|[x0, y0, x1, y1]| json!({"closed": true, "knots": [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]})).collect();
    app.run("shape.create", json!({"kind": "path", "path": {"subpaths": subpaths}, "fill": "#ff0000"})).unwrap();
}

/// The active shape layer's path.
fn active_path(app: &PhotocraftApp) -> Path {
    let st = app.session.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        LayerContent::Shape(s) => s.path.clone(),
        _ => panic!("the active layer is not a shape layer"),
    }
}

/// The first corner of each subpath of `path`.
fn corners_of(path: &Path) -> Vec<[f64; 2]> {
    path.subpaths.iter().map(|s| [s.knots[0].anchor.x, s.knots[0].anchor.y]).collect()
}

/// Three components: A 10..30 × 20..60, B 60..100 × 40..50, C 120..150 × 100..140.
const THREE: [[f64; 4]; 3] = [[10.0, 20.0, 30.0, 60.0], [60.0, 40.0, 100.0, 50.0], [120.0, 100.0, 150.0, 140.0]];

/// Without a shape every entry is greyed.
#[test]
fn entries_are_greyed_without_a_shape() {
    let mut h = bar(app());
    open_alignment(&mut h);
    for label in alignment_labels() {
        assert!(!enabled(&h, label), "{label}");
    }
}

/// Align is enabled with two components, Distribute and Distribute Spacing with three, and greyed
/// below that; Align To Canvas aligns even one.
#[test]
fn entries_are_enabled_by_the_shape_layers_components() {
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
    let mut app = app();
    shape_of(&mut app, &THREE[..1]);
    app.ui.path_align_to = 1;
    let mut h = bar(app);
    open_alignment(&mut h);
    assert!(enabled(&h, "Align Right Edges"));
    assert!(!enabled(&h, "Distribute Right Edges"));
    h.get_by_label("Align Right Edges").click();
    h.run_steps(2);
    assert_eq!(corners_of(&active_path(h.state())), [[380.0, 20.0]], "right edge on the 400 px canvas");
}

/// Each entry moves the shape layer's components as `path.alignComponents` and
/// `path.distributeComponents` do, in one undoable History step.
#[test]
fn each_entry_moves_the_components() {
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
    let before = active_path(&app);
    let mut h = bar(app);
    for (label, after) in want {
        let steps = h.state().session.active().unwrap().history.entries().len();
        open_alignment(&mut h);
        h.get_by_label(label).click();
        h.run_steps(2);
        assert_eq!(corners_of(&active_path(h.state())), after, "{label}");
        let st = h.state().session.active().unwrap();
        assert_eq!(st.history.entries().len(), steps + 1, "{label}: one step");
        let name = if label.starts_with("Align") { "Align Path Components" } else { "Distribute Path Components" };
        assert_eq!(st.history.undo_label(), Some(name), "{label}");
        assert!(h.state_mut().session.undo(), "{label}: undo");
        assert_eq!(active_path(h.state()), before, "{label}: undone");
        assert!(h.state_mut().session.redo(), "{label}: redo");
        assert_eq!(corners_of(&active_path(h.state())), after, "{label}: redone");
        assert!(h.state_mut().session.undo());
        h.run_steps(1);
    }
}

/// In Path mode the entries act on the Work Path, the path a Shape tool draws into.
#[test]
fn path_mode_aligns_the_work_path() {
    let mut app = app();
    app.ui.tool_options.shape_mode = VectorMode::Path;
    let path = json!({"subpaths": [{"knots": [[10, 10], [30, 10], [30, 30]]}, {"knots": [[50, 50], [90, 50], [90, 80]]}]});
    app.run("path.set", json!({"path": path})).unwrap();
    let mut h = bar(app);
    open_alignment(&mut h);
    h.get_by_label("Align Bottom Edges").click();
    h.run_steps(2);
    let work = h.state().session.active().unwrap().doc.work_path.clone().unwrap();
    assert_eq!(corners_of(&work), [[10.0, 60.0], [50.0, 50.0]], "bottoms at 80");
}
