//! #10700: recorded Link and Unlink must not replay as a state-dependent toggle.
use photocraft_doc::{Layer, LayerContent, LayerId, SmartObject, SmartSource};
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
    target_class("Lyr ")
}

fn target_class(class_id: &str) -> D {
    D::Reference(vec![ReferenceItem::Enumerated {
        class: Class { name: UnicodeString::new_nul(""), class_id: Id::new(class_id) },
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

fn play(s: &mut Session) {
    let report = s.execute("actions.play", json!({"action": "A", "set": "Links"})).unwrap();
    assert!(report.get("failed").is_none(), "{report}");
    assert_eq!(report["ran"], 1, "{report}");
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
            play(&mut replay);
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
            play(&mut replay);
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
            Descriptor::new("null").with("null", target_class("Dcmn")),
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

#[test]
fn link_descriptor_headers_normalize_without_changing_meaningful_keys() {
    for event in ["linkSelectedLayers", "unlinkSelectedLayers"] {
        let mut s = Session::new();
        let descriptor = Descriptor::new(event).with("null", target());
        assert_eq!(load(&mut s, &file(event, descriptor))["unsupported"], json!([]));
        let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
        let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
        let parsed = atn::parse(&bytes).unwrap();
        assert_eq!(parsed.actions[0].items[0].event, AtnEvent::name(event));
        assert_eq!(parsed.actions[0].items[0].descriptor, Some(Descriptor::new("null").with("null", target())));
    }
}

const SMART_PATHS: &[(&str, &str)] = &[
    ("placedLayerExportContents", "layer.smartObjects.exportContents"),
    ("placedLayerConvertToLinked", "layer.smartObjects.convertToLinked"),
    ("placedLayerRelinkToFile", "layer.smartObjects.relinkToFile"),
    ("placedLayerReplaceContents", "layer.smartObjects.replaceContents"),
];

fn path_value(path: &str) -> D {
    let units: Vec<_> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut bytes = b"txtu".to_vec();
    bytes.extend_from_slice(&u32::try_from(12 + units.len() * 2).unwrap().to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(units.len()).unwrap().to_le_bytes());
    bytes.extend(units.iter().flat_map(|unit| unit.to_le_bytes()));
    D::Path(bytes)
}

fn smart_path_descriptor(event: &str, path: &str) -> Descriptor {
    if event == "placedLayerConvertToLinked" {
        Descriptor::new("null").with("null", target()).with("Usng", path_value(path))
    } else {
        Descriptor::new("null").with("null", path_value(path))
    }
}

fn png(color: &str) -> Vec<u8> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "background": color})).unwrap();
    photocraft_io::export(&s.active().unwrap().doc, "png", &Default::default()).unwrap().bytes
}

fn smart_fixture(bytes: &[u8]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    photocraft_engine::file_cmds::place_bytes(&mut s, "original.png", bytes.to_vec(), None, &json!({})).unwrap();
    s
}

fn active_smart(s: &Session) -> &SmartObject {
    let st = s.active().unwrap();
    let LayerContent::Smart(smart) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("expected a Smart Object") };
    smart
}

fn assert_smart_file_output(event: &str, s: &Session, path: &str, original: &[u8], replacement: &[u8]) {
    match event {
        "placedLayerExportContents" => assert_eq!(std::fs::read(path).unwrap().as_slice(), original),
        "placedLayerConvertToLinked" => {
            assert_eq!(std::fs::read(path).unwrap().as_slice(), original);
            assert!(matches!(&active_smart(s).source, SmartSource::Linked { path: actual } if actual == path));
        }
        "placedLayerRelinkToFile" => assert!(matches!(&active_smart(s).source, SmartSource::Linked { path: actual } if actual == path)),
        "placedLayerReplaceContents" => {
            assert!(matches!(&active_smart(s).source, SmartSource::Embedded { bytes, .. } if bytes.as_slice() == replacement));
        }
        _ => unreachable!(),
    }
}

#[test]
fn smart_paths_roundtrip_and_play_real_files_on_the_current_target() {
    let original = png("#0000ff");
    let replacement = png("#ff0000");
    for &(event, id) in SMART_PATHS {
        for header in ["null", event] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("contents Ω.png");
            let path = path.to_str().unwrap();
            let replacing = matches!(event, "placedLayerRelinkToFile" | "placedLayerReplaceContents");
            if replacing {
                std::fs::write(path, &replacement).unwrap();
            }
            let canonical = smart_path_descriptor(event, path);
            let mut recorded = canonical.clone();
            recorded.class_id = Id::new(header);
            let mut s = smart_fixture(&original);
            if replacing {
                let hint = i32::MAX;
                assert!(s.active().unwrap().doc.walk().iter().all(|(_, _, layer)| layer.id.0 != hint as u64));
                recorded = recorded.with("LyrI", D::Integer(hint));
            }
            assert_eq!(load(&mut s, &file(event, recorded))["unsupported"], json!([]), "{event}");
            assert_eq!(s.actions.list[0].steps, [(id.to_string(), json!({"path": path}))]);
            let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
            assert_eq!(saved["skipped"], json!([]));
            let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
            let parsed = atn::parse(&bytes).unwrap();
            assert_eq!(parsed.actions[0].items[0].event, AtnEvent::name(event));
            // Reference #10603, docs/atn.md:197–201: normalize the descriptor class,
            // and omit session LyrI. All meaningful event/path keys remain identical.
            assert_eq!(parsed.actions[0].items[0].descriptor, Some(canonical));
            assert_eq!(load(&mut s, &bytes)["unsupported"], json!([]));
            let before = s.active().unwrap().doc.clone();
            let before_smart = active_smart(&s).clone();
            play(&mut s);
            assert_eq!(active_smart(&s).transform, before_smart.transform);
            assert_smart_file_output(event, &s, path, &original, &replacement);
            if replacing {
                assert!(photocraft_compose::flatten(&s.active().unwrap().doc).px.iter().all(|px| *px == [1.0, 0.0, 0.0, 1.0]));
            }
            if event == "placedLayerExportContents" {
                assert_eq!(s.active().unwrap().doc, before);
            } else {
                assert!(s.undo());
                assert_eq!(s.active().unwrap().doc, before);
            }
        }
    }
}

#[test]
fn smart_paths_keep_missing_unusable_and_unrepresented_descriptors_whole() {
    for &(event, _) in SMART_PATHS {
        let key = if event == "placedLayerConvertToLinked" { "Usng" } else { "null" };
        for descriptor in [
            Descriptor::new("null"),
            Descriptor::new("null").with(key, path_value("")),
            Descriptor::new("null").with(key, path_value("a\0b")),
            Descriptor::new("null").with(key, D::Text(UnicodeString::new("/tmp/file.png"))),
            Descriptor::new("null").with(key, D::Path(b"txtu".to_vec())),
            Descriptor::new("null").with("Usng", path_value("/tmp/file.png")),
            smart_path_descriptor(event, "/tmp/file.png").with("extra", D::Integer(1)),
            smart_path_descriptor(event, "/tmp/file.png").with("LyrI", D::Boolean(true)),
            smart_path_descriptor(event, "/tmp/file.png").with("LyrI", D::Integer(-1)),
            smart_path_descriptor(event, "/tmp/file.png").with("LyrI", D::LargeInteger(i64::from(i32::MAX) + 1)),
        ] {
            let mut s = Session::new();
            assert_eq!(load(&mut s, &file(event, descriptor.clone()))["unsupported"].as_array().unwrap().len(), 1);
            let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
            let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
            assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0].descriptor, Some(descriptor));
        }
    }
}

#[test]
fn smart_path_exports_refuse_unrepresentable_native_params() {
    for &(_, id) in SMART_PATHS {
        for params in [json!({}), json!({"path": ""}), json!({"path": "a\0b"}), json!({"path": 3}), json!({"path": "/tmp/a.png", "layer": 1})] {
            let action = Action::new("A", vec![(id.into(), params)]);
            assert!(atn_cmds::export("Links", &[&action], &[], false).is_err());
        }
    }
}

#[test]
fn unusable_smart_file_at_playback_fails_without_changing_the_document() {
    let original = png("#0000ff");
    let dir = tempfile::tempdir().unwrap();
    for &(event, id) in SMART_PATHS {
        let mut s = smart_fixture(&original);
        assert_eq!(load(&mut s, &file(event, smart_path_descriptor(event, dir.path().to_str().unwrap())))["unsupported"], json!([]));
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        let report = s.execute("actions.play", json!({"action": "A", "set": "Links"})).unwrap();
        assert_eq!(report["ran"], 0, "{event}: {report}");
        assert_eq!(report["failed"]["id"], id, "{event}: {report}");
        assert!(report["failed"]["error"].as_str().is_some_and(|error| !error.is_empty()), "{event}: {report}");
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}
