//! Tests that run vector commands, moved from the engine-core `atn_cmds` tests: 2 test(s).

use crate::atn_fixtures::*;
use craft_psd::atn::{self};
use photocraft_engine::Session;
use photocraft_engine::atn_cmds::PHOTOSHOP_STEP;

use photocraft_engine::actions_cmds::ActionStep;

use photocraft_engine::actions_cmds::ActionsExt;

use craft_psd::atn::{AtnAction, AtnEvent, AtnFile, AtnItem};

use craft_psd::descriptor::{Descriptor, ReferenceItem, UnicodeString, Value as D};

use serde_json::json;

fn item(event: &str, label: &str, d: Option<Descriptor>) -> AtnItem {
    let event = match event.as_bytes() {
        [a, b, c, dd] => AtnEvent::Code([*a, *b, *c, *dd]),
        _ => AtnEvent::name(event),
    };
    AtnItem { event, dictionary: label.as_bytes().to_vec(), descriptor: d, ..Default::default() }
}

fn b64(bytes: &[u8]) -> String {
    photocraft_paint::tile::b64_encode(bytes)
}

#[test]
fn layer_comp_apply_imports_plays_and_undoes() {
    let name = "合成 𠮷";
    let data = set_of("Comps", vec![("Apply", vec![item("applyComp", "Apply Layer Comp", Some(desc("null").with("null", name_ref("compsClass", name))))])]);
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        let loaded = s.execute("actions.load", json!({"data": data})).unwrap();
        assert_eq!(loaded["unsupported"], json!([]), "{loaded}");
        assert_eq!(s.actions().list.actions()[0].steps, [ActionStep::new("layerComp.apply", json!({"comp": name}))]);

        s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth})).unwrap();
        s.execute("layerComp.new", json!({"name": "Other"})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let played = s.execute("actions.play", json!({"action": "Apply"})).unwrap();
        assert_eq!(played["ran"], 0, "{played}");
        assert!(played["failed"]["error"].as_str().unwrap().contains(name), "{played}");
        assert_eq!(*s.active().unwrap().doc, *before, "a missing comp must not apply another comp");

        let comp = s.execute("layerComp.new", json!({"name": name})).unwrap()["comp"].clone();
        let layer = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.setProps", json!({"visible": false})).unwrap();
        s.execute("layerComp.new", json!({"name": "Hidden"})).unwrap();
        let played = s.execute("actions.play", json!({"action": "Apply"})).unwrap();
        assert_eq!(played["ran"], 1, "{played}");
        assert!(played.get("failed").is_none(), "{played}");
        assert!(s.active().unwrap().doc.layer(layer).unwrap().visible);
        assert_eq!(json!(s.active().unwrap().doc.last_applied_comp), comp);
        assert!(s.undo());
        assert!(!s.active().unwrap().doc.layer(layer).unwrap().visible);
        assert!(s.redo());
        assert!(s.active().unwrap().doc.layer(layer).unwrap().visible);
    }
}

#[test]
fn layer_comp_apply_unknown_references_are_kept_and_stop_playback() {
    let references = [
        None,
        Some(D::Integer(1)),
        Some(D::Reference(vec![])),
        Some(name_ref("Lyr ", "Comp 1")),
        Some(class_ref("compsClass")),
        Some(D::Reference(vec![ReferenceItem::Offset { class: class("compsClass"), offset: 1 }])),
        Some(D::Reference(vec![ReferenceItem::Enumerated { class: class("compsClass"), type_id: code("Ordn"), value: code("unsupported") }])),
        Some(D::Reference(vec![
            ReferenceItem::Name { class: class("compsClass"), name: UnicodeString::new_nul("Comp 1") },
            ReferenceItem::Name { class: class("Dcmn"), name: UnicodeString::new_nul("Other") },
        ])),
    ];
    let items: Vec<_> =
        references.into_iter().map(|reference| item("applyComp", "Apply Layer Comp", reference.map(|r| desc("null").with("null", r)))).collect();
    let data = set_of("Comps", vec![("Unknown", items.clone())]);
    let mut s = Session::new();
    let loaded = s.execute("actions.load", json!({"data": data})).unwrap();
    assert_eq!(loaded["unsupported"].as_array().unwrap().len(), items.len());
    assert!(s.actions().list.actions()[0].steps.iter().all(|st| st.command == PHOTOSHOP_STEP));
    assert!(s.actions().list.actions()[0].disabled.is_empty());
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layerComp.new", json!({"name": "Comp 1"})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let played = s.execute("actions.play", json!({"action": "Unknown"})).unwrap();
    assert_eq!(played["ran"], 0, "{played}");
    assert!(played["failed"]["error"].as_str().unwrap().contains("Apply Layer Comp"), "{played}");
    assert_eq!(*s.active().unwrap().doc, *before);
    let saved = s.execute("actions.save", json!({"set": "Comps"})).unwrap();
    let bytes = photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
    let file = atn::parse(&bytes).unwrap();
    assert_eq!(file.actions[0].items.len(), items.len());
    for (saved, original) in file.actions[0].items.iter().zip(&items) {
        assert_eq!(saved.event, original.event);
        assert_eq!(saved.descriptor, original.descriptor);
    }
}

fn set_of(name: &str, actions: Vec<(&str, Vec<AtnItem>)>) -> String {
    let file = AtnFile {
        name: UnicodeString::new_nul(name),
        expanded: false,
        actions: actions.into_iter().map(|(n, items)| AtnAction { name: UnicodeString::new_nul(n), items, ..Default::default() }).collect(),
        warnings: vec![],
        ..Default::default()
    };
    b64(&atn::write(&file).unwrap())
}
