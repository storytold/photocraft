//! #14900: the Gradient Overlay page's factory values (Normal, 100 %, grey 215 to white, Method
//! Smooth, Smoothness 100 %, Linear, Angle 90, Scale 100) and the rule that an overlay already
//! stored keeps its own gradient and method. The measured column of the factory overlay sits
//! within 2 levels of the reference application's on the white test document.

use super::*;
use crate::Session;
use photocraft_doc::GradientStyle;
use photocraft_doc::effects::GradientInterpolation;
use serde_json::json;

const GREY_215: f32 = 215.0 / 255.0;

/// The issue's test document: 400 x 300 px at 72 ppi, RGB 8 bit, a white Background and a pixel
/// layer with an orange (255,128,0) rectangle at x 100..299, y 75..224.
fn test_document() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 400, "height": 300, "resolution": 72, "background": "#ffffff"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    // Edit > Fill lives in photocraft-engine-paint, so the rectangle is written directly.
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("orange", |doc, _| {
        let surf = doc.layer_mut(id).unwrap().surface_mut().unwrap();
        let r = photocraft_geom::Rect::from_xywh(100, 75, 200, 150);
        let px = photocraft_raster::from_rgba(&surf.format(), [1.0, 128.0 / 255.0, 0.0, 1.0]);
        surf.write_region(r, &px.repeat(200 * 150));
        Ok(())
    })
    .unwrap();
    s
}

fn session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

/// The one effect the active layer carries.
fn only_effect(s: &Session) -> Effect {
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    match l.effects.items.as_slice() {
        [fx] => fx.clone(),
        other => panic!("not one effect: {other:?}"),
    }
}

fn close(a: &Color, rgb: [f32; 3]) -> bool {
    a.to_rgb().iter().zip(rgb).all(|(x, y)| (x - y).abs() < 1e-3)
}

/// #14900: Layer > Layer Style > Gradient Overlay... seeds a new instance from `effect_defaults`
/// (the page reads its stop list, so the stops carry the colours as well as From/To).
#[test]
fn a_new_gradient_overlay_starts_at_the_factory_values() {
    let d = effect_defaults("gradientOverlay");
    assert_eq!(d["blend"], json!("Normal"), "blend");
    assert_eq!(d["opacity"], json!(100), "opacity");
    assert_eq!(d["stops"], json!([[0, "#d7d7d7"], [1, "#ffffff"]]), "stops");
    assert_eq!((d["from"].as_str(), d["to"].as_str()), (Some("#d7d7d7"), Some("#ffffff")), "from / to");
    assert_eq!(d["method"], json!("smooth"), "method");
    assert_eq!(d["smoothness"], json!(100), "smoothness");
    assert_eq!(d["reverse"], json!(false), "reverse");
    assert_eq!(d["style"], json!("linear"), "style");
    assert_eq!(d["angle"], json!(90), "angle");
    assert_eq!(d["scale"], json!(100), "scale");
}

/// #14900: `layer.layerStyle.gradientOverlay` without colours or stops makes the factory overlay:
/// grey 215 to white with the Smooth method recorded, the rest at the page's values.
#[test]
fn a_gradient_overlay_command_without_colours_makes_the_factory_effect() {
    let mut s = session();
    s.execute("layer.layerStyle.gradientOverlay", json!({})).unwrap();
    let Effect::GradientOverlay { common, gradient, dither, interpolation } = only_effect(&s) else { panic!("not a gradient overlay") };
    assert_eq!((common.blend, common.opacity), (BlendMode::Normal, 1.0));
    assert!(!dither, "dither");
    let Some(GradientInterpolation { method, smoothness, stops }) = interpolation else { panic!("no recorded method") };
    assert_eq!((method.as_str(), smoothness), ("smooth", 100.0));
    assert_eq!(stops.len(), 2, "source stops {stops:?}");
    assert!(stops[0].0 == 0.0 && close(&stops[0].1, [GREY_215; 3]), "first stop {:?}", stops[0]);
    assert!(stops[1].0 == 1.0 && close(&stops[1].1, [1.0; 3]), "last stop {:?}", stops[1]);
    assert!(close(&gradient.stops.first().unwrap().1, [GREY_215; 3]) && close(&gradient.stops.last().unwrap().1, [1.0; 3]), "baked ends");
    assert!(gradient.stops.len() > 2, "the Smooth curve is baked, not straight: {:?}", gradient.stops);
    assert_eq!((gradient.style, gradient.angle, gradient.scale, gradient.reverse, gradient.align), (GradientStyle::Linear, 90.0, 1.0, false, true));
    // The dialog's OK on the factory values (its stop list, no From/To) builds the same effect.
    let mut dialog = effect_defaults("gradientOverlay");
    dialog.as_object_mut().unwrap().retain(|k, _| k != "from" && k != "to");
    assert_eq!(effect_from_params("gradientOverlay", &json!({})), effect_from_params("gradientOverlay", &dialog));
}

/// #14900 measured: OK on the factory values over the white test document. Red channel at
/// x 200 down the rectangle; the reference application gives
/// `255 255 251 246 240 235 235 230 224 219 215 215`, each within 2 levels. Outside stays white.
#[test]
fn the_factory_gradient_overlay_paints_the_measured_column() {
    let mut s = test_document();
    s.execute("layer.layerStyle.gradientOverlay", json!({})).unwrap();
    let d = s.active().unwrap();
    let px = photocraft_compose::render(&d.doc, d.doc.bounds()).into_px();
    let level = |x: usize, y: usize, c: usize| (px[y * 400 + x][c] * 255.0).round() as i32;
    let want = [(75, 255), (76, 255), (94, 251), (112, 246), (131, 240), (149, 235), (150, 235), (168, 230), (187, 224), (205, 219), (223, 215), (224, 215)];
    let got: Vec<i32> = want.iter().map(|&(y, _)| level(200, y, 0)).collect();
    for (&(y, w), g) in want.iter().zip(&got) {
        assert!((g - w).abs() <= 2, "red at (200, {y}): {g}, the reference gives {w}; column {got:?}");
    }
    for (x, y) in [(200, 74), (200, 225), (99, 150), (300, 150)] {
        assert_eq!([0, 1, 2].map(|c| level(x, y, c)), [255; 3], "outside the layer at ({x}, {y})");
    }
}

/// #14900: Reset to Default reads the factory values through `layer.layerStyle.defaultFor` until a
/// Make Default stores other ones; the saved values keep winning.
#[test]
fn reset_to_default_restores_the_gradient_overlay_factory_values() {
    let mut s = session();
    let d = s.execute("layer.layerStyle.defaultFor", json!({"kind": "gradientOverlay"})).unwrap();
    assert_eq!(d["user"], json!(false));
    assert_eq!(d["params"], effect_defaults("gradientOverlay"));
    assert_eq!((d["params"]["method"].as_str(), d["params"]["stops"][0][1].as_str()), (Some("smooth"), Some("#d7d7d7")));
    s.execute("layer.layerStyle.makeDefault", json!({"kind": "gradientOverlay", "params": {"opacity": 40, "method": "linear"}})).unwrap();
    let saved = s.execute("layer.layerStyle.defaultFor", json!({"kind": "gradientOverlay"})).unwrap();
    assert_eq!(saved["user"], json!(true));
    assert_eq!((saved["params"]["opacity"].as_f64(), saved["params"]["method"].as_str()), (Some(40.0), Some("linear")), "a saved default still wins");
}

/// #14900: an overlay the document already carries (a PSD or `.pcraft` one, edited in the dialog)
/// keeps its gradient and method through `layer.layerStyle.replace`, and a command that names its
/// colours still gets those colours.
#[test]
fn a_stored_gradient_overlay_keeps_its_own_gradient_and_method() {
    let mut s = session();
    let stored = Effect::GradientOverlay {
        common: FxCommon::new(BlendMode::Multiply, 0.5),
        gradient: Gradient { stops: vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))], angle: 30.0, ..Gradient::default() },
        dither: true,
        interpolation: None,
    };
    let snapshot = serde_json::to_value(&stored).unwrap();
    s.execute("layer.layerStyle.replace", json!({"effects": [{"kind": "gradientOverlay", "params": {}, "fx": snapshot}]})).unwrap();
    assert_eq!(only_effect(&s), stored, "an edited effect list left the overlay alone");

    s.execute("layer.layerStyle.gradientOverlay", json!({"from": "#ff0000", "to": "#0000ff"})).unwrap();
    let Effect::GradientOverlay { gradient, interpolation, .. } = only_effect(&s) else { panic!("not a gradient overlay") };
    assert_eq!(interpolation, None, "named colours keep today's straight stops");
    assert_eq!(gradient.stops, vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))]);
}
