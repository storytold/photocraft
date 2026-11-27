//! #12695: with the Curvature Pen and a finished path, ⌫/Delete on the selected anchor removes it,
//! and with no anchor selected deletes the Work Path, as the Pen does (#5702); never the layer.

use egui::Modifiers;
use photocraft_doc::vector::Path;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

/// 800 x 600 on white, pixel layer Red active, and a Work Path drawn with the Curvature Pen
/// through (100,300), (250,150), (400,300), ended with Esc.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.sync_views();
    app.run("layer.new.layer", json!({"name": "Red"})).unwrap();
    app.ui.tool = Tool::CurvaturePen;
    app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Path;
    for p in [[100.0, 300.0], [250.0, 150.0], [400.0, 300.0]] {
        click(&mut app, p);
    }
    press(&mut app, egui::Key::Escape);
    assert!(app.ui.pen.is_none(), "Esc ended the path");
    assert_eq!(work(&app).map(|p| anchors(&p).len()), Some(3), "the Work Path has the three anchors");
    app
}

fn work(app: &PhotocraftApp) -> Option<Path> {
    app.session.active().unwrap().doc.work_path.clone()
}

fn layer_names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.layers.iter().map(|l| l.name.clone()).collect()
}

fn click(app: &mut PhotocraftApp, at: [f64; 2]) {
    testing::click(app, at, Modifiers::NONE);
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
fn delete_on_the_selected_anchor_removes_it_not_the_layer() {
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let mut app = app();
        let names = layer_names(&app);
        click(&mut app, [400.0, 300.0]);
        assert_eq!(app.ui.direct_selection.anchors, vec![[0, 2]], "{key:?}: the click selects the anchor");
        let n = app.session.active().unwrap().history.past_len();
        press(&mut app, key);
        assert_eq!(layer_names(&app), names, "{key:?}: every layer is still there");
        assert_eq!(work(&app).map(|p| anchors(&p)), Some(vec![[100.0, 300.0], [250.0, 150.0]]), "{key:?}: two anchors are left");
        let st = app.session.active().unwrap();
        assert_eq!((st.history.past_len(), st.history.undo_label()), (n + 1, Some("Delete Anchor Point")), "{key:?}: one step");
    }
}

#[test]
fn without_a_selected_anchor_delete_removes_the_work_path_not_the_layer() {
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let mut app = app();
        assert!(app.ui.direct_selection.anchors.is_empty(), "{key:?}: no anchor selected");
        let names = layer_names(&app);
        press(&mut app, key);
        assert_eq!(layer_names(&app), names, "{key:?}: every layer is still there");
        assert!(work(&app).is_none(), "{key:?}: the Work Path is deleted, as with the Pen");
    }
}
