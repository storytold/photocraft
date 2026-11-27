//! #14977: the Satin page's factory values (Multiply, black, 50 %, Angle 90, Distance 50 px,
//! Size 80 px, Linear, Anti-aliased on, Invert on) and the rule that a Satin a document already
//! carries keeps its stored values. Reset to Default reads the same factory set; the saved Make
//! Default path is not measured and keeps today's behaviour. The measured pixels of the factory
//! Satin sit within 1 level of the reference application's on the white test document.

use super::*;
use crate::Session;
use serde_json::json;

fn session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

/// The one Satin the active layer carries.
fn satin_of(s: &Session) -> Satin {
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    match l.effects.items.as_slice() {
        [Effect::Satin(s)] => s.clone(),
        other => panic!("no satin: {other:?}"),
    }
}

/// #14977: Layer > Layer Style > Satin... on a layer without one starts from the reference
/// application's factory values. The dialog seeds a new instance from `effect_defaults`.
#[test]
fn a_new_satin_starts_at_the_factory_values() {
    let d = effect_defaults("satin");
    assert_eq!(d["blend"], json!("Multiply"), "blend mode");
    assert_eq!(d["color"], json!("#000000"), "colour");
    assert_eq!(d["opacity"], json!(50), "opacity");
    assert_eq!(d["angle"], json!(90), "angle");
    assert_eq!(d["distance"], json!(50), "distance");
    assert_eq!(d["size"], json!(80), "size");
    assert_eq!(d["contour"], json!("Linear"), "contour");
    assert_eq!(d["antiAlias"], json!(true), "anti-aliased");
    assert_eq!(d["invert"], json!(true), "invert");
}

/// #14977: `layer.layerStyle.satin` without params on a layer without one makes the same effect
/// the page starts at.
#[test]
fn a_satin_command_without_params_makes_the_factory_effect() {
    let mut s = session();
    s.execute("layer.layerStyle.satin", json!({})).unwrap();
    let fx = satin_of(&s);
    assert_eq!(fx.common.blend, BlendMode::Multiply, "blend mode");
    assert!((fx.common.opacity * 100.0 - 50.0).abs() < 1e-4, "opacity {} %", fx.common.opacity * 100.0);
    let rgb = fx.color.to_rgb();
    assert!(rgb.iter().all(|v| v.abs() < 1e-4), "colour {rgb:?} is not black");
    assert_eq!((fx.angle, fx.distance, fx.size), (90.0, 50.0, 80.0), "angle, distance, size");
    assert!(matches!(fx.contour, Contour::Linear), "contour {:?}", fx.contour);
    assert!(fx.anti_alias, "anti-aliased");
    assert!(fx.invert, "invert");
}

/// #14977: Reset to Default reads these factory values through `layer.layerStyle.defaultFor`.
/// A saved Make Default still wins: that path is not measured, so it keeps today's behaviour.
#[test]
fn reset_to_default_restores_the_satin_factory_values() {
    let mut s = session();
    let d = s.execute("layer.layerStyle.defaultFor", json!({"kind": "satin"})).unwrap();
    assert_eq!(d["user"], json!(false));
    let p = &d["params"];
    assert_eq!((p["blend"].as_str(), p["color"].as_str(), p["opacity"].as_f64()), (Some("Multiply"), Some("#000000"), Some(50.0)));
    assert_eq!((p["angle"].as_f64(), p["distance"].as_f64(), p["size"].as_f64()), (Some(90.0), Some(50.0), Some(80.0)));
    assert_eq!((p["contour"].as_str(), p["antiAlias"].as_bool(), p["invert"].as_bool()), (Some("Linear"), Some(true), Some(true)));
    s.execute("layer.layerStyle.makeDefault", json!({"kind": "satin", "params": {"angle": 19, "distance": 11, "size": 14}})).unwrap();
    let saved = s.execute("layer.layerStyle.defaultFor", json!({"kind": "satin"})).unwrap();
    assert_eq!(saved["user"], json!(true));
    assert_eq!(
        (saved["params"]["angle"].as_f64(), saved["params"]["distance"].as_f64(), saved["params"]["size"].as_f64()),
        (Some(19.0), Some(11.0), Some(14.0)),
        "a saved default still wins"
    );
}

/// #14977: a Satin the document already carries keeps the values it was read with. The Layer
/// Style dialog's OK (and so a PSD or `.pcraft` effect edited in it) goes through
/// `layer.layerStyle.replace` with the effect's own snapshot: absent params leave it alone.
#[test]
fn a_stored_satin_keeps_its_own_values_through_replace() {
    let mut s = session();
    s.execute("layer.layerStyle.satin", json!({"angle": 19, "distance": 11, "size": 14})).unwrap();
    let stored = satin_of(&s);
    let snapshot = serde_json::to_value(Effect::Satin(stored.clone())).unwrap();
    s.execute("layer.layerStyle.replace", json!({"effects": [{"kind": "satin", "params": {}, "fx": snapshot}]})).unwrap();
    let kept = satin_of(&s);
    assert_eq!(kept, stored, "an edited effect list left the satin's values alone");
    assert_eq!((kept.angle, kept.distance, kept.size), (19.0, 11.0, 14.0), "kept {kept:?}");
}

/// #14977: the same for a saved `.pcraft` round trip -- stored Satin values are read back as they
/// were, whatever the factory defaults are.
#[test]
fn a_pcraft_satin_keeps_its_stored_values() {
    let mut s = session();
    s.execute("layer.layerStyle.satin", json!({"angle": 45, "distance": 7, "size": 3, "opacity": 20})).unwrap();
    let stored = satin_of(&s);
    let pc = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let again = photocraft_format::load_from_bytes(&pc).unwrap();
    let read = again
        .layers
        .iter()
        .find_map(|l| l.effects.items.iter().find_map(|e| if let Effect::Satin(x) = e { Some(x.clone()) } else { None }))
        .expect("the saved document still carries the satin");
    assert_eq!(read, stored, "the round trip kept the stored values");
    assert_eq!((read.angle, read.distance, read.size), (45.0, 7.0, 3.0));
}
