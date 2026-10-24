//! Design timeline replay over a script with masks, clipping, groups, merges, a smart object
//! with a smart filter, reordering and strokes: suppressing any step never lands a later step
//! on a layer it didn't act on when recorded.
use photocraft_doc::design::{StepState, layer_changes};
use photocraft_engine::Session;
use serde_json::{Value, json};

/// Steps that lean on implicit state: the active layer, the selection, colours and tool settings.
fn harsh_script() -> Vec<(&'static str, Value)> {
    let stroke = |pts: Value, color: &str| json!({"points": pts, "size": 6, "hardness": 1.0, "color": color});
    vec![
        ("layer.new.layer", json!({"name": "A"})),
        ("paint.stroke", stroke(json!([[4, 4], [40, 30]]), "#ff0000")),
        ("layer.new.layer", json!({"name": "B"})),
        ("select.rect", json!({"x": 20, "y": 8, "width": 30, "height": 30})),
        ("edit.fillForeground", json!({})),
        ("select.deselect", json!({})),
        ("layer.layerMask.revealAll", json!({})),
        ("paint.stroke", json!({"points": [[20, 20], [50, 20]], "size": 8, "hardness": 1.0, "color": "#000000", "target": "mask"})),
        ("layer.newAdjustmentLayer.invert", json!({})),
        ("layer.createClippingMask", json!({})),
        ("layer.select", json!({"layer": "A"})),
        ("filter.noise.addNoise", json!({"amount": 20})),
        ("layer.groupLayers", json!({})),
        ("layer.select", json!({"layer": "A"})),
        ("layer.new.layer", json!({"name": "C"})),
        ("paint.stroke", stroke(json!([[10, 40], [60, 40]]), "#0000ff")),
        ("layer.mergeDown", json!({})),
        ("layer.select", json!({"layer": "B"})),
        ("layer.smartObjects.convertToSmartObject", json!({})),
        ("filter.blur.gaussianBlur", json!({"radius": 2})),
        ("layer.arrange.sendBackward", json!({})),
        ("layer.duplicate", json!({})),
        ("layer.setProps", json!({"opacity": 0.5})),
    ]
}

/// A session that recorded the harsh script with the timeline on.
fn recorded() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    for (id, p) in harsh_script() {
        s.execute(id, p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    s
}

#[test]
fn every_recorded_step_replays() {
    let mut s = recorded();
    s.execute("design.replay", json!({})).unwrap();
    let tl = s.active().unwrap().doc.design_timeline.clone().unwrap();
    for st in &tl.steps {
        assert!(!st.state.is_failure(), "{} {:?}", st.command, st.state);
    }
}

#[test]
fn suppressing_any_step_never_retargets_a_later_one() {
    let n = recorded().active().unwrap().doc.design_timeline.as_ref().unwrap().steps.len();
    let mut failed = 0;
    for k in 0..n {
        let mut s = recorded();
        let id = s.active().unwrap().doc.design_timeline.as_ref().unwrap().steps[k].step;
        s.execute("design.suppress", json!({"step": id})).unwrap_or_else(|e| panic!("suppress {k}: {e}"));
        let tl = s.active().unwrap().doc.design_timeline.clone().unwrap();
        let mut pre = tl.steps[k].checkpoint.0.clone().unwrap();
        for (i, st) in tl.steps.iter().enumerate().skip(k + 1) {
            let post = st.checkpoint.0.clone().unwrap();
            if st.state == StepState::Ok || st.state == StepState::PlacedElsewhere {
                let (_, changed) = layer_changes(&pre, &post);
                assert!(changed.iter().all(|id| st.changed.contains(id)), "drop {k}: step {i} {} changed {changed:?}, recorded {:?}", st.command, st.changed);
            } else {
                failed += 1;
            }
            pre = post;
        }
    }
    assert!(failed > 0, "dropping steps makes the steps that depended on them fail visibly");
}

#[test]
fn a_saved_timeline_replays_in_another_session() {
    let s = recorded();
    let doc = (*s.active().unwrap().doc).clone();
    let bytes = photocraft_format::save_to_bytes(&doc, &photocraft_format::SaveOptions::default()).unwrap();
    let mut r = Session::new();
    r.add_document(photocraft_format::load_from_bytes(&bytes).unwrap(), None);
    let content = |s: &Session| photocraft_doc::Document { design_timeline: None, ..(*s.active().unwrap().doc).clone() };
    let before = content(&r);
    r.execute("design.replay", json!({})).unwrap();
    assert_eq!(content(&r), before, "the saved steps rebuild the saved document");
    // Recording right after the load: no other command has moved the step clock yet.
    let mut r = Session::new();
    r.add_document(photocraft_format::load_from_bytes(&bytes).unwrap(), None);
    r.execute("layer.new.layer", json!({"name": "after reload"})).unwrap();
    let tl = r.active().unwrap().doc.design_timeline.clone().unwrap();
    assert_eq!(tl.steps.last().unwrap().command, "layer.new.layer", "recording continues after a reload");
    let ids: Vec<u64> = tl.steps.iter().map(|st| st.step).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "new steps get ids past the saved ones: {ids:?}");
}
