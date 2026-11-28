//! History names of the Layers panel's thumbnail gestures (#16786), run with the params the panel sends, and the
//! Layer and Select menus' calls of the same commands, which keep their names.

use photocraft_engine::Session;
use serde_json::{Value, json};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    s
}

fn active(s: &Session) -> u64 {
    s.active().unwrap().active_layer.unwrap().0
}

/// A new pixel layer filled with red, active.
fn pixel_layer(s: &mut Session) -> u64 {
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    active(s)
}

/// The name of the one history state `cmd` with `params` adds.
fn step(s: &mut Session, cmd: &str, params: Value) -> String {
    let before = s.active().unwrap().history.entries().len();
    s.execute(cmd, params.clone()).unwrap_or_else(|e| panic!("{cmd} {params}: {e}"));
    let after = s.active().unwrap().history.entries();
    assert_eq!(after.len(), before + 1, "{cmd} {params} adds one state: {after:?}");
    after[before].clone()
}

/// ⌘-click (with ⇧ / ⌥ too) on a pixel, type, smart object or adjustment thumbnail, Select Pixels and ⌘-click on a
/// layer mask are `Load Selection`; on a shape thumbnail or a vector mask, `Selection Change`.
#[test]
fn thumbnail_loads_are_named_as_their_kind() {
    let mut s = session();
    let pixel = pixel_layer(&mut s);
    pixel_layer(&mut s);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let smart = active(&s);
    let typ = s.execute("type.create", json!({"text": "A", "x": 5, "y": 20})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let adjustment = active(&s);
    for layer in [pixel, smart, typ, adjustment] {
        for operation in ["new", "add", "subtract", "intersect"] {
            let params = json!({"channel": "transparency", "layer": layer, "operation": operation});
            assert_eq!(step(&mut s, "select.loadSelection", params), "Load Selection", "layer {layer}, {operation}");
        }
    }
    s.execute("layer.layerMask.revealAll", json!({"layer": pixel})).unwrap();
    assert_eq!(step(&mut s, "select.loadSelection", json!({"channel": "mask", "layer": pixel, "operation": "new"})), "Load Selection");

    let shape = s.execute("shape.create", json!({"kind": "rect", "rect": [2, 2, 20, 20]})).unwrap()["layer"].as_u64().unwrap();
    let params = json!({"channel": "transparency", "layer": shape, "operation": "new"});
    assert_eq!(step(&mut s, "select.loadSelection", params), "Selection Change", "shape thumbnail");
    s.execute("layer.vectorMask.revealAll", json!({"layer": pixel})).unwrap();
    let params = json!({"channel": "vectorMask", "layer": pixel, "operation": "add"});
    assert_eq!(step(&mut s, "select.loadSelection", params), "Selection Change", "vector mask");
}

/// Select › Load Selection names no layer, and keeps its names.
#[test]
fn the_menu_load_keeps_its_names() {
    let mut s = session();
    let layer = pixel_layer(&mut s);
    s.execute("layer.layerMask.revealAll", json!({"layer": layer})).unwrap();
    assert_eq!(step(&mut s, "select.loadSelection", json!({"channel": "transparency"})), "Load Transparency");
    assert_eq!(step(&mut s, "select.loadSelection", json!({"channel": "mask"})), "Load Layer Mask");
}

/// ⇧-click on a mask and the chain say which way they went; the Layer menu's plain toggles keep their names.
#[test]
fn mask_toggles_and_chains_name_their_direction() {
    let mut s = session();
    let layer = pixel_layer(&mut s);
    s.execute("layer.layerMask.revealAll", json!({"layer": layer})).unwrap();
    s.execute("layer.vectorMask.revealAll", json!({"layer": layer})).unwrap();
    let cases = [
        ("layer.layerMask.enabled", "enabled", ["Disable Layer Mask", "Enable Layer Mask"], "Enable/Disable Layer Mask"),
        ("layer.layerMask.linked", "linked", ["Unlink Mask", "Link Mask"], "Link/Unlink Layer Mask"),
        ("layer.vectorMask.enabled", "enabled", ["Disable Vector Mask", "Enable Vector Mask"], "Edit Vector Mask"),
        ("layer.vectorMask.linked", "linked", ["Unlink Vector Mask", "Link Vector Mask"], "Edit Vector Mask"),
    ];
    for (cmd, key, [off, on], toggle) in cases {
        assert_eq!(step(&mut s, cmd, json!({"layer": layer, key: false})), off);
        assert_eq!(step(&mut s, cmd, json!({"layer": layer, key: true})), on);
        assert_eq!(step(&mut s, cmd, json!({})), toggle, "{cmd} from the menu");
    }
}

/// Dragging a layer mask onto another layer, or ⌥-dragging a copy there, is one `Add Layer Mask` state, and one undo
/// puts both layers back.
#[test]
fn a_mask_drop_is_one_add_layer_mask_state() {
    for copy in [false, true] {
        let mut s = session();
        let from = pixel_layer(&mut s);
        let to = pixel_layer(&mut s);
        s.execute("layer.layerMask.revealAll", json!({"layer": from})).unwrap();
        let before = s.active().unwrap().doc.clone();
        assert_eq!(step(&mut s, "layer.layerMask.moveTo", json!({"layer": from, "to": to, "copy": copy})), "Add Layer Mask", "copy {copy}");
        s.execute("edit.undo", json!({})).unwrap();
        let doc = &s.active().unwrap().doc;
        for id in [from, to] {
            let has_mask = |d: &photocraft_doc::Document| d.layer(photocraft_doc::LayerId(id)).unwrap().mask.is_some();
            assert_eq!(has_mask(doc), has_mask(&before), "copy {copy}: one undo restores layer {id}");
        }
    }
}
