//! #10700: recorded Link and Unlink must not replay as a state-dependent toggle.
use photocraft_doc::{Layer, LayerId};
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use photocraft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value as D},
};
use serde_json::json;
use std::sync::Arc;

fn fixture(groups: &[Option<u64>], selected: &[usize]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 2, "height": 2})).unwrap();
    let st = s.active_mut().unwrap();
    let doc = Arc::make_mut(&mut st.doc);
    doc.layers = groups
        .iter()
        .enumerate()
        .map(|(i, group)| {
            let mut layer = Layer::raster(format!("Layer {i}"), doc.pixel_format());
            layer.link_group = *group;
            layer
        })
        .collect();
    let ids: Vec<_> = selected.iter().map(|i| doc.layers[*i].id).collect();
    s.restore_layer_selection(ids.last().copied(), &ids, ids.first().copied()).unwrap();
    s
}

fn groups(s: &Session) -> Vec<Option<u64>> {
    s.active().unwrap().doc.layers.iter().map(|layer| layer.link_group).collect()
}

fn target() -> D {
    D::Reference(vec![ReferenceItem::Enumerated {
        class: Class { name: UnicodeString::new_nul(""), class_id: Id::new("Lyr ") },
        type_id: Id::new("Ordn"),
        value: Id::new("Trgt"),
    }])
}

fn file(event: &str, descriptor: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Links"),
        expanded: false,
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("A"),
            items: vec![AtnItem { event: AtnEvent::name(event), descriptor: Some(descriptor), ..Default::default() }],
            ..Default::default()
        }],
        warnings: vec![],
    })
    .unwrap()
}

fn load(s: &mut Session, bytes: &[u8]) -> serde_json::Value {
    s.execute("actions.load", json!({"data": photocraft_engine::paint::tile::b64_encode(bytes)})).unwrap()
}

#[test]
fn explicit_links_are_idempotent_and_omitted_state_still_toggles() {
    let mut s = fixture(&[None, None], &[0, 1]);
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    s.execute("layer.linkLayers", json!({"linked": false})).unwrap();
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.past_len(), history);
    s.execute("layer.linkLayers", json!({"linked": true})).unwrap();
    let linked = s.active().unwrap().doc.clone();
    assert!(groups(&s)[0].is_some());
    assert_eq!(groups(&s)[0], groups(&s)[1]);
    let history = s.active().unwrap().history.past_len();
    s.execute("layer.linkLayers", json!({"linked": true})).unwrap();
    assert_eq!(s.active().unwrap().doc, linked);
    assert_eq!(s.active().unwrap().history.past_len(), history);
    s.execute("layer.linkLayers", json!({})).unwrap();
    assert_eq!(groups(&s), [None, None]);
    s.execute("layer.linkLayers", json!({})).unwrap();
    assert!(groups(&s).iter().all(Option::is_some));
}

#[test]
fn unlink_cleans_singletons_preserves_other_groups_and_undoes() {
    for initial in [vec![Some(77), Some(77), Some(77), Some(93), Some(93)], vec![Some(77), None, Some(77), Some(93), Some(93)]] {
        let mut s = fixture(&initial, &[0, 1]);
        let before = s.active().unwrap().doc.clone();
        s.execute("layer.linkLayers", json!({"linked": false})).unwrap();
        assert_eq!(groups(&s), [None, None, None, Some(93), Some(93)]);
        let history = s.active().unwrap().history.past_len();
        s.execute("layer.linkLayers", json!({"linked": false})).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), history);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
    let mut one = fixture(&[Some(77), Some(77)], &[0]);
    one.execute("layer.linkLayers", json!({"linked": false})).unwrap();
    assert_eq!(groups(&one), [None, None]);
    let history = one.active().unwrap().history.past_len();
    one.execute("layer.linkLayers", json!({"linked": false})).unwrap();
    assert_eq!(one.active().unwrap().history.past_len(), history);
    assert!(!one.is_enabled("layer.linkLayers"), "the UI toggle still requires two unlinked layers");
}

#[test]
fn invalid_link_state_fails_without_editing() {
    let mut s = fixture(&[None, None], &[0, 1]);
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    for invalid in [json!(null), json!("false"), json!(0), json!([]), json!({})] {
        assert!(s.execute("layer.linkLayers", json!({"linked": invalid})).is_err());
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
    for active in [None, Some(LayerId(u64::MAX))] {
        let st = s.active_mut().unwrap();
        st.active_layer = active;
        st.selected_layers = active.into_iter().collect();
        assert!(s.execute("layer.linkLayers", json!({"linked": false})).is_err());
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
    assert!(Session::new().execute("layer.linkLayers", json!({"linked": false})).is_err());
}

#[test]
fn atn_link_events_keep_descriptors_and_replay_the_requested_operation() {
    for (event, linked) in [("linkSelectedLayers", true), ("unlinkSelectedLayers", false)] {
        for initial in [vec![None, None, None, Some(93), Some(93)], vec![Some(77), Some(77), Some(77), Some(93), Some(93)]] {
            let descriptor = Descriptor::new("null").with("null", target());
            let mut s = fixture(&initial, &[0, 1]);
            assert_eq!(load(&mut s, &file(event, descriptor.clone()))["unsupported"], json!([]));
            assert_eq!(s.actions.list[0].steps, [("layer.linkLayers".into(), json!({"linked": linked}))]);
            let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
            assert_eq!(saved["skipped"], json!([]));
            let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
            let parsed = atn::parse(&bytes).unwrap();
            assert_eq!(parsed.actions[0].items[0].event, AtnEvent::name(event));
            assert_eq!(parsed.actions[0].items[0].descriptor, Some(descriptor));
            let mut replay = fixture(&initial, &[0, 1]);
            assert_eq!(load(&mut replay, &bytes)["unsupported"], json!([]));
            replay.execute("actions.play", json!({"action": "A", "set": "Links"})).unwrap();
            if linked {
                let result = groups(&replay);
                assert!(result[0].is_some());
                assert_eq!(result[0], result[1]);
                assert_eq!(&result[2..], &[initial[2], Some(93), Some(93)]);
            } else {
                assert_eq!(groups(&replay), [None, None, None, Some(93), Some(93)]);
            }
            let before = replay.active().unwrap().doc.clone();
            let history = replay.active().unwrap().history.past_len();
            replay.execute("actions.play", json!({"action": "A", "set": "Links"})).unwrap();
            assert_eq!(replay.active().unwrap().doc, before);
            assert_eq!(replay.active().unwrap().history.past_len(), history);
        }
    }
}

#[test]
fn unsupported_link_descriptors_are_kept_and_ambiguous_exports_are_refused() {
    for event in ["linkSelectedLayers", "unlinkSelectedLayers"] {
        for descriptor in [
            Descriptor::new("null"),
            Descriptor::new("null").with("null", D::Integer(1)),
            Descriptor::new("other").with("null", target()),
            Descriptor::new("null").with("null", target()).with("extra", D::Boolean(true)),
            Descriptor::new("null").with("null", target()).with("null", target()),
        ] {
            let mut s = Session::new();
            assert_eq!(load(&mut s, &file(event, descriptor.clone()))["unsupported"].as_array().unwrap().len(), 1);
            let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
            let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
            let parsed = atn::parse(&bytes).unwrap();
            assert_eq!(parsed.actions[0].items[0].event, AtnEvent::name(event));
            assert_eq!(parsed.actions[0].items[0].descriptor, Some(descriptor));
        }
    }
    for params in [json!({}), json!({"linked": null}), json!({"linked": true, "layer": "Layer 0"}), json!({"linked": false, "layers": [1, 2]})] {
        let action = Action::new("A", vec![("layer.linkLayers".into(), params)]);
        assert!(atn_cmds::export("Links", &[&action], &[], false).is_err());
    }
}
