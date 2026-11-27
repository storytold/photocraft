//! #14204: Auto-Blend Layers is unavailable when a type layer or a smart object is among the
//! selected layers, instead of quietly blending only the pixel layers; a run is refused too and
//! changes nothing. Pixel layers alone still blend; Auto-Align's enabled state is unchanged.

use photocraft_doc::{Document, LayerId};
use photocraft_engine::Session;
use serde_json::json;

const BLEND_UNAVAILABLE: &str = "The command \"Auto-Blend Layers\" is not currently available.";

/// A document with two textured pixel layers, `a` and `b`, both selected.
fn two_pixel_layers() -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 240, "depth": 8, "background": "transparent"})).unwrap();
    let mut ids = Vec::new();
    for (name, dx) in [("A", 20), ("B", 31)] {
        s.execute("layer.new.layer", json!({"name": name})).unwrap();
        for i in 0..40 {
            let (x, y) = (dx + (i * 37) % 200, 20 + (i * 53) % 150);
            s.execute("select.rect", json!({"x": x, "y": y, "width": 9, "height": 7})).unwrap();
            s.execute("edit.fill", json!({"color": format!("#{:02x}{:02x}{:02x}", i * 6 % 256, 255 - i * 5 % 256, i * 11 % 256)})).unwrap();
        }
        s.execute("select.deselect", json!({})).unwrap();
        ids.push(s.active().unwrap().active_layer.unwrap());
    }
    s.execute("layer.select", json!({"layers": [ids[0].0, ids[1].0]})).unwrap();
    (s, ids[0], ids[1])
}

/// The document and History, to see that a refused run changed nothing.
fn snapshot(s: &Session) -> (std::sync::Arc<Document>, usize) {
    let st = s.active().unwrap();
    (st.doc.clone(), st.history.past_len())
}

fn assert_unavailable(s: &mut Session) {
    assert!(!s.is_enabled("edit.autoBlendLayers"), "the menu item is disabled");
    let (doc, steps) = snapshot(s);
    let err = s.execute("edit.autoBlendLayers", json!({"method": "panorama"})).unwrap_err().to_string();
    assert!(err.ends_with(BLEND_UNAVAILABLE), "{err}");
    assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc), "document unchanged");
    assert_eq!(s.active().unwrap().history.past_len(), steps, "no history state");
}

#[test]
fn pixel_layers_alone_still_blend() {
    let (mut s, ..) = two_pixel_layers();
    assert!(s.is_enabled("edit.autoBlendLayers"));
    s.execute("edit.autoBlendLayers", json!({"method": "panorama"})).unwrap();
}

#[test]
fn a_type_layer_among_the_selected_layers() {
    let (mut s, a, b) = two_pixel_layers();
    let text = s.execute("type.create", json!({"x": 2, "y": 20, "text": "I", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layers": [a.0, b.0, text]})).unwrap();
    assert_unavailable(&mut s);
}

#[test]
fn a_smart_object_among_the_selected_layers() {
    let (mut s, a, b) = two_pixel_layers();
    let smart = s.execute("layer.smartObjects.convertToSmartObject", json!({"layer": a.0})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layers": [smart, b.0]})).unwrap();
    assert_unavailable(&mut s);
}

#[test]
fn auto_align_keeps_its_enabled_state() {
    let (mut s, a, b) = two_pixel_layers();
    let text = s.execute("type.create", json!({"x": 2, "y": 20, "text": "I", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layers": [a.0, b.0, text]})).unwrap();
    assert!(s.is_enabled("edit.autoAlignLayers"), "Auto-Align still counts the pixel layers only");
}
