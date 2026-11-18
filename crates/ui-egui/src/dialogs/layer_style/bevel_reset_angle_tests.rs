//! #14843: Reset to Default on the Bevel & Emboss page replaced the page with the stored defaults
//! as they are, and those carry `angle: 120`. With Use Global Light on and Angle 45, the field
//! jumped to 120 while the dialog still applied the shared light 45 on OK, so field and result
//! disagreed. The reset now seeds the dialog's shared light angle into a globally lit effect, the
//! way a new instance starts from it (`fresh_params`, and the draft's own `add_instance`).
use craft::imaging::layer_style::{EffectId, EffectKind, LayerStyle, StyleParams, StyleValue};
use serde_json::json;

use super::reset_to_default;
use super::tests::{app_with_layer, defaults, draft, edit, first_of, ok};
use super::{DialogId, open};
use crate::PhotocraftApp;

/// The numeric parameter of one instance of the draft's style.
fn num(style: &LayerStyle, id: EffectId, key: &str) -> Option<f64> {
    style.effect(id).and_then(|e| e.params.number(key))
}

/// The text parameter of a parameter set.
fn text(params: &StyleParams, key: &str) -> Option<String> {
    match params.get(key) {
        Some(StyleValue::Text(t)) => Some(t.clone()),
        _ => None,
    }
}

fn instance_text(app: &PhotocraftApp, id: DialogId, effect: EffectId, key: &str) -> Option<String> {
    text(&draft(app, id).style.effect(effect).expect("the instance").params, key)
}

/// Use Global Light on, Angle 45 typed: Reset to Default keeps 45 and returns every other field
/// to its default; the other globally lit effect keeps the shared angle, and OK leaves the
/// document's Global Light at 45.
#[test]
fn reset_to_default_keeps_the_typed_shared_angle() {
    let mut app = app_with_layer(32);
    app.run("layer.layerStyle.bevelEmboss", json!({})).unwrap();
    app.run("layer.layerStyle.dropShadow", json!({})).unwrap();
    let id = open(&mut app, Some(EffectKind::BevelEmboss.key())).unwrap();
    let bevel = first_of(&app, id, EffectKind::BevelEmboss);
    let shadow = first_of(&app, id, EffectKind::DropShadow);
    // What the Angle row does when 45 is typed, plus two edits the reset has to undo.
    edit(&mut app, id, |d| {
        d.set_param(bevel, "angle", StyleValue::Number(45.0));
        d.set_param(bevel, "depth", StyleValue::Number(40.0));
        d.set_param(bevel, "size", StyleValue::Number(9.0));
    });
    reset_to_default(&mut app, id, bevel);
    let style = &draft(&app, id).style;
    assert_eq!(style.global_light, 45.0, "the reset does not move the shared light");
    assert_eq!(num(style, bevel, "angle"), Some(45.0), "the Angle field stays at the shared light angle");
    assert_eq!(style.effect(bevel).and_then(|e| e.params.bool("useGlobalLight")), Some(true), "Use Global Light stays on");
    assert_eq!(num(style, shadow, "angle"), Some(45.0), "the drop shadow keeps the shared angle");
    let want = defaults(EffectKind::BevelEmboss);
    for key in ["depth", "size", "soften", "altitude"] {
        assert_eq!(num(style, bevel, key), want.number(key), "{key} went back to its default");
    }
    assert_eq!(instance_text(&app, id, bevel, "style"), text(&want, "style"), "the bevel style went back to its default");
    ok(&mut app, id);
    assert_eq!(app.session.active().unwrap().doc.global_light.angle, 45.0, "OK sets the document's Global Light to 45");
}

/// The document's light already at 45 when the dialog opens, nothing typed: Reset to Default still
/// shows 45, and OK leaves the document at 45 instead of moving its Global Light to the factory 120.
#[test]
fn reset_to_default_does_not_move_the_documents_light() {
    let mut app = app_with_layer(32);
    app.run("layer.layerStyle.bevelEmboss", json!({})).unwrap();
    app.run("layer.layerStyle.globalLight", json!({"angle": 45})).unwrap();
    let id = open(&mut app, Some(EffectKind::BevelEmboss.key())).unwrap();
    let bevel = first_of(&app, id, EffectKind::BevelEmboss);
    assert_eq!(num(&draft(&app, id).style, bevel, "angle"), Some(45.0), "the page shows the shared light 45");
    edit(&mut app, id, |d| d.set_param(bevel, "size", StyleValue::Number(30.0)));
    reset_to_default(&mut app, id, bevel);
    let style = &draft(&app, id).style;
    assert_eq!(num(style, bevel, "angle"), Some(45.0), "the Angle field stays at the document's light angle");
    assert_eq!(num(style, bevel, "size"), defaults(EffectKind::BevelEmboss).number("size"), "the other fields still go back to their defaults");
    ok(&mut app, id);
    assert_eq!(app.session.active().unwrap().doc.global_light.angle, 45.0, "OK keeps the document's Global Light at 45");
}

/// A saved default with Use Global Light off keeps its own angle through Reset to Default: the
/// shared light (45 here) is only seeded into a globally lit effect. Guard test, passes before and
/// after the fix: it pins the gate of the seeding.
#[test]
fn reset_to_default_keeps_a_saved_defaults_own_angle_when_global_light_is_off() {
    let mut app = app_with_layer(32);
    let mut saved = app.run("layer.layerStyle.defaultFor", json!({"kind": "bevelEmboss"})).unwrap()["params"].clone();
    saved["useGlobalLight"] = json!(false);
    saved["angle"] = json!(77.0);
    saved["size"] = json!(12);
    app.run("layer.layerStyle.makeDefault", json!({"kind": "bevelEmboss", "params": saved})).unwrap();
    app.run("layer.layerStyle.bevelEmboss", json!({})).unwrap();
    app.run("layer.layerStyle.globalLight", json!({"angle": 45})).unwrap();
    let id = open(&mut app, Some(EffectKind::BevelEmboss.key())).unwrap();
    let bevel = first_of(&app, id, EffectKind::BevelEmboss);
    edit(&mut app, id, |d| d.set_param(bevel, "size", StyleValue::Number(30.0)));
    reset_to_default(&mut app, id, bevel);
    let style = &draft(&app, id).style;
    assert_eq!(style.effect(bevel).and_then(|e| e.params.bool("useGlobalLight")), Some(false), "the saved default's switch is off");
    assert_eq!(num(style, bevel, "angle"), Some(77.0), "the saved default's own angle, not the shared light");
    assert_eq!(num(style, bevel, "size"), Some(12.0), "the other fields are the saved default's too");
}
