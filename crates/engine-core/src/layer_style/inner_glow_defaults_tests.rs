//! #14917: the Inner Glow page's factory values (Screen, 35 %, Noise 0, white, Softer, Edge,
//! Choke 0, Size 7, Linear, Anti-aliased off, Range 50) and the rule that a glow already stored
//! keeps its own values. The measured pixels of the factory glow sit within 1 level of the
//! reference application's on the white test document.

use super::*;
use crate::Session;
use serde_json::json;

/// The issue's test document: 400 x 300 px at 72 ppi, RGB 8 bit, a white Background and a pixel
/// layer with an orange (255,128,0) rectangle at x 100..299, y 75..224.
fn test_document() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 400, "height": 300, "resolution": 72, "background": "#ffffff"})).unwrap();
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

/// The one glow the active layer carries, as an inner glow.
fn inner_glow(s: &Session) -> Glow {
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    match l.effects.items.as_slice() {
        [Effect::InnerGlow(g)] => g.clone(),
        other => panic!("no inner glow: {other:?}"),
    }
}

/// #14917: Layer > Layer Style > Inner Glow... on a layer without one starts from the reference
/// application's factory values; the dialog seeds a new instance from `effect_defaults`.
#[test]
fn a_new_inner_glow_starts_at_the_factory_values() {
    let d = effect_defaults("innerGlow");
    assert_eq!(d["blend"], json!("Screen"), "blend");
    assert_eq!(d["opacity"], json!(35), "opacity");
    assert_eq!(d["noise"], json!(0), "noise");
    assert_eq!(d["color"], json!("#ffffff"), "colour");
    assert_eq!(d["technique"], json!("softer"), "technique");
    assert_eq!(d["source"], json!("edge"), "source");
    assert_eq!(d["choke"], json!(0), "choke");
    assert_eq!(d["size"], json!(7), "size");
    assert_eq!(d["contour"], json!("Linear"), "contour");
    assert_eq!(d["antiAlias"], json!(false), "anti-aliased");
    assert_eq!(d["range"], json!(50), "range");
}

/// #14917: `layer.layerStyle.innerGlow` without params makes the same effect the page starts at.
#[test]
fn an_inner_glow_command_without_params_makes_the_factory_effect() {
    let mut s = session();
    s.execute("layer.layerStyle.innerGlow", json!({})).unwrap();
    let g = inner_glow(&s);
    assert_eq!(g.common.blend, BlendMode::Screen, "blend mode");
    assert!((g.common.opacity * 100.0 - 35.0).abs() < 1e-4, "opacity {} %", g.common.opacity * 100.0);
    let FxPaint::Color(c) = g.paint else { panic!("not a plain colour: {:?}", g.paint) };
    let rgb = c.to_rgb();
    assert!(rgb.iter().all(|v| (v - 1.0).abs() < 1e-4), "colour {rgb:?} is not white");
    assert_eq!((g.technique, g.source, g.spread, g.size), (GlowTechnique::Softer, GlowSource::Edge, 0.0, 7.0));
    assert!(matches!(g.contour, Contour::Linear), "contour {:?}", g.contour);
    assert!(!g.anti_alias, "anti-aliased");
    assert!((g.range - 0.5).abs() < 1e-4, "range {}", g.range);
    assert_eq!(g.noise, 0.0, "noise");
}

/// #14917: Reset to Default reads these factory values through `layer.layerStyle.defaultFor`
/// until a Make Default stores other ones; the saved values keep winning (the Make Default path
/// is not measured, so it keeps today's behaviour).
#[test]
fn reset_to_default_restores_the_inner_glow_factory_values() {
    let mut s = session();
    let d = s.execute("layer.layerStyle.defaultFor", json!({"kind": "innerGlow"})).unwrap();
    assert_eq!(d["user"], json!(false));
    assert_eq!((d["params"]["opacity"].as_f64(), d["params"]["color"].as_str(), d["params"]["size"].as_f64()), (Some(35.0), Some("#ffffff"), Some(7.0)));
    s.execute("layer.layerStyle.makeDefault", json!({"kind": "innerGlow", "params": {"opacity": 12, "size": 3}})).unwrap();
    let saved = s.execute("layer.layerStyle.defaultFor", json!({"kind": "innerGlow"})).unwrap();
    assert_eq!(saved["user"], json!(true));
    assert_eq!((saved["params"]["opacity"].as_f64(), saved["params"]["size"].as_f64()), (Some(12.0), Some(3.0)), "a saved default still wins");
}

/// #14917 measured: OK on the factory values over the white test document. Top edge of the
/// flattened image at x 200, y 75 inwards to 81, R.G.B; the reference application's are
/// 255.166.76 255.155.54 255.146.36 255.139.22 255.133.11 255.129.3 255.128.0, within 1 level.
#[test]
fn the_factory_inner_glow_paints_the_measured_edge() {
    let mut s = test_document();
    s.execute("layer.layerStyle.innerGlow", json!({})).unwrap();
    let d = s.active().unwrap();
    let px = photocraft_compose::render(&d.doc, d.doc.bounds()).into_px();
    let rgb = |x: usize, y: usize| px[y * 400 + x].map(|v| (v * 255.0).round() as i32);
    for (y, want) in
        [(75, [255, 166, 76]), (76, [255, 155, 54]), (77, [255, 146, 36]), (78, [255, 139, 22]), (79, [255, 133, 11]), (80, [255, 129, 3]), (81, [255, 128, 0])]
    {
        let got = rgb(200, y);
        assert!(got[..3].iter().zip(want).all(|(g, w)| (g - w).abs() <= 1), "top edge at (200, {y}): {:?}, the reference gives {want:?}", &got[..3]);
    }
}

/// #14917: an Inner Glow the document already carries keeps the values it was read with. The
/// Layer Style dialog's OK (and so a PSD or `.pcraft` effect edited in it) goes through
/// `layer.layerStyle.replace` with the effect's own snapshot: absent params leave it alone.
#[test]
fn a_stored_inner_glow_keeps_its_own_values_through_replace() {
    let mut s = session();
    s.execute("layer.layerStyle.innerGlow", json!({"opacity": 75, "color": "#ffffbe", "size": 5})).unwrap();
    let stored = inner_glow(&s);
    let snapshot = serde_json::to_value(Effect::InnerGlow(stored.clone())).unwrap();
    s.execute("layer.layerStyle.replace", json!({"effects": [{"kind": "innerGlow", "params": {}, "fx": snapshot}]})).unwrap();
    let kept = inner_glow(&s);
    assert_eq!(kept, stored, "an edited effect list left the glow's values alone");
    assert!((kept.common.opacity * 100.0 - 75.0).abs() < 1e-4 && (kept.size - 5.0).abs() < 1e-4, "kept {kept:?}");
}
