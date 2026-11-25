//! #14850: a Layer Style confirmation is one undoable edit.

use super::{
    tests::{edit, first_of},
    *,
};
use egui_kittest::Harness;
use photocraft_doc::Layer;

fn pixel_layer_app() -> Result<Box<PhotocraftApp>, String> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 16, "height": 16}))?;
    app.run("layer.new.layer", json!({}))?;
    Ok(app)
}

fn layer(app: &PhotocraftApp) -> Option<&Layer> {
    let state = app.session.active()?;
    state.doc.layer(state.active_layer?)
}

#[test]
fn opacity_only_ok_is_one_undoable_layer_style_edit() {
    let mut app = pixel_layer_app().unwrap();
    let before = app.session.active().unwrap().history.entries().to_vec();
    let id = open(&mut app, Some(BLENDING)).unwrap();
    edit(&mut app, id, |draft| draft.style.blending.opacity = 70.0);
    crate::dialogs::testing::confirm(&mut app, id).unwrap();

    let entries = app.session.active().unwrap().history.entries();
    assert_eq!(&entries[..before.len()], before.as_slice());
    assert_eq!(&entries[before.len()..], ["Layer Style"]);
    assert_eq!(layer(&app).unwrap().opacity, 0.7);
    assert!(layer(&app).unwrap().effects.items.is_empty());

    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(layer(&app).unwrap().opacity, 1.0);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(layer(&app).unwrap().opacity, 0.7);
}

#[test]
fn fill_stroke_and_global_light_undo_and_redo_together() {
    let mut app = pixel_layer_app().unwrap();
    app.run("layer.layerStyle.stroke", json!({"size": 3})).unwrap();
    let initial_effects = layer(&app).unwrap().effects.clone();
    let initial_light = app.session.active().unwrap().doc.global_light;
    let before = app.session.active().unwrap().history.entries().len();
    let id = open(&mut app, Some(BLENDING)).unwrap();
    let stroke = first_of(&app, id, EffectKind::Stroke);
    edit(&mut app, id, |draft| {
        draft.style.blending.fill_opacity = 40.0;
        draft.style.global_light = 45.0;
        draft.set_param(stroke, "size", StyleValue::Number(7.0));
    });
    crate::dialogs::testing::confirm(&mut app, id).unwrap();

    assert_eq!(&app.session.active().unwrap().history.entries()[before..], ["Layer Style"]);
    assert_eq!(layer(&app).unwrap().fill_opacity, 0.4);
    let edited_effects = layer(&app).unwrap().effects.clone();
    assert!(matches!(&edited_effects.items[0], photocraft_doc::Effect::Stroke(stroke) if stroke.size == 7.0));
    assert_eq!(app.session.active().unwrap().doc.global_light.angle, 45.0);

    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(layer(&app).unwrap().fill_opacity, 1.0);
    assert_eq!(layer(&app).unwrap().effects, initial_effects);
    assert_eq!(app.session.active().unwrap().doc.global_light, initial_light);

    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(layer(&app).unwrap().fill_opacity, 0.4);
    assert_eq!(layer(&app).unwrap().effects, edited_effects);
    assert_eq!(app.session.active().unwrap().doc.global_light.angle, 45.0);
}

#[test]
fn cancel_and_escape_leave_history_and_opacity_unchanged() {
    let app = pixel_layer_app().unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    let before = h.state().session.active().unwrap().history.entries().to_vec();
    for escape in [false, true] {
        let app = h.state_mut();
        let id = open(app, Some(BLENDING)).unwrap();
        edit(app, id, |draft| draft.style.blending.opacity = 70.0);
        h.run_steps(3);
        if escape {
            h.key_press(egui::Key::Escape);
            h.run_steps(3);
        } else {
            crate::dialogs::testing::cancel(h.state_mut(), id).unwrap();
        }
        assert!(h.state().framework.dialogs.get_by_id::<HostedLayerStyle>(id).is_none());
        assert_eq!(h.state().session.active().unwrap().history.entries(), before.as_slice());
        assert_eq!(layer(h.state()).unwrap().opacity, 1.0);
    }
}

#[test]
fn unchanged_ok_adds_at_most_one_step() {
    let mut app = pixel_layer_app().unwrap();
    let before = app.session.active().unwrap().history.entries().len();
    let id = open(&mut app, None).unwrap();
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    let entries = app.session.active().unwrap().history.entries();
    assert!(entries.len() <= before + 1);
    if let Some(label) = entries.get(before) {
        assert_eq!(label, "Layer Style");
    }
}

#[test]
fn rejected_effect_keeps_opacity_and_history_unchanged() {
    let mut app = pixel_layer_app().unwrap();
    app.run("layer.layerStyle.stroke", json!({"size": 3})).unwrap();
    let initial_effects = layer(&app).unwrap().effects.clone();
    let before = app.session.active().unwrap().history.entries().to_vec();
    let id = open(&mut app, Some(BLENDING)).unwrap();
    edit(&mut app, id, |draft| {
        draft.style.blending.opacity = 70.0;
        draft.style.effects[0].params.0.insert("fx".into(), StyleValue::Other("{}".into()));
    });
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    assert!(app.ui.status_error);

    assert_eq!(app.session.active().unwrap().history.entries(), before.as_slice());
    assert_eq!(layer(&app).unwrap().opacity, 1.0);
    assert_eq!(layer(&app).unwrap().effects, initial_effects);
}
