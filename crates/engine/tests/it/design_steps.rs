//! Design timeline steps that need the painting commands (fills, strokes) as well as the core ones.

use photocraft_engine::Session;
use serde_json::{Value, json};

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// The timeline's steps as `design.list` reports them.
fn steps(s: &mut Session) -> Vec<Value> {
    s.execute("design.list", json!({})).unwrap()["steps"].as_array().unwrap().clone()
}

/// The command id of every recorded step.
fn commands(s: &mut Session) -> Vec<String> {
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
fn every_step_has_its_own_id() {
    let mut s = script();
    let ids: Vec<u64> = steps(&mut s).iter().map(|x| x["step"].as_u64().unwrap()).collect();
    let unique: std::collections::HashSet<u64> = ids.iter().copied().collect();
    assert_eq!(unique.len(), ids.len(), "steps that mint no layer still get their own id: {ids:?}");
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

/// Deleting the selection a baked Paste Into was made in moves the paste into its place: its stored
/// result may depend on the deleted selection, so it is marked stale like the steps after a change.
#[test]
fn a_baked_step_moving_into_a_deleted_steps_place_is_stale() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 6, "height": 6})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 6, "height": 6})).unwrap();
    s.execute("edit.pasteSpecial.pasteInto", json!({})).unwrap();
    assert_eq!(commands(&mut s), ["select.rect", "edit.pasteSpecial.pasteInto"]);
    assert_eq!(steps(&mut s)[1]["baked"], true, "the clipboard is gone on replay");
    let select = step_id(&mut s, 0);
    s.execute("design.delete", json!({"step": select})).unwrap();
    assert_eq!(state(&mut s, 0), "stale");
}

/// The same with the marker between them: the paste, past the marker, is marked stale too, and
/// stays so when the marker rolls forward onto it.
#[test]
fn a_baked_step_past_the_marker_goes_stale_with_a_deletion_before_it() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 6, "height": 6})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 6, "height": 6})).unwrap();
    s.execute("edit.pasteSpecial.pasteInto", json!({})).unwrap();
    s.execute("design.rollTo", json!({"at": 1})).unwrap();
    let select = step_id(&mut s, 0);
    s.execute("design.delete", json!({"step": select})).unwrap();
    assert_eq!(state(&mut s, 0), "stale", "past the marker");
    s.execute("design.rollTo", json!({"at": null})).unwrap();
    assert_eq!(state(&mut s, 0), "stale", "and once it runs again");
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
    let pixels = photocraft_engine::pattern_cmds::resolve(&s, &blue).unwrap().surface;
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
    s.execute(photocraft_engine::pattern_fill_cmds::CREATE, json!({"pattern": "not-installed", "patternName": "Mine"})).unwrap();
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
    let pixels = photocraft_engine::pattern_cmds::resolve(&s, &blue).unwrap().surface;
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

/// A stroke painted with a pattern as its brush texture replays with that texture after the pattern
/// left the library: the texture setting finds the step's own version of it.
#[test]
fn a_brush_texture_deleted_from_the_library_still_replays() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    // Half black, half white: a texture that shows in the stroke.
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 2, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    let id = s.execute("edit.definePattern", json!({"name": "Grain"})).unwrap()["pattern"].as_str().unwrap().to_string();
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 4, "height": 4})).unwrap();
    s.execute("edit.definePattern", json!({"name": "Plain"})).unwrap();
    s.set_active(0);
    s.execute("brush.texturePattern", json!({"pattern": id})).unwrap();
    s.execute("paint.stroke", json!({"points": [[1, 8], [15, 8]], "size": 8, "color": "#00ff00"})).unwrap();
    let want = doc(&s);
    // The brush has another texture by now: a replay that can't set Grain paints with that one.
    s.execute("brush.texturePattern", json!({"pattern": "Plain"})).unwrap();
    s.execute("pattern.delete", json!({"pattern": id})).unwrap();
    s.execute("design.replay", json!({})).unwrap();
    let last = steps(&mut s).len() - 1;
    assert_eq!(state(&mut s, last), "ok");
    assert_eq!(doc(&s), want, "painted with the texture it was recorded with");
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
    assert_eq!(s.active().unwrap().doc.layers[1].surface().unwrap().pixel(1, 1), vec![0.0, 0.0, 1.0, 1.0], "the fill is B's blue now");
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

/// A suppressed step edited to paint with another pattern keeps that pattern's version for when
/// it is unsuppressed, after the library deleted it.
#[test]
fn a_suppressed_step_edited_to_another_pattern_keeps_its_version() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let a = s.execute("edit.definePattern", json!({"name": "A"})).unwrap()["pattern"].as_str().unwrap().to_string();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    let b = s.execute("edit.definePattern", json!({"name": "B"})).unwrap()["pattern"].as_str().unwrap().to_string();
    s.set_active(0);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "pattern", "pattern": a})).unwrap();
    let fill = step_id(&mut s, 2);
    s.execute("design.suppress", json!({"step": fill})).unwrap();
    s.execute("design.editStep", json!({"step": fill, "params": {"contents": "pattern", "pattern": b}})).unwrap();
    s.execute("pattern.delete", json!({"pattern": b})).unwrap();
    // Saved while it is suppressed, and opened where the library has no B.
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    s.execute("design.unsuppress", json!({"step": fill})).unwrap();
    assert_eq!(state(&mut s, 2), "ok");
    assert_eq!(s.active().unwrap().doc.layers[1].surface().unwrap().pixel(1, 1), vec![0.0, 0.0, 1.0, 1.0], "painted with B");
    let want = doc(&s);
    let mut r = Session::new();
    r.add_document(photocraft_format::load_from_bytes(&bytes).unwrap(), None);
    r.execute("design.unsuppress", json!({"step": fill})).unwrap();
    assert_eq!(state(&mut r, 2), "ok");
    assert_eq!(doc(&r), want, "after a save and load too");
}

/// A step whose own command takes a pattern from the library (here a deletion) leaves the
/// version an earlier step brought in for the steps after it.
#[test]
fn a_library_change_by_a_step_leaves_later_steps_their_versions() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let id = s.execute("edit.definePattern", json!({"name": "Mine"})).unwrap()["pattern"].as_str().unwrap().to_string();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "pattern", "pattern": id})).unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "pattern", "pattern": id})).unwrap();
    let want = doc(&s);
    // A step that deletes the pattern goes in between the fills, as editing another one into it could.
    let st = s.active_mut().unwrap();
    let mut d = (*st.doc).clone();
    let mut tl = (**d.design_timeline.as_ref().unwrap()).clone();
    let mut delete = tl.steps[5].clone();
    (delete.step, delete.command, delete.params, delete.prelude, delete.patterns) =
        (999, "pattern.delete".into(), json!({"pattern": id}), Vec::new(), Vec::new());
    (delete.created, delete.changed, delete.baked) = (Vec::new(), Vec::new(), None);
    tl.steps.insert(5, delete);
    d.design_timeline = Some(std::sync::Arc::new(tl));
    *st.doc_mut() = std::sync::Arc::new(d);
    s.execute("design.replay", json!({})).unwrap();
    assert_eq!(commands(&mut s)[4..], ["edit.fill", "pattern.delete", "select.rect", "edit.fill"]);
    assert_eq!(state(&mut s, 5), "ok", "the deletion ran");
    assert_eq!(state(&mut s, 7), "ok");
    assert_eq!(doc(&s).layers, want.layers);
}

/// Applying the selected pattern records which one: a replay doesn't take whatever the
/// library selects by then (a deleted pattern the replay put back, say).
#[test]
fn applying_the_selected_pattern_records_it() {
    let mut s = started();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let id = s.execute("edit.definePattern", json!({"name": "Mine"})).unwrap()["pattern"].as_str().unwrap().to_string();
    s.execute("pattern.presets.select", json!({"pattern": id})).unwrap();
    s.execute("pattern.presets.apply", json!({})).unwrap();
    assert_eq!(steps(&mut s).last().unwrap()["params"]["pattern"], json!(id));
}
