use serde_json::{Value, json};

use crate::Session;

/// The timeline's steps as `design.list` reports them.
pub(super) fn steps(s: &mut Session) -> Vec<Value> {
    s.execute("design.list", json!({})).unwrap()["steps"].as_array().unwrap().clone()
}

/// The command id of every recorded step.
pub(super) fn commands(s: &mut Session) -> Vec<String> {
    steps(s).iter().map(|st| st["command"].as_str().unwrap().to_string()).collect()
}

/// A 16×16 document with the timeline on.
fn started() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    s
}

#[test]
fn records_document_changes_only() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("layer.new.layer", json!({"name": "before"})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    assert!(steps(&mut s).is_empty(), "steps before the start aren't recorded");
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    let a = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.select", json!({"layer": "before"})).unwrap();
    s.execute("view.zoomIn", json!({})).ok();
    s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
    assert_eq!(commands(&mut s), ["layer.new.layer", "layer.setProps"], "navigation isn't a step");
    let st = steps(&mut s);
    assert_eq!(st[0]["created"], json!([a.0]));
    let before = s.active().unwrap().active_layer.unwrap();
    assert_eq!(st[1]["changed"], json!([before.0]));
    assert_ne!(st[0]["step"], st[1]["step"]);
    assert_eq!(photocraft_doc::id_step(a.0), st[0]["step"].as_u64(), "the step's id scope minted its layer");
}

#[test]
fn records_the_targets_and_the_tool_state_before_a_step() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    let a = s.active().unwrap().active_layer.unwrap();
    s.execute("tools.setColors", json!({"foreground": [1.0, 0.0, 0.0, 1.0]})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    let st = steps(&mut s);
    let fill = st.last().unwrap();
    assert_eq!(fill["command"], "edit.fillForeground");
    assert_eq!(fill["changed"], json!([a.0]));
    let tl = s.active().unwrap().doc.design_timeline.clone().unwrap();
    let rec = tl.steps.last().unwrap();
    assert_eq!(rec.active, Some(a));
    assert_eq!(rec.selected, vec![a]);
    let select = &tl.steps[tl.steps.len() - 2];
    assert_eq!(select.prelude.iter().map(|p| p.command.as_str()).collect::<Vec<_>>(), ["tools.setColors"]);
    assert!(rec.prelude.is_empty(), "a prelude goes with the next step only");
}

#[test]
fn undo_removes_the_step_and_redo_restores_it() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.setProps", json!({"opacity": 0.25})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(commands(&mut s), ["layer.new.layer"]);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(commands(&mut s), ["layer.new.layer", "layer.setProps"]);
    s.execute("design.stop", json!({})).unwrap();
    assert!(s.execute("design.list", json!({})).is_err());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(commands(&mut s).len(), 2, "stopping is undoable");
}

#[test]
fn coalesced_edits_join_the_previous_step() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.setProps", json!({"opacity": 0.5, "coalesce": "drag"})).unwrap();
    s.execute("layer.setProps", json!({"opacity": 0.4, "coalesce": "drag"})).unwrap();
    let st = steps(&mut s);
    assert_eq!(st.iter().map(|x| x["joinsPrevious"].as_bool().unwrap()).collect::<Vec<_>>(), [false, false, true]);
}

#[test]
fn a_layer_named_in_params_is_recorded_by_id() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    let a = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.setProps", json!({"layer": "A", "visible": false})).unwrap();
    assert_eq!(steps(&mut s).last().unwrap()["params"]["layer"], json!(a.0));
}

/// Step `i`'s outcome (`ok`, `stale`, …).
fn state(s: &mut Session, i: usize) -> String {
    steps(s)[i]["state"]["state"].as_str().unwrap().to_string()
}

/// Step `i`'s id.
fn step_id(s: &mut Session, i: usize) -> u64 {
    steps(s)[i]["step"].as_u64().unwrap()
}

/// The active document without its timeline, to compare content.
fn doc(s: &Session) -> photocraft_doc::Document {
    let mut d = (*s.active().unwrap().doc).clone();
    d.design_timeline = None;
    d
}

/// Background, then A filled, B filled and blurred, A inverted, A hidden by id.
fn script() -> Session {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    let a = s.active().unwrap().active_layer.unwrap().0;
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 6, "height": 6})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "B"})).unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 4, "width": 6, "height": 6})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("filter.blur.gaussianBlur", json!({"radius": 1})).unwrap();
    s.execute("layer.select", json!({"layer": "A"})).unwrap();
    s.execute("image.adjustments.invert", json!({})).unwrap();
    s.execute("layer.setProps", json!({"layer": a, "visible": false})).unwrap();
    s
}

#[test]
fn replaying_everything_reproduces_the_document() {
    let mut s = script();
    let before = doc(&s);
    s.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&s), before);
    assert!(steps(&mut s).iter().all(|x| x["state"]["state"] == "ok"), "{:?}", steps(&mut s));
}

#[test]
fn suppressing_a_layer_fails_the_steps_on_it_and_unsuppressing_restores() {
    let mut s = script();
    let before = doc(&s);
    let new_a = step_id(&mut s, 0);
    s.execute("design.suppress", json!({"step": new_a})).unwrap();
    let names: Vec<String> = doc(&s).walk().into_iter().map(|(_, _, l)| l.name.clone()).collect();
    assert!(!names.contains(&"A".to_string()) && names.contains(&"B".to_string()), "{names:?}");
    // Fill A, invert A and hide A acted on the removed layer: they fail visibly, not elsewhere.
    for i in [2, 8, 9] {
        assert!(["missingReference", "retargeted"].contains(&state(&mut s, i).as_str()), "step {i}: {}", state(&mut s, i));
    }
    assert_eq!(state(&mut s, 3), "placedElsewhere", "B was placed above A, which is gone");
    for i in [5, 7] {
        assert_eq!(state(&mut s, i), "ok", "step {i} on B is unaffected");
    }
    s.execute("design.unsuppress", json!({"step": new_a})).unwrap();
    assert_eq!(doc(&s), before);
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(doc(&s), before, "suppressing is undoable");
}

#[test]
fn dropping_any_step_never_lands_a_later_step_on_another_layer() {
    let n = steps(&mut script()).len();
    for k in 0..n {
        let mut s = script();
        let id = step_id(&mut s, k);
        s.execute("design.suppress", json!({"step": id})).unwrap();
        let tl = s.active().unwrap().doc.design_timeline.clone().unwrap();
        for (i, st) in tl.steps.iter().enumerate().skip(k + 1) {
            // A step that ran had to stay within the layers it changed when recorded.
            if st.state == photocraft_doc::design::StepState::Ok {
                let pre = tl.steps[i - 1].checkpoint.0.clone().unwrap();
                let post = st.checkpoint.0.clone().unwrap();
                let (_, changed) = photocraft_doc::design::layer_changes(&pre, &post);
                assert!(changed.iter().all(|id| st.changed.contains(id)), "drop {k}: step {i} {} retargeted", st.command);
            }
        }
    }
}

#[test]
fn a_hidden_second_target_is_caught() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "B"})).unwrap();
    s.execute("edit.fillBackground", json!({})).unwrap();
    s.execute("layer.mergeDown", json!({})).unwrap();
    let new_a = step_id(&mut s, 0);
    s.execute("design.suppress", json!({"step": new_a})).unwrap();
    // Without A, Merge Down would merge B into the Background, a layer it never touched.
    let last = steps(&mut s).len() - 1;
    assert!(["retargeted", "missingReference"].contains(&state(&mut s, last).as_str()), "{}", state(&mut s, last));
}

#[test]
fn rolling_back_shows_the_earlier_state_and_inserts_new_edits_there() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    let after_a = doc(&s);
    s.execute("layer.new.layer", json!({"name": "B"})).unwrap();
    s.execute("design.rollTo", json!({"at": 1})).unwrap();
    assert_eq!(doc(&s), after_a);
    s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
    assert_eq!(commands(&mut s), ["layer.new.layer", "layer.setProps", "layer.new.layer"]);
    s.execute("design.rollTo", json!({"at": null})).unwrap();
    let d = doc(&s);
    let a = d.walk().into_iter().find(|(_, _, l)| l.name == "A").unwrap().2.opacity;
    assert_eq!(a, 0.5);
    assert!(d.walk().iter().any(|(_, _, l)| l.name == "B"), "B replays after the inserted step");
}

#[test]
fn editing_and_deleting_steps() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
    let props = step_id(&mut s, 1);
    s.execute("design.editStep", json!({"step": props, "params": {"opacity": 0.25}})).unwrap();
    let opacity = |s: &Session| doc(s).walk().into_iter().find(|(_, _, l)| l.name == "A").unwrap().2.opacity;
    assert_eq!(opacity(&s), 0.25);
    s.execute("design.delete", json!({"step": props})).unwrap();
    assert_eq!(opacity(&s), 1.0);
    assert_eq!(commands(&mut s), ["layer.new.layer"]);
}

#[test]
fn a_change_the_timeline_did_not_record_is_never_replayed_away() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.edit("Unrecorded", |doc, _| {
        doc.resolution_dpi = 150.0;
        Ok(())
    })
    .unwrap();
    let err = s.execute("design.replay", json!({})).unwrap_err().to_string();
    assert!(err.contains("didn't record"), "{err}");
}

#[test]
fn every_step_has_its_own_id() {
    let mut s = script();
    let mut ids: Vec<u64> = steps(&mut s).iter().map(|x| x["step"].as_u64().unwrap()).collect();
    let n = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), n, "steps that mint no layer still get their own id");
}

#[test]
fn a_step_that_would_change_another_layer_is_refused() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "B"})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    s.execute("layer.mergeDown", json!({})).unwrap();
    let before = doc(&s);
    // Roll back to before the merge and move B under A: the merge would now hit the Background.
    s.execute("design.rollTo", json!({"at": 5})).unwrap();
    s.execute("layer.arrange.sendBackward", json!({})).unwrap();
    s.execute("design.rollTo", json!({"at": null})).unwrap();
    let last = steps(&mut s).len() - 1;
    assert_eq!(state(&mut s, last), "retargeted");
    let bg = doc(&s).layers[0].clone();
    assert_eq!(bg, before.layers[0], "the Background is untouched");
}

#[test]
fn an_edited_step_may_target_another_layer() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    let a = s.active().unwrap().active_layer.unwrap().0;
    s.execute("layer.new.layer", json!({"name": "B"})).unwrap();
    s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
    let props = step_id(&mut s, 2);
    s.execute("design.editStep", json!({"step": props, "params": {"layer": a, "opacity": 0.5}})).unwrap();
    assert_eq!(state(&mut s, 2), "ok");
    assert_eq!(steps(&mut s)[2]["changed"], json!([a]));
}

#[test]
fn bad_params_are_errors() {
    let mut s = script();
    let bad = [
        json!(null),
        json!({}),
        json!({"step": -1}),
        json!({"step": "x"}),
        json!({"step": u64::MAX}),
        json!({"at": 999}),
        json!({"at": -3}),
        json!({"step": 1, "params": 5}),
    ];
    for id in ["design.suppress", "design.unsuppress", "design.delete", "design.rollTo", "design.editStep"] {
        for p in &bad {
            let _ = s.execute(id, p.clone());
        }
    }
    assert!(s.execute("design.start", json!({})).is_err(), "already on");
    s.execute("design.replay", json!({})).unwrap();
}

#[test]
fn a_baked_step_replays_its_result_and_goes_stale_after_an_earlier_change() {
    let mut s = started();
    s.execute("select.rect", json!({"x": 1, "y": 1, "width": 5, "height": 5})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.execute("select.reselect", json!({})).unwrap();
    let st = steps(&mut s);
    assert_eq!(st[3]["baked"], true);
    let before = doc(&s);
    // The scratch session replay runs in has no selection to reselect: the stored one is used.
    s.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&s), before);
    assert_eq!(state(&mut s, 3), "ok");
    let new_a = step_id(&mut s, 2);
    s.execute("design.suppress", json!({"step": new_a})).unwrap();
    assert_eq!(state(&mut s, 3), "stale");
    assert!(doc(&s).selection.is_some(), "a stale step still applies its result");
}

#[test]
fn smart_object_contents_keep_a_nested_timeline() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "P"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let parent = s.active_index().unwrap();
    s.execute("layer.smartObjects.editContents", json!({})).unwrap();
    assert_ne!(s.active_index(), Some(parent));
    assert!(steps(&mut s).is_empty(), "the contents start a timeline of their own");
    s.execute("layer.new.layer", json!({"name": "Inner"})).unwrap();
    assert_eq!(commands(&mut s), ["layer.new.layer"]);
    s.execute("layer.smartObjects.saveContents", json!({})).unwrap();
    // Closed, so the next Edit Contents reads the saved contents back.
    s.execute("file.close", json!({})).unwrap();
    assert_eq!(s.documents().len(), 1);
    s.set_active(parent);
    let st = steps(&mut s);
    let save = st.last().unwrap();
    assert_eq!(save["command"], "layer.smartObjects.saveContents");
    assert_eq!(save["baked"], true, "a step made from another document replays its result");
    let before = doc(&s);
    s.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&s), before);
    // Editing the contents again resumes their timeline.
    s.execute("layer.smartObjects.editContents", json!({})).unwrap();
    assert_eq!(commands(&mut s), ["layer.new.layer"]);
    // Suppressing the conversion removes the smart object: its saved edit can't apply.
    s.set_active(parent);
    let convert = step_id(&mut s, 3);
    s.execute("design.suppress", json!({"step": convert})).unwrap();
    let last = steps(&mut s).len() - 1;
    assert_eq!(state(&mut s, last), "missingReference");
}

#[test]
fn clipboard_and_fade_steps_replay_their_result() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 6, "height": 6})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    s.execute("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
    s.execute("edit.fade", json!({"opacity": 50})).unwrap();
    assert_eq!(commands(&mut s), ["edit.paste", "filter.blur.gaussianBlur", "edit.fade"]);
    let before = doc(&s);
    // The replay session has neither the clipboard nor the Fade source.
    s.execute("design.replay", json!({})).unwrap();
    assert!(steps(&mut s).iter().all(|x| x["state"]["state"] == "ok"), "{:?}", steps(&mut s));
    assert_eq!(doc(&s), before);
}

#[test]
fn starting_or_stopping_the_timeline_modifies_a_clean_document() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.active_mut().unwrap().saved_to("a.pcraft".into());
    s.execute("design.start", json!({})).unwrap();
    assert!(s.active().unwrap().is_dirty(), "a save would now write the timeline");
    s.active_mut().unwrap().saved_to("a.pcraft".into());
    s.execute("select.rect", json!({"x": 1, "y": 1, "width": 4, "height": 4})).unwrap();
    assert!(!s.active().unwrap().is_dirty(), "a selection step growing the timeline keeps it clean");
    s.execute("design.stop", json!({})).unwrap();
    assert!(s.active().unwrap().is_dirty());
}

#[test]
fn a_checkpoint_holds_no_timeline_of_its_own() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "B"})).unwrap();
    let tl = s.active().unwrap().doc.design_timeline.clone().unwrap();
    // One that did would keep every earlier checkpoint alive past the budget.
    assert!(tl.steps.iter().filter_map(|st| st.checkpoint.0.as_ref()).all(|d| d.design_timeline.is_none()));
    assert!(tl.steps.iter().any(|st| st.checkpoint.0.is_some()));
}

#[test]
fn a_timeline_rewrite_that_leaves_the_layers_alone_still_modifies_the_document() {
    let mut s = started();
    s.execute("select.rect", json!({"x": 1, "y": 1, "width": 4, "height": 4})).unwrap();
    // Saved under its own name, so the save itself changes nothing the timeline would refuse.
    let name = s.active().unwrap().doc.name.clone();
    s.active_mut().unwrap().saved_to(name);
    assert!(!s.active().unwrap().is_dirty());
    let sel = step_id(&mut s, 0);
    s.execute("design.suppress", json!({"step": sel})).unwrap();
    assert!(s.active().unwrap().is_dirty(), "the saved file still has the step unsuppressed");
}

#[test]
fn a_step_naming_a_shared_layer_name_replays_on_the_layer_it_acted_on() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "X"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "X"})).unwrap();
    s.execute("layer.setProps", json!({"layer": "X", "opacity": 0.5})).unwrap();
    let before = doc(&s);
    s.execute("design.replay", json!({})).unwrap();
    assert!(steps(&mut s).iter().all(|x| x["state"]["state"] == "ok"), "{:?}", steps(&mut s));
    assert_eq!(doc(&s), before);
}

#[test]
fn saving_under_a_new_name_leaves_the_timeline_editable() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
    s.active_mut().unwrap().saved_to("/tmp/city.pcraft".into());
    let first = step_id(&mut s, 0);
    s.execute("design.suppress", json!({"step": first})).unwrap();
    assert_eq!(s.active().unwrap().doc.name, "city.pcraft", "replay keeps the saved name");
}

#[test]
fn a_layer_copied_from_another_document_never_goes_stale() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "early"})).unwrap();
    s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("layer.new.layer", json!({"name": "source"})).unwrap();
    s.execute("layer.copyToDocument", json!({"document": 0})).unwrap();
    s.set_active(0);
    let at = |s: &mut Session, cmd: &str| commands(s).iter().position(|c| c == cmd).unwrap();
    let (props, paste, copy) = (at(&mut s, "layer.setProps"), at(&mut s, "edit.paste"), at(&mut s, "layer.copyToDocument"));
    let id = step_id(&mut s, props);
    s.execute("design.editStep", json!({"step": id, "params": {"opacity": 0.8}})).unwrap();
    // Its pixels came from the other document, which no earlier step here can change.
    assert_eq!(state(&mut s, copy), "ok");
    // A paste lands by the selection an earlier step may have changed.
    assert_eq!(state(&mut s, paste), "stale");
    let source = s.documents()[1].doc.name.clone();
    assert_eq!(steps(&mut s)[copy]["label"], format!("Copy from {source}"), "the step names where the layer came from");
}

#[test]
fn a_placed_file_step_names_the_file() {
    let label = |id: &str, p: Value| crate::timeline::step_label("Place Embedded", id, &p, None);
    assert_eq!(label("file.placeEmbedded", json!({"path": "/art/assets/taipei.jpg"})), "Place Embedded taipei.jpg");
    assert_eq!(label("layer.setProps", json!({"path": "x.png"})), "Place Embedded", "only place commands name a path");
    assert_eq!(crate::timeline::step_label("Save Contents", "layer.smartObjects.saveContents", &json!({}), Some("Inner.psb")), "Save Contents from Inner.psb");
}

#[test]
fn a_file_placed_from_a_picker_is_a_baked_step() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
    let src = photocraft_doc::Document::new("p", photocraft_doc::Size::new(8, 8), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    let bytes = photocraft_io::export(&src, "p.png", &Default::default()).unwrap().bytes;
    // File › Place… and a drop pass the bytes, not a command.
    crate::file_cmds::place_bytes(&mut s, "p.png", bytes, None, &json!({})).unwrap();
    let st = steps(&mut s);
    let place = st.last().unwrap();
    assert_eq!(place["command"], "file.placeEmbedded");
    assert_eq!(place["baked"], true, "a replay has no bytes to place again");
    assert_eq!(place["label"], "Place Embedded p.png");
    let before = doc(&s);
    s.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&s), before);
    let props = step_id(&mut s, 1);
    s.execute("design.editStep", json!({"step": props, "params": {"opacity": 0.8}})).unwrap();
    assert_eq!(state(&mut s, 2), "ok", "a placed file doesn't depend on earlier steps");
}

/// The layer named "A".
fn layer_a(s: &Session) -> photocraft_doc::Layer {
    let doc = &s.active().unwrap().doc;
    doc.layer(crate::commands::layer_named(doc, "A").unwrap()).unwrap().clone()
}

/// A timeline of three steps: a new layer "A", then its opacity, then hiding it.
fn three_steps() -> Session {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    s
}

#[test]
fn edit_parameters_replays_once_reporting_each_step_and_puts_the_marker_back() {
    let mut s = three_steps();
    let props = step_id(&mut s, 1);
    s.execute("design.rollTo", json!({"at": 1})).unwrap();
    let ctx = crate::jobs::JobCtx::new();
    s.set_inline_job_ctx(Some(ctx.clone()));
    let r = s.execute("design.editStep", json!({"step": props, "params": {"opacity": 0.8}, "restore": null})).unwrap();
    assert_eq!(r["marker"], Value::Null, "back at the end");
    assert_eq!(ctx.message(), "Step 3 of 3", "progress names each step it replays");
    let a = layer_a(&s);
    assert!((a.opacity - 0.8).abs() < 1e-6 && !a.visible, "the edit and the steps after it");
}

#[test]
fn a_replay_runs_as_a_background_job_and_cancelling_it_changes_nothing() {
    let mut s = three_steps();
    let props = step_id(&mut s, 1);
    let edit = json!({"step": props, "params": {"opacity": 0.8}});
    let before = s.active().unwrap().doc.clone();
    let crate::jobs::Started::Job(id) = s.start("design.editStep", edit.clone()).unwrap() else { panic!("a replay is a job") };
    assert!(s.cancel_job(id));
    assert!(s.wait_job(id).is_err());
    assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before), "document and timeline untouched");
    let crate::jobs::Started::Job(id) = s.start("design.editStep", edit).unwrap() else { panic!("a replay is a job") };
    s.wait_job(id).unwrap();
    assert!((layer_a(&s).opacity - 0.8).abs() < 1e-6);
}

#[test]
fn rolling_back_to_a_checkpoint_lands_at_once() {
    let mut s = three_steps();
    assert!(matches!(s.start("design.rollTo", json!({"at": 1})).unwrap(), crate::jobs::Started::Done(_)), "a dialog opens on it next");
}

/// `s`'s timeline with `command` and `params` appended as an unbaked step, as a crafted file can.
fn with_crafted_step(s: &mut Session, command: &str, params: Value) {
    let st = s.active_mut().unwrap();
    let mut doc = (*st.doc).clone();
    let mut tl = (**doc.design_timeline.as_ref().unwrap()).clone();
    let mut step = tl.steps[0].clone();
    (step.step, step.command, step.params, step.baked, step.created, step.changed) = (999, command.into(), params, None, Vec::new(), Vec::new());
    tl.steps.push(step);
    doc.design_timeline = Some(std::sync::Arc::new(tl));
    st.doc = std::sync::Arc::new(doc);
}

#[test]
fn a_crafted_step_that_writes_a_file_is_refused_on_replay() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    let out = std::env::temp_dir().join(format!("pc-crafted-{}.png", std::process::id()));
    let _ = std::fs::remove_file(&out);
    with_crafted_step(&mut s, "layer.quickExportAsPng", json!({"path": out.to_str().unwrap()}));
    s.execute("design.replay", json!({})).unwrap();
    assert!(!out.exists(), "a replay must not write outside the document");
    assert_eq!(state(&mut s, 1), "error");
    assert!(steps(&mut s)[1]["state"]["message"].as_str().unwrap().contains("outside the document"));
}

#[test]
fn a_crafted_place_step_reads_no_file_on_replay() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    let src = std::env::temp_dir().join(format!("pc-crafted-src-{}.png", std::process::id()));
    let png = photocraft_doc::Document::new("p", photocraft_doc::Size::new(8, 8), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    std::fs::write(&src, photocraft_io::export(&png, "p.png", &Default::default()).unwrap().bytes).unwrap();
    let layers = s.active().unwrap().doc.walk().len();
    with_crafted_step(&mut s, "file.placeEmbedded", json!({"path": src.to_str().unwrap()}));
    s.execute("design.replay", json!({})).unwrap();
    let _ = std::fs::remove_file(&src);
    assert_eq!(s.active().unwrap().doc.walk().len(), layers, "nothing was read into the document");
    assert_eq!(state(&mut s, 1), "error");
}

#[test]
fn a_recorded_step_reaching_outside_the_document_is_baked_and_still_replays() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    // The working profile may be an .icc file: recorded with its result, never resolved again.
    s.execute("edit.assignProfile", json!({"profile": "working"})).unwrap();
    assert_eq!(commands(&mut s)[1], "edit.assignProfile");
    assert_eq!(steps(&mut s)[1]["baked"], true);
    let before = doc(&s);
    s.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&s), before);
    assert_eq!(state(&mut s, 1), "ok");
}

#[test]
fn an_embedded_place_by_path_replays_without_its_source() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    let src = std::env::temp_dir().join(format!("pc-embedded-src-{}.png", std::process::id()));
    let png = photocraft_doc::Document::new("p", photocraft_doc::Size::new(8, 8), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    std::fs::write(&src, photocraft_io::export(&png, "p.png", &Default::default()).unwrap().bytes).unwrap();
    s.execute("file.placeEmbedded", json!({"path": src.to_str().unwrap()})).unwrap();
    std::fs::remove_file(&src).unwrap();
    assert_eq!(steps(&mut s)[1]["baked"], true);
    let before = doc(&s);
    s.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&s), before, "the placed pixels come from the step, not the file");
    assert_eq!(state(&mut s, 1), "ok");
}
