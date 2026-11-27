//! File › Automate › Batch: OK checks for a missing source folder, then a missing destination
//! folder, as the reference does (#13669). The first that fails refuses OK with the reference's
//! message in a one-button alert titled "Batch"; the dialog stays open with every value, and
//! nothing runs. The same holds for a
//! confirm through the control channel.

use craft::automation::actions::{Action, ActionStep};
use craft::automation::ui::actions::ActionSelection;
use craft::ui::dialog::DialogId;
use photocraft_engine::actions_cmds::ActionsExt;
use serde_json::{Value, json};

use super::{NO_DESTINATION_FOLDER, NO_SOURCE_FOLDER};
use crate::PhotocraftApp;
use crate::control::{ControlRequest, Reply, handle};

const BATCH: &str = "file.automate.batch";

/// The Batch dialog, opened from its menu over a 2 × 1 document with the action Invert selected
/// in the Actions panel, after the frame that loads its defaults; `input` and `output` set to the
/// given paths.
fn batch(input: &str, output: &str) -> (Box<PhotocraftApp>, egui::Context, DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 2, "height": 1})).unwrap();
    app.session.actions_mut().list.push(Action::new("Invert", vec![ActionStep::new("image.adjustments.invert", json!({}))])).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Action(0));
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(&mut app, &ctx, BATCH, json!({})).expect("the Batch dialog opens");
    let want = reply["dialog"].as_str().expect("the reply names a dialog").to_owned();
    let id = app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|id| id.value().to_string() == want).expect("the dialog is open");
    let mut output_frame = ctx.run_ui(Default::default(), |ui| {
        let mut frame = eframe::Frame::_new_kittest();
        app.logic_frame(ui.ctx(), &mut frame);
        app.ui_frame(ui, &mut frame);
    });
    output_frame.textures_delta.clear();
    set(&mut app, &ctx, id, "input", json!(input));
    set(&mut app, &ctx, id, "output", json!(output));
    (app, ctx, id)
}

/// Sets the dialog's field `key` as automation does.
fn set(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId, key: &str, value: Value) {
    app.framework.dialogs.set_json_field(ctx, id, key, value).unwrap_or_else(|e| panic!("{key}: {e}"));
}

/// The values the open dialog `id` lists; `None` once it closed.
fn fields(app: &PhotocraftApp, id: DialogId) -> Option<Value> {
    app.framework.dialogs.open_dialogs().into_iter().find(|open| open.id == id).and_then(|open| open.fields)
}

/// The messages of the alerts the host has open (`HostedAlert`'s key is "alert").
fn alerts(app: &PhotocraftApp) -> Vec<String> {
    let open = app.framework.dialogs.open_dialogs().into_iter().filter(|d| d.key.as_deref() == Some("alert"));
    open.filter_map(|d| Some(d.fields.as_ref()?["message"].as_str()?.to_string())).collect()
}

/// A folder holding one 2 × 1 PNG, `in/a.png`, and the path of an output folder `out` beside it.
fn image_folder() -> (tempfile::TempDir, String, String) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("in")).unwrap();
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 2, "height": 1})).unwrap();
    session.execute("file.saveACopy", json!({"path": dir.path().join("in/a.png").to_string_lossy()})).unwrap();
    let input = dir.path().join("in").to_string_lossy().into_owned();
    let output = dir.path().join("out").to_string_lossy().into_owned();
    (dir, input, output)
}

#[test]
fn ok_without_a_source_folder_is_refused_and_keeps_the_dialog_open() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (dir, _, output) = image_folder();
        let (mut app, ctx, id) = batch("", &output);
        let before = fields(&app, id).expect("the dialog is open");
        assert_eq!(crate::dialogs::confirm(&mut app, &ctx, id), Err(NO_SOURCE_FOLDER.to_string()));
        assert_eq!(NO_SOURCE_FOLDER, "You must choose a source folder.");
        assert_eq!(alerts(&app), [NO_SOURCE_FOLDER], "the refusal shows in an alert");
        assert_eq!(fields(&app, id), Some(before), "the dialog stays open with every value");
        assert!(!dir.path().join("out").exists(), "nothing was written");
        assert_eq!(app.session.documents().len(), 1, "nothing was opened");
    });
}

#[test]
fn ok_without_a_destination_folder_is_refused_and_writes_nothing() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (dir, input, _) = image_folder();
        let (mut app, ctx, id) = batch(&input, "");
        let before = fields(&app, id).expect("the dialog is open");
        assert_eq!(crate::dialogs::confirm(&mut app, &ctx, id), Err(NO_DESTINATION_FOLDER.to_string()));
        assert_eq!(NO_DESTINATION_FOLDER, "You must choose a destination folder.");
        assert_eq!(alerts(&app), [NO_DESTINATION_FOLDER], "the refusal shows in an alert");
        assert_eq!(fields(&app, id), Some(before));
        let files: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(files, ["in"], "nothing was written");
    });
}

/// The checks run in the reference's order: with both folders missing, the source one is named.
/// A path of blanks names no folder either.
#[test]
fn the_source_folder_is_checked_before_the_destination() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        for (input, output, message) in [("", "", NO_SOURCE_FOLDER), ("  ", "", NO_SOURCE_FOLDER), ("in", "  ", NO_DESTINATION_FOLDER)] {
            let (mut app, ctx, id) = batch(input, output);
            assert_eq!(crate::dialogs::confirm(&mut app, &ctx, id), Err(message.to_string()), "input {input:?}, output {output:?}");
            assert!(fields(&app, id).is_some(), "the dialog stays open");
        }
    });
}

#[test]
fn a_confirm_over_the_control_channel_gets_the_same_check() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (mut app, ctx, id) = batch("", "");
        let before = fields(&app, id).expect("the dialog is open");
        let dialog = id.value().to_string();
        // Repeated confirms answer the same refusal.
        for _ in 0..2 {
            let (confirm, _rx) = ControlRequest::new("ui.dialog.confirm", json!({"dialog": dialog}));
            let Reply::Done(reply) = handle(&mut app, &ctx, &confirm) else { panic!("ui.dialog.confirm replies at once") };
            assert_eq!(reply["ok"], false, "{reply}");
            assert!(reply.to_string().contains(NO_SOURCE_FOLDER), "{reply}");
        }
        assert_eq!(fields(&app, id), Some(before));

        // With the source set, the destination is checked next.
        set(&mut app, &ctx, id, "input", json!("in"));
        let (confirm, _rx) = ControlRequest::new("ui.dialog.confirm", json!({"dialog": dialog}));
        let Reply::Done(reply) = handle(&mut app, &ctx, &confirm) else { panic!("ui.dialog.confirm replies at once") };
        assert!(reply.to_string().contains(NO_DESTINATION_FOLDER), "{reply}");
    });
}

/// With both folders chosen OK runs the batch and closes the dialog.
#[test]
fn ok_with_both_folders_runs_the_batch() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (dir, input, output) = image_folder();
        let (mut app, ctx, id) = batch(&input, &output);
        crate::dialogs::confirm(&mut app, &ctx, id).expect("OK is accepted");
        assert!(fields(&app, id).is_none(), "OK closes the dialog");
        assert!(alerts(&app).is_empty(), "an accepted OK shows no alert");
        assert!(dir.path().join("out/a.png").is_file(), "the batch wrote a.png");
    });
}

/// Opened Files reads no folder, and without an output folder it leaves the documents open and
/// changed: no check applies.
#[test]
fn opened_files_without_folders_is_not_refused() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (mut app, ctx, id) = batch("", "");
        set(&mut app, &ctx, id, "source", json!("open"));
        crate::dialogs::confirm(&mut app, &ctx, id).expect("OK is accepted");
        assert!(fields(&app, id).is_none(), "OK closes the dialog");
        assert_eq!(app.session.documents().len(), 1, "the document stays open");
    });
}
