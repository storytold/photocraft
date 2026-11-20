//! #12734: with the Convert Point tool, ⌫/Delete on an anchor selected with ⌘/Ctrl+click removes
//! the anchor and its two segments (a closed subpath opens there), never the active layer.

use egui::Modifiers;
use photocraft_doc::vector::Path;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

/// 800 x 600 on white, pixel layer Red active, saved path Path 1 (a closed corner square)
/// targeted in the Paths panel, the Convert Point tool on.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.sync_views();
    app.run("layer.new.layer", json!({"name": "Red"})).unwrap();
    let square = json!({"subpaths": [{"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]}]});
    app.run("path.set", json!({"name": "Path 1", "path": square})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    app.ui.tool = Tool::ConvertPoint;
    app
}

fn path1(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.paths.iter().find(|p| p.name == "Path 1").unwrap().path.clone()
}

fn layer_names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.layers.iter().map(|l| l.name.clone()).collect()
}

fn ctrl_click(app: &mut PhotocraftApp, at: [f64; 2]) {
    testing::click(app, at, Modifiers::COMMAND);
}

/// A key through the app's frame, as the canvas gets it: the overlay's session keys first, then
/// the app's key route.
fn press(app: &mut PhotocraftApp, key: egui::Key) {
    testing::key(app, key, Modifiers::NONE);
}

fn anchors(path: &Path) -> Vec<[f64; 2]> {
    path.subpaths.iter().flat_map(|sp| sp.knots.iter().map(|k| [k.anchor.x, k.anchor.y])).collect()
}

#[test]
fn delete_on_a_selected_anchor_removes_it_and_its_segments_not_the_layer() {
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let mut app = app();
        let (before, names) = (path1(&app), layer_names(&app));
        let n = app.session.active().unwrap().history.past_len();
        ctrl_click(&mut app, [300.0, 300.0]);
        assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]], "{key:?}: Ctrl+click selects the anchor");
        press(&mut app, key);
        assert_eq!(layer_names(&app), names, "{key:?}: every layer is still there");
        let p = path1(&app);
        assert_eq!((p.subpaths.len(), p.subpaths[0].closed), (1, false), "{key:?}: the square opens");
        assert_eq!(anchors(&p), vec![[300.0, 500.0], [100.0, 500.0], [100.0, 300.0]], "{key:?}");
        let st = app.session.active().unwrap();
        assert_eq!((st.history.past_len(), st.history.undo_label()), (n + 1, Some("Delete Anchor Point")), "{key:?}: one step");
        assert!(app.session.undo());
        assert_eq!(path1(&app), before, "{key:?}: Undo restores the closed square");
    }
}

#[test]
fn without_a_selected_anchor_delete_keeps_its_usual_route() {
    let mut app = app();
    let before = path1(&app);
    press(&mut app, egui::Key::Delete);
    assert_eq!(path1(&app), before, "no anchor selected: the path stays");
    assert_ne!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
}

#[test]
fn the_pen_tools_and_path_selection_keep_their_delete() {
    for tool in [Tool::Pen, Tool::FreeformPen, Tool::PathSelection] {
        let mut app = app();
        ctrl_click(&mut app, [300.0, 300.0]);
        app.ui.tool = tool;
        let before = path1(&app);
        press(&mut app, egui::Key::Delete);
        assert_eq!(path1(&app), before, "{tool:?}: no anchor is deleted");
    }
}
