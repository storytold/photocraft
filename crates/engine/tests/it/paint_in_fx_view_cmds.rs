//! Tests that run paint commands, moved from the engine-core `fx_view_cmds` tests: 3 test(s).

use photocraft_doc::LayerId;
use photocraft_engine::Session;
use serde_json::json;

fn session() -> (Session, u64, u64) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    let a = s.execute("layer.new.layer", json!({"name": "a"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s.execute("layer.layerStyle.dropShadow", json!({"layer": a})).unwrap();
    let b = s.execute("layer.new.layer", json!({"name": "b"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#00ff00"})).unwrap();
    s.execute("layer.layerStyle.stroke", json!({})).unwrap();
    (s, a, b)
}

/// Whether the layer's effects list is shown, i.e. it is not in the document's collapsed set.
fn open(s: &Session, id: u64) -> bool {
    !s.active().unwrap().fx_collapsed.contains(&LayerId(id))
}

/// Smart Filters hide and show with the layer's effects, as view state: no document edit, revision or history step;
/// the document's `Tracked` revision, which the Layers panel's rows follow, moves.
#[test]
fn smart_filters_share_the_effects_disclosure_without_an_edit() {
    let (mut s, a, b) = session();
    s.execute("layer.select", json!({"layer": a})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("image.adjustments.invert", json!({})).unwrap();
    let smart = s.active().unwrap().active_layer.unwrap();
    let doc = s.active().unwrap().doc.clone();
    let revisions = |s: &Session| s.active_tracked().map(|st| (st.revision, st.revision().count)).unwrap();
    let before = revisions(&s);
    let steps = s.active().unwrap().history.past_len();
    s.execute("layer.setEffectsExpanded", json!({"layer": smart.0, "expanded": false})).unwrap();
    assert!(!open(&s, smart.0));
    assert!(open(&s, b));
    assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc));
    let after = revisions(&s);
    assert!(after.0 == before.0 && after.1 > before.1, "the tracked revision only: {before:?} -> {after:?}");
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    s.execute("layer.setEffectsExpanded", json!({"expanded": false, "all": true})).unwrap();
    assert!(!open(&s, smart.0) && !open(&s, b));
    s.execute("layer.setEffectsExpanded", json!({"expanded": true, "all": true})).unwrap();
    assert!(s.active().unwrap().fx_collapsed.is_empty());
}

/// Toggling one layer or all of them flips only the disclosure state: no history step, no duplicates, and the document stays clean.
#[test]
fn toggles_one_layer_or_all_without_history() {
    let (mut s, a, b) = session();
    let st = s.active_mut().unwrap();
    st.saved_revision = st.revision;
    let steps = s.active().unwrap().history.entries().len();
    assert!(open(&s, a) && open(&s, b), "effects lists start open");
    assert_eq!(s.execute("layer.setEffectsExpanded", json!({"layer": a})).unwrap()["expanded"], false);
    assert!(!open(&s, a) && open(&s, b));
    assert_eq!(s.active().unwrap().history.entries().len(), steps, "no history step");
    assert_eq!(s.active().unwrap().saved_revision, s.active().unwrap().revision, "still clean");
    s.execute("layer.setEffectsExpanded", json!({"layer": a, "all": true})).unwrap();
    assert!(open(&s, a) && open(&s, b), "toggling an open-and-closed set from a closed layer opens all");
    s.execute("layer.setEffectsExpanded", json!({"all": true, "expanded": false})).unwrap();
    assert!(!open(&s, a) && !open(&s, b));
    s.execute("layer.setEffectsExpanded", json!({"layer": b, "expanded": true})).unwrap();
    assert!(!open(&s, a) && open(&s, b));
    assert_eq!(s.active().unwrap().fx_collapsed.len(), 1, "no duplicates");
}

#[test]
fn bad_params_fail_gracefully() {
    let (mut s, _, _) = session();
    let bg = s.active().unwrap().doc.layers[0].id.0;
    for p in [
        json!({"layer": bg}),
        json!({"layer": u64::MAX}),
        json!({"layer": "x"}),
        json!({"layer": -1}),
        json!({"expanded": "yes", "all": true}),
        json!({"all": 3}),
    ] {
        assert!(s.execute("layer.setEffectsExpanded", p.clone()).is_err(), "{p}");
    }
    let mut empty = Session::new();
    assert!(empty.execute("layer.setEffectsExpanded", json!({})).is_err());
    empty.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    assert!(empty.execute("layer.setEffectsExpanded", json!({"all": true})).is_err(), "no effects: disabled");
}
