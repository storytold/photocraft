//! Group from Layers adds one history state named "Create Group from Layers" (#14710), Group
//! Layers keeps "Group Layers", undo restores the stack and selection, and Frame from Layers
//! still adds only its own state.

use crate::Session;
use photocraft_doc::LayerId;
use serde_json::json;

/// A 32 x 32 document with the Background and the pixel layers "A" and "B", both selected.
fn session() -> (Session, [LayerId; 2]) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    let ids = ["A", "B"].map(|name| LayerId(s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap()));
    s.active_mut().unwrap().selected_layers = ids.to_vec();
    (s, ids)
}

fn steps(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

fn names(s: &Session) -> Vec<String> {
    s.active().unwrap().doc.layers.iter().map(|l| l.name.clone()).collect()
}

#[test]
fn group_from_layers_adds_one_create_group_from_layers_state() {
    for params in [json!({}), json!({"name": "Set", "color": "red", "mode": "normal", "opacity": 50})] {
        let (mut s, _) = session();
        let before = steps(&s);
        s.execute("layer.new.groupFromLayers", params.clone()).unwrap();
        let after = steps(&s);
        assert_eq!(after.len(), before.len() + 1, "exactly one state for {params}: {after:?}");
        assert_eq!(after.last().map(String::as_str), Some("Create Group from Layers"), "{params}");
    }
}

#[test]
fn group_from_layers_on_an_explicit_layer_is_named_the_same() {
    let (mut s, [a, _]) = session();
    s.execute("layer.new.groupFromLayers", json!({"layer": a.0})).unwrap();
    assert_eq!(steps(&s).last().map(String::as_str), Some("Create Group from Layers"));
}

#[test]
fn group_layers_keeps_its_group_layers_state() {
    let (mut s, _) = session();
    s.execute("layer.groupLayers", json!({})).unwrap();
    assert_eq!(steps(&s).last().map(String::as_str), Some("Group Layers"));
}

#[test]
fn undo_restores_the_stack_and_the_selection() {
    let (mut s, ids) = session();
    let (stack, before) = (names(&s), steps(&s));
    s.execute("layer.new.groupFromLayers", json!({})).unwrap();
    assert_ne!(names(&s), stack, "the layers went into a group");
    s.execute("history.undo", json!({})).unwrap();
    assert_eq!(names(&s), stack, "the stack is back");
    assert_eq!(steps(&s), before, "the state is gone");
    assert_eq!(s.active().unwrap().selected_layers(), ids, "the selection is back");
}

/// With one history state per action step, the played step is "Create Group from Layers".
#[test]
fn a_played_action_records_create_group_from_layers() {
    let (mut s, _) = session();
    s.execute("actions.setHistory", json!({"singleState": false})).unwrap();
    s.actions.list.push(crate::actions_cmds::Action::new("Group", vec![("layer.new.groupFromLayers".to_string(), json!({}))]));
    s.execute("actions.play", json!({"action": "Group"})).unwrap();
    assert_eq!(steps(&s).last().map(String::as_str), Some("Create Group from Layers"), "{:?}", steps(&s));
}

#[test]
fn frame_from_layers_still_adds_only_its_own_state() {
    let (mut s, ids) = session();
    s.edit("setup", |doc, _| {
        let fmt = doc.pixel_format();
        for id in ids {
            let surface = doc.layer_mut(id).and_then(|l| l.surface_mut()).ok_or(crate::EngineError::NoLayer(id))?;
            surface.fill_rect(photocraft_geom::Rect::new(4, 4, 20, 20), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
        }
        Ok(())
    })
    .unwrap();
    let before = steps(&s);
    s.execute("layer.new.frameFromLayers", json!({})).unwrap();
    let after = steps(&s);
    assert_eq!(after.len(), before.len() + 1, "{after:?}");
    assert_eq!(after.last().map(String::as_str), Some("Frame from Layers"));
}
