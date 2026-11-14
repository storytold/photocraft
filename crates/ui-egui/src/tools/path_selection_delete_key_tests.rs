use egui::{Key, Modifiers};
use photocraft_doc::{LayerContent, LayerId, Path};
use serde_json::{Value, json};

use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;
use crate::{PhotocraftApp, Services};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 220, "height": 180, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Red"})).unwrap();
    app.ui.tool = Tool::PathSelection;
    app.ui.extras.snap = false;
    app.sync_views();
    app
}

fn geometry() -> Value {
    json!({"subpaths": [
        {"closed": true, "knots": [[10,10], [80,10], [80,80], [10,80]]},
        {"closed": true, "op": "join", "knots": [[30,30], [30,60], [60,60], [60,30]]},
        {"closed": true, "knots": [[110,10], [180,10], [180,80], [110,80]]}
    ]})
}

fn saved(app: &mut PhotocraftApp, name: &str) {
    app.run("path.set", json!({"name": name, "path": geometry()})).unwrap();
    app.ui.selected_path = Some(name.into());
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64, mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x, y }, mods);
}

fn press(app: &mut PhotocraftApp, key: Key) {
    key_event(app, &egui::Context::default(), key, true);
}

fn key_event(app: &mut PhotocraftApp, ctx: &egui::Context, key: Key, pressed: bool) {
    let mods = Modifiers::NONE;
    let events = vec![egui::Event::ModifiersChanged(mods), egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: mods }];
    ctx.begin_pass(egui::RawInput { events, ..Default::default() });
    crate::shortcuts::handle(app, ctx);
    ctx.end_pass().textures_delta.clear();
}

fn saved_path(app: &PhotocraftApp) -> &Path {
    &app.session.active().unwrap().doc.paths.iter().find(|p| p.name == "Path 1").unwrap().path
}

fn shape_or_mask(app: &mut PhotocraftApp, mask: bool) -> LayerId {
    if mask {
        app.run("path.set", json!({"name": "Mask source", "path": geometry()})).unwrap();
        app.run("layer.vectorMask.fromPath", json!({"name": "Mask source"})).unwrap();
        app.ui.vector_mask_target = true;
    } else {
        app.run("shape.create", json!({"kind": "path", "path": geometry(), "fill": "#ff0000"})).unwrap();
    }
    app.ui.selected_path = Some("layer".into());
    app.session.active().unwrap().active_layer.unwrap()
}

fn layer_path(app: &PhotocraftApp, id: LayerId) -> &Path {
    let layer = app.session.active().unwrap().doc.layer(id).unwrap();
    match &layer.content {
        LayerContent::Shape(shape) => &shape.path,
        _ => &layer.vector_mask.as_ref().unwrap().path,
    }
}

#[test]
fn deleting_a_saved_component_and_its_joined_hole_keeps_every_layer() {
    for key in [Key::Delete, Key::Backspace] {
        let mut app = app();
        saved(&mut app, "Path 1");
        let before = saved_path(&app).clone();
        click(&mut app, 20.0, 20.0, Modifiers::NONE);
        assert_eq!(super::selected(&app).unwrap().2, [0]);
        let active = app.session.active().unwrap();
        let (layers, steps) = (active.doc.layers.clone(), active.history.past_len());
        press(&mut app, key);
        let active = app.session.active().unwrap();
        assert_eq!(active.doc.layers, layers);
        assert_eq!(saved_path(&app).subpaths, before.subpaths[2..]);
        assert_eq!((active.history.past_len(), active.history.undo_label()), (steps + 1, Some("Delete Selection")));
        assert!(app.ui.direct_selection.anchors.is_empty());
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(saved_path(&app), &before);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(saved_path(&app).subpaths, before.subpaths[2..]);
    }
}

#[test]
fn without_components_delete_removes_only_the_targeted_saved_path() {
    let mut app = app();
    saved(&mut app, "Keep");
    saved(&mut app, "Path 1");
    let active = app.session.active().unwrap();
    let (layers, steps) = (active.doc.layers.clone(), active.history.past_len());
    press(&mut app, Key::Delete);
    let active = app.session.active().unwrap();
    assert_eq!(active.doc.layers, layers);
    assert_eq!(active.doc.paths.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Keep"]);
    assert_eq!((active.history.past_len(), active.history.undo_label()), (steps + 1, Some("Delete Path")));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(saved_path(&app).subpaths.len(), 3);
}

#[test]
fn shape_paths_and_vector_masks_keep_the_layer_and_unselected_components() {
    for mask in [false, true] {
        let mut app = app();
        let id = shape_or_mask(&mut app, mask);
        let before = layer_path(&app, id).clone();
        click(&mut app, 145.0, 45.0, Modifiers::NONE);
        assert_eq!(super::selected(&app).unwrap().2, [2]);
        let active = app.session.active().unwrap();
        let (count, steps) = (active.doc.layer_count(), active.history.past_len());
        press(&mut app, Key::Backspace);
        let active = app.session.active().unwrap();
        assert_eq!((active.doc.layer_count(), active.active_layer), (count, Some(id)));
        assert_eq!(layer_path(&app, id).subpaths, before.subpaths[..2]);
        assert_eq!((active.history.past_len(), active.history.undo_label()), (steps + 1, Some("Delete Selection")));
    }
}

#[test]
fn a_newly_locked_shape_selection_cannot_fall_through_to_layer_delete() {
    let mut app = app();
    let id = shape_or_mask(&mut app, false);
    click(&mut app, 145.0, 45.0, Modifiers::NONE);
    assert_eq!(super::selected(&app).unwrap().2, [2]);
    app.session
        .edit("Lock shape", |doc, _| {
            doc.layer_mut(id).unwrap().locks.pixels = true;
            Ok(())
        })
        .unwrap();
    let active = app.session.active().unwrap();
    let (layers, steps) = (active.doc.layers.clone(), active.history.past_len());
    press(&mut app, Key::Delete);
    let active = app.session.active().unwrap();
    assert_eq!(active.doc.layers, layers);
    assert_eq!(active.history.past_len(), steps);
}

#[test]
fn deleting_all_saved_components_keeps_an_empty_saved_path() {
    let mut app = app();
    saved(&mut app, "Path 1");
    click(&mut app, 20.0, 20.0, Modifiers::NONE);
    click(&mut app, 145.0, 45.0, Modifiers::SHIFT);
    assert_eq!(super::selected(&app).unwrap().2, [0, 2]);
    press(&mut app, Key::Delete);
    assert!(saved_path(&app).subpaths.is_empty());
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Selection"));
}

#[test]
fn deleting_the_last_layer_component_keeps_the_shape_or_mask_intact() {
    for mask in [false, true] {
        let mut app = app();
        let id = shape_or_mask(&mut app, mask);
        click(&mut app, 20.0, 20.0, Modifiers::NONE);
        click(&mut app, 145.0, 45.0, Modifiers::SHIFT);
        assert_eq!(super::selected(&app).unwrap().2, [0, 2]);
        let active = app.session.active().unwrap();
        let (layers, steps) = (active.doc.layers.clone(), active.history.past_len());
        press(&mut app, Key::Delete);
        let active = app.session.active().unwrap();
        assert_eq!(active.doc.layers, layers);
        assert_eq!(active.active_layer, Some(id));
        assert_eq!(active.history.past_len(), steps);
        assert!(app.ui.status_error);
    }
}

#[test]
fn replacing_a_saved_path_invalidates_its_component_selection() {
    let mut app = app();
    saved(&mut app, "Path 1");
    click(&mut app, 20.0, 20.0, Modifiers::NONE);
    assert_eq!(super::selected(&app).unwrap().2, [0]);
    app.session.execute("path.set", json!({"name":"Path 1","path":{"subpaths":[{"closed":true,"knots":[[90,90],[120,90],[120,130],[90,130]]}]}})).unwrap();
    let before = saved_path(&app).clone();
    assert!(super::selected(&app).is_none());
    let active = app.session.active().unwrap();
    let (layers, steps) = (active.doc.layers.clone(), active.history.past_len());
    press(&mut app, Key::Delete);
    let active = app.session.active().unwrap();
    assert_eq!(active.doc.layers, layers);
    assert!(active.doc.paths.is_empty());
    assert_eq!((active.history.past_len(), active.history.undo_label()), (steps + 1, Some("Delete Path")));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(saved_path(&app), &before);
}

#[test]
fn the_finished_work_path_still_uses_delete_path_even_with_a_selection() {
    let mut app = app();
    saved(&mut app, "work");
    click(&mut app, 20.0, 20.0, Modifiers::NONE);
    assert_eq!(super::selected(&app).unwrap().2, [0]);
    let layers = app.session.active().unwrap().doc.layers.clone();
    press(&mut app, Key::Delete);
    let active = app.session.active().unwrap();
    assert!(active.doc.work_path.is_none());
    assert_eq!(active.doc.layers, layers);
    assert_eq!(active.history.undo_label(), Some("Delete Path"));
}

#[test]
fn deleting_with_type_path_components_selected_keeps_the_type_layer() {
    for key in [Key::Delete, Key::Backspace] {
        let mut app = app();
        app.run("path.set", json!({"path": geometry()})).unwrap();
        let id = app.run("type.create", json!({"x": 20, "y": 70, "text": "On a path", "size": 18})).unwrap()["layer"].as_u64().unwrap();
        app.run("type.attachToPath", json!({})).unwrap();
        app.ui.tool = Tool::PathSelection;
        let (target, path) = crate::vector_ui::active_type_path(&app).unwrap();
        assert_eq!(target, id);
        let anchor = path.subpaths[0].knots[0].anchor;
        super::press(&mut app, [anchor.x, anchor.y], false);
        assert_eq!(super::selected(&app).unwrap().0, crate::direct_select::PathRef::TypePath(id));
        for locked in [false, true] {
            if locked {
                app.session
                    .edit("Lock type layer", |doc, _| {
                        doc.layer_mut(LayerId(id)).unwrap().locks.pixels = true;
                        Ok(())
                    })
                    .unwrap();
            }
            let active = app.session.active().unwrap();
            let (doc, steps) = (active.doc.clone(), active.history.past_len());
            press(&mut app, key);
            let active = app.session.active().unwrap();
            assert_eq!(active.doc, doc);
            assert_eq!(active.history.past_len(), steps);
        }
    }
}

#[test]
fn path_delete_uses_the_modifiers_on_the_key_press() {
    for key in [Key::Backspace, Key::Delete] {
        for (at_press, at_end, deletes) in [(Modifiers::NONE, Modifiers::SHIFT, true), (Modifiers::SHIFT, Modifiers::NONE, false)] {
            let mut app = app();
            saved(&mut app, "Path 1");
            click(&mut app, 20.0, 20.0, Modifiers::NONE);
            let before = saved_path(&app).clone();
            let ctx = egui::Context::default();
            let events = vec![
                egui::Event::ModifiersChanged(at_press),
                egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: at_press },
                egui::Event::ModifiersChanged(at_end),
            ];
            ctx.begin_pass(egui::RawInput { events, ..Default::default() });
            assert_eq!(super::handle_delete_key(&mut app, &ctx), deletes, "{key:?}, {at_press:?}");
            ctx.end_pass().textures_delta.clear();
            if deletes {
                assert_eq!(saved_path(&app).subpaths.len(), 1);
            } else {
                assert_eq!(saved_path(&app), &before);
            }
        }
    }
}

#[test]
fn a_held_path_delete_preserves_the_remaining_path_and_layers() {
    for key in [Key::Delete, Key::Backspace] {
        for target in ["saved component", "saved path", "shape"] {
            let mut app = app();
            if target == "shape" {
                shape_or_mask(&mut app, false);
            } else {
                saved(&mut app, "Path 1");
            }
            if target != "saved path" {
                click(&mut app, 20.0, 20.0, Modifiers::NONE);
                assert_eq!(super::selected(&app).unwrap().2, [0]);
            }
            let ctx = egui::Context::default();
            let steps = app.session.active().unwrap().history.past_len();
            key_event(&mut app, &ctx, key, true);
            let active = app.session.active().unwrap();
            let after_press = active.doc.clone();
            assert_eq!(active.history.past_len(), steps + 1, "{key:?}, {target}");
            // egui marks presses of a key it already has down as repeats.
            for _ in 0..2 {
                key_event(&mut app, &ctx, key, true);
                let active = app.session.active().unwrap();
                assert_eq!(active.doc, after_press, "{key:?}, {target}");
                assert_eq!(active.history.past_len(), steps + 1, "{key:?}, {target}");
            }
            key_event(&mut app, &ctx, key, false);
            saved(&mut app, "Next");
            key_event(&mut app, &ctx, key, true);
            let active = app.session.active().unwrap();
            assert_eq!(active.history.undo_label(), Some("Delete Path"));
            assert!(active.doc.paths.iter().all(|p| p.name != "Next"));
        }
    }
}
