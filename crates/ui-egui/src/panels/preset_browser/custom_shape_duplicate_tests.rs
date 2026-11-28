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
    app_named("TriS", "TriS")
}

/// An app with two custom shapes: a triangle named `triangle`, then a square named `square`.
fn app_named(triangle: &str, square: &str) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    for (name, knots) in [(triangle, json!([[10, 10], [90, 10], [50, 80]])), (square, json!([[100, 100], [200, 100], [200, 200], [100, 200]]))] {
        app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": knots}]}})).unwrap();
        app.run("edit.defineCustomShape", json!({"name": name})).unwrap();
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

/// Renaming another shape to the selected shape's name keys both by position, and the selection and the Custom Shape
/// tool stay on the selected shape: the tool draws it and the footer's Delete deletes it, not the renamed one.
#[test]
fn renaming_another_shape_to_the_selected_name_keeps_the_selection() {
    let mut app = app_named("A", "B");
    let ctx = egui::Context::default();
    app.ui.presets_ui.selected.insert(SHAPES.id.into(), "B".into());
    app.ui.presets_ui.custom_shape = "B".into();
    apply(&mut app, &ctx, SHAPES, &[], &[], vec![PresetEvent::rename(PresetKey::Item("A".into()), "B")]);
    let square = custom_keys(&app)[1].clone();
    assert_ne!(square, "B", "a shared name is keyed by position");
    assert_eq!(app.ui.presets_ui.selected.get(SHAPES.id), Some(&square), "the selection stays on the square");
    assert_eq!(app.ui.presets_ui.custom_shape, square, "the tool keeps the square");
    finish_custom_shape(&mut app, [20.0, 20.0, 100.0, 100.0], true, json!(null), json!(null));
    assert_eq!(active_shape_knots(&app), 4, "the tool draws the square");
    apply(&mut app, &ctx, SHAPES, &[], &[], vec![PresetEvent::Footer(PresetFooterEvent::Delete)]);
    assert_eq!(library(&app), [("B".to_string(), 3)], "Delete deletes the square");
}

/// Renaming one of two "TriS" away gives the other back its name as its key, and the selection and the Custom Shape
/// tool on it follow, so the panel still shows it selected.
#[test]
fn renaming_a_duplicate_away_gives_the_selection_its_name_back() {
    let mut app = app();
    let ctx = egui::Context::default();
    let [first, second] = <[String; 2]>::try_from(custom_keys(&app)).unwrap();
    app.ui.presets_ui.selected.insert(SHAPES.id.into(), second.clone());
    app.ui.presets_ui.custom_shape = second;
    apply(&mut app, &ctx, SHAPES, &[], &[], vec![PresetEvent::rename(PresetKey::Item(first), "Tri")]);
    assert_eq!(custom_keys(&app), ["Tri", "TriS"]);
    assert_eq!(app.ui.presets_ui.selected.get(SHAPES.id).map(String::as_str), Some("TriS"), "the selection is the square's key");
    assert_eq!(app.ui.presets_ui.custom_shape, "TriS", "the tool's shape is the square's key");
}

/// Renaming the group of a selected shape whose name a custom shape shares keeps the selection and the tool on it under
/// the group's new name.
#[test]
fn renaming_the_group_of_a_shared_name_keeps_the_selection() {
    let mut app = app_named("Heart", "B");
    let ctx = egui::Context::default();
    let groups = all_groups(&app.session);
    let (gi, ii) = groups.iter().enumerate().find_map(|(gi, g)| Some((gi, g.items.iter().position(|s| s.name == "Heart")?))).unwrap();
    let group = groups[gi].name.clone();
    let heart = shape_keys(&groups)[gi][ii].clone();
    assert_ne!(heart, "Heart", "the preset Heart shares its name with the custom one");
    app.ui.presets_ui.selected.insert(SHAPES.id.into(), heart.clone());
    app.ui.presets_ui.custom_shape = heart;
    apply(&mut app, &ctx, SHAPES, &[], &[], vec![PresetEvent::rename(PresetKey::Group(group), "Mine")]);
    let groups = all_groups(&app.session);
    assert_eq!(groups[gi].name, "Mine");
    let heart = shape_keys(&groups)[gi][ii].clone();
    assert_eq!(app.ui.presets_ui.selected.get(SHAPES.id), Some(&heart));
    assert_eq!(app.ui.presets_ui.custom_shape, heart);
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
