//! #14931: a new Inner Shadow starts with the reference application's factory values: Multiply,
//! black, Opacity 35 %, Angle 120 with Use Global Light, Distance 3 px, Choke 0 %, Size 7 px,
//! Linear, Anti-aliased off, Noise 0 %. Inner Shadows read from a file keep their values.

use super::*;
use serde_json::json;

/// The issue's test document: 400 x 300 RGB 8 bit, white Background, a pixel layer with an orange
/// (255,128,0) rectangle at x 100..299, y 75..224, active; Global Light 120 / 30.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 100, "y": 75, "width": 200, "height": 150})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff8000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let light = &s.active().unwrap().doc.global_light;
    assert_eq!((light.angle, light.altitude), (120.0, 30.0));
    s
}

fn shadow(s: &Session) -> Shadow {
    let d = s.active().unwrap();
    match &d.doc.layer(d.active_layer.unwrap()).unwrap().effects.items[0] {
        Effect::InnerShadow(sh) => sh.clone(),
        other => panic!("not an inner shadow: {}", other.label()),
    }
}

fn rgb_at(s: &mut Session, x: i32, y: i32) -> [i32; 3] {
    let px = s.execute("document.pixel", json!({"x": x, "y": y})).unwrap();
    [0, 1, 2].map(|i| (px[i].as_f64().unwrap() * 255.0).round() as i32)
}

#[test]
fn the_factory_values_are_the_reference_ones() {
    let d = effect_defaults("innerShadow");
    for (key, want) in [
        ("blend", json!("Multiply")),
        ("color", json!("#000000")),
        ("opacity", json!(35)),
        ("angle", json!(120)),
        ("useGlobalLight", json!(true)),
        ("distance", json!(3)),
        ("choke", json!(0)),
        ("size", json!(7)),
        ("contour", json!("Linear")),
        ("antiAlias", json!(false)),
        ("noise", json!(0)),
    ] {
        assert_eq!(d[key], want, "{key}");
    }
}

#[test]
fn an_inner_shadow_without_params_is_the_factory_one() {
    let mut s = session();
    s.execute("layer.layerStyle.innerShadow", json!({})).unwrap();
    let sh = shadow(&s);
    assert_eq!((sh.common.blend, sh.common.opacity, sh.color), (BlendMode::Multiply, 0.35, Color::BLACK));
    assert_eq!((sh.angle, sh.use_global_light, sh.distance, sh.spread, sh.size), (120.0, true, 3.0, 0.0, 7.0));
    assert_eq!((sh.contour.clone(), sh.anti_alias, sh.noise, sh.knocks_out), (Contour::Linear, false, 0.0, false));
    assert_eq!(effect_from_params("innerShadow", &json!({})), effect_from_params("innerShadow", &effect_defaults("innerShadow")));
}

#[test]
fn ok_on_the_factory_values_gives_the_reference_top_edge() {
    // x 200, y 75..79 of the flattened image, within 1 level.
    let want = [[184, 92, 0], [193, 97, 0], [204, 102, 0], [217, 109, 0], [228, 114, 0]];
    for params in [json!({}), effect_defaults("innerShadow")] {
        let mut s = session();
        s.execute("layer.layerStyle.innerShadow", params.clone()).unwrap();
        for (y, w) in (75..80).zip(want) {
            let got = rgb_at(&mut s, 200, y);
            assert!(got.iter().zip(w).all(|(g, w)| (g - w).abs() <= 1), "{params} y {y}: got {got:?}, want {w:?}");
        }
    }
}

#[test]
fn the_drop_shadow_keeps_its_values() {
    // Its own item (#14878): 75 %, Distance 5, Size 5 for now.
    let d = effect_defaults("dropShadow");
    assert_eq!((d["opacity"].clone(), d["distance"].clone(), d["size"].clone()), (json!(75), json!(5), json!(5)));
    let Some(Effect::DropShadow(sh)) = effect_from_params("dropShadow", &json!({})) else { panic!("no drop shadow") };
    assert_eq!((sh.common.opacity, sh.distance, sh.size), (0.75, 5.0, 5.0));
}

#[test]
fn an_inner_shadow_read_from_a_file_keeps_its_values() {
    let mut s = session();
    let Some(Effect::InnerShadow(mut stored)) = effect_from_params("innerShadow", &json!({})) else { panic!("no inner shadow") };
    stored.common.opacity = 0.75;
    stored.distance = 5.0;
    stored.size = 5.0;
    let fx = Effect::InnerShadow(stored.clone());
    s.execute("layer.layerStyle.replace", json!({"effects": [{"kind": "innerShadow", "fx": fx}]})).unwrap();
    assert_eq!(shadow(&s), stored);
}

#[test]
fn the_command_advertises_the_new_defaults() {
    // The params doc callers read (CodeRabbit on #15569).
    let spec = crate::commands::find("layer.layerStyle.innerShadow").unwrap();
    for default in ["\"opacity\":0..100=35", "\"distance\":px=3", "\"size\":px=7"] {
        assert!(spec.params.contains(default), "{default} in {}", spec.params);
    }
}
