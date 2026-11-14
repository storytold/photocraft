//! Layer > Layer Style > Global Light is greyed in a document whose only layer is the Background
//! (#14681), and available as soon as there is any other layer, even with the Background active
//! and no effects. A lone non-Background layer was not measured and stays available.

use crate::Session;
use serde_json::json;

const GLOBAL_LIGHT: &str = "layer.layerStyle.globalLight";

fn background_only() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    s
}

fn light(s: &Session) -> (f32, f32) {
    let g = s.active().unwrap().doc.global_light;
    (g.angle, g.altitude)
}

fn assert_refused(mut s: Session, what: &str) {
    assert!(!s.is_enabled(GLOBAL_LIGHT), "{what}: the menu entry is greyed out");
    let (steps, before) = (s.active().unwrap().history.past_len(), light(&s));
    assert!(s.execute(GLOBAL_LIGHT, json!({"angle": 10, "altitude": 50})).is_err(), "{what}: running it is refused");
    assert_eq!(s.active().unwrap().history.past_len(), steps, "{what}: no history state");
    assert_eq!(light(&s), before, "{what}: Global Light is unchanged");
}

fn assert_runs(mut s: Session, what: &str) {
    assert!(s.is_enabled(GLOBAL_LIGHT), "{what}: the menu entry is enabled");
    let steps = s.active().unwrap().history.past_len();
    s.execute(GLOBAL_LIGHT, json!({"angle": 10, "altitude": 50})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "{what}: one history state");
    assert_eq!(light(&s), (10.0, 50.0), "{what}: Global Light is set");
}

#[test]
fn a_background_only_document_refuses_global_light() {
    assert_refused(background_only(), "RGB");
    let mut indexed = background_only();
    indexed.execute("image.mode.indexedColor", json!({"palette": "exact", "dither": "none", "forced": "none"})).unwrap();
    assert_refused(indexed, "Indexed Color");
}

#[test]
fn the_refusal_says_why() {
    let why = super::global_light_enabled(&background_only()).unwrap_err();
    assert!(why.contains("Background"), "{why}");
}

#[test]
fn any_other_layer_enables_global_light_even_with_the_background_active() {
    let mut s = background_only();
    s.execute("layer.new.layer", json!({})).unwrap();
    assert_runs(s, "the new layer active");

    let mut s = background_only();
    s.execute("layer.new.layer", json!({})).unwrap();
    let bg = s.active().unwrap().doc.layers[0].id;
    s.active_mut().unwrap().active_layer = Some(bg);
    assert!(s.active().unwrap().doc.is_background(bg));
    assert_runs(s, "the Background active, no effects");
}

/// Not measured: a document whose only layer is not the Background keeps today's behaviour.
#[test]
fn a_lone_non_background_layer_keeps_global_light() {
    let mut s = background_only();
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), 1);
    assert!(s.active().unwrap().doc.background().is_none());
    assert_runs(s, "a lone layer");
}

#[test]
fn no_document_is_unchanged() {
    let mut s = Session::new();
    assert!(!s.is_enabled(GLOBAL_LIGHT));
    let why = super::global_light_enabled(&s).unwrap_err();
    assert_eq!(why, "no document open");
    assert!(s.execute(GLOBAL_LIGHT, json!({})).is_err());
}
