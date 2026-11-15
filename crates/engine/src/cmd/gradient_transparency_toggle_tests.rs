//! #12009: `paint.gradient` takes `useTransparency` (true by default). Off, the gradient's opacity
//! stops are ignored: "Foreground to Transparent" with a black foreground paints opaque black
//! everywhere. On or omitted keeps today's fade.

use photocraft_doc::LayerId;
use serde_json::{Value, json};

use crate::Session;

/// An 800 × 600 document with an empty pixel layer (active) and a black foreground.
fn session() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    s.execute("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    let l = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    (s, l)
}

fn drag(s: &mut Session, extra: Value) -> Result<Value, String> {
    let mut p = json!({"from": [100, 300], "to": [700, 300], "gradient": "Foreground to Transparent", "dither": false});
    if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(extra.clone());
    }
    s.execute("paint.gradient", p).map_err(|e| e.to_string())
}

fn rgba(s: &Session, l: LayerId, x: i32) -> [i32; 4] {
    let px = s.active().unwrap().doc.layer(l).unwrap().surface().unwrap().pixel(x, 300);
    std::array::from_fn(|i| (px[i] * 255.0).round() as i32)
}

#[test]
fn off_paints_the_colours_fully_opaque() {
    for method in [json!(null), json!("smooth"), json!("classic")] {
        let (mut s, l) = session();
        drag(&mut s, json!({"useTransparency": false, "method": method})).unwrap();
        for x in [50, 400, 750] {
            assert_eq!(rgba(&s, l, x), [0, 0, 0, 255], "x {x}, method {method}");
        }
    }
}

#[test]
fn on_or_omitted_keeps_the_fade() {
    let (mut on, l) = session();
    drag(&mut on, json!({"useTransparency": true})).unwrap();
    assert_eq!(rgba(&on, l, 50)[3], 255);
    assert_eq!(rgba(&on, l, 750)[3], 0, "transparent past the end");
    let mid = rgba(&on, l, 400)[3];
    assert!(mid > 0 && mid < 255, "fading in the middle: {mid}");
    let (mut omitted, l2) = session();
    drag(&mut omitted, json!({})).unwrap();
    for x in [50, 250, 400, 550, 750] {
        assert_eq!(rgba(&omitted, l2, x), rgba(&on, l, x), "omitted is on, x {x}");
    }
}

#[test]
fn a_wrong_type_keeps_the_default_and_never_panics() {
    let (mut on, l) = session();
    drag(&mut on, json!({})).unwrap();
    for bad in [json!("no"), json!(0), json!(null), json!([false]), json!({"x": 1})] {
        let (mut s, l2) = session();
        drag(&mut s, json!({"useTransparency": bad})).unwrap();
        assert_eq!(rgba(&s, l2, 400), rgba(&on, l, 400), "{bad}: as on");
    }
}

#[test]
fn the_params_doc_lists_it() {
    let spec = crate::command_specs().iter().find(|c| c.id == "paint.gradient").unwrap();
    assert!(spec.params.contains("\"useTransparency\":bool=true"), "{}", spec.params);
}
