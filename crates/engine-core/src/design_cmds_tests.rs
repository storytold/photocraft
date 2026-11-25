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
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    s
}

#[test]
fn records_document_changes_only() {
    let mut s = crate::session();
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
fn starting_or_stopping_the_timeline_modifies_a_clean_document() {
    let mut s = crate::session();
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
    *st.doc_mut() = std::sync::Arc::new(doc);
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

/// A job that starts from a floating selection drops it first; cancelled, the job leaves the drop
/// in the document, so the drop is a step of its own and the timeline stays editable.
#[test]
fn a_cancelled_job_records_the_floating_selection_it_dropped() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 80, "height": 60, "background": "transparent"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("paint", |doc, _| {
        doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(10, 10, 30, 30), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("design.start", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    s.execute("select.float", json!({"dx": 15, "dy": 0})).unwrap();
    let crate::jobs::Started::Job(job) = s.start("filter.blur.gaussianBlur", json!({"radius": 4})).unwrap() else { panic!("a filter runs as a job") };
    assert!(s.cancel_job(job));
    assert!(s.wait_job(job).is_err());
    assert_eq!(commands(&mut s), ["select.rect", "select.drop"]);
    s.execute("design.rollTo", json!({"at": 1})).unwrap();
    s.execute("design.rollTo", json!({"at": null})).unwrap();
    assert_eq!(state(&mut s, 1), "ok");
}

/// Collapsing a group in the Layers panel is no step and doesn't stop the timeline changing; the
/// group stays collapsed through the replay.
#[test]
fn collapsing_a_group_leaves_the_timeline_editable() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    let group = s.execute("layer.groupLayers", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.setExpanded", json!({"layer": group, "expanded": false})).unwrap();
    s.execute("design.rollTo", json!({"at": 2})).unwrap();
    s.execute("design.rollTo", json!({"at": null})).unwrap();
    let expanded = |s: &Session| match &s.active().unwrap().doc.layer(photocraft_doc::LayerId(group)).unwrap().content {
        photocraft_doc::LayerContent::Group(g) => g.expanded,
        _ => panic!("not a group"),
    };
    assert!(!expanded(&s), "still collapsed");
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
    let mut b = crate::session();
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
    let mut s = crate::session();
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
    let mut r = crate::session();
    r.add_document(photocraft_format::load_from_bytes(&bytes).unwrap(), None);
    let before = doc(&r);
    r.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&r), before, "replayed with sRGB settings, still the Rec. 2020 conversion");
}

/// History without a byte limit (0, as with scratch disks on) keeps every checkpoint too.
#[test]
fn no_history_byte_limit_keeps_every_checkpoint() {
    let mut s = started();
    // Each invert of the white background replaces the tile: the earlier checkpoints hold pixels
    // the document no longer has.
    for _ in 0..3 {
        s.execute("image.adjustments.invert", json!({})).unwrap();
    }
    let st = s.active().unwrap();
    let mut tl = (*st.doc.design_timeline.clone().unwrap()).clone();
    assert!(tl.steps.iter().all(|st| st.checkpoint.0.is_some()));
    // An empty History (0 bytes: no limit), so the checkpoints alone hold the earlier fills.
    let history = photocraft_ops::History::new(50);
    crate::timeline::budget_checkpoints(&history, &st.doc, &mut tl);
    assert!(tl.steps.iter().all(|st| st.checkpoint.0.is_some()), "{:?}", tl.steps.iter().map(|st| st.checkpoint.0.is_some()).collect::<Vec<_>>());
}

/// The prelude commands of the timeline's last step in the active document.
fn last_prelude(s: &Session) -> Vec<String> {
    let tl = s.active().unwrap().doc.design_timeline.clone().unwrap();
    tl.steps.last().unwrap().prelude.iter().map(|p| p.command.clone()).collect()
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
    let mut r = crate::session();
    r.add_document(photocraft_format::load_from_bytes(&bytes).unwrap(), None);
    r.execute("design.replay", json!({})).unwrap();
    assert_eq!(doc(&r), want, "after a save and load too");
}

/// Conditional Mode Change records the colour settings its conversion ran with, as a mode
/// change does, and a built-in working space by its id.
#[test]
fn a_conditional_mode_change_replays_with_the_colour_settings_it_ran_with() {
    let mut s = crate::session();
    s.execute("edit.colorSettings", json!({"workingRgb": "Adobe RGB (1998)", "intent": "perceptual", "bpc": false})).unwrap();
    s.execute("file.new", json!({"width": 8, "height": 8, "mode": "gray", "depth": 8, "background": "white"})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    s.execute("file.automate.conditionalModeChange", json!({"to": "rgb"})).unwrap();
    let step = s.active().unwrap().doc.design_timeline.as_ref().unwrap().steps.last().unwrap().clone();
    assert_eq!(step.params["profile"], "adobe-rgb-compat", "{}", step.params);
    assert_eq!((&step.params["intent"], &step.params["bpc"]), (&json!("perceptual"), &json!(false)));
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let mut r = crate::session();
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

/// A delete of the last step that is cancelled keeps the step's tool settings with it only: the next
/// step recorded doesn't take them too.
#[test]
fn a_cancelled_delete_of_the_last_step_hands_over_no_tool_settings() {
    let mut s = started();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("tools.swapColors", json!({})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    // No checkpoints (as in a reopened file): the delete replays as a job.
    let st = s.active_mut().unwrap();
    let mut d = (*st.doc).clone();
    let mut tl = (*d.design_timeline.clone().unwrap()).clone();
    for step in &mut tl.steps {
        step.checkpoint = photocraft_doc::design::Checkpoint(None);
    }
    d.design_timeline = Some(std::sync::Arc::new(tl));
    *st.doc_mut() = std::sync::Arc::new(d);
    let last = step_id(&mut s, 1);
    let crate::jobs::Started::Job(id) = s.start("design.delete", json!({"step": last})).unwrap() else { panic!("a replay is a job") };
    assert!(s.cancel_job(id));
    assert!(s.wait_job(id).is_err());
    assert_eq!(steps(&mut s).len(), 2, "the step stays");
    s.execute("layer.new.layer", json!({})).unwrap();
    assert!(!last_prelude(&s).contains(&"tools.swapColors".to_string()), "{:?}", last_prelude(&s));
}

/// An explicit working-space alias is recorded as the working space it resolved to.
#[test]
fn a_working_profile_alias_is_recorded_as_the_working_space() {
    let mut s = crate::session();
    s.execute("edit.colorSettings", json!({"workingRgb": "rec2020"})).unwrap();
    s.execute("file.new", json!({"width": 8, "height": 8, "mode": "gray", "depth": 8, "background": "white"})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    s.execute("file.automate.conditionalModeChange", json!({"to": "rgb", "profile": "working"})).unwrap();
    let step = s.active().unwrap().doc.design_timeline.as_ref().unwrap().steps.last().unwrap().clone();
    assert_eq!(step.params["profile"], "rec2020", "{}", step.params);
}

/// A working space that no longer resolves (its .icc file gone) falls back to the built-in default;
/// the step records that default, so replay doesn't follow later Color Settings.
#[test]
fn an_unresolved_working_profile_records_the_built_in_fallback() {
    use crate::cmd::color_cmds::ColorExt as _;
    let mut s = crate::session();
    s.color_mut().settings.working_rgb = "/no/such/profile.icc".into();
    s.execute("file.new", json!({"width": 8, "height": 8, "mode": "gray", "depth": 8, "background": "white"})).unwrap();
    s.execute("design.start", json!({})).unwrap();
    s.execute("file.automate.conditionalModeChange", json!({"to": "rgb", "profile": "working"})).unwrap();
    let step = s.active().unwrap().doc.design_timeline.as_ref().unwrap().steps.last().unwrap().clone();
    assert_eq!(step.params["profile"], craft_cms::Builtin::Srgb.id(), "{}", step.params);
}

/// A timeline doesn't start while a job runs: one started before it could edit the document
/// without a step.
#[test]
fn the_timeline_does_not_start_while_a_job_runs() {
    let mut s = crate::session();
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

/// A pattern the library holds as a replay installed it counts as held, renamed too; one deleted
/// and added again under its id with other pixels doesn't, though the library's size and pixel
/// revision are as they were.
#[test]
fn a_pattern_replaced_under_its_id_is_not_held() {
    let mut s = started();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    // Black from the white background, then white again: two patterns of other pixels.
    s.execute("image.adjustments.invert", json!({})).unwrap();
    let a = s.execute("edit.definePattern", json!({"name": "A"})).unwrap()["pattern"].as_str().unwrap().to_string();
    s.execute("image.adjustments.invert", json!({})).unwrap();
    let b = s.execute("edit.definePattern", json!({"name": "B"})).unwrap()["pattern"].as_str().unwrap().to_string();
    let held = vec![crate::pattern_cmds::resolve(&s, &a).unwrap()];
    let i = s.patterns.items.iter().position(|p| p.id == a).unwrap();
    s.patterns.items[i] = held[0].clone();
    assert!(crate::timeline::holds(&s, &held));
    s.patterns.items[i].name = "Renamed".into();
    assert!(crate::timeline::holds(&s, &held), "a rename keeps it");
    let (len, rev) = (s.patterns.items.len(), s.patterns.pixels_rev);
    s.patterns.items[i].surface = crate::pattern_cmds::resolve(&s, &b).unwrap().surface;
    assert_eq!((s.patterns.items.len(), s.patterns.pixels_rev), (len, rev));
    assert!(!crate::timeline::holds(&s, &held));
    // A saved pattern held as a preview isn't the installed one either.
    s.patterns.items[i] = held[0].clone();
    s.patterns.stored.insert(a.clone(), (4, 4));
    assert!(!crate::timeline::holds(&s, &held), "stored");
    // Nor is one whose second tile changed.
    s.execute("file.new", json!({"width": 512, "height": 8})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 300, "height": 4})).unwrap();
    let wide = s.execute("edit.definePattern", json!({"name": "Wide"})).unwrap()["pattern"].as_str().unwrap().to_string();
    let held = vec![crate::pattern_cmds::resolve(&s, &wide).unwrap()];
    assert!(held[0].surface.tiles().count() > 1, "spans tiles");
    let i = s.patterns.items.iter().position(|p| p.id == wide).unwrap();
    s.patterns.items[i] = held[0].clone();
    assert!(crate::timeline::holds(&s, &held));
    let surface = &mut s.patterns.items[i].surface;
    let blue = [0.0, 0.0, 1.0, 1.0];
    let n = surface.format().channels();
    surface.fill_rect(photocraft_geom::Rect::from_xywh(290, 0, 4, 4), &blue[..n]);
    assert!(!crate::timeline::holds(&s, &held), "its last tile");
}
