//! Tests that run paint commands, moved from the engine-core `layer_list_cmds` tests: 1 test(s).

use photocraft_engine::Session;
use serde_json::json;

fn session_with_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s
}

#[test]
fn lists_ids_names_kinds_and_nesting_top_down() {
    let mut s = session_with_doc();
    let bg = s.active().unwrap().active_layer.unwrap().0;
    let inner = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("layer.groupLayers", json!({})).unwrap();
    let group = s.active().unwrap().doc.layers.last().unwrap().id.0;
    s.execute("layer.select", json!({"layer": inner})).unwrap();
    s.execute("layer.setProps", json!({"layer": bg, "visible": false})).unwrap();
    let r = s.execute("layer.list", json!({})).unwrap();
    let layers = r["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 2, "{r}");
    assert_eq!((layers[0]["id"].as_u64(), layers[0]["kind"].as_str()), (Some(group), Some("Group")), "{r}");
    assert_eq!(layers[0]["children"][0]["id"], inner, "{r}");
    assert_eq!(layers[0]["children"][0]["active"], true, "{r}");
    assert!(layers[0]["children"][0]["bounds"].is_array(), "{r}");
    assert_eq!(layers[0]["bounds"], layers[0]["children"][0]["bounds"], "a group spans its children");
    assert_eq!((layers[1]["id"].as_u64(), layers[1]["name"].as_str(), layers[1]["visible"].as_bool()), (Some(bg), Some("Background"), Some(false)));
    assert!(layers[1].get("children").is_none());
    assert_eq!(r["activeLayer"], inner);
    // Flat: one row per layer, with its depth and parent group.
    let r = s.execute("layer.list", json!({"flat": true})).unwrap();
    let rows: Vec<(u64, u64, Option<u64>)> =
        r["layers"].as_array().unwrap().iter().map(|l| (l["id"].as_u64().unwrap(), l["depth"].as_u64().unwrap(), l["parent"].as_u64())).collect();
    assert_eq!(rows, [(group, 0, None), (inner, 1, Some(group)), (bg, 0, None)]);
}
