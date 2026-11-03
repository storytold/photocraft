//! The editor publishes definitions only at Save; recording still edits the real document.
use photocraft_engine::{
    Session,
    jobs::Started,
    preset_store::{self, ACTIONS_FILE, MAX_ACTIONS_BYTES, MemBackend, PresetBackend},
};
use serde_json::{Value, json};

/// A real stored library, a named caller and conflicting action/menu shortcut bindings.
fn session() -> (Session, MemBackend) {
    let store = MemBackend::default();
    let mut s = Session::new();
    assert!(s.attach_preset_store(preset_store::open(Box::new(store.clone()))).is_empty());
    s.execute("file.new", json!({"width":40,"height":30,"background":"#804020"})).unwrap();
    s.execute("actions.record", json!({"name":"Original"})).unwrap();
    s.execute("layer.renameLayer", json!({"name":"Original layer"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    for name in ["Caller", "Other"] {
        s.execute("actions.record", json!({"name":name})).unwrap();
        s.execute("actions.stop", json!({})).unwrap();
    }
    s.execute("actions.insert", json!({"action":"Caller","id":"actions.play","params":{"action":"Original"}})).unwrap();
    s.execute(
        "edit.keyboardShortcuts",
        json!({"set":{"actions.play:Original":"F6","actions.play:Other":"F7","edit.undo":"F7"},"allowUnknown":true,"removeConflicts":false}),
    )
    .unwrap();
    (s, store)
}

/// Route every mutating request through its own editor identity.
fn editor(s: &mut Session, command: &str, draft: u64, mut params: Value) -> photocraft_engine::Result<Value> {
    params["draft"] = json!(draft);
    s.execute(&format!("actions.editor.{command}"), params)
}

/// Opening is an actual registered command, not a manually installed fake library row.
fn open(s: &mut Session, params: Value) -> u64 {
    s.execute("actions.editor.open", params).unwrap()["draft"].as_u64().unwrap()
}

/// Metadata, portable recorded targets and named calls stay out of the stored library.
#[test]
fn edits_and_recording_stay_draft_until_save_or_discard() {
    let (mut s, store) = session();
    let (list, rev, prefs) = (s.actions.list.clone(), s.actions.rev, s.prefs().clone());
    let bytes = store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap();
    let before = s.active().unwrap().doc.clone();
    let draft = open(&mut s, json!({"action":"Original"}));
    editor(&mut s, "update", draft, json!({"name":"Draft name","color":"teal","shortcut":"F7"})).unwrap();
    editor(&mut s, "record", draft, json!({})).unwrap();
    let layer = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.setProps", json!({"layer":layer.0,"visible":false})).unwrap();
    s.execute("actions.play", json!({"action":"Caller"})).unwrap();
    let shown = editor(&mut s, "get", draft, json!({})).unwrap();
    assert_eq!(shown["action"]["steps"].as_array().unwrap().len(), 3);
    assert!(shown["action"]["steps"][1][1].get("_actionLayer").is_some());
    assert!(shown["action"]["steps"][1][1].get("layer").is_none());
    assert_eq!(shown["action"]["steps"][2], json!(["actions.play",{"action":"Caller"}]));
    editor(&mut s, "stop", draft, json!({})).unwrap();
    assert_eq!(s.actions.list, list);
    assert_eq!(s.actions.rev, rev);
    assert_eq!(s.prefs(), &prefs);
    assert_eq!(store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), bytes);
    let (after, history, journal) = (s.active().unwrap().doc.clone(), s.active().unwrap().history.entries(), s.journal.clone());
    assert_ne!(after.as_ref(), before.as_ref());
    editor(&mut s, "discard", draft, json!({})).unwrap();
    assert_eq!(s.actions.list, list);
    assert_eq!(s.prefs(), &prefs);
    assert_eq!(s.journal, journal);
    assert_eq!(s.active().unwrap().history.entries(), history);
    assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &after));
    assert_eq!(store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), bytes);
}

/// Save publishes flags, rename references and shortcut reassignment together; repeats do nothing.
#[test]
fn save_commits_one_definition_and_its_dependent_references() {
    let (mut s, store) = session();
    s.execute("actions.toggle", json!({"action":"Original","step":0,"enabled":false})).unwrap();
    s.execute("actions.dialog", json!({"action":"Original","step":0,"enabled":true})).unwrap();
    // Match Classic assignment: removeConflicts:false does not clear or reject alternates.
    s.edit_prefs(|prefs| {
        prefs.alternate_shortcuts.insert("actions.play:Other".into(), vec!["F7".into()]);
    });
    let draft = open(&mut s, json!({"action":"Original"}));
    let (rev, before, history, journal) = (s.actions.rev, s.active().unwrap().doc.clone(), s.active().unwrap().history.entries(), s.journal.clone());
    editor(&mut s, "update", draft, json!({"name":"Renamed","color":"purple","shortcut":"F7"})).unwrap();
    editor(&mut s, "step", draft, json!({"op":"duplicate","step":0})).unwrap();
    editor(&mut s, "step", draft, json!({"op":"move","step":1,"to":0})).unwrap();
    editor(&mut s, "step", draft, json!({"op":"replace","step":1,"id":"layer.renameLayer","params":{"name":"Later"}})).unwrap();
    editor(&mut s, "step", draft, json!({"op":"enabled","step":0,"enabled":true})).unwrap();
    let saved = editor(&mut s, "save", draft, json!({})).unwrap();
    assert_eq!(saved["dirty"], false);
    assert_eq!(s.actions.rev, rev + 1);
    assert_eq!(s.actions.list[0].name, "Renamed");
    assert_eq!(s.actions.list[0].color, "purple");
    assert_eq!(s.actions.list[0].steps.len(), 2);
    assert_eq!(s.actions.list[0].disabled.iter().copied().collect::<Vec<_>>(), [1]);
    assert_eq!(s.actions.list[0].dialogs.iter().copied().collect::<Vec<_>>(), [0, 1]);
    assert_eq!(s.actions.list[1].steps[0], ("actions.play".into(), json!({"action":"Renamed"})));
    assert!(!s.prefs().shortcuts.contains_key("actions.play:Original"));
    assert_eq!(s.prefs().shortcuts["actions.play:Renamed"], "F7");
    assert_eq!(s.prefs().shortcuts["actions.play:Other"], "");
    assert_eq!(s.prefs().shortcuts["edit.undo"], "F7");
    assert_eq!(s.prefs().alternate_shortcuts["actions.play:Other"], ["F7"]);
    assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
    assert_eq!(s.active().unwrap().history.entries(), history);
    assert_eq!(s.journal, journal);
    let (state, prefs, bytes) = (s.actions.clone(), s.prefs().clone(), store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap());
    editor(&mut s, "save", draft, json!({})).unwrap();
    assert_eq!(s.actions, state);
    assert_eq!(s.prefs(), &prefs);
    assert_eq!(store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), bytes);
}

/// Invalid edits and Save collisions retain both the draft and every committed component.
#[test]
fn malformed_requests_and_collisions_fail_without_partial_save() {
    let (mut s, store) = session();
    let draft = open(&mut s, json!({"action":"Original"}));
    let original = s.actions.clone();
    for params in [json!({"shortcut":"not a key"}), json!({"color":"@atn-color:65535"}), json!({"name":5}), json!({"futureMetadata":"not implemented"})] {
        assert!(editor(&mut s, "update", draft, params).is_err());
        assert_eq!(s.actions, original);
    }
    for params in [json!({"op":"delete","step":u64::MAX}), json!({"op":"replace","step":0,"id":5}), json!({"op":"insert","step":0,"id":"no.such.command"})] {
        assert!(editor(&mut s, "step", draft, params).is_err());
        assert_eq!(s.actions, original);
    }
    editor(&mut s, "update", draft, json!({"name":"Other","shortcut":"F7"})).unwrap();
    let (state, prefs, bytes, journal) = (s.actions.clone(), s.prefs().clone(), store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), s.journal.clone());
    assert!(editor(&mut s, "save", draft, json!({})).is_err());
    assert_eq!(s.actions, state);
    assert_eq!(s.prefs(), &prefs);
    assert_eq!(s.journal, journal);
    assert_eq!(store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), bytes);
}

/// A stale library index or shortcut namespace never retargets Save or rolls back a foreign edit.
#[test]
fn stale_save_and_discard_preserve_external_library_or_shortcut_edits() {
    for shortcut_only in [false, true] {
        let (mut s, store) = session();
        let draft = open(&mut s, json!({"action":"Original"}));
        editor(&mut s, "update", draft, json!({"name":"Draft name"})).unwrap();
        if shortcut_only {
            s.execute("edit.keyboardShortcuts", json!({"set":{"actions.play:Original":"F9"},"allowUnknown":true})).unwrap();
        } else {
            s.execute("actions.move", json!({"action":"Original","to":2})).unwrap();
        }
        let (list, rev, prefs, bytes) = (s.actions.list.clone(), s.actions.rev, s.prefs().clone(), store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap());
        assert!(editor(&mut s, "save", draft, json!({})).is_err());
        assert_eq!(editor(&mut s, "get", draft, json!({})).unwrap()["action"]["name"], "Draft name");
        editor(&mut s, "discard", draft, json!({})).unwrap();
        assert_eq!(s.actions.list, list);
        assert_eq!(s.actions.rev, rev);
        assert_eq!(s.prefs(), &prefs);
        assert_eq!(store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), bytes);
    }
}

/// Unrelated edits preserve malformed flags, opaque steps, unknown colours and old shortcuts.
#[test]
fn complete_action_clone_retains_unedited_opaque_original_data() {
    let (mut s, _) = session();
    s.actions.list[0].color = "@atn-color:65535".into();
    s.actions.list[0].steps.push(("unknown.legacy.step".into(), json!(["opaque", 9])));
    s.actions.list[0].disabled.insert(999);
    s.actions.list[0].dialogs.insert(999);
    s.execute("actions.rename", json!({"action":0,"name":"Legacy"})).unwrap();
    s.edit_prefs(|prefs| {
        prefs.shortcuts.insert("actions.play:Legacy".into(), "unparsed original binding".into());
    });
    let mut expected = s.actions.list[0].clone();
    let draft = open(&mut s, json!({"action":0}));
    editor(&mut s, "update", draft, json!({"name":"Renamed"})).unwrap();
    editor(&mut s, "save", draft, json!({})).unwrap();
    expected.name = "Renamed".into();
    assert_eq!(s.actions.list[0], expected);
    assert_eq!(s.prefs().shortcuts["actions.play:Renamed"], "unparsed original binding");
}

/// A default new draft has no library row, and stale editor handles cannot change a later one.
#[test]
fn untouched_new_save_is_noop_and_old_editor_owner_is_rejected() {
    let (mut s, store) = session();
    let list = s.actions.list.clone();
    let bytes = store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap();
    let draft = open(&mut s, json!({}));
    let state = s.actions.clone();
    assert_eq!(editor(&mut s, "save", draft, json!({})).unwrap()["dirty"], false);
    assert_eq!(s.actions, state);
    assert_eq!(s.actions.list, list);
    assert_eq!(store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), bytes);
    editor(&mut s, "record", draft, json!({})).unwrap();
    let (rev, prefs, take) = (s.actions.rev, s.prefs().clone(), s.actions.take);
    let empty = editor(&mut s, "save", draft, json!({})).unwrap();
    assert_eq!(empty["dirty"], false);
    assert_eq!(empty["recording"], false);
    assert_eq!(s.actions.list, list);
    assert_eq!(s.actions.rev, rev);
    assert_eq!(s.actions.take, take);
    assert_eq!(s.prefs(), &prefs);
    assert_eq!(store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), bytes);
    editor(&mut s, "discard", draft, json!({})).unwrap();
    let next = open(&mut s, json!({}));
    let current = s.actions.clone();
    for command in ["update", "record", "stop", "save", "discard"] {
        assert!(editor(&mut s, command, draft, json!({})).is_err());
        assert_eq!(s.actions, current);
    }
    editor(&mut s, "step", next, json!({"op":"insert","step":0,"id":"edit.fill","params":{"color":"#00ff00"}})).unwrap();
    editor(&mut s, "save", next, json!({})).unwrap();
    assert_eq!(s.actions.list.len(), list.len() + 1);
    assert_eq!(s.actions.list.last().unwrap().steps.len(), 1);
}

/// The existing immediate Classic recorder still creates its row and flushes at Stop.
#[test]
fn classic_recording_remains_immediate_without_editor_save() {
    let (mut s, store) = session();
    let len = s.actions.list.len();
    s.execute("actions.record", json!({"name":"Classic"})).unwrap();
    assert_eq!(s.actions.list.len(), len + 1);
    let native: Value = serde_json::from_slice(&store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap()).unwrap();
    assert_eq!(native["actions"].as_array().unwrap().len(), len + 1);
    assert!(s.execute("actions.editor.open", json!({})).is_err());
    s.execute("edit.fill", json!({"color":"#00ff00"})).unwrap();
    assert_eq!(photocraft_engine::actions_cmds::pending_steps(&s, len).count(), 1);
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[len].steps.len(), 1);
    assert_eq!(s.actions.list[len].steps[0].0, "edit.fill");
    assert_eq!(s.actions.list[len].steps[0].1["color"], "#00ff00");
}

/// Failed Save keeps an active take; successful Save consumes its pending steps only once.
#[test]
fn save_finishes_its_own_capture_only_after_complete_validation() {
    let (mut s, store) = session();
    let draft = open(&mut s, json!({"name":"Captured"}));
    editor(&mut s, "record", draft, json!({})).unwrap();
    s.execute("layer.renameLayer", json!({"name":"Recorded name"})).unwrap();
    editor(&mut s, "update", draft, json!({"name":"Other"})).unwrap();
    let (state, bytes) = (s.actions.clone(), store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap());
    assert!(editor(&mut s, "save", draft, json!({})).is_err());
    assert_eq!(s.actions, state);
    assert_eq!(store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap(), bytes);
    editor(&mut s, "update", draft, json!({"name":"Captured"})).unwrap();
    let (doc, history, journal) = (s.active().unwrap().doc.clone(), s.active().unwrap().history.entries(), s.journal.clone());
    let saved = editor(&mut s, "save", draft, json!({})).unwrap();
    assert_eq!(saved["recording"], false);
    assert_eq!(saved["dirty"], false);
    assert_eq!(s.actions.list.len(), 4);
    assert_eq!(s.actions.list[3].steps.len(), 1);
    assert_eq!(s.actions.list[3].steps[0].0, "layer.renameLayer");
    assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &doc));
    assert_eq!(s.active().unwrap().history.entries(), history);
    assert_eq!(s.journal, journal);
}

/// A late job from a discarded take is journaled, but never enters the next draft capture.
#[test]
fn late_job_cannot_enter_a_discarded_or_later_draft() {
    let (mut s, _) = session();
    // The existing job-control fixture: a textured surface ensures real blur work.
    s.edit("pattern", |doc, active| {
        let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let mut pixels = Vec::new();
        for y in 0..30 {
            for x in 0..40 {
                let value = ((x * 7 + y * 13) % 31) as f32 / 30.0;
                pixels.extend_from_slice(&[value, 1.0 - value, 0.5, 1.0]);
            }
        }
        surface.write_region(photocraft_geom::Rect::new(0, 0, 40, 30), &pixels);
        Ok(())
    })
    .unwrap();
    let old = open(&mut s, json!({"name":"Old draft"}));
    editor(&mut s, "record", old, json!({})).unwrap();
    let job = match s.start("filter.blur.gaussianBlur", json!({"radius":1})).unwrap() {
        Started::Job(id) => id,
        _ => panic!("blur must use its real job path"),
    };
    editor(&mut s, "discard", old, json!({})).unwrap();
    let next = open(&mut s, json!({"name":"Next draft"}));
    editor(&mut s, "record", next, json!({})).unwrap();
    s.execute("file.new", json!({"width":8,"height":8})).unwrap();
    s.wait_job(job).unwrap();
    s.execute("image.adjustments.desaturate", json!({})).unwrap();
    let value = editor(&mut s, "stop", next, json!({})).unwrap();
    let ids: Vec<_> = value["action"]["steps"].as_array().unwrap().iter().map(|step| step[0].as_str().unwrap()).collect();
    assert_eq!(ids, ["file.new", "image.adjustments.desaturate"]);
    assert_eq!(s.actions.list.len(), 3);
    assert!(s.journal.iter().any(|(id, _)| id == "filter.blur.gaussianBlur"));
}
