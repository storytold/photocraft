//! The remaining panel commands compose with the same definitions, journal and ATN boundary.
use photocraft_engine::{EngineError, Session, actions_cmds::Action, actions_edit_cmds, atn_cmds, jobs::Started};
use serde_json::{Value, json};

/// Tiny real document with one empty action keeps recording and background jobs observable.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":40,"height":30})).unwrap();
    s.actions.list.push(Action::new("A", vec![]));
    s
}

/// A painted stripe gives both the remove tool and blur a real input before recording starts.
fn stroke(s: &mut Session, color: &str) {
    s.execute("paint.stroke", json!({"points":[[8,15],[32,15]],"size":3,"color":color})).unwrap();
}

/// Mode changes flush under the previous mode without altering painting or ordinary recording.
#[test]
fn tool_recording_is_opt_in_and_toggles_do_not_reclassify_pending_steps() {
    let mut s = session();
    s.execute("actions.record", json!({"action":"A"})).unwrap();
    stroke(&mut s, "#ff0000");
    s.execute("select.all", json!({})).unwrap();
    let original = s.actions.clone();
    assert!(s.execute("actions.allowToolRecording", json!({"enabled":"yes"})).is_err());
    assert_eq!(s.actions, original, "invalid mode must not flush pending work");
    s.execute("actions.allowToolRecording", json!({"enabled":true})).unwrap();
    stroke(&mut s, "#00ff00");
    s.execute("actions.allowToolRecording", json!({"enabled":false})).unwrap();
    stroke(&mut s, "#0000ff");
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[0].steps.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["select.all", "paint.stroke", "select.deselect"]);
    assert_eq!(s.actions.list[0].steps[1].1["color"], "#00ff00");
    assert_eq!(s.journal.iter().filter(|s| s.0 == "paint.stroke").count(), 3, "all actual painting remains journaled");
    assert_eq!(s.execute("actions.list", json!({})).unwrap()["toolRecording"], false);
}

/// A stroke started while off never joins a later enabled recording; ordinary jobs keep their owner.
#[test]
fn background_stroke_ownership_does_not_suppress_ordinary_filter_jobs() {
    for command in ["paint.remove", "filter.blur.gaussianBlur"] {
        let mut s = session();
        stroke(&mut s, "#bb3300");
        s.execute("actions.record", json!({"action":"A"})).unwrap();
        let params = if command == "paint.remove" { json!({"points":[[20,15],[21,15]],"size":3}) } else { json!({"radius":1}) };
        let Started::Job(job) = s.start(command, params).unwrap() else { panic!("expected the real background job: {command}") };
        s.execute("actions.allowToolRecording", json!({"enabled":true})).unwrap();
        s.wait_job(job).unwrap();
        s.execute("actions.stop", json!({})).unwrap();
        let ids = s.actions.list[0].steps.iter().map(|s| s.0.as_str()).collect::<Vec<_>>();
        if command == "paint.remove" {
            assert!(ids.is_empty());
        } else {
            assert_eq!(ids, [command]);
        }
    }
}

/// Interleaved membership and numeric calls expose incorrect folder placement or target rewriting.
#[test]
fn set_duplication_preserves_metadata_call_targets_and_empty_folder_order() {
    let mut s = session();
    s.actions.list.clear();
    for (name, set) in [("A", "S"), ("Other", "T"), ("B", "S")] {
        let mut a = Action::new(name, vec![("actions.play".into(), json!({"action":2}))]);
        a.set = Some(set.into());
        a.color = "red".into();
        a.disabled.insert(0);
        a.dialogs.insert(0);
        s.actions.list.push(a);
    }
    s.prefs.edit(|p| {
        p.shortcuts.insert("actions.play:A".into(), "F5".into());
    });
    let result = s.execute("actions.duplicate", json!({"set":"S"})).unwrap();
    assert_eq!(result["set"], "S copy");
    assert_eq!(actions_edit_cmds::set_names(&s), ["S", "S copy", "T"]);
    assert_eq!(s.actions.list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["A", "A copy", "B copy", "Other", "B"]);
    for a in &s.actions.list {
        assert_eq!(a.steps[0].1["action"], 4);
        assert_eq!(a.color, "red");
        assert!(a.disabled.contains(&0));
        assert!(a.dialogs.contains(&0));
    }
    assert!(!s.prefs().shortcuts.contains_key("actions.play:A copy"));
    let mut empty = Session::new();
    for name in ["E", "F"] {
        empty.execute("actions.newSet", json!({"name":name})).unwrap();
    }
    empty.execute("actions.duplicate", json!({"set":"E"})).unwrap();
    assert_eq!(actions_edit_cmds::set_names(&empty), ["E", "E copy", "F"]);
    let original = empty.actions.clone();
    for params in [json!({"set":"Missing"}), json!({"set":"E","action":0}), json!({"set":false})] {
        assert!(empty.execute("actions.duplicate", params).is_err());
        assert_eq!(empty.actions, original);
    }
}

/// Menu insertion keeps descriptor absence and kind; empty descriptors and ambiguous commands differ.
#[test]
fn inserted_menu_events_roundtrip_absent_settings_and_stateless_items_execute() {
    assert!(!atn_cmds::menu_item_needs_settings("filter.noise.despeckle"));
    assert!(atn_cmds::menu_item_needs_settings("filter.blur.gaussianBlur"));
    for id in ["image.imageSize", "filter.blur.gaussianBlur", "filter.noise.despeckle"] {
        for kind in ["name", "code"] {
            let mut s = session();
            s.execute("actions.insertMenuItem", json!({"action":"A","id":id,"eventKind":kind})).unwrap();
            let bytes = atn_cmds::export("S", &[&s.actions.list[0]], &[], false).unwrap().0;
            let raw = photocraft_psd::atn::parse(&bytes).unwrap();
            assert!(raw.actions[0].items[0].descriptor.is_none());
            let imported = atn_cmds::import(&bytes).unwrap();
            let (command, mut params) = imported.actions[0].steps[0].clone();
            assert!(params.as_object_mut().unwrap().remove("_atnEvent").is_some(), "the imported step retains its save-only source metadata");
            assert_eq!((command, params), (atn_cmds::MENU_ITEM.into(), json!({"id":id,"eventKind":kind})));
            let exported = atn_cmds::export("S", &[&imported.actions[0]], &[], false).unwrap().0;
            assert_eq!(photocraft_psd::atn::parse(&exported).unwrap().actions[0].items, raw.actions[0].items);
            let played = s.execute("actions.play", json!({"action":"A"})).unwrap();
            assert_eq!(played.get("failed").is_none(), !atn_cmds::menu_item_needs_settings(id), "{id}: {played}");
        }
    }
    let mut s = session();
    let original = s.actions.clone();
    for params in
        [json!({"action":"A","id":"layer.setProps"}), json!({"action":"A","id":"file.open"}), json!({"action":"A","id":"image.imageSize","params":{"width":4}})]
    {
        assert!(s.execute("actions.insertMenuItem", params).is_err());
        assert_eq!(s.actions, original);
    }
    let mut raw = photocraft_psd::atn::AtnFile::default();
    raw.actions.push(photocraft_psd::atn::AtnAction {
        items: vec![photocraft_psd::atn::AtnItem {
            event: photocraft_psd::atn::AtnEvent::name("gaussianBlur"),
            descriptor: Some(photocraft_psd::descriptor::Descriptor::new("null")),
            ..Default::default()
        }],
        ..Default::default()
    });
    let loaded = atn_cmds::import(&photocraft_psd::atn::write(&raw).unwrap()).unwrap();
    assert_eq!(loaded.actions[0].steps[0].0, "filter.blur.gaussianBlur");
}

/// A full curve is snapshotted independently of later source edits and replays as the work path.
#[test]
fn insert_path_captures_cubic_geometry_without_editing_the_document() {
    let mut s = session();
    let path = json!({"fillRule":"evenodd","inverted":true,"subpaths":[{"closed":false,"op":"combine","knots":[{"anchor":[1.5,2.25],"in":[-1.0,3.0],"out":[5.0,8.0],"smooth":true},{"anchor":[20.0,18.0],"in":[12.0,5.0],"out":[21.0,19.0]}]}]});
    s.execute("path.set", json!({"name":"Curve","path":path})).unwrap();
    let expected = s.execute("path.info", json!({"name":"Curve"})).unwrap()["path"].clone();
    let revision = s.active().unwrap().revision;
    assert!(s.execute("actions.insertPath", json!({"action":"A","name":"work"})).is_err());
    assert!(s.actions.list[0].steps.is_empty());
    s.execute("actions.insertPath", json!({"action":"A","name":"Curve"})).unwrap();
    assert_eq!(s.active().unwrap().revision, revision);
    assert_eq!(s.actions.list[0].steps[0].1["path"], expected);
    s.execute("path.set", json!({"name":"Curve","path":{"subpaths":[]}})).unwrap();
    let played = s.execute("actions.play", json!({"action":"A"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(s.execute("path.info", json!({"name":"work"})).unwrap()["path"], expected);
    let (bytes, skipped) = atn_cmds::export("S", &[&s.actions.list[0]], &[], false).unwrap();
    assert!(skipped.is_empty());
    let wire = photocraft_psd::atn::parse(&bytes).unwrap();
    assert_eq!(
        wire.actions[0].items[0].event,
        photocraft_psd::atn::AtnEvent::name("photocraftCommand"),
        "unmeasured inverted geometry uses the canonical native carrier, not a guessed Photoshop descriptor"
    );
    let back = atn_cmds::import(&bytes).unwrap();
    assert_eq!(back.actions[0].steps, s.actions.list[0].steps, "the full native path value must survive");
}

/// Reject the actual Reset operation before it can replace a canonical definition.
fn deny_reset(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id == "actions.reset" { Err(EngineError::Other("reset denied".into())) } else { Ok(()) }
}

/// Real starter definitions are installed only after consent and all destructive admission checks.
#[test]
fn reset_defaults_append_or_replace_only_after_valid_consent() {
    let mut s = session();
    let original = s.actions.clone();
    for p in [json!({}), json!({"mode":"replace"}), json!({"mode":"wrong","confirm":true}), json!({"mode":"append","confirm":false})] {
        assert!(s.execute("actions.reset", p).is_err());
        assert_eq!(s.actions, original);
    }
    s.authorize = Some(deny_reset);
    assert!(s.execute("actions.reset", json!({"mode":"replace","confirm":true})).is_err());
    assert_eq!(s.actions, original);
    s.authorize = None;
    let result = s.execute("actions.reset", json!({"mode":"append","confirm":true})).unwrap();
    assert_eq!(result["actions"], 2);
    assert_eq!(&s.actions.list[..original.list.len()], original.list.as_slice());
    assert_eq!(s.actions.list.len(), original.list.len() + 2);
    s.execute("actions.reset", json!({"mode":"replace","confirm":true})).unwrap();
    assert_eq!(s.actions.list.len(), 2);
    assert_eq!(s.actions.sets, [actions_edit_cmds::DEFAULT_SET_NAME]);
    for (action, command) in s.actions.list.iter().zip(["image.adjustments.invert", "image.adjustments.desaturate"]) {
        assert_eq!(action.set.as_deref(), Some(actions_edit_cmds::DEFAULT_SET_NAME));
        assert_eq!(action.steps, [(command.into(), json!({}))]);
    }
}

/// Edit Undo consumes only the latest panel Reset; physical-folder Save does not merge captions.
#[test]
fn reset_has_one_panel_undo_and_saves_the_selected_physical_occurrence() {
    let mut s = session();
    let original = s.actions.list.clone();
    let document = s.active().unwrap().doc.clone();
    let revision = s.active().unwrap().revision;
    s.execute("actions.reset", json!({"mode":"append","confirm":true})).unwrap();
    let result = s.execute("actions.reset", json!({"mode":"append","confirm":true})).unwrap();
    let folders = actions_edit_cmds::set_occurrences(&s).unwrap();
    let defaults: Vec<_> = folders.iter().filter(|row| row.name == actions_edit_cmds::DEFAULT_SET_NAME).collect();
    assert_eq!(defaults.len(), 2);
    assert_ne!(defaults[0].origin, defaults[1].origin);
    assert!(defaults.iter().all(|row| row.actions.len() == 2));
    let saved = s.execute("actions.save", json!({"set":actions_edit_cmds::DEFAULT_SET_NAME,"setSlot":result["slot"]})).unwrap();
    assert_eq!(saved["actions"].as_array().unwrap().len(), 2);
    let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
    assert_eq!(atn_cmds::import(&bytes).unwrap().actions.len(), 2);
    assert!(s.execute("actions.save", json!({"setSlot":999999})).is_err());
    assert!(s.execute("actions.save", json!({"setSlot":result["slot"],"set":"Wrong"})).is_err());
    assert_eq!(s.execute("edit.undo", json!({})).unwrap(), json!(true));
    assert_eq!(s.actions.list.len(), original.len() + 2);
    assert_eq!(actions_edit_cmds::set_occurrences(&s).unwrap().iter().filter(|row| row.name == actions_edit_cmds::DEFAULT_SET_NAME).count(), 1);
    assert!(!actions_edit_cmds::reset_undo_available(&s));
    assert!(s.execute("actions.undoReset", json!({})).is_err());
    assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &document));
    assert_eq!(s.active().unwrap().revision, revision);
}

/// Classic and Draft use one tool opt-in while retaining the canonical spelling projection.
#[test]
fn tool_recording_opt_in_has_the_same_journal_policy_in_both_recorders() {
    for draft in [false, true] {
        let mut s = session();
        s.execute("type.create", json!({"x":1,"y":8,"text":"Teh end","size":8})).unwrap();
        s.execute("layer.new.layer", json!({"name":"Paint"})).unwrap();
        let id = if draft {
            let id = s.execute("actions.editor.open", json!({"name":"Draft"})).unwrap()["draft"].as_u64().unwrap();
            s.execute("actions.editor.record", json!({"draft":id})).unwrap();
            Some(id)
        } else {
            s.execute("actions.record", json!({"action":"A"})).unwrap();
            None
        };
        s.execute("select.all", json!({})).unwrap();
        stroke(&mut s, "#ff0000");
        s.execute("actions.allowToolRecording", json!({"enabled":true})).unwrap();
        stroke(&mut s, "#00ff00");
        s.execute("actions.allowToolRecording", json!({"enabled":false})).unwrap();
        stroke(&mut s, "#0000ff");
        s.execute("edit.checkSpelling", json!({"action":"suggest","word":"recieve"})).unwrap();
        s.execute("edit.checkSpelling", json!({"allLayers":true})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let steps = if let Some(id) = id {
            s.execute("actions.editor.stop", json!({"draft":id})).unwrap();
            s.execute("actions.editor.get", json!({"draft":id})).unwrap()["action"]["steps"].clone()
        } else {
            s.execute("actions.stop", json!({})).unwrap();
            serde_json::to_value(&s.actions.list[0].steps).unwrap()
        };
        let ids: Vec<_> = steps.as_array().unwrap().iter().map(|step| step[0].as_str().unwrap()).collect();
        assert_eq!(ids, ["select.all", "paint.stroke", "edit.checkSpelling", "select.deselect"], "Draft={draft}");
        assert_eq!(s.journal.iter().filter(|(id, _)| id == "paint.stroke").count(), 3);
    }
}

fn physical_session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":6})).unwrap();
    for _ in 0..2 {
        s.execute("actions.reset", json!({"mode":"append","confirm":true})).unwrap();
    }
    s
}
fn physical_counts(s: &Session) -> Vec<usize> {
    actions_edit_cmds::set_occurrences(s).unwrap().iter().map(|row| row.actions.len()).collect()
}

/// Registered recorder/new-draft adapters retain the chosen physical occurrence through Save.
#[test]
fn physical_record_and_new_draft_save_keep_destinations_and_runtime_counters() {
    let mut s = physical_session();
    s.actions.playback_nonce = 73;
    s.actions.playback_steps = 19;
    let recorded = s.execute("actions.record", json!({"name":"Earlier recorder","setSlot":0,"set":actions_edit_cmds::DEFAULT_SET_NAME})).unwrap();
    assert_eq!(recorded["index"], 2);
    s.execute("image.adjustments.invert", json!({})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(physical_counts(&s), [3, 2]);
    let id = s.execute("actions.editor.open", json!({"name":"Second draft","setSlot":1})).unwrap()["draft"].as_u64().unwrap();
    let take = s.actions.take;
    s.execute("actions.editor.update", json!({"draft":id,"description":"Published only by Save"})).unwrap();
    let saved = s.execute("actions.editor.save", json!({"draft":id})).unwrap();
    assert_eq!(saved["source"], 5);
    assert_eq!(saved["dirty"], false);
    assert_eq!(physical_counts(&s), [3, 3]);
    assert_eq!(s.actions.take, take);
    assert_eq!((s.actions.playback_nonce, s.actions.playback_steps), (73, 19));
    let revision = s.actions.rev;
    s.execute("actions.editor.save", json!({"draft":id})).unwrap();
    assert_eq!(s.actions.rev, revision, "unchanged Save is still a no-op");
    s.execute("actions.editor.discard", json!({"draft":id})).unwrap();
    let before = s.actions.list.len();
    let id = s.execute("actions.editor.open", json!({"name":"Untouched new draft","setSlot":0})).unwrap()["draft"].as_u64().unwrap();
    s.execute("actions.editor.save", json!({"draft":id})).unwrap();
    assert_eq!(s.actions.list.len(), before, "unchanged New Save keeps C's no-publication contract");
}

/// A placement-only move changes revision despite logical equality; occurrence deletion owns both maps.
#[test]
fn physical_assign_duplicate_delete_preserve_namesakes_and_both_binding_maps() {
    let mut s = physical_session();
    let index = s.execute("actions.record", json!({"name":"Move me","setSlot":1})).unwrap()["index"].as_u64().unwrap() as usize;
    s.execute("select.all", json!({})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    s.prefs.edit(|prefs| {
        prefs.shortcuts.insert("actions.play:Move me".into(), "F4".into());
        prefs.alternate_shortcuts.insert("actions.play:Move me".into(), vec!["F5".into()]);
        prefs.shortcuts.insert("actions.play:Invert Colors".into(), "F6".into());
    });
    let old = s.actions.list[index].clone();
    let revision = s.actions.rev;
    let moved = s.execute("actions.assignSet", json!({"action":index,"setSlot":0})).unwrap();
    let moved_index = moved["index"].as_u64().unwrap() as usize;
    assert_eq!(s.actions.list[moved_index], old, "ordinary logical equality remains unchanged");
    assert!(!photocraft_engine::actions_cmds::library::same_placement(&s.actions.list[moved_index], &old));
    assert!(s.actions.rev > revision);
    assert_eq!(physical_counts(&s), [3, 2]);
    let revision = s.actions.rev;
    s.execute("actions.assignSet", json!({"action":moved_index,"setSlot":0})).unwrap();
    assert_eq!(s.actions.rev, revision, "same actual occurrence is a no-op");
    let document = s.active().unwrap().doc.clone();
    let take = s.actions.take;
    let copied = s.execute("actions.duplicate", json!({"setSlot":1})).unwrap();
    assert_eq!(copied["actions"].as_array().unwrap().len(), 2);
    assert_eq!(physical_counts(&s), [3, 2, 2]);
    s.execute("actions.delete", json!({"setSlot":0})).unwrap();
    assert_eq!(physical_counts(&s), [2, 2]);
    assert!(!s.prefs().shortcuts.contains_key("actions.play:Move me"));
    assert!(!s.prefs().alternate_shortcuts.contains_key("actions.play:Move me"));
    assert!(s.prefs().shortcuts.contains_key("actions.play:Invert Colors"), "the other namesake still owns this name");
    assert!(std::sync::Arc::ptr_eq(&document, &s.active().unwrap().doc));
    assert_eq!(s.actions.take, take);
    // Set-only remains a logical group operation, including actions carrying explicit placement.
    s.execute("actions.duplicate", json!({"set":copied["set"]})).unwrap();
}

/// Invalid selectors and stale Saves never flush or consume the active Classic/draft capture.
#[test]
fn physical_adapter_refusals_preserve_capture_and_stale_draft() {
    let mut s = physical_session();
    s.execute("actions.record", json!({"action":0})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    let cursor = s.actions.recording;
    let take = s.actions.take;
    let definitions = s.actions.list.clone();
    for params in [json!({"setSlot":99}), json!({"setSlot":null}), json!({"setSlot":0,"set":null}), json!({"setSlot":0,"action":0})] {
        assert!(s.execute("actions.record", params.clone()).is_err());
        assert!(s.execute("actions.editor.open", params.clone()).is_err());
        assert!(s.execute("actions.save", params).is_err());
        assert_eq!(s.actions.recording, cursor);
        assert_eq!(s.actions.take, take);
        assert_eq!(s.actions.list, definitions);
    }
    assert!(s.execute("actions.assignSet", json!({"action":0,"setSlot":1})).is_err());
    assert!(s.execute("actions.delete", json!({"setSlot":1})).is_err());
    s.execute("actions.stop", json!({})).unwrap();
    let id = s.execute("actions.editor.open", json!({"name":"Held destination","setSlot":1})).unwrap()["draft"].as_u64().unwrap();
    s.execute("actions.editor.update", json!({"draft":id,"description":"Still unpublished"})).unwrap();
    let draft = s.actions.editor.clone();
    let take = s.actions.take;
    s.execute("actions.newSet", json!({"name":"External change"})).unwrap();
    let definitions = s.actions.list.clone();
    assert!(s.execute("actions.editor.save", json!({"draft":id})).is_err());
    assert_eq!(s.actions.editor, draft);
    assert_eq!(s.actions.take, take);
    assert_eq!(s.actions.list, definitions);
}

/// Direct list reorder follows numeric Conditional branches as well as Play.
#[test]
fn list_reorder_remaps_both_call_forms_without_traversing_foreign_json() {
    let mut s = physical_session();
    let foreign = json!({"actions":[{"steps":[["actions.play",{"action":0}]]}]});
    s.actions.list[1].steps.extend([
        ("actions.play".into(), json!({"action":0})),
        ("actions.conditional".into(), json!({"if":"always","then":0,"else":1})),
        ("file.automate.batch".into(), foreign.clone()),
    ]);
    s.execute("actions.move", json!({"action":0,"to":1})).unwrap();
    let steps = &s.actions.list[0].steps;
    assert_eq!(steps[1].1, json!({"action":1}));
    assert_eq!(steps[2].1, json!({"if":"always","then":1,"else":0}));
    assert_eq!(steps[3].1, foreign);
}
