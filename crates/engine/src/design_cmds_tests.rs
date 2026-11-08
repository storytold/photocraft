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
            let r = s.execute(id, p.clone());
            // rollTo takes no `step` (and a missing `at` means the end); the rest need a valid step.
            let invalid = match id {
                "design.rollTo" => p == &json!({"at": -3}) || p == &json!({"at": 999}),
                // Step 1 is one of this timeline's: only Edit Step looks at `params`.
                "design.editStep" => true,
                _ => p.get("params").is_none(),
            };
            assert!(!invalid || r.is_err(), "{id} accepted {p}");
        }
    }
    let step = step_id(&mut s, 0);
    let e = s.execute("design.editStep", json!({"step": step, "params": 5})).unwrap_err().to_string();
    assert!(e.contains("`params` must be an object"), "{e}");
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

/// The window's picker and drops start a place job (`start_place_bytes`): its step is recorded,
/// baked, when the job applies.
#[test]
fn a_file_placed_as_a_job_is_a_baked_step() {
    let mut s = started();
    let src = photocraft_doc::Document::new("p", photocraft_doc::Size::new(8, 8), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    let bytes = photocraft_io::export(&src, "p.png", &Default::default()).unwrap().bytes;
    let crate::jobs::Started::Job(id) = s.start_place_bytes("p.png", bytes, None, json!({})).unwrap() else { panic!("expected a job") };
    s.wait_job(id).unwrap();
    let st = steps(&mut s);
    let place = st.last().unwrap();
    assert_eq!((&place["command"], &place["baked"], &place["label"]), (&json!("file.placeEmbedded"), &json!(true), &json!("Place Embedded p.png")), "{st:?}");
    let before = doc(&s);
    s.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&s), before);
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

/// An action played inside another command's step (a script) mints under that step: its layer
/// doesn't reuse an id the command already minted.
#[test]
fn an_action_played_inside_a_command_shares_its_step() {
    let mut s = started();
    s.execute("actions.record", json!({"name": "New"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Recorded"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    {
        // What a script's dispatch installs around the commands it runs.
        let scope = photocraft_doc::IdScope::new(s.next_step).owned_by(s.scope_owner.0);
        let _g = scope.enter();
        s.execute("layer.new.layer", json!({"name": "Script"})).unwrap();
        s.execute("actions.play", json!({"action": "New"})).unwrap();
    }
    let ids: Vec<_> = doc(&s).walk().iter().map(|(_, _, l)| l.id).collect();
    assert_eq!(ids.len(), ids.iter().collect::<std::collections::HashSet<_>>().len(), "{ids:?}");
}

/// An edit made in another timeline document while a job runs is that edit's step only: the
/// job's step, recorded when it applies, doesn't take it in as well.
#[test]
fn a_job_records_only_its_own_changes() {
    let mut s = started();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let group = s.execute("layer.groupLayers", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap();
    s.set_active(0);
    let src = photocraft_doc::Document::new("p", photocraft_doc::Size::new(8, 8), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    let bytes = photocraft_io::export(&src, "p.png", &Default::default()).unwrap().bytes;
    let crate::jobs::Started::Job(id) = s.start_place_bytes("p.png", bytes, None, json!({})).unwrap() else { panic!("expected a job") };
    s.set_active(1);
    // Not a step of its own (view state), so a job attributing it to itself would bake the group.
    s.execute("layer.setExpanded", json!({"layer": group, "expanded": false})).unwrap();
    s.wait_job(id).unwrap();
    let commands =
        |s: &Session, i: usize| -> Vec<String> { s.documents()[i].doc.design_timeline.as_ref().unwrap().steps.iter().map(|st| st.command.clone()).collect() };
    assert_eq!(commands(&s, 1), ["layer.new.layer", "layer.groupLayers"], "no step for the job there");
    assert_eq!(commands(&s, 0), ["file.placeEmbedded"]);
}

/// Revert loads whatever the file holds now, maybe saved by another session with later steps:
/// steps made afterwards reuse none of its step numbers or layer ids.
#[test]
fn steps_after_a_revert_reuse_no_step_of_the_file() {
    let path = std::env::temp_dir().join(format!("revert-steps-{}.pcraft", std::process::id()));
    let save = |s: &Session| std::fs::write(&path, photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap()).unwrap();
    let mut a = started();
    save(&a);
    a.add_document((*a.active().unwrap().doc).clone(), Some(path.to_string_lossy().into_owned()));
    // Another session takes the file further.
    let mut b = Session::new();
    b.add_document(photocraft_format::load_from_bytes(&std::fs::read(&path).unwrap()).unwrap(), None);
    for _ in 0..3 {
        b.execute("layer.new.layer", json!({})).unwrap();
    }
    save(&b);
    a.execute("file.revert", json!({})).unwrap();
    a.execute("layer.new.layer", json!({})).unwrap();
    let _ = std::fs::remove_file(&path);
    let d = &a.active().unwrap().doc;
    let steps: Vec<u64> = d.design_timeline.as_ref().unwrap().steps.iter().map(|st| st.step).collect();
    assert!(steps.windows(2).all(|w| w[0] < w[1]), "{steps:?}");
    let ids: Vec<_> = d.walk().iter().map(|(_, _, l)| l.id).collect();
    assert_eq!(ids.len(), ids.iter().collect::<std::collections::HashSet<_>>().len(), "{ids:?}");
}

/// A mode change records the working space and conversion options it ran with, so a session with
/// other Color Settings replays the same conversion.
#[test]
fn a_mode_change_replays_with_the_colour_settings_it_ran_with() {
    let mut s = Session::new();
    s.execute("edit.colorSettings", json!({"workingRgb": "rec2020", "intent": "perceptual", "bpc": false})).unwrap();
    s.execute("file.new", json!({"width": 8, "height": 8, "mode": "gray", "depth": 8, "background": "white"})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    s.execute("image.mode.rgb", json!({})).unwrap();
    let step = s.active().unwrap().doc.design_timeline.as_ref().unwrap().steps.last().unwrap().clone();
    assert_eq!(step.params["profile"], "rec2020", "{}", step.params);
    // Not the defaults (relative, bpc on), so a replay without them would convert differently.
    assert_eq!(step.params["intent"], "perceptual", "{}", step.params);
    assert_eq!(step.params["bpc"], false, "{}", step.params);
    assert!(step.baked.is_none(), "a built-in working space replays");
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let mut r = Session::new();
    r.add_document(photocraft_format::load_from_bytes(&bytes).unwrap(), None);
    let before = doc(&r);
    r.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&r), before, "replayed with sRGB settings, still the Rec. 2020 conversion");
}

/// History without a byte limit (0, as with scratch disks on) keeps every checkpoint too.
#[test]
fn no_history_byte_limit_keeps_every_checkpoint() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    // Each fill replaces the tile: the earlier checkpoints hold pixels the document no longer has.
    for color in ["#ff0000", "#00ff00", "#0000ff"] {
        s.execute("edit.fill", json!({"color": color})).unwrap();
    }
    let st = s.active().unwrap();
    let mut tl = (*st.doc.design_timeline.clone().unwrap()).clone();
    assert!(tl.steps.iter().all(|st| st.checkpoint.0.is_some()));
    // An empty History (0 bytes: no limit), so the checkpoints alone hold the earlier fills.
    let history = photocraft_ops::History::new(50);
    crate::timeline::budget_checkpoints(&history, &st.doc, &mut tl);
    assert!(tl.steps.iter().all(|st| st.checkpoint.0.is_some()), "{:?}", tl.steps.iter().map(|st| st.checkpoint.0.is_some()).collect::<Vec<_>>());
}

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// The prelude commands of the timeline's last step in the active document.
fn last_prelude(s: &Session) -> Vec<String> {
    let tl = s.active().unwrap().doc.design_timeline.clone().unwrap();
    tl.steps.last().unwrap().prelude.iter().map(|p| p.command.clone()).collect()
}

/// Rolling the marker back and forward replays a fill with the colour set before an earlier step.
#[test]
fn a_resumed_replay_keeps_the_tool_settings_of_the_steps_before_it() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("tools.setColors", json!({"foreground": RED})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 4, "height": 4})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    let want = doc(&s);
    s.execute("tools.setColors", json!({"foreground": BLUE})).unwrap();
    s.execute("design.rollTo", json!({"at": 4})).unwrap();
    s.execute("design.rollTo", json!({"at": null})).unwrap();
    assert_eq!(state(&mut s, 4), "ok");
    assert_eq!(doc(&s), want, "the last fill replayed red");
}

/// A step in one document doesn't take the tool settings another document's next step needs.
#[test]
fn a_step_in_another_document_leaves_this_ones_tool_settings() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("paint.setSymmetry", json!({"mode": "vertical"})).unwrap();
    s.set_active(0);
    s.execute("edit.fillForeground", json!({})).unwrap();
    s.set_active(1);
    s.execute("edit.fillForeground", json!({})).unwrap();
    assert_eq!(last_prelude(&s), ["paint.setSymmetry"]);
}

/// A suppressed or deleted step's tool settings still reach the steps after it.
#[test]
fn a_suppressed_or_deleted_step_keeps_its_tool_settings() {
    for cmd in ["design.suppress", "design.delete"] {
        let mut s = started();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("tools.setColors", json!({"foreground": RED})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
        s.execute("select.all", json!({})).unwrap();
        s.execute("edit.fillForeground", json!({})).unwrap();
        let want = doc(&s);
        s.execute("tools.setColors", json!({"foreground": BLUE})).unwrap();
        let rect = step_id(&mut s, 1);
        s.execute(cmd, json!({"step": rect})).unwrap();
        assert_eq!(doc(&s), want, "{cmd}: the fill is still red");
    }
}

/// Tool settings changed while a job runs go with the next step, not the job's.
#[test]
fn a_jobs_step_takes_the_tool_settings_from_when_it_started() {
    let mut s = started();
    let src = photocraft_doc::Document::new("p", photocraft_doc::Size::new(8, 8), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    let bytes = photocraft_io::export(&src, "p.png", &Default::default()).unwrap().bytes;
    let crate::jobs::Started::Job(id) = s.start_place_bytes("p.png", bytes, None, json!({})).unwrap() else { panic!("expected a job") };
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("tools.swapColors", json!({})).unwrap();
    s.wait_job(id).unwrap();
    s.set_active(0);
    assert!(!last_prelude(&s).contains(&"tools.swapColors".to_string()), "set after the job started");
    s.execute("layer.new.layer", json!({})).unwrap();
    assert!(last_prelude(&s).contains(&"tools.swapColors".to_string()), "it goes with the next step");
}

/// Clearing a targeted layer mask records the target, so replay clears the mask again.
#[test]
fn a_step_on_the_channels_panel_target_records_it() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("channel.target", json!({"channel": "mask"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    s.execute("edit.clear", json!({})).unwrap();
    assert_eq!(steps(&mut s).last().unwrap()["params"]["target"], "mask");
    let want = doc(&s);
    s.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&s), want, "the mask was cleared, not the pixels");
}

/// Painting from a History state is baked: a replay session has no History panel.
#[test]
fn a_fill_from_history_replays_its_result() {
    let mut s = started();
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "history"})).unwrap();
    assert_eq!(steps(&mut s).last().unwrap()["baked"], true);
    let want = doc(&s);
    s.execute("design.replay", json!({})).unwrap();
    assert_eq!(state(&mut s, 2), "ok");
    assert_eq!(doc(&s), want);
}

/// A baked step keeps the document state it changed besides layers: imported data sets.
#[test]
fn a_baked_step_keeps_the_document_state_it_changed() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    let layer = s.active().unwrap().active_layer.unwrap().0;
    s.execute("image.variables.define", json!({"defs": [{"name": "show", "layer": layer, "type": "visibility"}]})).unwrap();
    let dir = std::env::temp_dir().join(format!("pc-design-sets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("sets.csv");
    std::fs::write(&csv, "DataSet,show\nrow,true\n").unwrap();
    s.execute("file.import.variableDataSets", json!({"path": csv.to_string_lossy()})).unwrap();
    assert_eq!(steps(&mut s).last().unwrap()["baked"], true, "it read a file");
    let want = doc(&s);
    assert_eq!(want.variables.data_sets.len(), 1);
    s.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&s), want);
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let mut r = Session::new();
    r.add_document(photocraft_format::load_from_bytes(&bytes).unwrap(), None);
    r.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&r), want, "after a save and load too");
}

/// Conditional Mode Change records the colour settings its conversion ran with, as a mode
/// change does, and a built-in working space by its id.
#[test]
fn a_conditional_mode_change_replays_with_the_colour_settings_it_ran_with() {
    let mut s = Session::new();
    s.execute("edit.colorSettings", json!({"workingRgb": "Adobe RGB (1998)", "intent": "perceptual", "bpc": false})).unwrap();
    s.execute("file.new", json!({"width": 8, "height": 8, "mode": "gray", "depth": 8, "background": "white"})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    s.execute("file.automate.conditionalModeChange", json!({"to": "rgb"})).unwrap();
    let step = s.active().unwrap().doc.design_timeline.as_ref().unwrap().steps.last().unwrap().clone();
    assert_eq!(step.params["profile"], "adobe-rgb-compat", "{}", step.params);
    assert_eq!((&step.params["intent"], &step.params["bpc"]), (&json!("perceptual"), &json!(false)));
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let mut r = Session::new();
    r.add_document(photocraft_format::load_from_bytes(&bytes).unwrap(), None);
    let before = doc(&r);
    r.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&r), before, "replayed with default settings, still the Adobe RGB conversion");
}

/// A step edited to name another layer acts on it even though the layer it acted on is gone.
#[test]
fn an_edited_step_may_replace_a_missing_target() {
    let mut s = started();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "B"})).unwrap();
    let b = s.active().unwrap().active_layer.unwrap().0;
    s.execute("layer.select", json!({"layer": "A"})).unwrap();
    s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
    let (new_a, props) = (step_id(&mut s, 0), step_id(&mut s, 2));
    s.execute("design.suppress", json!({"step": new_a})).unwrap();
    assert_eq!(state(&mut s, 2), "missingReference");
    s.execute("design.editStep", json!({"step": props, "params": {"layer": b, "opacity": 0.25}})).unwrap();
    assert_eq!(state(&mut s, 2), "ok");
    assert_eq!(steps(&mut s)[2]["changed"], json!([b]));
    // An edit naming no layer acts on none: the layer it ran on is still gone.
    s.execute("design.editStep", json!({"step": props, "params": {"opacity": 0.75}})).unwrap();
    assert_ne!(state(&mut s, 2), "ok");
}

/// A stroke in one document is that document's step, never a tool setting for another one.
#[test]
fn a_stroke_in_one_document_is_not_replayed_in_another() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    s.set_active(0);
    s.execute("paint.pencil", json!({"points": [[4, 4]], "size": 4, "color": "#ff0000"})).unwrap();
    s.set_active(1);
    s.execute("layer.new.layer", json!({})).unwrap();
    assert!(!last_prelude(&s).contains(&"paint.pencil".to_string()), "{:?}", last_prelude(&s));
    let want = doc(&s);
    s.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&s), want);
}

/// The painting symmetry a stroke ran with replays with it, whatever was queued before the
/// timeline started and whatever is set now.
#[test]
fn a_stroke_replays_with_the_symmetry_it_ran_with() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("paint.setSymmetry", json!({"mode": "vertical"})).unwrap();
    s.execute("tools.swapColors", json!({})).unwrap();
    s.execute("design.stop", json!({})).unwrap();
    s.execute("paint.setSymmetry", json!({"mode": "horizontal"})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    s.execute("paint.pencil", json!({"points": [[2, 3]], "size": 2, "color": "#ff0000"})).unwrap();
    assert!(!last_prelude(&s).contains(&"tools.swapColors".to_string()), "nothing queued for the old timeline");
    let want = doc(&s);
    s.execute("paint.symmetryDisable", json!({})).unwrap();
    s.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&s), want, "mirrored horizontally, as painted");
}

/// Deleting the last step leaves its tool settings for the next step recorded.
#[test]
fn deleting_the_last_step_keeps_its_tool_settings_for_the_next() {
    let mut s = started();
    s.execute("tools.swapColors", json!({})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let last = step_id(&mut s, 0);
    s.execute("design.delete", json!({"step": last})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    assert!(last_prelude(&s).contains(&"tools.swapColors".to_string()), "{:?}", last_prelude(&s));
}

/// An explicit working-space alias is recorded as the working space it resolved to.
#[test]
fn a_working_profile_alias_is_recorded_as_the_working_space() {
    let mut s = Session::new();
    s.execute("edit.colorSettings", json!({"workingRgb": "rec2020"})).unwrap();
    s.execute("file.new", json!({"width": 8, "height": 8, "mode": "gray", "depth": 8, "background": "white"})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    s.execute("file.automate.conditionalModeChange", json!({"to": "rgb", "profile": "working"})).unwrap();
    let step = s.active().unwrap().doc.design_timeline.as_ref().unwrap().steps.last().unwrap().clone();
    assert_eq!(step.params["profile"], "rec2020", "{}", step.params);
}

/// A stroke mirrored across a symmetry path keeps its result: nothing in a prelude restores the
/// path's axis after the timeline restarts or the path changes.
#[test]
fn a_stroke_mirrored_across_a_path_replays_its_result() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("design.stop", json!({})).unwrap();
    s.execute("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[8, 0], [8, 16]]}]}})).unwrap();
    s.execute("paint.symmetryFromPath", json!({"name": "work"})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    s.execute("paint.pencil", json!({"points": [[2, 3]], "size": 2, "color": "#ff0000"})).unwrap();
    assert_eq!(steps(&mut s).last().unwrap()["baked"], true);
    let want = doc(&s);
    s.execute("paint.symmetryDisable", json!({})).unwrap();
    s.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&s), want);
}

/// A step put in at a rolled-back marker doesn't change the colours the step after it ran with.
#[test]
fn a_step_inserted_before_another_leaves_it_its_colours() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    let layer = s.active().unwrap().active_layer.unwrap();
    s.execute("tools.setColors", json!({"foreground": RED})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    s.execute("design.rollTo", json!({"at": 2})).unwrap();
    s.execute("tools.setColors", json!({"foreground": BLUE})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("design.rollTo", json!({"at": null})).unwrap();
    assert_eq!(commands(&mut s)[2..], ["layer.new.layer", "edit.fillForeground"], "put in before the fill");
    assert_eq!(state(&mut s, 3), "ok");
    let fill = s.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().pixel(1, 1);
    assert_eq!(fill, RED.to_vec(), "the fill after the inserted step is still red");
}

/// A stroke in a document without a timeline is no tool setting for one that has one.
#[test]
fn a_stroke_in_a_document_without_a_timeline_is_no_tool_setting() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("paint.pencil", json!({"points": [[4, 4]], "size": 4, "color": "#ff0000"})).unwrap();
    s.set_active(0);
    s.execute("layer.new.layer", json!({})).unwrap();
    assert!(!last_prelude(&s).contains(&"paint.pencil".to_string()), "{:?}", last_prelude(&s));
}

/// A pattern rename reaches the next step of every other timeline, whether the document it ran in
/// has a timeline (it records a step there) or not: it renames the shared library too, and a
/// later step can name the pattern by its new name.
#[test]
fn a_pattern_rename_is_a_tool_setting_for_the_other_timelines() {
    for tracked in [false, true] {
        let mut s = started();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
        let pattern = s.execute("edit.definePattern", json!({"name": "Red"})).unwrap()["pattern"].as_str().unwrap().to_string();
        s.execute("layer.newFillLayer.pattern", json!({"pattern": pattern})).unwrap();
        if tracked {
            s.execute("design.start", json!({})).unwrap();
        }
        let before = s.active().unwrap().doc.clone();
        s.execute("pattern.rename", json!({"pattern": pattern, "name": "Crimson"})).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc), "it renamed the document's copy too");
        s.set_active(0);
        s.execute("edit.fill", json!({"contents": "pattern", "pattern": "Crimson"})).unwrap();
        assert!(last_prelude(&s).contains(&"pattern.rename".to_string()), "tracked {tracked}: {:?}", last_prelude(&s));
        let want = doc(&s);
        // Renamed again: the replay renames it back first, so the fill still finds it.
        s.execute("pattern.rename", json!({"pattern": pattern, "name": "Scarlet"})).unwrap();
        s.execute("design.replay", json!({})).unwrap();
        assert_eq!(state(&mut s, 1), "ok", "tracked {tracked}");
        assert_eq!(doc(&s), want, "tracked {tracked}");
    }
}

/// A timeline doesn't start while a job runs: one started before it could edit the document
/// without a step.
#[test]
fn the_timeline_does_not_start_while_a_job_runs() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let src = photocraft_doc::Document::new("p", photocraft_doc::Size::new(8, 8), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    let bytes = photocraft_io::export(&src, "p.png", &Default::default()).unwrap().bytes;
    let crate::jobs::Started::Job(id) = s.start_place_bytes("p.png", bytes, None, json!({})).unwrap() else { panic!("expected a job") };
    // Not this document's job, so the running-job check of every edit lets it by.
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let e = s.execute("design.start", json!({})).unwrap_err().to_string();
    assert!(e.contains("running"), "{e}");
    s.wait_job(id).unwrap();
    s.execute("design.start", json!({})).unwrap();
}

/// A timeline keeps the pattern versions its steps painted with: a replay paints with them after
/// the library deleted, renamed or replaced them, and after a save and load into a session
/// without them. A fill with the panel's current pattern records which one that was.
#[test]
fn a_timeline_keeps_the_pattern_versions_its_steps_used() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let define = |s: &mut Session, color: &str, name: &str| {
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
        s.execute("edit.fill", json!({"color": color})).unwrap();
        s.execute("edit.definePattern", json!({"name": name})).unwrap()["pattern"].as_str().unwrap().to_string()
    };
    let id = define(&mut s, "#ff0000", "Mine");
    let blue = define(&mut s, "#0000ff", "Blue");
    let dir = std::env::temp_dir().join(format!("pc-design-pat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    s.execute("pattern.export", json!({"path": dir.join("lib.pat").to_string_lossy()})).unwrap();
    s.set_active(0);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "pattern", "pattern": "Mine"})).unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "pattern", "pattern": id})).unwrap();
    // Re-imported with other pixels under the same id: the next fill paints those.
    let pixels = crate::pattern_cmds::resolve(&s, &blue).unwrap().surface;
    s.patterns.items.iter_mut().find(|p| p.id == id).unwrap().surface = pixels;
    s.execute("select.rect", json!({"x": 0, "y": 8, "width": 8, "height": 8})).unwrap();
    s.execute("pattern.presets.select", json!({"pattern": id})).unwrap();
    s.execute("edit.fill", json!({"contents": "pattern"})).unwrap();
    let st = steps(&mut s);
    assert_eq!([&st[2], &st[4], &st[6]].map(|st| st["params"]["pattern"].clone()), [json!(id), json!(id), json!(id)], "kept by id");
    assert!(st.iter().all(|st| st["baked"] == false), "nothing baked for it");
    let tl = s.active().unwrap().doc.design_timeline.clone().unwrap();
    assert_eq!(tl.steps.iter().map(|st| st.patterns.len()).collect::<Vec<_>>(), [0, 0, 1, 0, 0, 0, 1], "a version where it changed");
    let want = doc(&s);
    s.execute("pattern.delete", json!({"pattern": id})).unwrap();
    s.execute("design.replay", json!({})).unwrap();
    assert!([2, 4, 6].iter().all(|i| state(&mut s, *i) == "ok"));
    assert_eq!(doc(&s), want, "each fill with its own version, the pattern deleted");
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let mut r = Session::new();
    r.add_document(photocraft_format::load_from_bytes(&bytes).unwrap(), None);
    r.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&r), want, "after a save and load too");
}

/// A pattern an imported action's step names by an id this session lacks, and by `patternName`,
/// is kept by the id it resolved to.
#[test]
fn a_pattern_found_by_its_recorded_name_is_kept() {
    let mut s = started();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let id = s.execute("edit.definePattern", json!({"name": "Mine"})).unwrap()["pattern"].as_str().unwrap().to_string();
    s.execute(crate::pattern_fill_cmds::CREATE, json!({"pattern": "not-installed", "patternName": "Mine"})).unwrap();
    let step = steps(&mut s).last().unwrap().clone();
    assert_eq!(step["params"]["pattern"], json!(id));
    let tl = s.active().unwrap().doc.design_timeline.clone().unwrap();
    assert_eq!(tl.steps.last().unwrap().patterns.iter().map(|p| p.id.clone()).collect::<Vec<_>>(), [id]);
}

/// A pattern version brought in by one step reaches the steps after it however the timeline is
/// edited: a replay resuming after it, a step put in before them with another version, or the
/// step deleted.
#[test]
fn a_pattern_version_reaches_later_steps_through_timeline_edits() {
    let red_at = |s: &Session, x: i32, y: i32| s.active().unwrap().doc.layers[1].surface().unwrap().pixel(x, y) == vec![1.0, 0.0, 0.0, 1.0];
    let setup = || {
        let mut s = started();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let id = s.execute("edit.definePattern", json!({"name": "Mine"})).unwrap()["pattern"].as_str().unwrap().to_string();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        let blue = s.execute("edit.definePattern", json!({"name": "Blue"})).unwrap()["pattern"].as_str().unwrap().to_string();
        s.set_active(0);
        // 1: fill the top left red (the version comes in here), 3: the bottom right.
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
        s.execute("edit.fill", json!({"contents": "pattern", "pattern": id})).unwrap();
        s.execute("select.rect", json!({"x": 8, "y": 8, "width": 8, "height": 8})).unwrap();
        s.execute("edit.fill", json!({"contents": "pattern", "pattern": id})).unwrap();
        (s, id, blue)
    };
    // Resuming at step 3 (after the version came in), with the pattern gone from the library.
    let (mut s, id, _) = setup();
    s.execute("pattern.delete", json!({"pattern": id})).unwrap();
    s.execute("design.rollTo", json!({"at": 3})).unwrap();
    s.execute("design.rollTo", json!({"at": null})).unwrap();
    assert_eq!(state(&mut s, 4), "ok", "resumed");
    // Deleting the step that brought it in: the later fill still has it.
    let first = step_id(&mut s, 2);
    s.execute("design.delete", json!({"step": first})).unwrap();
    assert_eq!(state(&mut s, 3), "ok", "after a delete");
    assert!(red_at(&s, 12, 12));
    // A step put in before the second fill with other pixels under the same id leaves it red.
    let (mut s, id, blue) = setup();
    let pixels = crate::pattern_cmds::resolve(&s, &blue).unwrap().surface;
    s.execute("design.rollTo", json!({"at": 4})).unwrap();
    s.patterns.items.iter_mut().find(|p| p.id == id).unwrap().surface = pixels;
    s.execute("select.rect", json!({"x": 0, "y": 8, "width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "pattern", "pattern": id})).unwrap();
    s.execute("design.rollTo", json!({"at": null})).unwrap();
    assert_eq!(commands(&mut s)[4..], ["select.rect", "edit.fill", "edit.fill"]);
    // The inserted selection is the later fill's now: it paints over the inserted blue fill.
    assert!(red_at(&s, 4, 12), "the later fill keeps the version it painted with");
}

/// A replayed prelude that deletes a pattern (then imports it again, which a replay refuses)
/// doesn't take the version a later fill paints with.
#[test]
fn a_replayed_pattern_delete_leaves_the_version_a_step_paints_with() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let id = s.execute("edit.definePattern", json!({"name": "Mine"})).unwrap()["pattern"].as_str().unwrap().to_string();
    let dir = std::env::temp_dir().join(format!("pc-design-reimport-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lib.pat").to_string_lossy().into_owned();
    s.execute("pattern.export", json!({"path": path})).unwrap();
    s.set_active(0);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "pattern", "pattern": id})).unwrap();
    s.execute("pattern.delete", json!({"pattern": id})).unwrap();
    s.execute("pattern.import", json!({"path": path})).unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "pattern", "pattern": id})).unwrap();
    let want = doc(&s);
    s.execute("design.replay", json!({})).unwrap();
    assert_eq!(state(&mut s, 4), "ok");
    assert_eq!(doc(&s), want);
}

/// A step edited to paint with another pattern keeps that pattern's version: a replay after it
/// is deleted, and after a save and load, paints with it.
#[test]
fn an_edited_step_keeps_the_version_of_its_new_pattern() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let a = s.execute("edit.definePattern", json!({"name": "A"})).unwrap()["pattern"].as_str().unwrap().to_string();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("edit.definePattern", json!({"name": "B"})).unwrap();
    s.set_active(0);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "pattern", "pattern": a})).unwrap();
    let fill = step_id(&mut s, 2);
    s.execute("design.editStep", json!({"step": fill, "params": {"contents": "pattern", "pattern": "B"}})).unwrap();
    let st = steps(&mut s);
    let b = s.patterns.items.iter().find(|p| p.name == "B").unwrap().id.clone();
    assert_eq!(st[2]["params"]["pattern"], json!(b), "kept by id");
    let want = doc(&s);
    s.execute("pattern.delete", json!({"pattern": b})).unwrap();
    s.execute("design.replay", json!({})).unwrap();
    assert_eq!(state(&mut s, 2), "ok");
    assert_eq!(doc(&s), want);
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let mut r = Session::new();
    r.add_document(photocraft_format::load_from_bytes(&bytes).unwrap(), None);
    r.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&r), want, "after a save and load too");
}
