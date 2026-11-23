use super::*;
use crate::actions_cmds::{ActionList, ActionStep, ActionsFile};

/// PhotoCraft's own roundtrip accepts a combined Set, but Photoshop applies only its rename.
#[test]
fn compound_properties_export_as_separate_sets_with_rename_last() {
    for target in [None, Some("Original")] {
        let mut params = json!({"name": "Renamed", "opacity": 0.37, "blend": "Multiply"});
        if let Some(name) = target {
            params["layer"] = json!(name);
        }
        let action = Action::new("Properties", vec![ActionStep::new("layer.setProps", params.clone())]);
        let (bytes, skipped) = export("Set", &[&action], &[], false).unwrap();
        assert!(skipped.is_empty());
        let file = atn::parse(&bytes).unwrap();
        let items = &file.actions[0].items;
        assert_eq!(items.len(), 3);
        let expected = [("Opct", pct(37.0)), ("Md  ", enm("BlnM", "Mltp")), ("Nm  ", text("Renamed"))];
        for (item, (key, value)) in items.iter().zip(expected) {
            assert_eq!(item.event.id(), "setd");
            let descriptor = item.descriptor.as_ref().unwrap();
            assert_eq!(descriptor.get("null"), Some(&layer_ref(&params).unwrap()));
            let properties = get_obj(descriptor, "T   ").unwrap();
            assert!(properties.class_id.is("Lyr "));
            assert_eq!(properties.items.len(), 1, "Photoshop must receive each property independently");
            assert_eq!(properties.get(key), Some(&value));
        }
    }
}

#[test]
fn expanded_properties_keep_flags_and_original_skip_indices() {
    let mut action = Action::new(
        "Flags",
        vec![
            ActionStep::new("layer.setProps", json!({"name": "Renamed", "opacity": 0.37, "blend": "Multiply"})),
            ActionStep::new("filter.blur.gaussianBlur", json!({"radius": 2})),
            ActionStep::new("unsupported.forTest", json!({})),
        ],
    );
    action.disabled.insert(0);
    action.dialogs.insert(0);
    let (bytes, skipped) = export("Set", &[&action], &[], true).unwrap();
    assert_eq!(skipped, [Skipped { action: "Flags".into(), step: 2, id: "unsupported.forTest".into() }]);
    let file = atn::parse(&bytes).unwrap();
    let items = &file.actions[0].items;
    assert_eq!(items.len(), 4);
    for item in &items[..3] {
        assert!(!item.enabled);
        assert!(item.with_dialog);
        assert_eq!(item.dialog_options, 2);
    }
    assert_eq!(items[3].event.id(), "GsnB");
    assert!(items[3].enabled);
    assert!(!items[3].with_dialog);
    assert_eq!(items[3].dialog_options, 0);
    let back = import(&bytes).unwrap();
    assert_eq!(back.actions[0].disabled.iter().copied().collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(back.actions[0].dialogs.iter().copied().collect::<Vec<_>>(), [0, 1, 2]);
}

#[test]
fn invalid_compound_properties_fail_or_skip_the_whole_source_step() {
    let action = Action::new(
        "Atomic",
        vec![
            ActionStep::new("filter.blur.gaussianBlur", json!({"radius": 2})),
            ActionStep::new("layer.setProps", json!({"name": "Renamed", "opacity": 0.37, "blend": "invalid blend"})),
            ActionStep::new("select.inverse", json!({})),
        ],
    );
    let error = export("Set", &[&action], &[], false).unwrap_err();
    assert!(error.contains("step 2"), "{error}");
    let (bytes, skipped) = export("Set", &[&action], &[], true).unwrap();
    assert_eq!(skipped, [Skipped { action: "Atomic".into(), step: 1, id: "layer.setProps".into() }]);
    let file = atn::parse(&bytes).unwrap();
    let events: Vec<_> = file.actions[0].items.iter().map(|item| item.event.id()).collect();
    assert_eq!(events, ["GsnB", "Invs"]);
}

#[test]
fn single_layer_properties_keep_their_existing_descriptor() {
    for params in [json!({"name": "Renamed"}), json!({"opacity": 0.37}), json!({"blend": "Multiply"}), json!({"fill": 0.5}), json!({"visible": false})] {
        let action = Action::new("Single", vec![ActionStep::new("layer.setProps", params.clone())]);
        let expected = export_item("layer.setProps", &params, "Set").unwrap();
        let (bytes, skipped) = export("Set", &[&action], &[], false).unwrap();
        assert!(skipped.is_empty());
        let file = atn::parse(&bytes).unwrap();
        assert_eq!(file.actions[0].items.len(), 1);
        assert_eq!(file.actions[0].items[0].event, expected.event);
        assert_eq!(file.actions[0].items[0].descriptor, expected.descriptor);
    }
}

#[test]
fn fill_and_visibility_also_export_separately_before_rename() {
    let action = Action::new("Other properties", vec![ActionStep::new("layer.setProps", json!({"name": "Renamed", "fill": 0.5, "visible": false}))]);
    let (bytes, _) = export("Set", &[&action], &[], false).unwrap();
    let file = atn::parse(&bytes).unwrap();
    let keys: Vec<_> = file.actions[0]
        .items
        .iter()
        .map(|item| {
            let properties = get_obj(item.descriptor.as_ref().unwrap(), "T   ").unwrap();
            assert_eq!(properties.items.len(), 1);
            trimmed(&properties.items[0].0)
        })
        .collect();
    assert_eq!(keys, ["fillOpacity", "Vsbl", "Nm"]);
}

#[test]
fn expanded_properties_replay_like_direct_edits_at_every_depth() {
    for depth in [8, 16, 32] {
        for named in [false, true] {
            let prepare = || {
                let mut session = crate::session();
                session.execute("file.new", json!({"width": 2, "height": 2, "depth": depth})).unwrap();
                session.execute("layer.new.layer", json!({"name": "Original"})).unwrap();
                session.execute("layer.new.layer", json!({"name": "Other"})).unwrap();
                if !named {
                    session.execute("layer.select", json!({"layer": "Original"})).unwrap();
                }
                session
            };
            let mut params = json!({"name": "Renamed", "opacity": 0.37, "blend": "Multiply"});
            if named {
                params["layer"] = json!("Original");
            }
            let mut direct = prepare();
            direct.execute("layer.setProps", params.clone()).unwrap();
            let action = Action::new("Properties", vec![ActionStep::new("layer.setProps", params)]);
            let (bytes, _) = export("Set", &[&action], &[], false).unwrap();
            let mut replay = prepare();
            replay.actions.list = ActionList::from_file(ActionsFile::new(import(&bytes).unwrap().actions, Vec::new()));
            let result = replay.execute("actions.play", json!({"action": "Properties"})).unwrap();
            assert!(result["failed"].is_null(), "{result}");
            assert_eq!(result["ran"], 3);
            let state = |session: &Session| {
                session
                    .active()
                    .unwrap()
                    .doc
                    .walk()
                    .iter()
                    .map(|(_, _, layer)| (layer.name.clone(), layer.opacity, layer.blend, layer.visible))
                    .collect::<Vec<_>>()
            };
            assert_eq!(state(&direct), state(&replay), "depth={depth}, named={named}");
        }
    }
}

const ADJUSTMENT_COMMAND: &str = "layer.newAdjustmentLayer.brightnessContrast";
const ADJUSTMENT_ACTION: &str = "Make adjustment";

fn adjustment_session() -> Session {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 2, "height": 2})).unwrap();
    session
}

fn set_adjustment_action(session: &mut Session, opacity: Option<f64>) {
    let mut params = json!({"brightness": 100});
    if let Some(opacity) = opacity {
        params["opacity"] = json!(opacity);
    }
    let actions = vec![Action::new(ADJUSTMENT_ACTION, vec![ActionStep::new(ADJUSTMENT_COMMAND, params)])];
    session.actions.list = ActionList::from_file(ActionsFile::new(actions, Vec::new()));
}

fn play_adjustment_opacity(session: &mut Session) -> f32 {
    let result = session.execute("actions.play", json!({"action": ADJUSTMENT_ACTION})).unwrap();
    assert!(result["failed"].is_null(), "{result}");
    let state = session.active().unwrap();
    state.doc.layer(state.active_layer.unwrap()).unwrap().opacity
}

#[test]
fn adjustment_opacity_strict_export_refuses_loss_before_writing() {
    let mut session = adjustment_session();
    set_adjustment_action(&mut session, Some(0.371));
    assert_eq!(play_adjustment_opacity(&mut session), 0.371_f32, "native playback keeps the finer opacity");

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.atn");
    let previous = b"existing action";
    std::fs::write(&path, previous).unwrap();
    let request = json!({"action": ADJUSTMENT_ACTION, "path": path, "skipUnsupported": false});
    let error = session.execute("actions.save", request).unwrap_err().to_string();
    assert!(error.contains(ADJUSTMENT_COMMAND), "{error}");
    assert_eq!(std::fs::read(path).unwrap(), previous);

    let saved = session.execute("actions.save", json!({"action": ADJUSTMENT_ACTION, "skipUnsupported": true})).unwrap();
    assert_eq!(saved["skipped"], json!([{"action": ADJUSTMENT_ACTION, "step": 0, "id": ADJUSTMENT_COMMAND}]));
    assert_eq!(saved["steps"], 0);
}

#[test]
fn adjustment_opacity_strict_export_preserves_representable_values() {
    for opacity in [None, Some(0.0), Some(95.0 / 255.0), Some(f64::from(95.0_f32 / 255.0)), Some(179.0 / 255.0), Some(1.0)] {
        let mut source = adjustment_session();
        set_adjustment_action(&mut source, opacity);
        let expected = play_adjustment_opacity(&mut source);
        let saved = source.execute("actions.save", json!({"action": ADJUSTMENT_ACTION})).unwrap();
        assert_eq!(saved["skipped"], json!([]));
        let data = saved["data"].as_str().unwrap();

        let mut restored = adjustment_session();
        restored.execute("actions.load", json!({"data": data})).unwrap();
        assert_eq!(play_adjustment_opacity(&mut restored), expected, "{opacity:?}");

        if opacity == Some(179.0 / 255.0) {
            let bytes = photocraft_paint::tile::b64_decode(data).unwrap();
            let file = atn::parse(&bytes).unwrap();
            let using = get_obj(file.actions[0].items[0].descriptor.as_ref().unwrap(), "Usng").unwrap();
            assert_eq!(get_num(using, "Opct"), Some(70.0), "keep the recorded whole percent");
        }
    }
}
