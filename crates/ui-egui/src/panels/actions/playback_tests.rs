//! #5212: Playback Options. Step by Step and Pause For play the whole action from one Play, with a
//! frame (and the wait) between steps, in one history state when Create a single history state is
//! on; Esc and the Stop button end them keeping the steps applied. Measured in the reference
//! application 2026-11-05.
use super::*;
use web_time::{Duration, Instant};

fn names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.walk().iter().map(|(_, _, l)| l.name.clone()).filter(|n| n != "Background").collect()
}

/// `Three` makes layers A, B and C.
fn app_with(performance: &str, seconds: u8, single: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 6})).unwrap();
    app.run("actions.setPlayback", json!({"performance": performance, "seconds": seconds, "singleState": single})).unwrap();
    let steps = ["A", "B", "C"].map(|n| ("layer.new.layer".to_string(), json!({"name": n}))).to_vec();
    app.session.actions.list.push(Action::new("Three", steps));
    app
}

/// One frame of the app's loop as far as pacing goes, with `events` as its input.
fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput { events, ..Default::default() };
    ctx.run_ui(raw, |ui| playback::tick(app, ui.ctx())).drop_without_applying_deltas();
}

/// Let the wait run out, then one frame.
fn next_step(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if let Some(t) = app.paused_playbacks.timer.as_mut() {
        t.due = Instant::now();
    }
    frame(app, ctx, Vec::new());
}

fn esc() -> egui::Event {
    egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
}

fn assert_idle(app: &PhotocraftApp) {
    assert_eq!(app.session.actions.playing, 0);
    assert!(app.session.actions.playback_stack.is_empty());
    assert!(app.paused_playbacks.paused.is_empty());
    assert!(app.paused_playbacks.timer.is_none());
}

/// No continuation keeps the history policy leased once a playback ended.
fn assert_released(app: &mut PhotocraftApp) {
    let single = app.session.prefs().actions.create_single_history_state;
    app.run("actions.setHistory", json!({"singleState": single})).unwrap();
}

fn web_action_app(dir: &std::path::Path) -> PhotocraftApp {
    let mut app = app_with("accelerated", 1, true);
    let mut child = Action::new(
        "Export",
        vec![
            (
                "file.export.saveForWebLegacy".into(),
                json!({"format":"jpeg","quality":17,"dir":dir,"slices":"all","html":false,"imagesFolder":"assets",
            "fileNaming":{"components":["slice"],"wholeImage":true,"fileName":"Recorded.jpg","windows":true,"unix":true,"extension":"lower"}}),
            ),
            ("layer.new.layer".into(), json!({"name":"Child"})),
        ],
    );
    child.dialogs.insert(0);
    let parent = Action::new("Parent", vec![("actions.play".into(), json!({"action":"Export"})), ("layer.new.layer".into(), json!({"name":"Parent"}))]);
    let (bytes, skipped) = photocraft_engine::atn_cmds::export("Web", &[&child, &parent], &[], false).unwrap();
    assert!(skipped.is_empty());
    let imported = photocraft_engine::atn_cmds::import(&bytes).unwrap();
    assert!(imported.unsupported.is_empty());
    assert!(imported.actions[0].dialogs.contains(&0));
    app.session.actions.list = imported.actions;
    app
}

#[test]
fn recorded_web_dialog_waits_for_settings_and_the_chosen_filename() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("Chosen.png");
    let mut app = web_action_app(dir.path());
    let (picker, requests) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::SaveTo(output.to_string_lossy().into_owned()))]);
    app.services.file_dialog = Some(picker);
    let played = app.run("actions.play", json!({"action":"Parent"})).unwrap();
    let id = played["paused"]["dialog"].as_u64().expect("Save for Web must pause");
    assert!(names(&app).is_empty());
    assert!(!dir.path().join("Recorded.jpg").exists());
    let f = &mut app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!(f["format"], "jpeg");
    assert_eq!(f["quality"], 17);
    assert_eq!(f["resample"], "bicubic");
    f.insert("format".into(), json!("png24"));
    let confirmed = crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(confirmed["fileDialog"], "save");
    assert!(app.file_dialog_open());
    assert!(!output.exists(), "settings confirmation must not write before the picker answers");
    assert!(names(&app).is_empty());
    app.poll_file_dialog(&egui::Context::default(), None);
    assert_eq!(requests.borrow().len(), 1);
    assert!(output.exists());
    assert!(!dir.path().join("Recorded.jpg").exists());
    assert!(!dir.path().join("assets").exists());
    assert_eq!(&std::fs::read(&output).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(names(&app), ["Child", "Parent"]);
    assert_idle(&app);
}

#[test]
fn recorded_web_dialog_cancel_stops_at_settings_or_the_filename_picker() {
    for picker_cancel in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut app = web_action_app(dir.path());
        let (picker, _) = crate::file_dialog::fake(vec![None]);
        app.services.file_dialog = Some(picker);
        let played = app.run("actions.play", json!({"action":"Parent"})).unwrap();
        let id = played["paused"]["dialog"].as_u64().unwrap();
        if picker_cancel {
            crate::dialogs::confirm(&mut app, id).unwrap();
            app.poll_file_dialog(&egui::Context::default(), None);
        } else {
            crate::dialogs::cancel(&mut app, id).unwrap();
        }
        assert_idle(&app);
        assert!(!app.file_dialog_open());
        assert!(names(&app).is_empty());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        assert!(!app.ui.status_error, "{}", app.ui.status);
    }
}

#[test]
fn recorded_web_dialog_waits_while_the_filename_picker_is_open() {
    for update_at in [None, Some("settings"), Some("picker")] {
        web_filename_picker_wait(update_at);
    }
}

fn web_filename_picker_wait(update_at: Option<&str>) {
    let link_update = update_at.is_some();
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("Renamed.jpg");
    let source = dir.path().join("Source.png");
    let reply = std::rc::Rc::new(std::cell::RefCell::new(None));
    let answer = reply.clone();
    let mut app = web_action_app(dir.path());
    let mut replacement = Vec::new();
    if link_update {
        app.run("file.export.saveForWebLegacy", json!({"path":source,"format":"png24"})).unwrap();
        app.run("file.placeLinked", json!({"path":source})).unwrap();
        app.run("layer.smartObjects.updateChanged", json!({})).unwrap();
        let mut other = photocraft_engine::Session::new();
        other.execute("file.new", json!({"width":8,"height":6,"background":"#123456"})).unwrap();
        let path = dir.path().join("Replacement.png");
        other.execute("file.export.saveForWebLegacy", json!({"path":path,"format":"png24"})).unwrap();
        replacement = std::fs::read(path).unwrap();
    }
    app.services.file_dialog = Some(Box::new(move |_, _, pending| *answer.borrow_mut() = Some(pending)));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    let played = h.state_mut().run("actions.play", json!({"action":"Parent"})).unwrap();
    let id = played["paused"]["dialog"].as_u64().unwrap();
    let before = h.state().session.active().unwrap().doc.clone();
    let queue_update = |app: &mut PhotocraftApp| {
        std::fs::write(&source, &replacement).unwrap();
        let st = app.session.active().unwrap();
        let doc = st.doc.id.0;
        let layer = st.active_layer.unwrap().0;
        // A worker finishes after the export paused, as when an earlier action rewrote a link.
        app.session.link_scan = Some(std::sync::Arc::new(std::sync::Mutex::new(Some((
            doc,
            vec![(source.to_string_lossy().into_owned(), vec![layer], Some((replacement.len() as u64 + 1, 1)))],
        )))));
    };
    if update_at == Some("settings") {
        queue_update(h.state_mut());
    }
    h.run_steps(4);
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    if update_at == Some("picker") {
        queue_update(h.state_mut());
    }
    h.event(egui::Event::WindowFocused(false));
    h.run_steps(16);
    assert!(!output.exists());
    assert_eq!(names(h.state()).len(), usize::from(link_update));
    assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.active().unwrap().doc));
    reply.borrow_mut().take().unwrap().send(Some(crate::file_dialog::FileDialogAnswer::SaveTo(output.to_string_lossy().into_owned())));
    h.event(egui::Event::WindowFocused(true));
    h.run_steps(4);
    assert!(output.exists(), "{}", h.state().ui.status);
    assert!(names(h.state()).ends_with(&["Child".into(), "Parent".into()]));
    assert_idle(h.state());
    if link_update {
        let st = h.state().session.active().unwrap();
        assert_eq!(st.history.undo_label(), Some("Update Modified Content"), "apply the queued refresh after playback finishes");
    }
}

#[test]
fn recorded_web_dialog_rejects_an_edit_while_the_filename_picker_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("Renamed.jpg");
    let mut app = web_action_app(dir.path());
    let (picker, _) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::SaveTo(output.to_string_lossy().into_owned()))]);
    app.services.file_dialog = Some(picker);
    let played = app.run("actions.play", json!({"action":"Parent"})).unwrap();
    let id = played["paused"]["dialog"].as_u64().unwrap();
    crate::dialogs::confirm(&mut app, id).unwrap();
    app.run("layer.new.layer", json!({"name":"Independent edit"})).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
    assert!(!output.exists());
    assert!(app.ui.status.contains("the paused action's document or history changed"));
    assert_eq!(names(&app), ["Independent edit"]);
    assert_idle(&app);
}

#[test]
fn recorded_web_dialog_saves_on_the_first_play_with_a_layer_in_a_closed_group() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = web_action_app(dir.path());
    app.run("layer.new.layer", json!({"name":"Inside"})).unwrap();
    let inside = app.session.active().unwrap().active_layer.unwrap();
    let group = app.run("layer.groupLayers", json!({"name":"Closed"})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.new.layer", json!({"name":"Outside"})).unwrap();
    let child = &mut app.session.actions.list[0];
    child.steps.insert(0, ("layer.select".into(), json!({"layer":inside.0})));
    child.dialogs.clear();
    child.dialogs.insert(1);
    let reply = std::rc::Rc::new(std::cell::RefCell::new(None));
    let answer = reply.clone();
    app.services.file_dialog = Some(Box::new(move |_, _, pending| *answer.borrow_mut() = Some(pending)));
    app.background_jobs = true;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    for attempt in 0..2 {
        let played = h.state_mut().run("actions.play", json!({"action":"Parent"})).unwrap();
        let id = played["paused"]["dialog"].as_u64().unwrap();
        let before = h.state().session.active().unwrap().doc.clone();
        h.run_steps(4);
        crate::dialogs::confirm(h.state_mut(), id).unwrap();
        h.run_steps(8);
        assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.active().unwrap().doc), "revealing a layer must wait while export owns the snapshot");
        let output = dir.path().join(format!("Renamed-{attempt}.jpg"));
        reply.borrow_mut().take().unwrap().send(Some(crate::file_dialog::FileDialogAnswer::SaveTo(output.to_string_lossy().into_owned())));
        h.run_steps(4);
        assert!(output.exists(), "attempt {attempt}: {}", h.state().ui.status);
        assert_eq!(names(h.state()).iter().filter(|name| name.as_str() == "Child").count(), attempt + 1);
        assert_eq!(names(h.state()).iter().filter(|name| name.as_str() == "Parent").count(), attempt + 1);
        assert_idle(h.state());
        let doc = &h.state().session.active().unwrap().doc;
        assert!(matches!(&doc.layer(photocraft_doc::LayerId(group)).unwrap().content, photocraft_doc::LayerContent::Group(group) if group.expanded));
    }
}

#[test]
fn recorded_web_dialog_off_and_programmatic_playback_keep_the_recorded_destination() {
    for programmatic in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut app = web_action_app(dir.path());
        app.programmatic_playback = programmatic;
        if !programmatic {
            app.session.actions.list[0].dialogs.clear();
        }
        let played = app.run("actions.play", json!({"action":"Parent"})).unwrap();
        assert!(played.get("failed").is_none() && played.get("paused").is_none(), "{played}");
        assert!(dir.path().join("Recorded.jpg").exists());
        assert!(app.ui.dialogs.is_empty());
        assert!(!app.file_dialog_open());
        assert_eq!(names(&app), ["Child", "Parent"]);
        assert_idle(&app);
    }
}

#[test]
fn recorded_web_dialog_keeps_authorization_for_edited_settings_and_the_picked_path() {
    fn gate(command: &str, p: &Value) -> photocraft_engine::Result<()> {
        if command == "file.export.saveForWebLegacy"
            && (p["quality"].as_u64().is_some_and(|q| q > 50) || p["dir"].as_str().is_some_and(|dir| dir.contains("refused")))
        {
            return Err(photocraft_engine::EngineError::Other("export refused".into()));
        }
        Ok(())
    }
    for edited_settings in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut app = web_action_app(dir.path());
        let target = dir.path().join("refused").join("Chosen.jpg");
        let (picker, requests) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::SaveTo(target.to_string_lossy().into_owned()))]);
        app.services.file_dialog = Some(picker);
        app.session.authorize = Some(gate);
        let played = app.run("actions.play", json!({"action":"Parent"})).unwrap();
        let id = played["paused"]["dialog"].as_u64().unwrap();
        app.session.authorize = None;
        if edited_settings {
            app.ui.dialog_mut(id).unwrap().fields.insert("quality".into(), json!(99));
            assert!(crate::dialogs::confirm(&mut app, id).unwrap_err().contains("export refused"));
            assert!(requests.borrow().is_empty(), "refuse edited settings before opening the picker");
        } else {
            crate::dialogs::confirm(&mut app, id).unwrap();
            app.poll_file_dialog(&egui::Context::default(), None);
            assert!(app.ui.status_error && app.ui.status.contains("export refused"), "{}", app.ui.status);
        }
        assert!(!target.exists());
        assert!(names(&app).is_empty());
        assert_idle(&app);
    }
}

#[test]
fn recorded_web_dialog_does_not_export_another_document() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = web_action_app(dir.path());
    let played = app.run("actions.play", json!({"action":"Parent"})).unwrap();
    let id = played["paused"]["dialog"].as_u64().unwrap();
    app.run("file.new", json!({"width":32,"height":24})).unwrap();
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_idle(&app);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

    let mut app = web_action_app(dir.path());
    let _ = app.session.close(0);
    let played = app.run("actions.play", json!({"action":"Parent"})).unwrap();
    assert!(played.get("failed").is_some(), "an unavailable export must fail instead of opening an empty dialog");
    assert!(app.ui.dialogs.is_empty());
    assert_idle(&app);
}

#[test]
fn recorded_web_dialog_write_failure_does_not_run_the_tail() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("not-a-folder");
    std::fs::write(&parent, b"unchanged").unwrap();
    let target = parent.join("Chosen.jpg");
    let mut app = web_action_app(dir.path());
    let (picker, _) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::SaveTo(target.to_string_lossy().into_owned()))]);
    app.services.file_dialog = Some(picker);
    let played = app.run("actions.play", json!({"action":"Parent"})).unwrap();
    let id = played["paused"]["dialog"].as_u64().unwrap();
    crate::dialogs::confirm(&mut app, id).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
    assert!(app.ui.status_error);
    assert!(names(&app).is_empty());
    assert_eq!(std::fs::read(parent).unwrap(), b"unchanged");
    assert_idle(&app);
}

#[test]
fn recorded_move_inside_a_hidden_group_continues_in_the_desktop_player() {
    let mut app = app_with("accelerated", 1, true);
    let id = app.run("shape.create", json!({"kind":"rect","rect":[1,1,3,2],"name":"Tile"})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.groupLayers", json!({})).unwrap();
    let group = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.setProps", json!({"visible":false})).unwrap();
    app.run("layer.select", json!({"layer":id})).unwrap();
    let child = Action::new("Move", vec![("layer.translate".into(), json!({"dx":-19,"dy":0}))]);
    let parent = Action::new("Parent", vec![("actions.play".into(), json!({"action":"Move"})), ("layer.new.layer".into(), json!({"name":"After"}))]);
    let (bytes, skipped) = photocraft_engine::atn_cmds::export("Moves", &[&child, &parent], &[], false).unwrap();
    assert!(skipped.is_empty());
    let imported = photocraft_engine::atn_cmds::import(&bytes).unwrap();
    assert!(imported.unsupported.is_empty());
    app.session.actions.list = imported.actions;
    let played = app.run("actions.play", json!({"action":"Parent"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let doc = &app.session.active().unwrap().doc;
    let tile = doc.layer(photocraft_doc::LayerId(id)).unwrap();
    let photocraft_doc::LayerContent::Shape(shape) = &tile.content else { panic!("shape") };
    let photocraft_doc::LiveShape::Rect { rect, .. } = shape.live.as_ref().unwrap() else { panic!("rectangle") };
    assert_eq!(*rect, [-18.0, 1.0, 3.0, 2.0]);
    assert!(tile.visible);
    assert!(!doc.layer(group).unwrap().visible);
    assert!(doc.walk().iter().any(|(_, _, layer)| layer.name == "After"));
    assert_eq!(app.session.actions.playing, 0);
    assert!(app.session.actions.playback_stack.is_empty());
}

#[test]
fn close_returns_to_recent_document_before_parent_continues() {
    let mut app = app_with("accelerated", 1, false);
    let receiver = app.session.active().unwrap().doc.id;
    app.run("layer.new.layer", json!({"name":"Receiver"})).unwrap();
    for name in ["Copy", "Palette", "Work"] {
        app.run("file.new", json!({"name":name,"width":8,"height":8})).unwrap();
    }
    app.session.set_active(0);
    app.run("document.activate", json!({"offset":3})).unwrap();
    let child = Action::new("Close and select", vec![("file.close".into(), json!({})), ("layer.select".into(), json!({"layer":"Receiver"}))]);
    let parent =
        Action::new("Parent", vec![("actions.play".into(), json!({"action":"Close and select"})), ("layer.new.layer".into(), json!({"name":"After"}))]);
    let (bytes, skipped) = photocraft_engine::atn_cmds::export("Playback", &[&child, &parent], &[], false).unwrap();
    assert!(skipped.is_empty());
    let imported = photocraft_engine::atn_cmds::import(&bytes).unwrap();
    assert!(imported.unsupported.is_empty());
    app.session.actions.list = imported.actions;
    let result = play(&mut app, &json!({"action":"Parent"})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    assert_eq!(app.session.active().unwrap().doc.id, receiver);
    assert_eq!(names(&app), ["Receiver", "After"]);
    assert_eq!(app.session.documents().len(), 3);
}

#[test]
fn deselect_atn_noop_keeps_nested_playback_pacing_and_parent_continuation() {
    let child = Action::new(
        "Clear",
        vec![("select.deselect".into(), json!({})), ("select.deselect".into(), json!({})), ("layer.new.layer".into(), json!({"name":"Child"}))],
    );
    let parent = Action::new("Parent", vec![("actions.play".into(), json!({"action":"Clear"})), ("layer.new.layer".into(), json!({"name":"Parent"}))]);
    let (bytes, skipped) = photocraft_engine::atn_cmds::export("Playback", &[&child, &parent], &[], false).unwrap();
    assert!(skipped.is_empty());
    for performance in ["accelerated", "step", "pause"] {
        for single in [false, true] {
            for selected in [false, true] {
                let mut app = app_with(performance, 1, single);
                if selected {
                    app.run("select.rect", json!({"x":1,"y":1,"width":3,"height":2})).unwrap();
                }
                let before = app.session.active().unwrap().doc.clone();
                let past = app.session.active().unwrap().history.past_len();
                let imported = photocraft_engine::atn_cmds::import(&bytes).unwrap();
                assert!(imported.unsupported.is_empty());
                assert_eq!(imported.actions[0].steps[0].0, "select.deselect");
                app.session.actions.list = imported.actions;
                let result = play(&mut app, &json!({"action":"Parent"})).unwrap();
                assert!(result.get("failed").is_none(), "{performance} single={single} selected={selected}: {result}");
                if performance != "accelerated" {
                    assert!(app.paused_playbacks.timer.is_some(), "a no-op still counts as a paced step");
                    assert!(names(&app).is_empty(), "wait after Deselect, before making the child's layer");
                }
                let ctx = egui::Context::default();
                for _ in 0..8 {
                    if app.paused_playbacks.timer.is_none() {
                        break;
                    }
                    next_step(&mut app, &ctx);
                }
                assert_idle(&app);
                assert!(!app.ui.status_error, "{}", app.ui.status);
                assert_eq!(names(&app), ["Child", "Parent"]);
                let st = app.session.active().unwrap();
                assert!(st.doc.selection.is_none());
                assert_eq!(st.history.past_len(), past + if single { 1 } else { 2 + usize::from(selected) });
                for _ in 0..if single { 1 } else { 2 + usize::from(selected) } {
                    assert!(app.session.undo());
                }
                assert_eq!(app.session.active().unwrap().doc, before);
            }
        }
    }
}

#[test]
fn deselect_noop_preserves_history_redo_and_reselect() {
    for single in [false, true] {
        let mut app = app_with("accelerated", 1, single);
        app.run("select.rect", json!({"x":1,"y":1,"width":3,"height":2})).unwrap();
        let selection = app.session.active().unwrap().doc.selection.clone();
        app.run("select.deselect", json!({})).unwrap();
        app.run("layer.new.layer", json!({"name":"Redo"})).unwrap();
        assert!(app.session.undo());
        let st = app.session.active().unwrap();
        let before = (st.doc.clone(), st.revision, st.history.past_len(), st.history.redo_label().map(str::to_owned));
        let journal = app.session.journal.len();
        assert!(!app.session.is_enabled("select.deselect"), "the ordinary menu stays grey");
        assert!(app.session.execute("select.deselect", json!({})).is_err());
        app.session.actions.list = vec![Action::new("Clear", vec![("select.deselect".into(), json!({})), ("select.deselect".into(), json!({}))])];
        let result = play(&mut app, &json!({"action":"Clear"})).unwrap();
        assert!(result.get("failed").is_none(), "{result}");
        assert_eq!(result["ran"], 2);
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.clone(), st.revision, st.history.past_len(), st.history.redo_label().map(str::to_owned)), before);
        assert_eq!(app.session.journal.len(), journal);
        app.run("select.reselect", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.selection, selection);
    }
}

#[test]
fn deselect_noop_does_not_bypass_document_authorization_or_other_preconditions() {
    for case in ["no document", "denied", "other disabled step"] {
        let mut app = app_with("accelerated", 1, false);
        let id = if case == "other disabled step" { "select.inverse" } else { "select.deselect" };
        app.session.actions.list = vec![Action::new("Clear", vec![(id.into(), json!({})), ("layer.new.layer".into(), json!({"name":"After"}))])];
        if case == "no document" {
            app.session.close(0).unwrap();
        } else if case == "denied" {
            app.session.authorize =
                Some(|id, _| if id == "select.deselect" { Err(photocraft_engine::EngineError::Other("denied Deselect".into())) } else { Ok(()) });
        }
        let result = play(&mut app, &json!({"action":"Clear"})).unwrap();
        assert_eq!(result["ran"], 0, "{case}: {result}");
        assert_eq!(result["failed"]["step"], 0, "{case}: {result}");
        assert_eq!(result["failed"]["id"], id);
        assert!(app.session.active().is_none_or(|st| st.doc.walk().iter().all(|(_, _, l)| l.name != "After")));
    }
}

#[test]
fn step_by_step_plays_every_step_from_one_play_with_a_frame_between() {
    for single in [true, false] {
        let mut app = app_with("step", 1, single);
        let ctx = egui::Context::default();
        let past = app.session.active().unwrap().history.past_len();
        let v = play(&mut app, &json!({"action": "Three"})).unwrap();
        assert_eq!(v["ran"], 1, "{v}");
        assert_eq!(names(&app), ["A"], "the first step is shown before the next runs");
        let due = app.paused_playbacks.timer.unwrap().due.saturating_duration_since(Instant::now());
        assert!(due > Duration::from_millis(500) && due <= playback::STEP_GAP, "{due:?}");
        frame(&mut app, &ctx, Vec::new());
        assert_eq!(names(&app), ["A"], "nothing runs before the wait is over");
        next_step(&mut app, &ctx);
        assert_eq!(names(&app), ["A", "B"]);
        next_step(&mut app, &ctx);
        assert_eq!(names(&app), ["A", "B", "C"]);
        assert!(app.paused_playbacks.timer.is_none(), "no wait after the last step");
        assert_idle(&app);
        assert_released(&mut app);
        assert_eq!(app.ui.status, "Played 3 steps");
        let st = app.session.active().unwrap();
        if single {
            assert_eq!(st.history.past_len(), past + 1, "one history state for the whole play");
            assert_eq!(st.history.undo_label(), Some("Action: Three"));
            assert!(app.session.undo());
            assert!(names(&app).is_empty(), "one undo takes back every step");
        } else {
            assert_eq!(st.history.past_len(), past + 3, "one state per step with the option off");
        }
    }
}

#[test]
fn pause_for_waits_its_seconds_after_each_step_but_the_last() {
    let mut app = app_with("pause", 5, true);
    let ctx = egui::Context::default();
    play(&mut app, &json!({"action": "Three"})).unwrap();
    assert_eq!(names(&app), ["A"], "no wait before the first step");
    let due = app.paused_playbacks.timer.unwrap().due.saturating_duration_since(Instant::now());
    assert!(due > Duration::from_millis(5500) && due <= playback::STEP_GAP + Duration::from_secs(5), "{due:?}");
    frame(&mut app, &ctx, Vec::new());
    assert_eq!(names(&app), ["A"]);
    next_step(&mut app, &ctx);
    assert_eq!(names(&app), ["A", "B"]);
    assert!(app.paused_playbacks.timer.is_some(), "waits again between B and C");
    next_step(&mut app, &ctx);
    assert_eq!(names(&app), ["A", "B", "C"]);
    assert_idle(&app);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Action: Three"));
}

#[test]
fn accelerated_and_agents_play_at_once() {
    let mut app = app_with("accelerated", 1, true);
    assert_eq!(play(&mut app, &json!({"action": "Three"})).unwrap()["ran"], 3);
    assert_idle(&app);
    let mut app = app_with("pause", 60, true);
    app.automation_input = true;
    assert_eq!(play(&mut app, &json!({"action": "Three"})).unwrap()["ran"], 3, "agents never wait");
    assert_idle(&app);
}

#[test]
fn esc_or_stop_ends_the_playback_and_keeps_the_steps_applied() {
    for how in ["esc", "stop button", "list edited", "new play"] {
        let mut app = app_with("pause", 3, true);
        let ctx = egui::Context::default();
        let past = app.session.active().unwrap().history.past_len();
        play(&mut app, &json!({"action": "Three"})).unwrap();
        next_step(&mut app, &ctx);
        assert_eq!(names(&app), ["A", "B"]);
        match how {
            "esc" => frame(&mut app, &ctx, vec![esc()]),
            "stop button" => assert!(playback::stop(&mut app)),
            "list edited" => {
                app.run("actions.newSet", json!({"name": "Other"})).unwrap();
                frame(&mut app, &ctx, Vec::new());
            }
            _ => {
                app.session.actions.list.push(Action::new("One", vec![("layer.new.layer".into(), json!({"name": "D"}))]));
                app.run("actions.setPlayback", json!({"performance": "accelerated"})).unwrap();
                play(&mut app, &json!({"action": "One"})).unwrap();
            }
        }
        next_step(&mut app, &ctx);
        let expected: &[&str] = if how == "new play" { &["A", "B", "D"] } else { &["A", "B"] };
        assert_eq!(names(&app), expected, "{how}: C never runs");
        assert_idle(&app);
        assert_released(&mut app);
        let st = app.session.active().unwrap();
        let added = if how == "new play" { 2 } else { 1 };
        assert_eq!(st.history.past_len(), past + added, "{how}: the applied steps stay one history state");
        if how != "new play" {
            assert_eq!(app.ui.status, "Action playback stopped", "{how}");
        }
    }
}

#[test]
fn an_edit_between_steps_ends_the_playback_without_joining_it() {
    let mut app = app_with("step", 1, true);
    let ctx = egui::Context::default();
    play(&mut app, &json!({"action": "Three"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Mine"})).unwrap();
    next_step(&mut app, &ctx);
    assert_eq!(names(&app), ["A", "Mine"], "the action doesn't go on over a document someone changed");
    assert!(app.ui.status_error, "{}", app.ui.status);
    assert_idle(&app);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("New Layer"));
}

/// A called action is paced too, and its caller goes on after it in the same playback.
#[test]
fn nested_and_set_playback_stay_one_play() {
    let mut app = app_with("step", 1, true);
    let ctx = egui::Context::default();
    app.session.actions.list.push(Action::new(
        "Caller",
        vec![
            ("layer.new.layer".into(), json!({"name": "Before"})),
            ("actions.play".into(), json!({"action": "Three"})),
            ("layer.new.layer".into(), json!({"name": "After"})),
        ],
    ));
    let past = app.session.active().unwrap().history.past_len();
    play_targets(&mut app, &[1], 0).unwrap();
    // One step per frame, also into and out of the called action.
    let mut seen = vec![names(&app)];
    while app.paused_playbacks.timer.is_some() {
        next_step(&mut app, &ctx);
        seen.push(names(&app));
    }
    assert_eq!(seen.iter().map(Vec::len).collect::<Vec<_>>(), [1, 2, 3, 4, 5], "{seen:?}");
    assert_eq!(names(&app), ["Before", "A", "B", "C", "After"]);
    assert_idle(&app);
    assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Action: Caller"));
    // Counted as Accelerated counts it: the Play Action step is one step of its caller.
    assert_eq!(app.ui.status, "Played 3 steps");
    let mut fast = app_with("accelerated", 1, true);
    fast.session.actions.list.push(app.session.actions.list[1].clone());
    let v = play_targets(&mut fast, &[1], 0).unwrap().unwrap();
    assert_eq!(v["ran"], 3, "{v}");
}

#[test]
fn the_panel_menu_opens_playback_options_and_ok_saves_them() {
    let mut app = app_with("accelerated", 1, true);
    let opened = playback::open_dialog(&mut app).unwrap();
    let id = opened["dialog"].as_u64().unwrap();
    let d = app.ui.dialog_mut(id).unwrap();
    assert_eq!(crate::dialogs::title(d), "Playback Options");
    assert_eq!(d.fields["performance"], "accelerated");
    assert_eq!(d.fields["seconds"], 1);
    assert_eq!(d.fields["singleState"], true);
    d.fields.insert("performance".into(), json!("pause"));
    d.fields.insert("seconds".into(), json!(75));
    d.fields.insert("singleState".into(), json!(false));
    crate::dialogs::confirm(&mut app, id).unwrap();
    let a = &app.session.prefs().actions;
    assert_eq!(a.performance, photocraft_engine::prefs::PlaybackPerformance::Pause);
    assert_eq!(a.pause_seconds, 60, "a number past the range takes the closest one");
    assert!(!a.create_single_history_state);
    // Cancel keeps them.
    let id = playback::open_dialog(&mut app).unwrap()["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("performance".into(), json!("step"));
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert_eq!(app.session.prefs().actions.performance, photocraft_engine::prefs::PlaybackPerformance::Pause);
}

#[test]
fn the_dialog_shows_the_reference_layout() {
    use egui_kittest::{Harness, kittest::Queryable};
    let app = app_with("step", 9, true);
    let mut h = Harness::builder().with_size(egui::vec2(520.0, 420.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    playback::open_dialog(h.state_mut()).unwrap();
    h.run_steps(3);
    for label in ["Performance", "Accelerated", "Step by Step", "Pause For:", "seconds", "History", "Create a single history state", "OK", "Cancel"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    h.get_by_label("Pause For:").click();
    h.run_steps(2);
    let id = h.state().ui.dialogs[0].id;
    assert_eq!(h.state_mut().ui.dialog_mut(id).unwrap().fields["performance"], "pause");
    assert_eq!(h.state_mut().ui.dialog_mut(id).unwrap().fields["seconds"], 9, "Pause For shows the kept seconds");
    h.get_by_label("Create a single history state").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(2);
    let a = &h.state().session.prefs().actions;
    assert_eq!((a.performance, a.pause_seconds, a.create_single_history_state), (photocraft_engine::prefs::PlaybackPerformance::Pause, 9, false));
}

#[test]
fn a_paced_play_clears_an_earlier_playback_error() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = app_with("step", 1, true);
    app.ui.status = "Step 2 (layer.select) failed: no such layer".into();
    app.ui.status_error = true;
    let mut h = Harness::builder().with_size(egui::vec2(400.0, 320.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(3);
    h.get_by_label("Play selection").click();
    h.run_steps(3);
    assert_eq!(names(h.state()), ["A"], "the play waits after its first step");
    assert!(h.state().paused_playbacks.timer.is_some());
    assert!(!h.state().ui.status_error, "the old failure is gone: {}", h.state().ui.status);
    assert!(h.state().ui.status.is_empty(), "{}", h.state().ui.status);
    // A status that isn't an error stays (a step's own message, say).
    let mut app = app_with("step", 1, true);
    app.ui.status = "Saved".into();
    play_targets(&mut app, &[0], 0).map(|v| report_play(&mut app, &v.unwrap())).unwrap();
    assert_eq!(app.ui.status, "Saved");
}

#[test]
fn playback_options_ok_refuses_values_automation_set_that_the_dialog_cannot_show() {
    use photocraft_engine::prefs::PlaybackPerformance;
    let control = |app: &mut PhotocraftApp, method: &str, params: Value| {
        let (request, _) = crate::control::ControlRequest::new(method, params);
        match crate::control::handle(app, &egui::Context::default(), &request) {
            crate::control::Outcome::Done(v) => v,
            _ => panic!("{method} answers at once"),
        }
    };
    for (field, value) in
        [("performance", Value::Null), ("performance", json!("fast")), ("performance", json!(2)), ("singleState", Value::Null), ("singleState", json!("yes"))]
    {
        let mut app = app_with("step", 4, false);
        let id = playback::open_dialog(&mut app).unwrap()["dialog"].as_u64().unwrap();
        assert_eq!(control(&mut app, "ui.dialog.set", json!({"dialog": id, "field": field, "value": value}))["ok"], true);
        let r = control(&mut app, "ui.dialog.confirm", json!({"dialog": id}));
        assert!(r["error"].is_string(), "{field} = {value}: {r}");
        let a = &app.session.prefs().actions;
        assert_eq!(
            (a.performance, a.pause_seconds, a.create_single_history_state),
            (PlaybackPerformance::StepByStep, 4, false),
            "{field} = {value} saves nothing"
        );
    }
    // Valid values through the same path are saved.
    let mut app = app_with("step", 4, false);
    let id = playback::open_dialog(&mut app).unwrap()["dialog"].as_u64().unwrap();
    control(&mut app, "ui.dialog.set", json!({"dialog": id, "field": "performance", "value": "accelerated"}));
    control(&mut app, "ui.dialog.set", json!({"dialog": id, "field": "singleState", "value": true}));
    assert_eq!(control(&mut app, "ui.dialog.confirm", json!({"dialog": id}))["ok"], true);
    let a = &app.session.prefs().actions;
    assert_eq!((a.performance, a.create_single_history_state), (PlaybackPerformance::Accelerated, true));
}
