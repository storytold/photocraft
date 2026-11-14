use super::*;

const COMMAND: &str = "image.adjustments.colorBalance";

fn session_with_pixels() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
    s.execute("edit.fill", json!({"color": "#906040"})).unwrap();
    s
}

#[test]
fn zero_color_balance_preserves_document_revision_and_redo() {
    let mut s = session_with_pixels();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    let before = s.active().unwrap();
    let doc = before.doc.clone();
    let revision = before.revision;
    let history = before.history.past_len();
    assert!(before.history.can_redo());

    for params in [
        json!({}),
        json!({"shadows": [0, 0, 0], "midtones": [0, 0, 0], "highlights": [0, 0, 0]}),
        json!({"shadows": [0, -0.0, 0], "midtones": [0, 0, 0], "highlights": [0, 0, 0], "preserveLuminosity": false}),
    ] {
        assert_eq!(s.execute(COMMAND, params).unwrap(), Value::Null);
        let after = s.active().unwrap();
        assert!(std::sync::Arc::ptr_eq(&after.doc, &doc));
        assert_eq!(after.revision, revision);
        assert_eq!(after.history.past_len(), history);
        assert!(after.history.can_redo());
    }
}

#[test]
fn nonzero_color_balance_records_one_named_edit_for_each_tone() {
    for tone in ["shadows", "midtones", "highlights"] {
        let mut s = session_with_pixels();
        let history = s.active().unwrap().history.past_len();
        s.execute(COMMAND, json!({(tone): [38, 0, 0], "preserveLuminosity": false})).unwrap();
        let after = s.active().unwrap();
        assert_eq!(after.history.past_len(), history + 1);
        assert_eq!(after.history.entries().last().map(String::as_str), Some("Color Balance"));
    }
}

#[test]
fn zero_color_balance_keeps_parameter_and_layer_validation() {
    let mut s = session_with_pixels();
    let history = s.active().unwrap().history.past_len();
    for params in [json!({"midtones": [0, 0]}), json!({"preserveLuminosity": 1}), json!({"layer": "Missing Layer"})] {
        assert!(s.execute(COMMAND, params).is_err());
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
    s.execute("layer.setProps", json!({"locks": {"pixels": true}})).unwrap();
    let history = s.active().unwrap().history.past_len();
    assert!(s.execute(COMMAND, json!({})).unwrap_err().to_string().contains("locked"));
    assert_eq!(s.active().unwrap().history.past_len(), history);
}

#[test]
fn zero_color_balance_preserves_smart_filter_mask_channel_and_action_edits() {
    for (setup, adjust, params) in [
        ("layer.smartObjects.convertToSmartObject", COMMAND, json!({})),
        ("layer.layerMask.revealAll", COMMAND, json!({"target": "mask"})),
        ("channel.new", COMMAND, json!({"target": {"channel": 0}})),
        ("", "actions.play", json!({"action": "Neutral adjustment"})),
    ] {
        let mut s = session_with_pixels();
        if !setup.is_empty() {
            s.execute(setup, json!({})).unwrap();
        }
        if adjust == "actions.play" {
            s.execute("actions.setHistory", json!({"singleState": false})).unwrap();
            s.actions.list.push(crate::actions_cmds::Action::new("Neutral adjustment", vec![(COMMAND.into(), json!({}))]));
        }
        let history = s.active().unwrap().history.past_len();
        s.execute(adjust, params).unwrap();
        let after = s.active().unwrap();
        assert_eq!(after.history.past_len(), history + 1, "{setup}");
        if setup == "layer.smartObjects.convertToSmartObject" {
            let layer = after.doc.layer(after.active_layer.unwrap()).unwrap();
            let LayerContent::Smart(smart) = &layer.content else { panic!("expected smart object") };
            assert_eq!(smart.smart_filters.last().unwrap().command, COMMAND);
        }
    }
}
