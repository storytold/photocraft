//! Tests that run paint commands, moved from the engine-core `fx_visibility_cmds` tests: 6 test(s).

use photocraft_engine::Session;
use serde_json::json;

use photocraft_doc::LayerId;

/// A red layer with a drop shadow and a blue colour overlay, and the background (no effects).
fn session() -> (Session, u64, u64) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    let bg = s.active().unwrap().doc.layers[0].id.0;
    let a = s.execute("layer.new.layer", json!({"name": "a"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s.execute("layer.layerStyle.dropShadow", json!({"layer": a})).unwrap();
    s.execute("layer.layerStyle.colorOverlay", json!({"layer": a, "color": "#0000ff"})).unwrap();
    (s, a, bg)
}

/// (the whole list is on, each effect is on).
fn state(s: &Session, id: u64) -> (bool, Vec<bool>) {
    let d = s.active().unwrap();
    let fx = &d.doc.layer(LayerId(id)).unwrap().effects;
    (fx.enabled, fx.items.iter().map(|e| e.enabled()).collect())
}

#[test]
fn named_nonactive_effects_layer_can_be_replayed() {
    let (mut s, a, _) = session();
    s.execute("layer.new.layer", json!({"name": "top"})).unwrap();
    let replay = s.active().unwrap().doc.as_ref().clone();
    s.execute("actions.record", json!({"name": "Hide effects"})).unwrap();
    s.execute("layer.setEffectsVisible", json!({"layer": a, "visible": false})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert!(!state(&s, a).0);
    let steps = s.execute("actions.get", json!({"action": "Hide effects"})).unwrap()["steps"].clone();
    assert_eq!(steps[0][1]["layer"], "a");
    s.add_document(replay, None);
    let played = s.execute("actions.play", json!({"action": "Hide effects"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(!state(&s, a).0, "the recorded name targeted the nonactive layer on replay");
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn flat(s: &Session) -> photocraft_compose::Buffer {
    photocraft_compose::flatten(&s.active().unwrap().doc)
}

#[test]
fn the_effects_row_eye_hides_and_shows_the_whole_list() {
    let (mut s, a, _) = session();
    let n = steps(&s);
    assert_eq!(s.execute("layer.setEffectsVisible", json!({"layer": a})).unwrap(), json!({"layer": a, "index": null, "visible": false}));
    assert_eq!(state(&s, a), (false, vec![true, true]), "each effect keeps its own state");
    assert_eq!(steps(&s), n + 1, "one history step");
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Hide Layer Effects"));
    // The active layer is the default target.
    assert_eq!(s.execute("layer.setEffectsVisible", json!({})).unwrap()["visible"], true);
    assert_eq!(state(&s, a), (true, vec![true, true]));
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Show Layer Effects"));
}

#[test]
fn one_effect_eye_hides_only_that_effect_and_undoes() {
    let (mut s, a, _) = session();
    assert_eq!(s.execute("layer.setEffectsVisible", json!({"layer": a, "index": 1})).unwrap()["visible"], false);
    assert_eq!(state(&s, a), (true, vec![true, false]));
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Hide Color Overlay"));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(state(&s, a), (true, vec![true, true]));
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(state(&s, a), (true, vec![true, false]));
    s.execute("layer.setEffectsVisible", json!({"layer": a, "index": 1, "visible": true})).unwrap();
    assert_eq!(state(&s, a), (true, vec![true, true]));
}

#[test]
fn hiding_changes_the_composite_and_keeps_the_settings() {
    let (mut s, a, _) = session();
    let before = s.active().unwrap().doc.layer(LayerId(a)).unwrap().effects.items.clone();
    let shown = flat(&s);
    s.execute("layer.setEffectsVisible", json!({"layer": a, "visible": false})).unwrap();
    assert_ne!(flat(&s), shown, "hidden effects are not drawn");
    s.execute("layer.setEffectsVisible", json!({"layer": a, "visible": true})).unwrap();
    assert_eq!(flat(&s), shown);
    assert_eq!(s.active().unwrap().doc.layer(LayerId(a)).unwrap().effects.items, before);
}

#[test]
fn asking_for_the_current_state_adds_no_history_step() {
    let (mut s, a, _) = session();
    let n = steps(&s);
    assert_eq!(s.execute("layer.setEffectsVisible", json!({"layer": a, "visible": true})).unwrap()["visible"], true);
    assert_eq!(s.execute("layer.setEffectsVisible", json!({"layer": a, "index": 0, "visible": true})).unwrap()["visible"], true);
    assert_eq!(steps(&s), n);
}

#[test]
fn bad_params_fail_gracefully() {
    let (mut s, a, bg) = session();
    let before = state(&s, a);
    for p in [
        json!({"layer": bg}),
        json!({"layer": u64::MAX}),
        json!({"layer": "x"}),
        json!({"layer": -1}),
        json!({"layer": a, "index": 2}),
        json!({"layer": a, "index": -1}),
        json!({"layer": a, "index": 1.5}),
        json!({"layer": a, "index": "0"}),
        json!({"layer": a, "index": u64::MAX}),
        json!({"layer": a, "visible": "yes"}),
        json!({"layer": a, "visible": 1}),
    ] {
        assert!(s.execute("layer.setEffectsVisible", p.clone()).is_err(), "{p}");
    }
    assert_eq!(state(&s, a), before, "a refused call changes nothing");
    let mut empty = Session::new();
    assert!(empty.execute("layer.setEffectsVisible", json!({})).is_err());
    empty.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    assert!(empty.execute("layer.setEffectsVisible", json!({})).is_err(), "no effects: disabled");
}
