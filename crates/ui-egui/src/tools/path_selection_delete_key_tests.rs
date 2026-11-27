//! #13269: with Path Selection, Delete removes the selected components of a shape layer, a saved
//! path or a vector mask (or the targeted saved path when none is selected), never the layer.

use egui::{Key, Modifiers};
use serde_json::json;

use crate::PhotocraftApp;
use crate::direct_select::PathRef;
use crate::state::Tool;

const SQUARE: [[i32; 2]; 4] = [[100, 300], [300, 300], [300, 500], [100, 500]];
const TRIANGLE: [[i32; 2]; 3] = [[400, 300], [600, 300], [500, 500]];

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app
}

fn two_components() -> serde_json::Value {
    json!({"subpaths": [{"closed": true, "knots": SQUARE}, {"closed": true, "knots": TRIANGLE}]})
}

fn path_selection(app: &mut PhotocraftApp) {
    app.sync_views();
    app.ui.tool = Tool::PathSelection;
}

fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

fn subpaths(path: &photocraft_doc::Path) -> usize {
    path.subpaths.len()
}

fn press_delete(app: &mut PhotocraftApp) {
    crate::canvas::testing::key(app, Key::Delete, Modifiers::NONE);
}

#[test]
fn delete_removes_the_selected_component_of_a_saved_path() {
    let mut app = app();
    app.run("path.set", json!({"name": "Path 1", "path": two_components()})).unwrap();
    path_selection(&mut app);
    app.ui.selected_path = Some("Path 1".into());
    crate::path_components::press(&mut app, [300.0, 300.0], false);
    assert_eq!(crate::path_components::selected(&app).map(|s| s.0), Some(PathRef::Saved(0)));
    let n = layers(&app);
    press_delete(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!(subpaths(&st.doc.paths[0].path), 1, "the triangle stays");
    assert_eq!(layers(&app), n, "the layer stays");
    assert!(crate::path_components::selected(&app).is_none(), "the stale selection drops");
    // With nothing selected, Delete removes the targeted saved path.
    press_delete(&mut app);
    assert!(app.session.active().unwrap().doc.paths.is_empty());
    assert_eq!(layers(&app), n);
}

#[test]
fn delete_removes_the_selected_component_of_a_shape_layer() {
    let mut app = app();
    let id = app.run("shape.create", json!({"kind": "rect", "rect": [100, 300, 200, 200], "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
    app.run("shape.create", json!({"kind": "ellipse", "rect": [400, 300, 200, 200], "addTo": id, "op": "subtract"})).unwrap();
    path_selection(&mut app);
    let n = layers(&app);
    crate::path_components::press(&mut app, [400.0, 400.0], false);
    assert!(matches!(crate::path_components::selected(&app), Some((PathRef::Layer(l), ..)) if l == id), "the ellipse is selected");
    press_delete(&mut app);
    assert_eq!(layers(&app), n, "the layer stays");
    let st = app.session.active().unwrap();
    let photocraft_doc::LayerContent::Shape(sh) = &st.doc.layer(photocraft_doc::LayerId(id)).unwrap().content else { panic!("shape") };
    assert_eq!(subpaths(&sh.path), 1, "only the ellipse went");
}

#[test]
fn delete_removes_the_selected_component_of_a_vector_mask() {
    let mut app = app();
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    app.run("layer.vectorMask.add", json!({"layer": id, "path": two_components()})).unwrap();
    app.ui.vector_mask_target = true;
    path_selection(&mut app);
    let n = layers(&app);
    crate::path_components::press(&mut app, [300.0, 300.0], false);
    assert!(matches!(crate::path_components::selected(&app), Some((PathRef::Layer(l), ..)) if l == id), "the square is selected");
    press_delete(&mut app);
    assert_eq!(layers(&app), n, "the layer stays");
    let st = app.session.active().unwrap();
    let mask = st.doc.layer(photocraft_doc::LayerId(id)).unwrap().vector_mask.as_ref().expect("the mask stays");
    assert_eq!(subpaths(&mask.path), 1, "the triangle stays");
}
