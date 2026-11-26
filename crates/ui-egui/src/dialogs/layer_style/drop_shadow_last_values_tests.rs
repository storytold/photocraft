//! #14877: a new Drop Shadow starts with the values of the last Drop Shadow committed with OK, also on another
//! document, with that document's Global Light angle; Cancel doesn't change them, and Reset to Default still gives
//! the factory values.

use craft::imaging::layer_style::{EffectKind, StyleValue};
use serde_json::json;

use super::reset_to_default;
use super::tests::{app_with_layer, cancel, defaults, draft, edit, first_of, ok};
use super::{DialogId, open};
use crate::PhotocraftApp;

const KEYS: [&str; 4] = ["opacity", "distance", "spread", "size"];

/// Opens the dialog on a new Drop Shadow of the active layer.
fn new_shadow(app: &mut PhotocraftApp) -> DialogId {
    open(app, Some(EffectKind::DropShadow.key())).unwrap()
}

fn values(app: &PhotocraftApp, id: DialogId) -> Vec<Option<f64>> {
    let shadow = first_of(app, id, EffectKind::DropShadow);
    let e = draft(app, id).style.effect(shadow).unwrap();
    KEYS.iter().map(|k| e.params.number(k)).collect()
}

fn angle(app: &PhotocraftApp, id: DialogId) -> Option<f64> {
    let shadow = first_of(app, id, EffectKind::DropShadow);
    draft(app, id).style.effect(shadow).and_then(|e| e.params.number("angle"))
}

/// OK on a new Drop Shadow with Opacity 50, Distance 10, Spread 20, Size 15.
fn commit_shadow(app: &mut PhotocraftApp) {
    let id = new_shadow(app);
    let shadow = first_of(app, id, EffectKind::DropShadow);
    edit(app, id, |d| {
        for (k, v) in KEYS.iter().zip([50.0, 10.0, 20.0, 15.0]) {
            d.set_param(shadow, k, StyleValue::Number(v));
        }
    });
    ok(app, id);
}

/// A new document with a layer and its Global Light at `angle`.
fn other_document(app: &mut PhotocraftApp, angle: f64) {
    app.run("file.new", json!({"width": 48, "height": 48})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("layer.layerStyle.globalLight", json!({"angle": angle})).unwrap();
}

#[test]
fn a_new_drop_shadow_starts_with_the_last_committed_values_and_the_documents_light() {
    let mut app = app_with_layer(48);
    let first = new_shadow(&mut app);
    let factory = values(&app, first);
    cancel(&mut app, first);
    commit_shadow(&mut app);
    other_document(&mut app, 30.0);
    let id = new_shadow(&mut app);
    assert_eq!(values(&app, id), vec![Some(50.0), Some(10.0), Some(20.0), Some(15.0)], "factory was {factory:?}");
    assert_eq!(angle(&app, id), Some(30.0), "the angle is the new document's Global Light");
}

#[test]
fn cancel_does_not_change_the_starting_values() {
    let mut app = app_with_layer(48);
    commit_shadow(&mut app);
    other_document(&mut app, 120.0);
    let id = new_shadow(&mut app);
    let shadow = first_of(&app, id, EffectKind::DropShadow);
    edit(&mut app, id, |d| d.set_param(shadow, "opacity", StyleValue::Number(90.0)));
    cancel(&mut app, id);
    let id = new_shadow(&mut app);
    assert_eq!(values(&app, id)[0], Some(50.0), "the cancelled 90 is not reused");
}

#[test]
fn reset_to_default_still_gives_the_factory_values() {
    let mut app = app_with_layer(48);
    commit_shadow(&mut app);
    other_document(&mut app, 120.0);
    let id = new_shadow(&mut app);
    let shadow = first_of(&app, id, EffectKind::DropShadow);
    reset_to_default(&mut app, id, shadow);
    let want = defaults(EffectKind::DropShadow);
    assert_eq!(values(&app, id), KEYS.iter().map(|k| want.number(k)).collect::<Vec<_>>());
}
