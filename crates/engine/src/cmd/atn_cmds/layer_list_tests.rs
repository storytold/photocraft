use super::*;
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

fn two_layers() -> (Session, u64, u64) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("layer.new.layer", json!({"name": "First"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Second"})).unwrap();
    let first = s.active().unwrap().doc.walk()[1].2.id.0;
    let second = s.active().unwrap().doc.walk()[2].2.id.0;
    (s, first, second)
}

fn opacities(s: &Session) -> Vec<f64> {
    s.active().unwrap().doc.walk().iter().skip(1).map(|(_, _, l)| l.opacity as f64).collect()
}

/// An unavailable Photoshop mapping must remain one exact native step, not a partial Set
/// or an active-layer fallback. Decoding that step does not bypass command validation.
fn assert_native_layer_fallback(s: &mut Session, action: &Action) {
    assert_eq!(action.steps.len(), 1);
    let (id, params) = &action.steps[0];
    assert!(super::convert::export_items(id, params, "S").is_none(), "the Photoshop projection must still refuse {params}");
    let before_actions = s.actions.list.clone();
    let before_revision = s.active().unwrap().revision;
    let before_history = s.active().unwrap().history.past_len();
    let layers = |s: &Session| s.active().unwrap().doc.walk().iter().map(|(_, _, layer)| (layer.id.0, layer.name.clone(), layer.opacity)).collect::<Vec<_>>();
    let before_layers = layers(s);
    let (bytes, skipped) = export("S", &[action], &[], false).unwrap();
    assert!(skipped.is_empty());
    let file = atn::parse(&bytes).unwrap();
    let [item] = file.actions[0].items.as_slice() else { panic!("one whole native step, no partial Photoshop Sets") };
    assert_eq!(item.event, AtnEvent::name("photocraftCommand"));
    assert_eq!(item.enabled, action.step_enabled(0));
    assert_eq!(item.with_dialog, action.dialogs.contains(&0));
    let descriptor = item.descriptor.as_ref().unwrap();
    let Some(D::Text(command)) = descriptor.get("command") else { panic!("missing native command") };
    let Some(D::Text(stored)) = descriptor.get("params") else { panic!("missing native params") };
    assert_eq!(command.to_string_lossy(), *id);
    assert_eq!(serde_json::from_str::<Value>(&stored.to_string_lossy()).unwrap(), *params);
    let imported = import(&bytes).unwrap();
    assert_eq!(imported.actions.len(), 1);
    assert_eq!(imported.actions[0].steps, action.steps, "all layer selectors and property values survive exactly");
    let (decoded_id, decoded_params) = &imported.actions[0].steps[0];
    assert!(s.execute(decoded_id, decoded_params.clone()).is_err(), "native encoding must not bypass layer validation");
    assert_eq!(s.actions.list, before_actions);
    assert_eq!(s.active().unwrap().revision, before_revision);
    assert_eq!(s.active().unwrap().history.past_len(), before_history);
    assert_eq!(layers(s), before_layers, "an invalid target must not edit the active or a partial first layer");
    let (bytes, skipped) = export("S", &[action], &[], true).unwrap();
    assert_eq!(skipped, [Skipped { action: action.name.clone(), step: 0, id: id.clone() }]);
    assert!(atn::parse(&bytes).unwrap().actions[0].items.is_empty(), "skipUnsupported omits the whole native step");
}

/// The issue's procedure: an opacities step that named both layers, undone, and strictly
/// exported/imported, replays on both layers of the same document — the id keeps exactly
/// the ones it changed (#7246).
#[test]
fn the_layers_list_survives_a_strict_roundtrip() {
    let (mut s, first, second) = two_layers();
    s.actions.list.push(Action::new("Targets", vec![("layer.setProps".into(), json!({"layers": [first, second], "opacity": 0.5}))]));
    s.actions.list[0].set = Some("S".into());

    s.execute("actions.play", json!({"action": "Targets"})).unwrap();
    assert_eq!(opacities(&s), [0.5, 0.5]);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(opacities(&s), [1.0, 1.0]);

    let action = s.actions.list[0].clone();
    let (bytes, skipped) = export("S", &[&action], &[], false).unwrap();
    assert_eq!(skipped, []);
    let file = atn::parse(&bytes).unwrap();
    let items = &file.actions[0].items;
    assert_eq!(items.len(), 2, "one Set per layer: {}", items.len());
    for (item, id) in items.iter().zip([first, second]) {
        assert_eq!(item.event.id(), "setd");
        let d = item.descriptor.as_ref().unwrap();
        let [ReferenceItem::Identifier { class, id: target }] = get_ref(d, "null").unwrap() else { panic!("{d:?}") };
        assert!(class.class_id.is("Lyr "), "{class:?}");
        assert_eq!(*target as u64, id);
        let props = get_obj(d, "T   ").unwrap();
        assert!(props.class_id.is("Lyr "));
        assert_eq!(props.items.len(), 1);
        assert_eq!(props.get("Opct"), Some(&pct(50.0)));
    }

    // Strictly back into the same document, ids unchanged.
    let r = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(bytes.as_slice()), "set": "S"})).unwrap();
    assert_eq!(r["actions"], json!(["Targets"]));
    assert_eq!(r["steps"], 2);
    let played = s.execute("actions.play", json!({"action": "Targets"})).unwrap();
    assert!(played["failed"].is_null(), "{played}");
    assert_eq!(played["ran"], 2);
    assert_eq!(opacities(&s), [0.5, 0.5]);
}

/// Import: a Set on a Photoshop layer id (`Idnt`) becomes the id the `layer` param takes.
#[test]
fn a_layer_id_reference_imports_as_the_layer_id() {
    let descriptor = desc("null")
        .with("null", D::Reference(vec![ReferenceItem::Identifier { class: class("Lyr "), id: 7 }]))
        .with("T   ", D::Descriptor(desc("Lyr ").with("Opct", pct(30.0))));
    let bytes = atn::write(&AtnFile {
        name: UnicodeString::new_nul("S"),
        expanded: true,
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items: vec![item("setd", "Set", Some(descriptor))], ..Default::default() }],
        warnings: Vec::new(),
        encoding: None,
    })
    .unwrap();
    let imported = import(&bytes).unwrap();
    assert_eq!(imported.unsupported, []);
    assert_eq!(imported.actions[0].steps, [("layer.setProps".to_string(), json!({"layer": 7, "opacity": 0.3}))]);

    // Re-exporting the imported step keeps the id: the reference is the same `Idnt` again,
    // not the implicit target layer (#7246, review).
    let (out, skipped) = export("S", &[&imported.actions[0]], &[], false).unwrap();
    assert_eq!(skipped, []);
    let file = atn::parse(&out).unwrap();
    assert_eq!(file.actions[0].items.len(), 1);
    let id_ref = get_ref(file.actions[0].items[0].descriptor.as_ref().unwrap(), "null").unwrap();
    let [ReferenceItem::Identifier { class, id }] = id_ref else { panic!("{id_ref:?}") };
    assert!(class.class_id.is("Lyr "), "{class:?}");
    assert_eq!(*id, 7);
}

/// A one-layer list exports as a single Set by its id, so the step roundtrips by the same
/// layer even though no name rides along.
#[test]
fn a_one_layer_list_exports_one_exact_set() {
    let (_s, first, _) = two_layers();
    let action = Action::new("Targets", vec![("layer.setProps".into(), json!({"layers": [first], "opacity": 0.5}))]);
    let (bytes, skipped) = export("S", &[&action], &[], false).unwrap();
    assert_eq!(skipped, []);
    let file = atn::parse(&bytes).unwrap();
    assert_eq!(file.actions[0].items.len(), 1);
    let steps = import(&bytes).unwrap().actions[0].steps.clone();
    assert_eq!(steps, [("layer.setProps".to_string(), json!({"layer": first, "opacity": 0.5}))]);
}

/// Idnt holds u32: retain a larger layer id natively instead of retargeting the active layer.
#[test]
fn an_out_of_idnt_range_layers_list_stays_exact_native_data() {
    let (mut s, _, _) = two_layers();
    let action = Action::new("Targets", vec![("layer.setProps".into(), json!({"layers": [u32::MAX as u64 + 1], "opacity": 0.5}))]);
    assert_native_layer_fallback(&mut s, &action);
}

/// A multiple-layer rename stays unprojectable and fails validation after a native roundtrip.
#[test]
fn a_native_layers_list_rename_still_refuses_to_edit() {
    let (mut s, first, second) = two_layers();
    let action = Action::new("Targets", vec![("layer.setProps".into(), json!({"layers": [first, second], "name": "Both"}))]);
    assert_native_layer_fallback(&mut s, &action);
}

/// A present non-array layers value must not become an implicit active-layer Set (#7246).
#[test]
fn a_native_non_array_layers_value_does_not_target_the_active_layer() {
    let (mut s, _, _) = two_layers();
    let action = Action::new("Targets", vec![("layer.setProps".into(), json!({"layers": "7,8", "opacity": 0.5}))]);
    assert_native_layer_fallback(&mut s, &action);
}

/// Several explicit layers with several properties: per layer (in list order), per property
/// (the step's exact keys and values, opacity then blend), each pitched to that layer's
/// `Idnt`; the step's enabled/dialog flags ride the expansion; an unrepresentable id anywhere
/// retains the whole source step natively or omits it with skipUnsupported, so no partial
/// first-layer Set escapes (#7246, coord with #8371).
#[test]
fn a_list_with_properties_exports_exact_targets_in_order() {
    let (mut s, first, second) = two_layers();
    let mut action = Action::new("Targets", vec![("layer.setProps".into(), json!({"layers": [first, second], "opacity": 0.5, "blend": "Multiply"}))]);
    // The panel's unchecked and dialog columns: the expansion must not lose them.
    action.disabled.insert(0);
    action.dialogs.insert(0);
    let (bytes, skipped) = export("S", &[&action], &[], false).unwrap();
    assert_eq!(skipped, []);
    let file = atn::parse(&bytes).unwrap();
    let items = &file.actions[0].items;
    let expected: Vec<(u64, &str, D)> =
        [first, second].iter().flat_map(|&l| [(l, "Opct", pct(50.0)), (l, "Md  ", enm("BlnM", "Mltp"))].map(|(l, k, v)| (l, k, v)).into_iter()).collect();
    assert_eq!(items.len(), expected.len(), "one property Set per layer, properties split per item");
    for (item, (id, key, value)) in items.iter().zip(expected) {
        assert!(item.event.id() == "setd", "property events stay Sets");
        let d = item.descriptor.as_ref().unwrap();
        let [ReferenceItem::Identifier { class, id: target }] = get_ref(d, "null").unwrap() else { panic!("{d:?}") };
        assert!(class.class_id.is("Lyr "), "{class:?}");
        assert_eq!(*target as u64, id, "each property event pitches its own layer by id");
        let props = get_obj(d, "T   ").unwrap();
        assert_eq!(props.items.len(), 1);
        let (k, v) = props.items.first().unwrap();
        assert!(k.is(key), "{k:?} not {key:?}");
        assert_eq!(v, &value, "{k:?}");
        assert!(!item.enabled, "an unchecked source step stays unchecked through expansion");
        assert!(item.with_dialog, "a dialog source step keeps its dialog through expansion");
    }
    assert!(s.active().unwrap().doc.walk()[1].2.blend == photocraft_color::BlendMode::Normal, "export writes no edit into the document");

    let malformed = Action::new("Targets", vec![("layer.setProps".into(), json!({"layers": [first, "nope"], "opacity": 0.5}))]);
    assert_native_layer_fallback(&mut s, &malformed);
}

/// Numeric selectors outside Idnt remain exact native JSON, never the current selection.
#[test]
fn a_native_numeric_layer_outside_idnt_range_does_not_target_the_active_layer() {
    let (mut s, _, _) = two_layers();
    for bad in [json!(u32::MAX as u64 + 1), json!(-5)] {
        let action = Action::new("Targets", vec![("layer.setProps".into(), json!({"layer": bad, "opacity": 0.5}))]);
        assert_native_layer_fallback(&mut s, &action);
    }
}
