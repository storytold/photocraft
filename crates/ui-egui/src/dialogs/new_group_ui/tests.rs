use photocraft_color::BlendMode;
use photocraft_doc::{LabelColor, Layer, LayerContent, LayerId};
use serde_json::json;

use super::*;
use crate::new_layer_ui::testing::set;

/// Background, then `A` and `B` (`B` active).
fn app() -> (PhotocraftApp, LayerId, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    let a = LayerId(app.run("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap());
    let b = LayerId(app.run("layer.new.layer", json!({"name": "B"})).unwrap()["layer"].as_u64().unwrap());
    (app, a, b)
}

fn groups(app: &PhotocraftApp) -> Vec<Layer> {
    app.session.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l).filter(|l| l.is_group()).cloned().collect()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn ok_creates_the_group_with_the_options_in_one_step_and_cancel_changes_nothing() {
    let (mut app, _, b) = app();
    let ctx = app.ctx().clone();
    let initial = steps(&app);
    let cancelled = open(&mut app, NEW_GROUP);
    set(&mut app, cancelled, "name", json!("Nope")).unwrap();
    crate::dialogs::cancel(&mut app, &ctx, cancelled).unwrap();
    assert_eq!(steps(&app), initial);
    assert!(groups(&app).is_empty());

    let id = open(&mut app, NEW_GROUP);
    set(&mut app, id, "name", json!("Sky")).unwrap();
    set(&mut app, id, "color", json!("blue")).unwrap();
    set(&mut app, id, "mode", json!(crate::panels::layers_tree::blend_key(BlendMode::Multiply))).unwrap();
    set(&mut app, id, "opacity", json!(60.0)).unwrap();
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert_eq!(steps(&app), initial + 1, "one history step");
    let g = &groups(&app)[0];
    assert_eq!((g.name.as_str(), g.label, g.blend), ("Sky", LabelColor::Blue, BlendMode::Multiply));
    assert!((g.opacity - 0.6).abs() < 1e-6);
    let top: Vec<LayerId> = app.session.active().unwrap().doc.layers.iter().map(|l| l.id).collect();
    assert_eq!(top.last(), Some(&g.id), "an empty group above the active layer B");
    assert_eq!(top[top.len() - 2], b);
}

#[test]
fn ok_from_layers_groups_the_selection_collapsed() {
    let (mut app, a, b) = app();
    let ctx = app.ctx().clone();
    app.run("layer.select", json!({"layer": a.0})).unwrap();
    app.run("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    let id = open(&mut app, FROM_LAYERS);
    set(&mut app, id, "name", json!("Pair")).unwrap();
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    let g = &groups(&app)[0];
    assert_eq!(g.name, "Pair");
    assert_eq!(g.children().unwrap().iter().map(|l| l.id).collect::<Vec<_>>(), vec![a, b]);
    assert!(matches!(&g.content, LayerContent::Group(g) if !g.expanded), "collapsed, as in Photoshop");
}
