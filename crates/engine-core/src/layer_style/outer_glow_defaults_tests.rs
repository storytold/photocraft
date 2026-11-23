//! #14944: the Outer Glow page's factory values (Screen, 35 %, white, Softer, Spread 0, Size 7,
//! Linear, Anti-aliased off, Range 50) and the rule that a glow already stored keeps its own
//! values. The measured pixels of the factory glow sit within 2 levels of the reference
//! application's on the grey test document.

use super::*;
use crate::Session;
use serde_json::json;

/// The issue's test document: 400 x 300 px at 72 ppi, RGB 8 bit, a filled (64,64,64)
/// Background and a pixel layer with an orange (255,128,0) rectangle at x 100..299, y 75..224.
fn test_document() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 400, "height": 300, "resolution": 72, "background": "#404040"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 100, "y": 75, "width": 200, "height": 150})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff8000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

/// The one glow the active layer carries, as an outer glow.
fn outer_glow(s: &Session) -> Glow {
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    match l.effects.items.as_slice() {
        [Effect::OuterGlow(g)] => g.clone(),
        other => panic!("no outer glow: {other:?}"),
    }
}

/// #14944: Layer > Layer Style > Outer Glow... on a layer without one starts from the reference
/// application's factory values. The dialog seeds a new instance from `effect_defaults`, and the
/// command without params builds the same effect, so both are checked here.
#[test]
fn a_new_outer_glow_starts_at_the_factory_values() {
    let d = effect_defaults("outerGlow");
    assert_eq!(d["blend"], json!("Screen"), "blend");
    assert_eq!(d["opacity"], json!(35), "opacity");
    assert_eq!(d["color"], json!("#ffffff"), "colour");
    assert_eq!(d["technique"], json!("softer"), "technique");
    assert_eq!(d["spread"], json!(0), "spread");
    assert_eq!(d["size"], json!(7), "size");
    assert_eq!(d["contour"], json!("Linear"), "contour");
    assert_eq!(d["antiAlias"], json!(false), "anti-aliased");
    assert_eq!(d["range"], json!(50), "range");
    assert_eq!(d["noise"], json!(0), "noise");
}

/// #14944: `layer.layerStyle.outerGlow` without params makes the same effect the page starts at.
#[test]
fn an_outer_glow_command_without_params_makes_the_factory_effect() {
    let mut s = session();
    s.execute("layer.layerStyle.outerGlow", json!({})).unwrap();
    let g = outer_glow(&s);
    assert_eq!(g.common.blend, BlendMode::Screen, "blend mode");
    assert!((g.common.opacity * 100.0 - 35.0).abs() < 1e-4, "opacity {} %", g.common.opacity * 100.0);
    let FxPaint::Color(c) = g.paint else { panic!("not a plain colour: {:?}", g.paint) };
    let rgb = c.to_rgb();
    assert!(rgb.iter().all(|v| (v - 1.0).abs() < 1e-4), "colour {rgb:?} is not white");
    assert_eq!((g.technique, g.spread, g.size), (GlowTechnique::Softer, 0.0, 7.0));
    assert!(matches!(g.contour, Contour::Linear), "contour {:?}", g.contour);
    assert!(!g.anti_alias, "anti-aliased");
    assert!((g.range - 0.5).abs() < 1e-4, "range {}", g.range);
    assert_eq!(g.noise, 0.0, "noise");
}

/// #14944: Reset to Default reads these factory values through `layer.layerStyle.defaultFor`
/// until a Make Default stores other ones; the saved values keep winning (the Make Default path
/// is not re-measured, so it keeps today's behaviour).
#[test]
fn reset_to_default_restores_the_outer_glow_factory_values() {
    let mut s = session();
    let d = s.execute("layer.layerStyle.defaultFor", json!({"kind": "outerGlow"})).unwrap();
    assert_eq!(d["user"], json!(false));
    assert_eq!((d["params"]["opacity"].as_f64(), d["params"]["color"].as_str(), d["params"]["size"].as_f64()), (Some(35.0), Some("#ffffff"), Some(7.0)));
    s.execute("layer.layerStyle.makeDefault", json!({"kind": "outerGlow", "params": {"opacity": 12, "size": 3}})).unwrap();
    let saved = s.execute("layer.layerStyle.defaultFor", json!({"kind": "outerGlow"})).unwrap();
    assert_eq!(saved["user"], json!(true));
    assert_eq!((saved["params"]["opacity"].as_f64(), saved["params"]["size"].as_f64()), (Some(12.0), Some(3.0)), "a saved default still wins");
}

/// #14944 measured: OK on the factory values over the grey test document. Top edge at x 200,
/// y 74 outwards to 68, and the corner pixel (97, 72), as grey levels; the reference
/// application's are 121 104 91 80 72 66 64 and 69, within 2 levels.
#[test]
fn the_factory_outer_glow_paints_the_measured_edge() {
    let mut s = test_document();
    s.execute("layer.layerStyle.outerGlow", json!({})).unwrap();
    let d = s.active().unwrap();
    let px = photocraft_compose::render(&d.doc, d.doc.bounds()).into_px();
    let level = |x: usize, y: usize| (px[y * 400 + x][0] * 255.0).round() as i32;
    for (y, want) in [(74, 121), (73, 104), (72, 91), (71, 80), (70, 72), (69, 66), (68, 64)] {
        let got = level(200, y);
        assert!((got - want).abs() <= 2, "top edge at (200, {y}): {got}, the reference gives {want}");
    }
    let corner = level(97, 72);
    assert!((corner - 69).abs() <= 2, "corner (97, 72): {corner}, the reference gives 69");
}

/// #14944: an Outer Glow the document already carries keeps the values it was read with. The
/// Layer Style dialog's OK (and so a PSD or `.pcraft` effect edited in it) goes through
/// `layer.layerStyle.replace` with the effect's own snapshot: absent params leave it alone.
#[test]
fn a_stored_outer_glow_keeps_its_own_values_through_replace() {
    let mut s = session();
    s.execute("layer.layerStyle.outerGlow", json!({"opacity": 75, "color": "#ffffbe", "size": 5})).unwrap();
    let stored = outer_glow(&s);
    let snapshot = serde_json::to_value(Effect::OuterGlow(stored.clone())).unwrap();
    s.execute("layer.layerStyle.replace", json!({"effects": [{"kind": "outerGlow", "params": {}, "fx": snapshot}]})).unwrap();
    let kept = outer_glow(&s);
    assert_eq!(kept, stored, "an edited effect list left the glow's values alone");
    assert!((kept.common.opacity * 100.0 - 75.0).abs() < 1e-4 && (kept.size - 5.0).abs() < 1e-4, "kept {kept:?}");
}

/// #14944 is only about the Outer Glow page; the Inner Glow's factory values are #14917 and stay
/// as they are today (the two glows share their fresh effect in the engine).
#[test]
fn an_inner_glow_still_starts_at_todays_values() {
    let d = effect_defaults("innerGlow");
    assert_eq!((d["opacity"].as_f64(), d["color"].as_str(), d["size"].as_f64()), (Some(75.0), Some("#ffffbe"), Some(5.0)));
    let mut s = session();
    s.execute("layer.layerStyle.innerGlow", json!({})).unwrap();
    let g = match &s.active().unwrap().doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().effects.items[0] {
        Effect::InnerGlow(g) => g.clone(),
        other => panic!("{other:?}"),
    };
    assert!((g.common.opacity * 100.0 - 75.0).abs() < 1e-4, "opacity {}", g.common.opacity);
    assert_eq!(g.size, 5.0, "size");
    let FxPaint::Color(c) = g.paint else { panic!("{:?}", g.paint) };
    let rgb = c.to_rgb();
    assert!((rgb[0] - 1.0).abs() < 1e-3 && (rgb[1] - 1.0).abs() < 1e-3 && (rgb[2] - 190.0 / 255.0).abs() < 1e-3, "colour {rgb:?}");
}
