//! #14874: in a Grayscale document a layer effect's RGB colour is stored as the grey the reference
//! application gives, through Gray Gamma 2.2 (D50): pure blue is grey 71 (K 72 %), not the
//! Rec.601 luma 29. RGB, Lab and CMYK documents keep today's results.

use super::*;
use photocraft_color::ColorMode;
use photocraft_doc::effects::FxPaint;
use serde_json::json;

/// A 16 x 16 document in `mode` with a white-filled pixel layer, active.
fn session(mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "mode": mode, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "white"})).unwrap();
    s
}

fn effects(s: &Session) -> Vec<Effect> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().effects.items.clone()
}

/// The composite's first channel at 8,8 on the 0..255 scale.
fn grey(s: &mut Session) -> f32 {
    let px = s.execute("document.pixel", json!({"x": 8, "y": 8})).unwrap();
    px[0].as_f64().unwrap() as f32 * 255.0
}

fn assert_level(got: f32, want: f32, what: &str) {
    assert!((got - want).abs() <= 1.0, "{what}: got {got}, want {want} within 1 level");
}

#[test]
fn a_blue_color_overlay_is_grey_71() {
    let mut s = session("gray");
    s.execute("layer.layerStyle.colorOverlay", json!({"color": "#0000ff", "blend": "Normal", "opacity": 100})).unwrap();
    let Effect::ColorOverlay { color, .. } = effects(&s)[0] else { panic!("not a color overlay") };
    assert_eq!(color.mode, ColorMode::Grayscale);
    assert_level(color.c[0] * 255.0, 71.0, "stored grey");
    assert_level(grey(&mut s), 71.0, "the layer's composite");
}

#[test]
fn red_and_green_follow_the_same_conversion() {
    // Gray Gamma 2.2 of the D50 luminance: red 129, green 219 (luma would give 76 and 150).
    for (hex, want) in [("#ff0000", 129.0), ("#00ff00", 219.0), ("#0000ff", 71.0)] {
        let mut s = session("gray");
        s.execute("layer.layerStyle.colorOverlay", json!({"color": hex})).unwrap();
        assert_level(grey(&mut s), want, hex);
    }
}

#[test]
fn the_whole_list_replace_converts_the_same_way() {
    let mut s = session("gray");
    s.execute("layer.layerStyle.replace", json!({"effects": [{"kind": "colorOverlay", "params": {"color": "#0000ff"}}]})).unwrap();
    assert_level(grey(&mut s), 71.0, "replace");
}

#[test]
fn every_effect_colour_goes_through_the_same_conversion() {
    let blue = |c: &Color, what: &str| {
        assert_eq!(c.mode, ColorMode::Grayscale, "{what}");
        assert_level(c.c[0] * 255.0, 71.0, what);
    };
    let mut s = session("gray");
    for (kind, params) in [
        ("dropShadow", json!({"color": "#0000ff"})),
        ("innerShadow", json!({"color": "#0000ff"})),
        ("outerGlow", json!({"color": "#0000ff"})),
        ("innerGlow", json!({"color": "#0000ff"})),
        ("stroke", json!({"color": "#0000ff"})),
        ("satin", json!({"color": "#0000ff"})),
        ("bevelEmboss", json!({"highlightColor": "#0000ff", "shadowColor": "#0000ff"})),
        ("gradientOverlay", json!({"from": "#0000ff", "to": "#0000ff"})),
    ] {
        s.execute(&format!("layer.layerStyle.{kind}"), params).unwrap();
    }
    for fx in effects(&s) {
        match &fx {
            Effect::DropShadow(sh) | Effect::InnerShadow(sh) => blue(&sh.color, fx.label()),
            Effect::OuterGlow(g) | Effect::InnerGlow(g) => match &g.paint {
                FxPaint::Color(c) => blue(c, fx.label()),
                other => panic!("{other:?}"),
            },
            Effect::Stroke(st) => match &st.paint {
                FxPaint::Color(c) => blue(c, fx.label()),
                other => panic!("{other:?}"),
            },
            Effect::Satin(sa) => blue(&sa.color, fx.label()),
            Effect::BevelEmboss(b) => {
                blue(&b.highlight_color, "bevel highlight");
                blue(&b.shadow_color, "bevel shadow");
            }
            Effect::GradientOverlay { gradient, .. } => gradient.stops.iter().for_each(|st| blue(&st.1, "gradient stop")),
            other => panic!("unexpected {}", other.label()),
        }
    }
}

#[test]
fn neutral_greys_keep_their_value() {
    // Not measured: a grey keeps its level, so a stored grey the dialog sends back as R = G = B
    // does not move on every OK.
    let mut s = session("gray");
    s.execute("layer.layerStyle.colorOverlay", json!({"color": "#808080"})).unwrap();
    let Effect::ColorOverlay { color, .. } = effects(&s)[0] else { panic!("not a color overlay") };
    assert!((color.c[0] - 128.0 / 255.0).abs() < 1e-6, "{color:?}");
    for _ in 0..3 {
        let entry = json!({"kind": "colorOverlay", "fx": effects(&s)[0], "params": {"color": "#808080"}});
        s.execute("layer.layerStyle.replace", json!({"effects": [entry]})).unwrap();
    }
    let Effect::ColorOverlay { color, .. } = effects(&s)[0] else { panic!("not a color overlay") };
    assert!((color.c[0] - 128.0 / 255.0).abs() < 1e-6, "{color:?}");
}

#[test]
fn a_stored_grey_snapshot_is_kept() {
    // An effect that already holds a grey (from a PSD, or an earlier edit) keeps it.
    let mut s = session("gray");
    let fx = Effect::ColorOverlay {
        common: photocraft_doc::effects::FxCommon::new(BlendMode::Normal, 1.0),
        color: Color { mode: ColorMode::Grayscale, c: [0.2, 0.0, 0.0, 0.0], alpha: 1.0 },
    };
    s.execute("layer.layerStyle.replace", json!({"effects": [{"kind": "colorOverlay", "fx": fx}]})).unwrap();
    let Effect::ColorOverlay { color, .. } = effects(&s)[0] else { panic!("not a color overlay") };
    assert_eq!(color.c[0], 0.2);
}

#[test]
fn other_depths_keep_todays_luma() {
    // Not measured: 16- and 32-bit Grayscale documents keep the Rec.601 luma.
    for depth in [16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16, "mode": "gray", "depth": depth, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("layer.layerStyle.colorOverlay", json!({"color": "#0000ff"})).unwrap();
        let Effect::ColorOverlay { color, .. } = effects(&s)[0] else { panic!("not a color overlay") };
        assert_eq!(color, Color::rgb(0.0, 0.0, 1.0).in_mode(ColorMode::Grayscale), "{depth}-bit");
    }
}

#[test]
fn rgb_lab_and_cmyk_documents_keep_todays_colours() {
    for mode in ["rgb", "lab", "cmyk"] {
        let mut s = session(mode);
        s.execute("layer.layerStyle.colorOverlay", json!({"color": "#0000ff"})).unwrap();
        let Effect::ColorOverlay { color, .. } = effects(&s)[0] else { panic!("not a color overlay") };
        let doc_mode = s.active().unwrap().doc.mode;
        assert_eq!(color, Color::rgb(0.0, 0.0, 1.0).in_mode(doc_mode), "{mode}");
    }
}
