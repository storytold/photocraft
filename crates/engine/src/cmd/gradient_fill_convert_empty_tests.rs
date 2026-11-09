//! #12015: a live Gradient drag on an empty pixel layer turns that layer into the Gradient fill
//! layer (same name, id and place, a white mask) instead of adding one above it.

use photocraft_doc::{LayerContent, LayerId};
use serde_json::json;

use crate::Session;
use crate::gradient_fill_cmds::CREATE;

fn drag() -> serde_json::Value {
    json!({"from": [0, 0], "to": [48, 0]})
}

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 48, "height": 32, "background": "white"})).unwrap();
    s
}

fn add_layer(s: &mut Session, name: &str) -> LayerId {
    LayerId(s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap())
}

fn names(s: &Session) -> Vec<String> {
    s.active().unwrap().doc.layers.iter().map(|l| l.name.clone()).collect()
}

#[test]
fn an_empty_pixel_layer_becomes_the_gradient_fill_layer_in_place() {
    let mut s = session();
    let top = add_layer(&mut s, "Top");
    let steps = s.active().unwrap().history.past_len();
    let r = s.execute(CREATE, drag()).unwrap();
    assert_eq!(r["layer"].as_u64(), Some(top.0));
    assert_eq!(names(&s), ["Background", "Top"], "the layer count and names do not change");
    let d = s.active().unwrap();
    assert_eq!(d.active_layer, Some(top));
    let l = d.doc.layer(top).unwrap();
    assert!(matches!(l.content, LayerContent::Fill(_)), "{}", l.content.kind_name());
    let m = l.mask.as_ref().expect("a white mask");
    assert_eq!((m.value(0, 0), m.value(47, 31)), (1.0, 1.0));
    assert_eq!(d.history.past_len(), steps + 1, "one history state");
    assert_eq!(d.history.entries().last().map(String::as_str), Some("New Gradient Fill Layer"));
    // Undo restores the empty pixel layer.
    s.execute("edit.undo", json!({})).unwrap();
    let l = s.active().unwrap().doc.layer(top).unwrap();
    assert!(matches!(l.content, LayerContent::Raster(_)) && l.mask.is_none());
}

#[test]
fn a_layer_with_pixels_or_the_background_gets_a_new_layer_above() {
    let mut s = session();
    s.execute(CREATE, drag()).unwrap();
    assert_eq!(names(&s), ["Background", "Gradient Fill 1"]);

    let mut s = session();
    let top = add_layer(&mut s, "Top");
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute(CREATE, drag()).unwrap();
    assert_eq!(names(&s), ["Background", "Top", "Gradient Fill 1"]);
    assert!(matches!(s.active().unwrap().doc.layer(top).unwrap().content, LayerContent::Raster(_)));
}

#[test]
fn a_hidden_or_locked_empty_layer_keeps_getting_a_new_layer() {
    for props in [json!({"visible": false}), json!({"locks": {"all": true}})] {
        let mut s = session();
        let top = add_layer(&mut s, "Top");
        let mut p = props.clone();
        p["layer"] = json!(top.0);
        s.execute("layer.setProps", p).unwrap();
        // A hidden layer refuses the live drag outright; a locked one gets a new layer above.
        let _ = s.execute(CREATE, drag());
        assert!(matches!(s.active().unwrap().doc.layer(top).unwrap().content, LayerContent::Raster(_)), "{props}");
    }
}

#[test]
fn with_a_selection_the_converted_layers_mask_is_the_selection() {
    let mut s = session();
    let top = add_layer(&mut s, "Top");
    s.execute("select.rect", json!({"x": 4, "y": 4, "width": 10, "height": 8})).unwrap();
    s.execute(CREATE, drag()).unwrap();
    assert_eq!(names(&s), ["Background", "Top"]);
    let l = s.active().unwrap().doc.layer(top).unwrap();
    assert!(matches!(l.content, LayerContent::Fill(_)));
    let m = l.mask.as_ref().unwrap();
    assert_eq!((m.value(5, 5), m.value(30, 20)), (1.0, 0.0));
}
