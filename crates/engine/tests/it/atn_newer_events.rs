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
            Descriptor::new("null").with("null", D::Reference(vec![])),
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

const TARGET_EVENTS: &[(&str, &str)] = &[
    ("reverse", "layer.arrange.reverse"),
    ("selectLinkedLayers", "layer.selectLinkedLayers"),
    ("disableLayerStyle", "layer.layerStyle.clear"),
    ("placedLayerMakeCopy", "layer.smartObjects.newSmartObjectViaCopy"),
    ("placedLayerConvertToLayers", "layer.smartObjects.convertToLayers"),
    ("placedLayerConvertToEmbedded", "layer.smartObjects.convertToEmbedded"),
    ("placedLayerUpdateAllModified", "layer.smartObjects.updateAllModifiedContent"),
    ("rasterizeAll", "layer.rasterize.allLayers"),
    ("addBlankVideoLayer", "layer.videoLayers.newBlankVideoLayer"),
    ("isolateLayers", "select.isolateLayers"),
    ("textLoadDefaultTypeStyles", "type.loadDefaultTypeStyles"),
    ("textSaveDefaultTypeStyles", "type.saveDefaultTypeStyles"),
    ("recordMeasurements", "image.analysis.recordMeasurements"),
];

fn targeted(event: &str, explicit: bool) -> Descriptor {
    let d = Descriptor::new(event);
    if explicit { d.with("null", target()) } else { d }
}

fn load_targeted(s: &mut Session, event: &str, id: &str, explicit: bool) {
    assert_eq!(load(s, &file(event, targeted(event, explicit)))["unsupported"], json!([]), "{event}");
    let p = if explicit { json!({}) } else { json!({"_atnNoTarget": true}) };
    assert_eq!(s.actions.list[0].steps, [(id.into(), p)]);
}

#[test]
fn target_events_keep_empty_and_explicit_target_descriptors_distinct() {
    for &(event, id) in TARGET_EVENTS {
        for explicit in [false, true] {
            let mut s = Session::new();
            load_targeted(&mut s, event, id, explicit);
            let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
            let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
            let item = &atn::parse(&bytes).unwrap().actions[0].items[0];
            assert_eq!(item.event, AtnEvent::name(event));
            assert_eq!(item.descriptor, Some(targeted("null", explicit)), "{event}");
            assert_eq!(load(&mut s, &bytes)["unsupported"], json!([]));
        }
    }
    // Reference lines197–199 permits writing an empty descriptor for an absent one.
    let mut f = atn::parse(&file("reverse", Descriptor::new("null"))).unwrap();
    f.actions[0].items[0].descriptor = None;
    let mut s = Session::new();
    assert_eq!(load(&mut s, &atn::write(&f).unwrap())["unsupported"], json!([]));
    let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
    let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
    assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0].descriptor, Some(Descriptor::new("null")));
}

#[test]
fn bare_link_events_keep_their_operation_and_empty_keys() {
    for (event, linked) in [("linkSelectedLayers", true), ("unlinkSelectedLayers", false)] {
        let initial = if linked { None } else { Some(77) };
        let mut s = fixture(&[initial, initial], &[0, 1]);
        assert_eq!(load(&mut s, &file(event, Descriptor::new(event)))["unsupported"], json!([]));
        assert_eq!(s.actions.list[0].steps[0].1, json!({"linked": linked, "_atnNoTarget": true}));
        let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
        let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
        assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0].descriptor, Some(Descriptor::new("null")));
        let before = s.active().unwrap().doc.clone();
        play(&mut s);
        assert_ne!(s.active().unwrap().doc, before);
        let after = s.active().unwrap().doc.clone();
        assert!(groups(&s).iter().all(|group| group.is_some() == linked));
        assert_eq!(groups(&s)[0], groups(&s)[1]);
        play(&mut s);
        assert_eq!(s.active().unwrap().doc, after);
    }
}

#[test]
fn target_events_keep_unknown_targets_and_refuse_native_options() {
    for &(event, id) in TARGET_EVENTS {
        for descriptor in [
            Descriptor::new(event).with("null", D::Integer(1)),
            Descriptor::new(event).with("null", target_class("Dcmn")),
            targeted(event, true).with("null", target()),
            targeted(event, true).with("extra", D::Boolean(true)),
        ] {
            let mut s = Session::new();
            assert_eq!(load(&mut s, &file(event, descriptor.clone()))["unsupported"].as_array().unwrap().len(), 1);
            let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
            let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
            assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0].descriptor, Some(descriptor));
        }
        for p in [json!({"layer": 1}), json!({"on": true}), json!({"_atnNoTarget": false}), json!({"_atnNoTarget": "true"})] {
            let action = Action::new("A", vec![(id.into(), p)]);
            assert!(atn_cmds::export("Links", &[&action], &[], false).is_err(), "{event}");
        }
    }
    for linked in [false, true] {
        let action = Action::new("A", vec![("layer.linkLayers".into(), json!({"linked": linked, "_atnNoTarget": false}))]);
        assert!(atn_cmds::export("Links", &[&action], &[], false).is_err());
    }
}

#[test]
fn reverse_linked_selection_and_clear_style_events_do_their_edits() {
    for explicit in [false, true] {
        let mut s = fixture(&[None, None, None], &[0, 1, 2]);
        let before = s.active().unwrap().doc.clone();
        let ids: Vec<_> = before.layers.iter().map(|layer| layer.id).rev().collect();
        load_targeted(&mut s, "reverse", "layer.arrange.reverse", explicit);
        play(&mut s);
        assert_eq!(s.active().unwrap().doc.layers.iter().map(|layer| layer.id).collect::<Vec<_>>(), ids);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);

        let mut s = fixture(&[Some(77), Some(77), None], &[0]);
        let ids: Vec<_> = s.active().unwrap().doc.layers[..2].iter().map(|layer| layer.id).collect();
        load_targeted(&mut s, "selectLinkedLayers", "layer.selectLinkedLayers", explicit);
        play(&mut s);
        assert_eq!(s.active().unwrap().selected_layers(), ids);

        let mut s = fixture(&[None], &[0]);
        s.execute("layer.layerStyle.colorOverlay", json!({"color": "#ff0000"})).unwrap();
        let before = s.active().unwrap().doc.clone();
        assert_eq!(before.layers[0].effects.items.len(), 1);
        load_targeted(&mut s, "disableLayerStyle", "layer.layerStyle.clear", explicit);
        play(&mut s);
        assert!(s.active().unwrap().doc.layers[0].effects.items.is_empty());
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

fn smart_target_scene(event: &str, bytes: &[u8], replacement: &[u8], path: &str) -> Session {
    let mut s = smart_fixture(bytes);
    if matches!(event, "placedLayerConvertToEmbedded" | "placedLayerUpdateAllModified") {
        s.execute("layer.smartObjects.convertToLinked", json!({"path": path})).unwrap();
    }
    if event == "placedLayerUpdateAllModified" {
        std::fs::write(path, replacement).unwrap();
    }
    s
}

fn assert_smart_target_edit(event: &str, s: &Session, original_id: photocraft_doc::SmartContentsId, pixels: &[[f32; 4]], bytes: &[u8]) {
    match event {
        "placedLayerMakeCopy" => {
            assert_ne!(active_smart(s).contents_id, original_id);
            assert_eq!(s.active().unwrap().doc.walk().iter().filter(|(_, _, layer)| matches!(layer.content, LayerContent::Smart(_))).count(), 2);
        }
        "placedLayerConvertToLayers" => assert!(s.active().unwrap().doc.walk().iter().all(|(_, _, layer)| !matches!(layer.content, LayerContent::Smart(_)))),
        "placedLayerConvertToEmbedded" => assert!(matches!(&active_smart(s).source, SmartSource::Embedded { bytes: actual, .. } if actual.as_slice() == bytes)),
        "placedLayerUpdateAllModified" => assert!(photocraft_compose::flatten(&s.active().unwrap().doc).px.iter().all(|px| *px == [1.0, 0.0, 0.0, 1.0])),
        _ => unreachable!(),
    }
    if event != "placedLayerUpdateAllModified" {
        assert_eq!(photocraft_compose::flatten(&s.active().unwrap().doc).px, pixels);
    }
}

#[test]
fn smart_target_events_copy_unpack_embed_and_refresh_real_contents() {
    let original = png("#0000ff");
    let replacement = png("#ff0000");
    for &(event, id) in &TARGET_EVENTS[3..7] {
        for explicit in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("linked.png");
            let mut s = smart_target_scene(event, &original, &replacement, path.to_str().unwrap());
            let before = s.active().unwrap().doc.clone();
            let pixels = photocraft_compose::flatten(&before).px;
            let contents_id = active_smart(&s).contents_id;
            load_targeted(&mut s, event, id, explicit);
            play(&mut s);
            assert_smart_target_edit(event, &s, contents_id, &pixels, &original);
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc, before);
        }
    }
}

#[test]
fn rasterize_all_event_visits_nested_and_top_level_smart_layers() {
    for explicit in [false, true] {
        let mut s = smart_fixture(&png("#0000ff"));
        let first = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.smartObjects.newSmartObjectViaCopy", json!({})).unwrap();
        let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
        let nested = doc.remove(first).unwrap();
        doc.layers.push(Layer::group("Nested", vec![nested]));
        let before = s.active().unwrap().doc.clone();
        let pixels = photocraft_compose::flatten(&before).px;
        load_targeted(&mut s, "rasterizeAll", "layer.rasterize.allLayers", explicit);
        play(&mut s);
        assert!(s.active().unwrap().doc.walk().iter().all(|(_, _, layer)| !matches!(layer.content, LayerContent::Smart(_))));
        assert_eq!(photocraft_compose::flatten(&s.active().unwrap().doc).px, pixels);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

#[test]
fn video_and_isolate_events_use_current_document_state() {
    for explicit in [false, true] {
        let mut s = fixture(&[None], &[0]);
        let before = s.active().unwrap().doc.clone();
        load_targeted(&mut s, "addBlankVideoLayer", "layer.videoLayers.newBlankVideoLayer", explicit);
        play(&mut s);
        let st = s.active().unwrap();
        let video = st.doc.layer(st.active_layer.unwrap()).unwrap().video.as_ref().unwrap();
        assert_eq!(video.frames.len(), 1);
        assert_eq!(video.fps, 30.0);
        assert!(video.frames[0].content_bounds().is_empty());
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);

        let mut s = fixture(&[None, None, None], &[0, 2]);
        let before = s.active().unwrap().doc.clone();
        let ids = s.active().unwrap().selected_layers();
        let history = s.active().unwrap().history.past_len();
        load_targeted(&mut s, "isolateLayers", "select.isolateLayers", explicit);
        play(&mut s);
        assert_eq!(s.active().unwrap().isolated_layers, ids);
        play(&mut s);
        assert!(s.active().unwrap().isolated_layers.is_empty());
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}

fn active_text(s: &Session) -> &photocraft_doc::TextLayer {
    let st = s.active().unwrap();
    let LayerContent::Text(text) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("expected Type") };
    text
}

#[test]
fn type_default_events_save_then_restore_changed_styles() {
    for explicit in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        s.execute("type.create", json!({"text": "Hi", "size": 14})).unwrap();
        assert!(s.type_defaults.is_none());
        load_targeted(&mut s, "textSaveDefaultTypeStyles", "type.saveDefaultTypeStyles", explicit);
        play(&mut s);
        assert_eq!(s.type_defaults.as_ref().unwrap().0.size_pt, 14.0);
        s.execute("type.setStyle", json!({"size": 28})).unwrap();
        assert_eq!(active_text(&s).char_runs()[0].style.size_pt, 28.0);
        let before = s.active().unwrap().doc.clone();
        load_targeted(&mut s, "textLoadDefaultTypeStyles", "type.loadDefaultTypeStyles", explicit);
        play(&mut s);
        assert_eq!(active_text(&s).char_runs()[0].style.size_pt, 14.0);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

#[test]
fn measurement_event_records_actual_selected_area() {
    for explicit in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4, "height": 4, "background": "white"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 2, "height": 2})).unwrap();
        let before = s.active().unwrap().doc.clone();
        load_targeted(&mut s, "recordMeasurements", "image.analysis.recordMeasurements", explicit);
        play(&mut s);
        assert_eq!(s.analysis.log.len(), 1);
        assert_eq!(s.analysis.log[0].values["source"], "Selection");
        assert_eq!(s.analysis.log[0].values["area"].as_f64(), Some(4.0));
        assert_eq!(s.active().unwrap().doc, before);
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
