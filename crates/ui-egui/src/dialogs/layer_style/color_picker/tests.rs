use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::dialogs::layer_style::tests::{
    ColorStop, DialogId, EffectId, EffectKind, EffectSpec, FieldKind, GradientStops, KIND_KEYS, LayerStyle, LayerStyleDraft, LayerStylePage, Srgba, StyleValue,
    cancel, defaults, draft, edit, kind, ok, param,
};
use crate::dialogs::layer_style::{self, preview_document};
use crate::{PhotocraftApp, color_picker_ui};

fn new_app(depth: u32) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("prefs.set", json!({"path":"interface.language","value":"en"})).unwrap();
    app.run("file.new", json!({"width":32,"height":24,"depth":depth,"background":"#406080"})).unwrap();
    app
}

/// Opens the child picker of `parent` for one field.
fn open_picker(app: &mut PhotocraftApp, parent: DialogId, effect: &str, field: &str, stop: Option<usize>) -> Result<DialogId, String> {
    open(app, parent.value(), effect, field, stop)
}

fn style(app: &PhotocraftApp, id: DialogId) -> LayerStyle {
    draft(app, id).style.clone()
}

/// The child picker answers `hex`, as the user editing its colour does.
fn set_color(app: &mut PhotocraftApp, picker: DialogId, hex: &str) {
    color_picker_ui::testing::set_color(app, picker, hex);
}

/// The effect page the dialog shows.
fn selected(app: &PhotocraftApp, parent: DialogId) -> EffectId {
    match draft(app, parent).page {
        LayerStylePage::Effect(id) => id,
        ref other => panic!("an effect page is shown, not {other:?}"),
    }
}

fn color_of(style: &LayerStyle, id: EffectId, key: &str) -> Srgba {
    match param(style, id, key) {
        Some(StyleValue::Color(c)) => c,
        other => panic!("{key} is a colour, not {other:?}"),
    }
}

fn stop_colors(style: &LayerStyle, id: EffectId) -> Vec<Srgba> {
    match param(style, id, "stops") {
        Some(StyleValue::Gradient(g)) => g.color.iter().map(|s| s.color).collect(),
        other => panic!("stops are a gradient, not {other:?}"),
    }
}

fn others(style: &LayerStyle, id: EffectId) -> Vec<(String, StyleValue)> {
    style.effect(id).unwrap().params.0.iter().filter(|(_, v)| matches!(v, StyleValue::Other(_))).map(|(k, v)| (k.clone(), v.clone())).collect()
}

fn top(app: &PhotocraftApp) -> Option<DialogId> {
    app.framework.dialogs.stack().top()
}

#[test]
fn every_exposed_color_field_uses_the_shared_picker_and_only_updates_its_target() {
    for key in KIND_KEYS {
        let kind = kind(key);
        for field in EffectSpec::photoshop(kind).fields {
            if !matches!(field.kind, FieldKind::Color) {
                continue;
            }
            let mut app = new_app(8);
            let parent = layer_style::open(&mut app, Some(kind.key())).unwrap();
            let effect = selected(&app, parent);
            let original = style(&app, parent);
            let tools = app.session.tools.clone();
            let doc = app.session.active().unwrap().doc.clone();
            let history = app.session.active().unwrap().history.past_len();
            let picker = open_picker(&mut app, parent, &effect.key(), &field.key, None).unwrap();
            assert_eq!(top(&app), Some(picker), "the shared picker is the top dialog");
            set_color(&mut app, picker, "#23a567");
            let wanted = Srgba::from_hex("#23a567").unwrap();
            let shown = preview_style(&app).expect("the preview shows the picker's colour");
            assert!(color_of(&shown, effect, &field.key).same_rgb8(wanted), "{key}.{}", field.key);
            assert_eq!(style(&app, parent), original, "the draft waits for the picker's OK");
            ok(&mut app, picker);
            let now = style(&app, parent);
            let got = color_of(&now, effect, &field.key);
            assert!(got.same_rgb8(wanted), "{key}.{}", field.key);
            let mut expected = LayerStyleDraft::new(original);
            expected.set_param(effect, &field.key, StyleValue::Color(got));
            assert_eq!(now, expected.style, "{key}.{}: only the target changes", field.key);
            assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &doc));
            assert_eq!(app.session.active().unwrap().history.past_len(), history);
            assert_eq!(app.session.tools.foreground, tools.foreground);
            assert_eq!(app.session.tools.background, tools.background);
        }
    }
}

#[test]
fn preview_cancel_ok_parent_cancel_and_apply_are_transactional_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut app = new_app(depth);
        let parent = layer_style::open(&mut app, Some(EffectKind::ColorOverlay.key())).unwrap();
        let effect = selected(&app, parent);
        edit(&mut app, parent, |d| {
            d.add_instance(EffectKind::Stroke, &defaults(EffectKind::Stroke));
        });
        let original = style(&app, parent);
        let st = app.session.active().unwrap();
        let (doc, history, revision, dirty) = (st.doc.clone(), st.history.past_len(), st.revision, st.is_dirty());
        let picker = open_picker(&mut app, parent, &effect.key(), "color", None).unwrap();
        set_color(&mut app, picker, "#00ff00");
        let (shown, key) = preview_document(&app).expect("the preview renders");
        assert!(!Arc::ptr_eq(&shown, &doc));
        assert_eq!(key, preview_document(&app).unwrap().1, "an unchanged preview keeps its key");
        let pixel = photocraft_compose::flatten(&shown).get(3, 3);
        assert!(pixel[1] > pixel[0] && pixel[1] > pixel[2], "live preview is green at {depth} bits");
        let st = app.session.active().unwrap();
        assert_eq!((st.history.past_len(), st.revision, st.is_dirty()), (history, revision, dirty));
        assert!(Arc::ptr_eq(&st.doc, &doc));
        cancel(&mut app, picker);
        assert_eq!(style(&app, parent), original);
        assert_ne!(preview_document(&app).unwrap().1, key, "the child's colour is no longer shown");
        let picker = open_picker(&mut app, parent, &effect.key(), "color", None).unwrap();
        set_color(&mut app, picker, "#00ff00");
        ok(&mut app, picker);
        assert_eq!(app.session.active().unwrap().history.past_len(), history, "the child's OK sets the draft only");
        cancel(&mut app, parent);
        assert!(preview_document(&app).is_none_or(|(shown, _)| Arc::ptr_eq(&shown, &doc)), "parent Cancel drops everything");
        assert!(app.framework.dialogs.is_empty());
        let parent = layer_style::open(&mut app, Some(EffectKind::ColorOverlay.key())).unwrap();
        let effect = selected(&app, parent);
        let picker = open_picker(&mut app, parent, &effect.key(), "color", None).unwrap();
        set_color(&mut app, picker, "#00ff00");
        ok(&mut app, picker);
        ok(&mut app, parent);
        let applied = app.session.active().unwrap().doc.clone();
        let pixel = photocraft_compose::flatten(&applied).get(3, 3);
        assert!(pixel[1] > pixel[0]);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layer(doc.layers[0].id).unwrap().effects.items.len(), 0);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layer(doc.layers[0].id).unwrap().effects, applied.layer(doc.layers[0].id).unwrap().effects);
    }
}

#[test]
fn unchanged_ok_preserves_missing_parameters_and_imported_effect_snapshot() {
    let mut app = new_app(8);
    app.run("layer.layerStyle.stroke", json!({"size":7,"color":"#123456","add":true})).unwrap();
    app.run("layer.layerStyle.stroke", json!({"size":9,"color":"#abcdef","add":true})).unwrap();
    let parent = layer_style::open(&mut app, Some(EffectKind::Stroke.key())).unwrap();
    let original = style(&app, parent);
    let (first, second) = (EffectId(1), EffectId(2));
    for effect in [first, second] {
        let picker = open_picker(&mut app, parent, &effect.key(), "color", None).unwrap();
        ok(&mut app, picker);
        assert_eq!(style(&app, parent), original, "an unchanged OK writes nothing");
    }
    let picker = open_picker(&mut app, parent, &second.key(), "color", None).unwrap();
    set_color(&mut app, picker, "#223344");
    ok(&mut app, picker);
    let current = style(&app, parent);
    assert_eq!(current.effect(first), original.effect(first));
    assert_eq!(others(&current, second), others(&original, second), "the imported snapshot stays");
    cancel(&mut app, parent);

    let parent = layer_style::open(&mut app, Some(EffectKind::GradientOverlay.key())).unwrap();
    let effect = selected(&app, parent);
    edit(&mut app, parent, |d| {
        d.style.effect_mut(effect).unwrap().params.0.remove("stops");
    });
    let original = style(&app, parent);
    let picker = open_picker(&mut app, parent, &effect.key(), "stops", Some(0)).unwrap();
    ok(&mut app, picker);
    assert_eq!(style(&app, parent), original, "a missing stops parameter stays missing");
}

#[test]
fn stale_targets_and_invalid_colors_never_apply_or_leave_preview() {
    // Targets the picker cannot take are refused at open.
    let mut app = new_app(8);
    let parent = layer_style::open(&mut app, Some(EffectKind::ColorOverlay.key())).unwrap();
    let effect = selected(&app, parent);
    assert!(open_picker(&mut app, parent, "fx99", "color", None).is_err(), "no such instance");
    assert!(open_picker(&mut app, parent, &effect.key(), "opacity", None).is_err(), "not a colour field");
    assert!(open_picker(&mut app, parent, &effect.key(), "nothing", None).is_err(), "no such field");
    cancel(&mut app, parent);

    for change in ["document", "parent", "effect"] {
        let mut app = new_app(8);
        let parent = layer_style::open(&mut app, Some(EffectKind::ColorOverlay.key())).unwrap();
        let effect = selected(&app, parent);
        let picker = open_picker(&mut app, parent, &effect.key(), "color", None).unwrap();
        set_color(&mut app, picker, "#00ff00");
        match change {
            "document" => {
                app.run("file.new", json!({"width":3,"height":3})).unwrap();
            }
            "parent" => {
                let ctx = app.ctx().clone();
                app.framework.dialogs.cancel_by_id(&ctx, parent);
            }
            _ => edit(&mut app, parent, |d| d.remove_instance(effect)),
        }
        let before = (change != "parent" && change != "document").then(|| style(&app, parent));
        if app.framework.dialogs.stack().contains(picker) {
            ok(&mut app, picker);
        }
        if let Some(before) = before {
            assert_eq!(style(&app, parent), before, "{change}: the answer applies to nothing");
            assert_eq!(preview_style(&app), Some(before), "{change}: no colour stays in the preview");
        }
        let st = app.session.active().unwrap();
        assert!(st.doc.layers.iter().all(|l| l.effects.items.is_empty()), "{change}: nothing reached the document");
        assert!(!app.framework.dialogs.stack().contains(picker), "{change}: the picker is gone");
    }
}

/// A Gradient Overlay's stop (#5177): the picker edits one colour of `stops`, previewing it, and leaves the other
/// stops alone; a stop that doesn't exist is refused.
#[test]
fn gradient_stop_colors_go_through_the_picker() {
    let mut app = new_app(8);
    let parent = layer_style::open(&mut app, Some(EffectKind::GradientOverlay.key())).unwrap();
    let effect = selected(&app, parent);
    let (red, green, blue) = (Srgba::rgb(1.0, 0.0, 0.0), Srgba::rgb(0.0, 1.0, 0.0), Srgba::rgb(0.0, 0.0, 1.0));
    let three = GradientStops::new(vec![ColorStop::new(0.0, red), ColorStop::new(0.5, green), ColorStop::new(1.0, blue)]);
    edit(&mut app, parent, |d| d.set_param(effect, "stops", StyleValue::Gradient(three)));
    assert!(open_picker(&mut app, parent, &effect.key(), "stops", Some(3)).is_err(), "no fourth stop");
    assert!(open_picker(&mut app, parent, &effect.key(), "stops", None).is_err(), "stops is not a plain colour");
    let picker = open_picker(&mut app, parent, &effect.key(), "stops", Some(1)).unwrap();
    set_color(&mut app, picker, "#123456");
    let wanted = Srgba::from_hex("#123456").unwrap();
    let shown = stop_colors(&preview_style(&app).unwrap(), effect);
    assert!(shown[1].same_rgb8(wanted), "previewed");
    assert!(stop_colors(&style(&app, parent), effect)[1].same_rgb8(green), "not stored yet");
    ok(&mut app, picker);
    let stops = stop_colors(&style(&app, parent), effect);
    assert!(stops[0].same_rgb8(red) && stops[1].same_rgb8(wanted) && stops[2].same_rgb8(blue), "{stops:?}");
}
