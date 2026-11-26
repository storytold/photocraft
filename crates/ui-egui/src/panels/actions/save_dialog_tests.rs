//! #15945: a recorded save step with its dialog on shows its dialogs when a person plays it, and
//! waits for them; Cancel stops the action. Agents keep the silent replay.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use craft::automation::actions::ActionStep;
use craft::host::{PickKind, Picked, ScriptedFiles};
use serde_json::{Value, json};

use photocraft_engine::actions_cmds::Action;

use super::save_dialog::{self, SAVE_DIALOG};
use crate::PhotocraftApp;

/// The paths the app's writer was given.
type Written = Rc<RefCell<Vec<String>>>;

/// An app with a 4 x 4 document whose file dialogs stay open until the test answers them, whose
/// writer records the paths it wrote, and whose one action `Save` is `step` with its dialog on,
/// then a new layer named `after`.
fn setup(step: (&str, Value)) -> (Box<PhotocraftApp>, Arc<ScriptedFiles>, Written) {
    setup_writing(step, true)
}

/// [`setup`] with a writer that fails every write unless `writes`.
fn setup_writing(step: (&str, Value), writes: bool) -> (Box<PhotocraftApp>, Arc<ScriptedFiles>, Written) {
    let files = Arc::new(ScriptedFiles::held());
    let written: Written = Default::default();
    let w = written.clone();
    let services = crate::Services {
        export: Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new())))),
        write: Some(Box::new(move |path: &str, _: &[u8]| {
            if !writes {
                return Err("the disk is full".to_string());
            }
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        files: Some(files.clone()),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.background_jobs = false;
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    let steps = vec![ActionStep::from((step.0.to_string(), step.1)), ActionStep::from(("layer.new.layer".to_string(), json!({"name": "after"})))];
    let mut action = Action::new("Save", steps);
    action.dialogs.insert(0);
    app.session.actions.list.push(action).unwrap();
    (app, files, written)
}

/// A person presses Play on `Save`.
fn play(app: &mut PhotocraftApp) -> Value {
    super::play(app, &json!({"action": "Save"})).unwrap()
}

/// The user answers the open save dialog with `picked`; the next frame sees the dialogs end.
fn answer(app: &mut PhotocraftApp, files: &ScriptedFiles, picked: Picked) {
    files.answer(picked).expect("a save dialog is open");
    app.poll_file_dialog();
    save_dialog::tick(app);
}

/// The answer of a save dialog that chose `path`.
fn chose(path: &str) -> Picked {
    Picked::Paths(vec![PathBuf::from(path)])
}

/// Whether the step after the save ran.
fn after_ran(app: &PhotocraftApp) -> bool {
    app.session.active().unwrap().doc.walk().into_iter().any(|(_, _, l)| l.name == "after")
}

/// `name` in a folder that exists (the system's temporary folder), as a recorded step's file.
fn existing(name: &str) -> String {
    std::env::temp_dir().join(name).to_string_lossy().into_owned()
}

/// The value of `field` in the newest open dialog (a format's options).
fn newest_dialog_field(app: &PhotocraftApp, field: &str) -> Value {
    let fields = app.framework.dialogs.open_dialogs().last().and_then(|d| d.fields.clone()).unwrap_or(Value::Null);
    fields.get(field).cloned().unwrap_or(Value::Null)
}

/// The file the one open save dialog suggests, as `(folder, name)`.
fn suggested(files: &ScriptedFiles) -> (Option<PathBuf>, Option<String>) {
    assert_eq!(files.open(), 1, "one save dialog is open");
    let request = files.requests().pop().expect("a request");
    assert_eq!(request.kind, PickKind::Save, "{request:?}");
    (request.directory, request.file_name)
}

#[test]
fn a_save_as_step_with_its_dialog_on_asks_where_to_save_and_continues_after_the_save() {
    let (mut app, files, written) = setup(("file.saveAs", json!({"path": existing("played.psd")})));
    let v = play(&mut app);
    assert_eq!(v["paused"]["dialog"], SAVE_DIALOG, "{v}");
    assert_eq!(suggested(&files), (Some(std::env::temp_dir()), Some("played.psd".into())), "the dialog opens on the recorded file");
    assert!(written.borrow().is_empty(), "nothing is written before the dialog is answered");
    assert!(!after_ran(&app), "the steps after the save wait for its dialog");
    // The frames while the dialog is up change nothing.
    save_dialog::tick(&mut app);
    assert!(!after_ran(&app));
    answer(&mut app, &files, chose("/elsewhere/chosen.psd"));
    assert_eq!(*written.borrow(), ["/elsewhere/chosen.psd"], "the file chosen in the dialog, not the recorded one");
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/elsewhere/chosen.psd"));
    assert!(after_ran(&app), "the action goes on after the save: {}", app.ui.status);
    assert!(app.ui.status.starts_with("Played"), "{}", app.ui.status);
    assert!(app.paused_playbacks.paused.is_empty() && app.paused_playbacks.save.is_none());
}

#[test]
fn cancel_in_the_save_dialog_writes_nothing_and_stops_the_action() {
    let (mut app, files, written) = setup(("file.saveAs", json!({"path": "/out/played.psd"})));
    play(&mut app);
    answer(&mut app, &files, Picked::Cancelled);
    assert!(written.borrow().is_empty(), "{:?}", written.borrow());
    assert!(!after_ran(&app), "the action stops at the cancelled save");
    assert_eq!(app.ui.status, "Action playback stopped");
    assert!(app.paused_playbacks.paused.is_empty() && app.paused_playbacks.save.is_none());
    // The next Play asks again.
    play(&mut app);
    assert_eq!(files.open(), 1);
    // An error left in the status bar from before doesn't pass for this step's: Cancel still
    // says the action stopped.
    answer(&mut app, &files, Picked::Cancelled);
    app.ui.status = "Save failed: an earlier error".into();
    app.ui.status_error = true;
    play(&mut app);
    answer(&mut app, &files, Picked::Cancelled);
    assert_eq!(app.ui.status, "Action playback stopped");
    // Nor does an error reported while the dialog is open that isn't the step's write.
    play(&mut app);
    app.ui.status = "Something else failed".into();
    app.ui.status_error = true;
    answer(&mut app, &files, Picked::Cancelled);
    assert_eq!(app.ui.status, "Action playback stopped");
}

#[test]
fn the_action_waits_through_the_format_options_and_stops_at_their_cancel() {
    let (mut app, files, written) = setup(("file.saveAs", json!({"path": "/out/played.psd"})));
    play(&mut app);
    answer(&mut app, &files, chose("/out/chosen.jpg"));
    assert!(app.save_options_open(), "a JPEG asks its JPEG Options, as File › Save As does");
    assert!(written.borrow().is_empty() && !after_ran(&app), "the action waits for JPEG Options too");
    let ctx = app.ctx().clone();
    crate::format_prompts::confirm(&mut app, &ctx);
    save_dialog::tick(&mut app);
    assert_eq!(*written.borrow(), ["/out/chosen.jpg"]);
    assert!(after_ran(&app), "OK in JPEG Options continues the action: {}", app.ui.status);

    let (mut app, files, written) = setup(("file.saveAs", json!({"path": "/out/played.psd"})));
    play(&mut app);
    answer(&mut app, &files, chose("/out/chosen.jpg"));
    let ctx = app.ctx().clone();
    let id = app.framework.dialogs.open_dialogs().last().map(|d| d.id).expect("JPEG Options is open");
    app.framework.dialogs.cancel_by_id(&ctx, id);
    crate::format_prompts::take(&mut app, &ctx);
    save_dialog::tick(&mut app);
    assert!(written.borrow().is_empty() && !after_ran(&app), "Cancel in JPEG Options stops the action");
    assert_eq!(app.ui.status, "Action playback stopped");
}

#[test]
fn a_save_a_copy_step_with_its_dialog_on_writes_a_copy_where_chosen() {
    let (mut app, files, written) = setup(("file.saveACopy", json!({"path": existing("copy.psd")})));
    let v = play(&mut app);
    assert_eq!(v["paused"]["dialog"], SAVE_DIALOG, "{v}");
    assert_eq!(suggested(&files), (Some(std::env::temp_dir()), Some("copy.psd".into())));
    answer(&mut app, &files, chose("/out/other copy.psd"));
    assert_eq!(*written.borrow(), ["/out/other copy.psd"]);
    assert_eq!(app.session.active().unwrap().path, None, "a copy leaves the document's own file alone");
    assert!(after_ran(&app));
}

#[test]
fn an_export_as_step_with_its_dialog_on_opens_on_the_recorded_settings_and_cancel_stops() {
    let (mut app, _files, written) = setup((crate::export_dialog::COMMAND, json!({"path": "/out/web.jpg", "format": "jpg", "quality": 42})));
    let v = play(&mut app);
    assert_eq!(v["paused"]["dialog"], SAVE_DIALOG, "{v}");
    let dialog = app.framework.dialogs.get::<craft::ui::patterns::form_dialogs::HostedExportAs>().expect("Export As is open");
    assert_eq!((dialog.options().format.as_str(), dialog.options().quality), ("jpg", 42), "the recorded settings, not the remembered ones");
    save_dialog::tick(&mut app);
    assert!(!after_ran(&app), "the action waits while Export As is open");
    let ctx = app.ctx().clone();
    let id = app.export_open.as_ref().map(|o| o.dialog).expect("Export As is open");
    app.framework.dialogs.cancel_by_id(&ctx, id);
    crate::export_dialog::take(&mut app, &ctx);
    save_dialog::tick(&mut app);
    assert!(written.borrow().is_empty() && !after_ran(&app));
    assert_eq!(app.ui.status, "Action playback stopped");
}

#[test]
fn agents_and_a_step_with_its_dialog_off_save_without_asking() {
    // An agent's Play: the dialog toggle doesn't apply.
    let (mut app, files, written) = setup(("file.saveAs", json!({"path": "/out/played.psd"})));
    app.programmatic_playback = true;
    let v = app.run("actions.play", json!({"action": "Save"})).unwrap();
    assert!(v.get("paused").is_none() && v.get("failed").is_none(), "{v}");
    assert!(files.requests().is_empty(), "no dialog for an agent");
    assert_eq!(*written.borrow(), ["/out/played.psd"]);
    assert!(after_ran(&app));
    // A person's Play with the toggle off.
    let (mut app, files, written) = setup(("file.saveAs", json!({"path": "/out/played.psd"})));
    app.session.actions.list.set_dialog(0, 0, false).unwrap();
    let v = play(&mut app);
    assert!(v.get("paused").is_none(), "{v}");
    assert!(files.requests().is_empty());
    assert_eq!(*written.borrow(), ["/out/played.psd"]);
}

/// A gate that keeps writes out of `/outside`.
fn deny_outside(_id: &str, p: &Value) -> photocraft_engine::Result<()> {
    match p.get("path").and_then(Value::as_str) {
        Some(path) if path.starts_with("/outside") => Err(photocraft_engine::EngineError::Other("writes outside the allowed folder are denied".into())),
        _ => Ok(()),
    }
}

#[test]
fn the_file_chosen_in_the_dialog_is_judged_by_the_playbacks_gate() {
    for step in [("file.saveAs", json!({"path": "/out/played.psd"})), ("file.saveACopy", json!({"path": "/out/copy.psd"}))] {
        let (mut app, files, written) = setup(step.clone());
        app.session.authorize = Some(deny_outside);
        play(&mut app);
        answer(&mut app, &files, chose("/outside/chosen.psd"));
        assert!(written.borrow().is_empty(), "{}: the gate refuses the chosen file, so nothing is written", step.0);
        assert!(!after_ran(&app), "{}: the action ends at the refused save", step.0);
        assert!(app.ui.status_error && app.ui.status.contains("denied"), "{}: the denial stays in the status bar: {}", step.0, app.ui.status);
        assert!(app.paused_playbacks.paused.is_empty() && app.paused_playbacks.save.is_none());
    }
}

#[test]
fn a_failed_write_keeps_its_error_and_stops_the_action() {
    let (mut app, files, _written) = setup_writing(("file.saveAs", json!({"path": "/out/played.psd"})), false);
    app.background_jobs = true;
    play(&mut app);
    assert!(app.background_jobs, "background jobs stay on while the step's dialog only waits for the person");
    answer(&mut app, &files, chose("/out/chosen.psd"));
    assert!(!after_ran(&app), "a save that failed doesn't continue the action");
    assert!(app.ui.status_error && app.ui.status.contains("the disk is full"), "the write's error stays: {}", app.ui.status);
    assert!(app.background_jobs, "background jobs are back on once the dialogs closed");
    assert!(app.paused_playbacks.paused.is_empty() && app.paused_playbacks.save.is_none());
    // Played again over that same error, the same failure still keeps it.
    let error = app.ui.status.clone();
    play(&mut app);
    answer(&mut app, &files, chose("/out/chosen.psd"));
    assert!(!after_ran(&app));
    assert!(app.ui.status_error && app.ui.status == error, "the repeated failure keeps its error: {}", app.ui.status);
}

#[test]
fn an_export_that_wrote_its_files_continues_without_a_journaled_step() {
    // Several Scale All sizes journal no step: Export As marks the export itself.
    let (mut app, _files, _written) = setup((crate::export_dialog::COMMAND, json!({"path": "/out/web.jpg", "format": "jpg"})));
    play(&mut app);
    save_dialog::exported(&mut app);
    let ctx = app.ctx().clone();
    let id = app.export_open.as_ref().map(|o| o.dialog).expect("Export As is open");
    app.framework.dialogs.cancel_by_id(&ctx, id);
    crate::export_dialog::take(&mut app, &ctx);
    save_dialog::tick(&mut app);
    assert!(after_ran(&app), "the action goes on after the export: {}", app.ui.status);
}

#[test]
fn a_save_answered_on_another_active_document_writes_nothing_and_ends_the_action() {
    let (mut app, files, written) = setup(("file.saveACopy", json!({"path": "/out/copy.psd"})));
    play(&mut app);
    // Another document comes to the front while the dialog is open.
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    answer(&mut app, &files, chose("/out/copy.psd"));
    assert!(written.borrow().is_empty(), "nothing is written once the step's document isn't the active one: {:?}", written.borrow());
    assert!(!after_ran(&app), "the rest doesn't play on the other document");
    assert!(app.ui.status_error && app.ui.status.contains("active document changed"), "{}", app.ui.status);
    assert!(app.paused_playbacks.paused.is_empty() && app.paused_playbacks.save.is_none());
}

#[test]
fn a_save_made_another_way_while_the_dialog_is_open_doesnt_continue_a_cancelled_step() {
    let (mut app, files, written) = setup(("file.saveAs", json!({"path": "/out/played.psd"})));
    play(&mut app);
    // A control request saves the document while the step's save dialog is still open.
    app.save_as(Some("/elsewhere/by-request.psd".into())).unwrap();
    assert_eq!(*written.borrow(), ["/elsewhere/by-request.psd"]);
    answer(&mut app, &files, Picked::Cancelled);
    assert!(!after_ran(&app), "Cancel stops the action: that save wasn't the dialog's");
    assert_eq!(app.ui.status, "Action playback stopped");

    // The same with a JPEG, whose JPEG Options write goes through the format prompts' write.
    let (mut app, files, written) = setup(("file.saveAs", json!({"path": "/out/played.psd"})));
    app.session.authorize = Some(deny_outside);
    play(&mut app);
    app.save_as(Some("/outside/by-request.jpg".into())).unwrap();
    assert!(app.save_options_open(), "the request's own JPEG Options");
    let ctx = app.ctx().clone();
    crate::format_prompts::confirm(&mut app, &ctx);
    assert_eq!(*written.borrow(), ["/outside/by-request.jpg"], "another request's save is neither judged nor refused as the step's");
    answer(&mut app, &files, Picked::Cancelled);
    assert!(!after_ran(&app), "Cancel stops the action: the JPEG save wasn't the dialog's");
    assert_eq!(app.ui.status, "Action playback stopped");

    // A save journaled by another request while the step's own JPEG Options is open isn't the
    // step's: Cancel in JPEG Options still stops.
    let (mut app, files, written) = setup(("file.saveAs", json!({"path": "/out/played.psd"})));
    play(&mut app);
    answer(&mut app, &files, chose("/out/chosen.jpg"));
    assert!(app.save_options_open());
    app.session.journal.push((crate::export_dialog::COMMAND.into(), json!({"path": "/elsewhere/web.png"})));
    let ctx = app.ctx().clone();
    let id = app.framework.dialogs.open_dialogs().last().map(|d| d.id).expect("JPEG Options is open");
    app.framework.dialogs.cancel_by_id(&ctx, id);
    crate::format_prompts::take(&mut app, &ctx);
    save_dialog::tick(&mut app);
    assert!(written.borrow().is_empty() && !after_ran(&app), "Cancel stops the action: that save wasn't the step's");
    assert_eq!(app.ui.status, "Action playback stopped");

    // The same with another Export As finishing while the step's save dialog is open.
    let (mut app, files, _written) = setup(("file.saveAs", json!({"path": "/out/played.psd"})));
    play(&mut app);
    save_dialog::exported(&mut app);
    answer(&mut app, &files, Picked::Cancelled);
    assert!(!after_ran(&app), "Cancel stops the action: that export wasn't the step's");
    assert_eq!(app.ui.status, "Action playback stopped");
}

/// Answers the open JPEG Options with OK, then lets the next frame see the dialogs end.
fn confirm_jpeg_options(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    crate::format_prompts::confirm(app, &ctx);
    save_dialog::tick(app);
}

#[test]
fn a_format_options_write_is_checked_again_when_it_is_confirmed() {
    // Another document came to the front while JPEG Options was open.
    let (mut app, files, written) = setup(("file.saveAs", json!({"path": "/out/played.psd"})));
    play(&mut app);
    answer(&mut app, &files, chose("/out/chosen.jpg"));
    assert!(app.save_options_open());
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    confirm_jpeg_options(&mut app);
    assert!(written.borrow().is_empty(), "nothing is written for the step's document once another is active");
    assert!(!after_ran(&app));
    assert!(app.ui.status_error && app.ui.status.contains("active document changed"), "{}", app.ui.status);

    // A gate in force by the time JPEG Options is confirmed refuses the write.
    let (mut app, files, written) = setup(("file.saveACopy", json!({"path": "/out/copy.psd"})));
    play(&mut app);
    answer(&mut app, &files, chose("/outside/copy.jpg"));
    assert!(app.save_options_open());
    app.session.authorize = Some(deny_outside);
    confirm_jpeg_options(&mut app);
    assert!(written.borrow().is_empty(), "the current gate judges the format options' write");
    assert!(!after_ran(&app));
    assert!(app.ui.status_error && app.ui.status.contains("denied"), "{}", app.ui.status);
    assert!(app.paused_playbacks.paused.is_empty() && app.paused_playbacks.save.is_none());
}

#[test]
fn a_format_prompt_opens_on_the_steps_recorded_answer() {
    for (id, recorded, chosen) in
        [("file.saveAs", existing("played.jpg"), existing("chosen.jpg")), ("file.saveACopy", existing("copy.jpg"), existing("chosen copy.jpg"))]
    {
        let (mut app, files, written) = setup((id, json!({"path": recorded, "quality": 3, "jpegFormat": "progressive"})));
        play(&mut app);
        answer(&mut app, &files, chose(&chosen));
        assert!(app.save_options_open(), "{id}: JPEG Options asks");
        assert_eq!(newest_dialog_field(&app, "quality"), json!(3), "{id}: JPEG Options starts at the recorded quality, as the silent replay writes it");
        confirm_jpeg_options(&mut app);
        assert_eq!(*written.borrow(), [chosen.as_str()]);
        assert!(after_ran(&app), "{id}: {}", app.ui.status);
    }
}

#[test]
fn a_recorded_folder_that_is_gone_isnt_where_the_dialog_starts() {
    let gone = std::env::temp_dir().join("photocraft-15945-no-such-folder");
    assert!(!gone.exists());
    for id in ["file.saveAs", "file.saveACopy"] {
        let (mut app, files, _written) = setup((id, json!({"path": gone.join("played.psd").to_string_lossy()})));
        play(&mut app);
        let (folder, name) = suggested(&files);
        assert_ne!(folder, Some(gone.clone()), "{id}: the dialog doesn't start in a folder that is gone");
        assert_eq!(name.as_deref(), Some("played.psd"), "{id}: it still suggests the recorded name");
    }
}
