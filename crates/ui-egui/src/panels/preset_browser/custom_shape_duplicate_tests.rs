//! #14291: two custom shapes can share a name, so the Shapes panel and the Shape picker key each shape on its own:
//! renaming, deleting and placing one of two "TriS" from the panel or the Custom Shape tool changes or uses only that
//! one. A name no other shape has stays the key, as before.

use photocraft_doc::{LayerContent, LayerId};
use photocraft_engine::presets::shapes::{CUSTOM_GROUP, all_groups};
use serde_json::json;

use super::{Act, PresetEvent, PresetFooterEvent, PresetKey, SHAPES, act, apply, finish_custom_shape, shape_keys, shape_place, shape_ref};
use crate::PhotocraftApp;

/// An app with two custom shapes named "TriS": a triangle, then a square.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    for knots in [json!([[10, 10], [90, 10], [50, 80]]), json!([[100, 100], [200, 100], [200, 200], [100, 200]])] {
        app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": knots}]}})).unwrap();
        app.run("edit.defineCustomShape", json!({"name": "TriS"})).unwrap();
    }
    app
}

/// The panel keys of the custom group.
fn custom_keys(app: &PhotocraftApp) -> Vec<String> {
    let groups = all_groups(&app.session);
    let at = groups.iter().position(|g| g.name == CUSTOM_GROUP).unwrap();
    shape_keys(&groups).swap_remove(at)
}

fn knots(path: &photocraft_doc::Path) -> usize {
    path.subpaths.iter().map(|sp| sp.knots.len()).sum()
}

fn library(app: &PhotocraftApp) -> Vec<(String, usize)> {
    photocraft_engine::edit_menu_cmds::custom_shapes(&app.session).iter().map(|c| (c.name.clone(), knots(&c.path))).collect()
}

/// Runs what the Shapes panel does for `event`.
fn perform(app: &mut PhotocraftApp, event: PresetEvent<PresetKey>) {
    let Some(Act::Run(command, params)) = act(SHAPES, &event, &[], None) else { panic!("{event:?} runs no command") };
    app.run(&command, params).unwrap();
}

fn active_layer(app: &PhotocraftApp) -> photocraft_doc::LayerId {
    app.session.active().unwrap().active_layer.unwrap()
}

fn active_shape_knots(app: &PhotocraftApp) -> usize {
    let st = app.session.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        LayerContent::Shape(sh) => knots(&sh.path),
        other => panic!("not a shape layer: {other:?}"),
    }
}

#[test]
fn shapes_of_one_name_get_their_own_keys() {
    let app = app();
    let keys = custom_keys(&app);
    assert_eq!(keys.len(), 2);
    assert_ne!(keys[0], keys[1]);
    assert_eq!(shape_ref(&keys[1], true), json!({"preset": "TriS", "group": CUSTOM_GROUP, "position": 1}));
    // A unique name is still its own key.
    let groups = all_groups(&app.session);
    let heart = shape_keys(&groups).into_iter().flatten().find(|k| k == "Heart");
    assert_eq!(heart.as_deref(), Some("Heart"));
    assert_eq!(shape_ref("Heart", true), json!({"preset": "Heart"}));
}

#[test]
fn renaming_the_second_renames_only_it() {
    let mut app = app();
    let second = custom_keys(&app)[1].clone();
    perform(&mut app, PresetEvent::rename(PresetKey::Item(second), "Square"));
    assert_eq!(library(&app), [("TriS".to_string(), 3), ("Square".to_string(), 4)]);
}

#[test]
fn deleting_the_second_deletes_only_it() {
    let mut app = app();
    let second = custom_keys(&app)[1].clone();
    perform(&mut app, PresetEvent::Delete(PresetKey::Item(second)));
    assert_eq!(library(&app), [("TriS".to_string(), 3)]);
}

#[test]
fn placing_the_second_places_it() {
    let mut app = app();
    let second = custom_keys(&app)[1].clone();
    let r = app.run("shape.presets.place", shape_place(&app, &second, None)).unwrap();
    assert!(r["layer"].as_u64().map(LayerId).is_some());
    assert_eq!(active_shape_knots(&app), 4, "the panel places the square");
    // The Custom Shape tool draws the shape the panel or picker chose.
    let placed = active_layer(&app);
    app.ui.presets_ui.custom_shape = second;
    finish_custom_shape(&mut app, [20.0, 20.0, 100.0, 100.0], true, json!(null), json!(null));
    assert_ne!(active_layer(&app), placed, "the tool draws a new layer");
    assert_eq!(active_shape_knots(&app), 4, "the tool draws the square");
    let square = active_layer(&app);
    app.ui.presets_ui.custom_shape = custom_keys(&app)[0].clone();
    finish_custom_shape(&mut app, [20.0, 20.0, 100.0, 100.0], true, json!(null), json!(null));
    assert_ne!(active_layer(&app), square, "the first draws a layer of its own");
    assert_eq!(active_shape_knots(&app), 3, "and the triangle for the first");
}

/// A key whose position went stale (the other "TriS" was deleted) draws the shape of that name.
#[test]
fn a_stale_key_falls_back_to_the_name() {
    let mut app = app();
    let [first, second] = <[String; 2]>::try_from(custom_keys(&app)).unwrap();
    perform(&mut app, PresetEvent::Delete(PresetKey::Item(first)));
    app.ui.presets_ui.custom_shape = second;
    finish_custom_shape(&mut app, [20.0, 20.0, 100.0, 100.0], true, json!(null), json!(null));
    assert_eq!(active_shape_knots(&app), 4);
    assert!(!app.ui.status_error, "no error left behind: {}", app.ui.status);
}

/// Renaming the selected second "TriS" keeps it selected by its own key, also when the new name is another shape's:
/// the footer's Delete then deletes it, not the first "TriS".
#[test]
fn a_renamed_selection_keeps_its_shape() {
    let mut app = app();
    let ctx = egui::Context::default();
    let panel = |app: &mut PhotocraftApp, event| apply(app, &ctx, SHAPES, &[], &[], vec![event]);
    let second = custom_keys(&app)[1].clone();
    app.ui.presets_ui.selected.insert(SHAPES.id.into(), second.clone());
    panel(&mut app, PresetEvent::rename(PresetKey::Item(second.clone()), "TriS"));
    assert_eq!(app.ui.presets_ui.selected.get(SHAPES.id), Some(&second), "renamed to its own name");
    panel(&mut app, PresetEvent::rename(PresetKey::Item(second.clone()), "Square"));
    assert_eq!(app.ui.presets_ui.selected.get(SHAPES.id).map(String::as_str), Some("Square"), "a unique name is its key");
    panel(&mut app, PresetEvent::rename(PresetKey::Item("Square".into()), "TriS"));
    assert_eq!(app.ui.presets_ui.selected.get(SHAPES.id), Some(&second), "a shared name keys it by its position");
    panel(&mut app, PresetEvent::Footer(PresetFooterEvent::Delete));
    assert_eq!(library(&app), [("TriS".to_string(), 3)], "Delete deletes the renamed square");
}

/// A Custom Shape tool drag with a path operation adds the chosen one of two "TriS" to the active shape layer.
#[test]
fn a_drag_with_a_path_operation_adds_the_chosen_shape() {
    use crate::tools::vector_ui::shapes::{finish_drag, kind_of};
    use craft::draw::geometry::ShapeGeometry;
    let mut app = app();
    app.ui.tool = crate::Tool::CustomShape;
    let second = custom_keys(&app)[1].clone();
    app.run("shape.presets.place", shape_place(&app, &second, None)).unwrap();
    let (layer, layers) = (active_layer(&app), app.session.active().unwrap().doc.walk().len());
    app.ui.presets_ui.custom_shape = second;
    app.ui.tool_options.shape_path_op = "combine".into();
    let geometry = ShapeGeometry::Custom { rect: craft::behavior::kurbo::Rect::new(20.0, 20.0, 120.0, 120.0), keep_proportions: true };
    finish_drag(&mut app, kind_of(crate::Tool::CustomShape), &geometry, craft::keymap::Mods::default());
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().doc.walk().len(), layers, "no new layer");
    assert_eq!(active_layer(&app), layer);
    assert_eq!(active_shape_knots(&app), 8, "the square joins the square");
}
