//! Tests that run paint commands, moved from the engine-core `background_cmds` tests: 3 test(s).

use photocraft_doc::{Document, LayerId};
use photocraft_engine::Session;
use serde_json::json;

use photocraft_geom::Rect;

/// Bottom to top: Layer 0 (the old Background, now a normal layer), A, B. A has a 4×4 red
/// square at (2, 2); the rest of it is transparent.
fn session(depth: u32) -> (Session, [LayerId; 3]) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 12, "depth": depth})).unwrap();
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    let l0 = doc(&s).layers[0].id;
    let id = |s: &mut Session, name: &str| LayerId(s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap());
    let a = id(&mut s, "A");
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 4, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let b = id(&mut s, "B");
    (s, [l0, a, b])
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn order(s: &Session) -> Vec<LayerId> {
    doc(s).layers.iter().map(|l| l.id).collect()
}

fn pixel(s: &Session, x: i32, y: i32) -> Vec<f32> {
    doc(s).layers[0].surface().unwrap().read_region(Rect::new(x, y, x + 1, y + 1))
}

/// A Background (the white one `file.new` makes) with A and B above it.
fn with_background() -> (Session, [LayerId; 3]) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
    let bg = doc(&s).layers[0].id;
    let a = LayerId(s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap());
    let b = LayerId(s.execute("layer.new.layer", json!({"name": "B"})).unwrap()["layer"].as_u64().unwrap());
    (s, [bg, a, b])
}

#[test]
fn makes_the_layer_the_background_at_the_bottom() {
    for depth in [8, 16, 32] {
        let (mut s, [l0, a, b]) = session(depth);
        s.tools.background = [0.0, 0.0, 1.0, 1.0];
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        assert!(s.is_enabled("layer.new.backgroundFromLayer"), "{depth}");
        assert_eq!(s.execute("layer.new.backgroundFromLayer", json!({})).unwrap()["layer"], a.0);
        assert_eq!(order(&s), [a, l0, b], "{depth}: drops to the bottom");
        let l = &doc(&s).layers[0];
        assert!(doc(&s).is_background(l.id) && l.name == "Background", "{depth}");
        assert_eq!(s.active().unwrap().active_layer, Some(a));
        // Its pixels stay; transparency takes the background colour (blue).
        let red = pixel(&s, 3, 3);
        let blue = pixel(&s, 10, 10);
        assert!((red[0] - 1.0).abs() < 1e-3 && red[2].abs() < 1e-3 && (red[3] - 1.0).abs() < 1e-3, "{depth}: {red:?}");
        assert!(blue[0].abs() < 1e-3 && (blue[2] - 1.0).abs() < 1e-3 && (blue[3] - 1.0).abs() < 1e-3, "{depth}: {blue:?}");
        // Now there is a Background: the item greys, Layer from Background is back.
        assert!(!s.is_enabled("layer.new.backgroundFromLayer"));
        assert!(s.execute("layer.new.backgroundFromLayer", json!({"layer": b.0})).is_err(), "{depth}: one Background per document");
        assert!(s.is_enabled("layer.new.layerFromBackground"));
        // One step: undo restores the layer where it was, transparent.
        assert!(s.undo());
        assert_eq!(order(&s), [l0, a, b], "{depth}");
        let l = doc(&s).layer(a).unwrap();
        assert!(!doc(&s).is_background(l.id) && l.name == "A");
        assert!(s.redo());
        assert_eq!(order(&s), [a, l0, b], "{depth}");
        assert!(doc(&s).is_background(doc(&s).layers[0].id));
    }
}

#[test]
fn a_layer_in_a_group_leaves_it_and_keeps_its_look() {
    let (mut s, [l0, a, b]) = session(8);
    s.execute("layer.select", json!({"layer": b.0})).unwrap();
    s.execute("layer.groupLayers", json!({})).unwrap();
    s.execute("layer.setProps", json!({"layer": b.0, "opacity": 0.5, "visible": false})).unwrap();
    s.execute("layer.select", json!({"layer": b.0})).unwrap();
    s.execute("layer.new.backgroundFromLayer", json!({})).unwrap();
    let d = doc(&s);
    assert_eq!(d.layers[0].id, b);
    assert!(d.layers[1..].iter().all(|l| l.id != b));
    assert_eq!(d.layers[1].id, l0);
    assert_eq!(d.layers[2].id, a);
    let bg = &d.layers[0];
    assert!(d.is_background(bg.id) && !bg.visible, "visibility is kept");
    assert_eq!(bg.opacity, 1.0);
}

#[test]
fn refuses_groups_adjustments_missing_layers_and_bad_params() {
    // No document.
    let mut s = Session::new();
    assert!(!s.is_enabled("layer.new.backgroundFromLayer"));
    assert!(s.execute("layer.new.backgroundFromLayer", json!({})).is_err());
    // Already a Background: unavailable, for any layer.
    let (mut s, [bg, a, _]) = with_background();
    assert!(!s.is_enabled("layer.new.backgroundFromLayer"));
    assert!(s.execute("layer.new.backgroundFromLayer", json!({"layer": a.0})).is_err());
    assert!(s.execute("layer.new.backgroundFromLayer", json!({"layer": bg.0})).is_err());
    // Bad params.
    let (mut s, [_, a, _]) = session(8);
    for p in [json!({"layer": "A"}), json!({"layer": -1}), json!({"layer": 1.5}), json!({"layer": u64::MAX}), json!({"layer": [a.0]})] {
        assert!(s.execute("layer.new.backgroundFromLayer", p.clone()).is_err(), "{p}");
    }
    assert!(doc(&s).background().is_none());
    // A group or an adjustment layer can't become the Background.
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.groupLayers", json!({})).unwrap();
    assert!(!s.is_enabled("layer.new.backgroundFromLayer"), "the group is active");
    assert!(s.execute("layer.new.backgroundFromLayer", json!({})).is_err());
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    assert!(!s.is_enabled("layer.new.backgroundFromLayer"));
    assert!(s.execute("layer.new.backgroundFromLayer", json!({})).is_err());
    assert!(doc(&s).background().is_none());
}
