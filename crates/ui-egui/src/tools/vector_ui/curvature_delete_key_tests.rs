//! #12695: with the Curvature Pen and a finished path, ⌫/Delete on the selected anchor removes it,
//! and with no anchor selected deletes the Work Path, as the Pen does (#5702); never the layer.

use egui::Modifiers;
use photocraft_doc::vector::Path;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// 800 x 600 on white, pixel layer Red active, and a Work Path drawn with the Curvature Pen
/// through (100,300), (250,150), (400,300), ended with Esc.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.sync_views();
    app.run("layer.new.layer", json!({"name": "Red"})).unwrap();
    app.ui.tool = Tool::CurvaturePen;
    app.ui.tool_options.vector_mode = "path".into();
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
    tool_event(app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: at[0], y: at[1] }, Modifiers::NONE);
}

/// A key through the app's shortcut handling, as the canvas gets it.
fn press(app: &mut PhotocraftApp, key: egui::Key) {
    let ctx = egui::Context::default();
    let m = Modifiers::NONE;
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(m), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: m }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
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
