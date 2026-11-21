//! #14867: a new Color Overlay starts as the reference application's factory one: Normal, mid
//! grey 129/129/129, 100 %, not red. Overlays read from a file keep their stored colour.

use super::*;
use serde_json::json;

const GREY: f32 = 129.0 / 255.0;

/// The issue's test document: 400 x 300, white Background, a pixel layer with an orange
/// (255,128,0) rectangle at x 100..299, y 75..224, active.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 100, "y": 75, "width": 200, "height": 150})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff8000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn overlay(s: &Session) -> Effect {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().effects.items[0].clone()
}

fn rgb_at(s: &mut Session, x: i32, y: i32) -> [u8; 3] {
    let px = s.execute("document.pixel", json!({"x": x, "y": y})).unwrap();
    [0, 1, 2].map(|i| (px[i].as_f64().unwrap() * 255.0).round() as u8)
}

#[test]
fn the_factory_values_are_normal_grey_129_at_100_percent() {
    assert_eq!(effect_defaults("colorOverlay"), json!({"blend": "Normal", "color": "#818181", "opacity": 100}));
    // Reset to Default reads the same values while no Make Default is saved.
    let mut s = session();
    let r = s.execute("layer.layerStyle.defaultFor", json!({"kind": "colorOverlay"})).unwrap();
    assert_eq!(r, json!({"params": {"blend": "Normal", "color": "#818181", "opacity": 100}, "user": false}));
}

#[test]
fn a_color_overlay_without_params_is_the_factory_grey() {
    let mut s = session();
    s.execute("layer.layerStyle.colorOverlay", json!({})).unwrap();
    let Effect::ColorOverlay { common, color } = overlay(&s) else { panic!("not a color overlay") };
    assert_eq!((common.blend, common.opacity), (BlendMode::Normal, 1.0));
    assert_eq!(color, Color::rgb(GREY, GREY, GREY));
    // The same effect as the dialog's factory values.
    assert_eq!(effect_from_params("colorOverlay", &json!({})), effect_from_params("colorOverlay", &effect_defaults("colorOverlay")));
}

#[test]
fn ok_on_the_factory_values_turns_the_layer_grey() {
    for params in [json!({}), effect_defaults("colorOverlay")] {
        let mut s = session();
        s.execute("layer.layerStyle.colorOverlay", params.clone()).unwrap();
        assert_eq!(rgb_at(&mut s, 200, 150), [129, 129, 129], "{params}");
        assert_eq!(rgb_at(&mut s, 99, 74), [255, 255, 255], "{params}");
        assert_eq!(rgb_at(&mut s, 50, 50), [255, 255, 255], "{params}");
    }
}

#[test]
fn an_overlay_read_from_a_file_keeps_its_colour() {
    // A stored overlay (here a red snapshot, as a PSD or .pcraft gives it) is not reset.
    let mut s = session();
    let red = Effect::ColorOverlay { common: photocraft_doc::effects::FxCommon::new(BlendMode::Normal, 1.0), color: Color::rgb(1.0, 0.0, 0.0) };
    s.execute("layer.layerStyle.replace", json!({"effects": [{"kind": "colorOverlay", "fx": red}]})).unwrap();
    assert_eq!(overlay(&s), red);
    assert_eq!(rgb_at(&mut s, 200, 150), [255, 0, 0]);
}
