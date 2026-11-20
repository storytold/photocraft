//! #10920: with Path Selection and components selected, the arrow keys nudge them 1 px (⇧ 10 px),
//! through the command a drag runs, and they stay selected. A run of nudges is one History step
//! (`nudge_bundle`); with Bundle Arrow-Key Nudges off every press is one.

use egui::{Key, Modifiers};
use photocraft_doc::vector::Path;
use photocraft_doc::{LayerContent, ShapeLayer};
use serde_json::json;

use crate::PhotocraftApp;
use crate::direct_select::PathRef;
use crate::state::Tool;

/// 800 x 600 with the saved path "Path 1" (a square and a triangle) targeted, and the square
/// selected as a component.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    let path = json!({"subpaths": [
        {"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]},
        {"closed": true, "knots": [[400, 300], [600, 300], [500, 500]]}
    ]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PathSelection;
    app.ui.selected_path = Some("Path 1".into());
    crate::path_components::press(&mut app, [300.0, 300.0], false);
    let (target, _, heads) = crate::path_components::selected(&app).expect("the square is selected");
    assert_eq!((target, heads), (PathRef::Saved(0), vec![0]));
    app
}

fn press(app: &mut PhotocraftApp, key: Key, modifiers: Modifiers) {
    crate::canvas::testing::key(app, key, modifiers);
}

fn path_1(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.paths.iter().find(|p| p.name == "Path 1").unwrap().path.clone()
}

/// The anchors of subpath `s`.
fn anchors(path: &Path, s: usize) -> Vec<[f64; 2]> {
    path.subpaths[s].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn square_selected(app: &PhotocraftApp) -> bool {
    crate::path_components::selected(app).is_some_and(|(r, _, heads)| r == PathRef::Saved(0) && heads == [0])
}

#[test]
fn arrows_nudge_the_selected_component_one_pixel_and_shift_ten() {
    let mut app = app();
    app.session.edit_prefs(|p| p.tools.bundle_nudges = false);
    let (before, n) = (path_1(&app), steps(&app));
    press(&mut app, Key::ArrowRight, Modifiers::NONE);
    let after = path_1(&app);
    assert_eq!(anchors(&after, 0), [[101.0, 300.0], [301.0, 300.0], [301.0, 500.0], [101.0, 500.0]], "the square moves 1 px right");
    assert_eq!(anchors(&after, 1), anchors(&before, 1), "the triangle is not selected: it stays");
    assert_eq!(steps(&app), n + 1, "one History step");
    assert!(square_selected(&app), "the square stays selected");
    press(&mut app, Key::ArrowUp, Modifiers::SHIFT);
    assert_eq!(anchors(&path_1(&app), 0), [[101.0, 290.0], [301.0, 290.0], [301.0, 490.0], [101.0, 490.0]], "⇧ moves 10 px");
    assert_eq!(steps(&app), n + 2, "with bundling off, one step per press");
    assert!(square_selected(&app));
    assert!(app.session.undo());
    assert_eq!(anchors(&path_1(&app), 0), anchors(&after, 0), "Undo takes back one press");
    assert!(app.session.undo());
    assert_eq!(path_1(&app), before);
}

#[test]
fn a_run_of_nudges_is_one_history_step() {
    let mut app = app();
    crate::canvas::testing::still_clock(&mut app);
    let (before, n) = (path_1(&app), steps(&app));
    for _ in 0..3 {
        press(&mut app, Key::ArrowDown, Modifiers::NONE);
    }
    press(&mut app, Key::ArrowLeft, Modifiers::SHIFT);
    let after = path_1(&app);
    assert_eq!(anchors(&after, 0), [[90.0, 303.0], [290.0, 303.0], [290.0, 503.0], [90.0, 503.0]]);
    assert_eq!(anchors(&after, 1), anchors(&before, 1));
    assert_eq!(steps(&app), n + 1, "four presses, one state");
    assert!(square_selected(&app));
    assert!(app.session.undo());
    assert_eq!(path_1(&app), before, "one Undo takes back the run");
}

fn shape(app: &PhotocraftApp) -> ShapeLayer {
    let st = app.session.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        LayerContent::Shape(sh) => sh.clone(),
        _ => panic!("not a shape layer"),
    }
}

/// The only component of a live rectangle selected: the whole shape moves, as a drag moves it, and
/// stays live.
#[test]
fn with_every_component_selected_the_whole_live_shape_moves_and_stays_live() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [20, 20, 60, 40], "fill": "#ff0000"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PathSelection;
    app.session.edit_prefs(|p| p.tools.bundle_nudges = false);
    crate::path_components::press(&mut app, [50.0, 40.0], false);
    assert_eq!(crate::path_components::selected(&app).map(|(.., h)| h), Some(vec![0]));
    let n = steps(&app);
    press(&mut app, Key::ArrowRight, Modifiers::SHIFT);
    press(&mut app, Key::ArrowDown, Modifiers::NONE);
    assert_eq!(steps(&app), n + 2);
    let sh = shape(&app);
    let (x0, y0, ..) = sh.path.control_bounds().unwrap();
    assert_eq!((x0.round(), y0.round()), (30.0, 21.0), "moved");
    match sh.live {
        Some(photocraft_doc::vector::LiveShape::Rect { rect, .. }) => assert_eq!(rect, [30.0, 21.0, 60.0, 40.0]),
        other => panic!("still live, got {other:?}"),
    }
    assert!(crate::path_components::selected(&app).is_some(), "the component stays selected");
}

#[test]
fn with_no_component_selected_an_arrow_moves_nothing() {
    let mut app = app();
    crate::path_components::deselect_components(&mut app);
    let (before, n) = (path_1(&app), steps(&app));
    press(&mut app, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(path_1(&app), before);
    assert_eq!(steps(&app), n);
}
