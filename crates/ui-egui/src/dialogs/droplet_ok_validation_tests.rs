//! #13715: OK in Create Droplet says what is missing and keeps the dialog open. The reference
//! application checks, in this order, the droplet file, the destination folder and, with Errors on
//! Log Errors to File, the log file, and stops at the first that fails: one alert with its words, the
//! dialog still open with every value as typed, and nothing written. On this base (the stack of
//! #13713) the destination is the form's own Output folder row, so the folder check reads that row;
//! once #13711's Destination dropdown lands, the same words cover "Destination is Folder and no
//! folder chosen". An empty File Naming row (#13712) is not measured here and keeps today's answer.

use craft::automation::actions::{Action, ActionStep};
use craft::automation::ui::actions::ActionSelection;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use photocraft_engine::actions_cmds::ActionsExt;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The reference application's words for each missing choice.
const NO_DROPLET_FILE: &str = "You must choose a file to save the droplet in.";
const NO_DESTINATION_FOLDER: &str = "You must choose a destination folder.";
const NO_ERROR_LOG: &str = "You must specify a file to log errors to.";

/// An app with a document and one action: the Create Droplet form opens on the selected action.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 4, "height": 3})).unwrap();
    app.session.actions_mut().list.push(Action::new("Invert", vec![ActionStep::new("image.adjustments.invert", json!({}))])).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Action(0));
    app
}

/// Opens Create Droplet with the form the menu opens.
fn open(app: &mut PhotocraftApp) -> DialogId {
    super::file_forms::open_create_droplet(app).expect("an action is selected")
}

fn set(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId, field: &str, value: Value) {
    app.framework.dialogs.set_json_field(ctx, id, field, value).unwrap_or_else(|e| panic!("{field}: {e}"));
}

/// The form's values, as the open dialog shows them.
fn values(app: &PhotocraftApp, id: DialogId) -> Vec<(String, Option<String>)> {
    let dialog = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("the Create Droplet form is open");
    ["path", "name", "action", "output", "format", "errors", "errorLog"]
        .iter()
        .map(|key| ((*key).to_owned(), dialog.fields().text(key).map(str::to_owned)))
        .collect()
}

/// The message of one field of the open form.
fn value(app: &PhotocraftApp, id: DialogId, key: &str) -> Option<String> {
    values(app, id).into_iter().find(|(at, _)| at == key).and_then(|(_, value)| value)
}

/// The id of the open alert, if one is open. A hosted alert is keyed "alert" and the host keeps one
/// at a time, so this is the alert the refusal raised.
fn alert_id(app: &PhotocraftApp) -> Option<DialogId> {
    app.framework.dialogs.open_dialogs().into_iter().find(|d| d.key.as_deref() == Some("alert")).map(|d| d.id)
}

/// The messages of the open alerts.
fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.framework
        .dialogs
        .open_dialogs()
        .into_iter()
        .filter(|d| d.key.as_deref() == Some("alert"))
        .filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_string))
        .collect()
}

/// The files under `dir`, recursively, named relative to it.
fn written(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).expect("the folder can be read") {
            let path = entry.expect("the entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path.strip_prefix(dir).expect("under the folder").display().to_string());
            }
        }
    }
    out.sort();
    out
}

/// #13715: with no droplet file OK is refused with the reference's alert. The dialog stays open with
/// its values, and nothing is written: neither in the chosen output folder nor at the path the form
/// opened with, so neither a `.pcdroplet` nor a launcher appears.
#[test]
fn ok_without_a_droplet_file_alerts_and_writes_nothing() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out").to_string_lossy().into_owned();
    let id = open(&mut app);
    let opened_at = value(&app, id, "path").expect("the form opens with a droplet path");
    set(&mut app, &ctx, id, "output", json!(output));
    set(&mut app, &ctx, id, "path", json!(""));
    assert_eq!(crate::dialogs::confirm(&mut app, &ctx, id), Err(NO_DROPLET_FILE.to_string()), "OK is refused with the reference's words");
    assert_eq!(alerts(&app), [NO_DROPLET_FILE], "one alert carrying these words (its title is Create Droplet)");
    assert_eq!(value(&app, id, "path").as_deref(), Some(""), "the emptied path stays as typed");
    assert_eq!(value(&app, id, "output").as_deref(), Some(output.as_str()), "the folder stays as typed");
    assert_eq!(value(&app, id, "name").as_deref(), Some("Invert"), "and the other values are untouched");
    assert!(written(dir.path()).is_empty(), "nothing was written: {:?}", written(dir.path()));
    assert!(!std::path::Path::new(&opened_at).exists(), "nor at the path the form opened with: {opened_at}");
}

/// #13715: with neither a droplet file nor a destination folder only the droplet-file alert shows:
/// the checks run in the measured order.
#[test]
fn the_droplet_file_is_checked_before_the_destination_folder() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let dir = tempfile::tempdir().unwrap();
    let id = open(&mut app);
    set(&mut app, &ctx, id, "path", json!(""));
    set(&mut app, &ctx, id, "output", json!(""));
    assert_eq!(crate::dialogs::confirm(&mut app, &ctx, id), Err(NO_DROPLET_FILE.to_string()));
    assert_eq!(alerts(&app), [NO_DROPLET_FILE], "one alert, and not the folder's");
    assert!(written(dir.path()).is_empty(), "nothing was written: {:?}", written(dir.path()));
}

/// #13715: a chosen droplet file and no destination folder gives the folder's alert and writes
/// neither the droplet nor its output folder.
#[test]
fn ok_without_a_destination_folder_alerts_and_writes_nothing() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let dir = tempfile::tempdir().unwrap();
    let droplet = dir.path().join("Invert").to_string_lossy().into_owned();
    let id = open(&mut app);
    set(&mut app, &ctx, id, "path", json!(droplet));
    set(&mut app, &ctx, id, "output", json!(""));
    assert_eq!(crate::dialogs::confirm(&mut app, &ctx, id), Err(NO_DESTINATION_FOLDER.to_string()));
    assert_eq!(alerts(&app), [NO_DESTINATION_FOLDER]);
    assert!(written(dir.path()).is_empty(), "nothing was written: {:?}", written(dir.path()));
}

/// #13715 on #13713's Errors dropdown: Log Errors to File with no log file chosen gives the log's
/// alert, the choice stays as picked, and no droplet is written.
#[test]
fn ok_with_log_errors_to_file_and_no_log_file_alerts() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let dir = tempfile::tempdir().unwrap();
    let id = open(&mut app);
    set(&mut app, &ctx, id, "path", json!(dir.path().join("Invert.pcdroplet").to_string_lossy()));
    set(&mut app, &ctx, id, "output", json!(dir.path().join("out").to_string_lossy()));
    set(&mut app, &ctx, id, "errors", json!("log"));
    assert_eq!(crate::dialogs::confirm(&mut app, &ctx, id), Err(NO_ERROR_LOG.to_string()));
    assert_eq!(alerts(&app), [NO_ERROR_LOG]);
    assert_eq!(value(&app, id, "errors").as_deref(), Some("log"), "the Errors choice stays as chosen");
    assert!(written(dir.path()).is_empty(), "nothing was written: {:?}", written(dir.path()));
}

/// #15869 review: spaces are not a log file. The run reads the saved `errorLog` with
/// `filter(|path| !path.trim().is_empty())`, so a whitespace-only path is stored, then ignored, and the
/// droplet silently writes no log - the very thing the Errors row was set to do. OK asks for a real
/// file, with the same words and the same held-OK refusal.
#[test]
fn ok_with_a_whitespace_only_log_path_alerts() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let dir = tempfile::tempdir().unwrap();
    let id = open(&mut app);
    set(&mut app, &ctx, id, "path", json!(dir.path().join("Invert.pcdroplet").to_string_lossy()));
    set(&mut app, &ctx, id, "output", json!(dir.path().join("out").to_string_lossy()));
    set(&mut app, &ctx, id, "errors", json!("log"));
    set(&mut app, &ctx, id, "errorLog", json!("   "));
    assert_eq!(crate::dialogs::confirm(&mut app, &ctx, id), Err(NO_ERROR_LOG.to_string()), "spaces are no log file");
    assert_eq!(alerts(&app), [NO_ERROR_LOG]);
    assert_eq!(value(&app, id, "errorLog").as_deref(), Some("   "), "the typed spaces stay as typed");
    assert!(written(dir.path()).is_empty(), "nothing was written: {:?}", written(dir.path()));
}

/// The blank check refuses nothing else: a name with a space in it, and a name padded with spaces, are
/// log paths the engine keeps verbatim, so OK still accepts them and the value rides into the droplet
/// exactly as typed.
#[test]
fn a_spaced_or_padded_log_path_is_still_accepted() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let dir = tempfile::tempdir().unwrap();
    let droplet = dir.path().join("Invert.pcdroplet").to_string_lossy().into_owned();
    let log = format!("{} ", dir.path().join("drop log.txt").to_string_lossy());
    let id = open(&mut app);
    set(&mut app, &ctx, id, "path", json!(droplet.clone()));
    set(&mut app, &ctx, id, "output", json!(dir.path().join("out").to_string_lossy()));
    set(&mut app, &ctx, id, "errors", json!("log"));
    set(&mut app, &ctx, id, "errorLog", json!(log.clone()));
    crate::dialogs::confirm(&mut app, &ctx, id).expect("a path with spaces in it is a chosen file");
    assert!(alerts(&app).is_empty(), "{:?}", alerts(&app));
    let saved: Value = serde_json::from_slice(&std::fs::read(&droplet).expect("the droplet was written")).unwrap();
    assert_eq!(saved["options"]["errorLog"], json!(log), "stored as typed, not trimmed away");
}

/// The log check belongs to Log Errors to File alone: the form's own Stop for Errors leaves the log
/// field empty as a normal state, and OK writes the droplet.
#[test]
fn stop_for_errors_needs_no_log_file() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let dir = tempfile::tempdir().unwrap();
    let droplet = dir.path().join("Invert.pcdroplet").to_string_lossy().into_owned();
    let id = open(&mut app);
    set(&mut app, &ctx, id, "path", json!(droplet));
    set(&mut app, &ctx, id, "output", json!(dir.path().join("out").to_string_lossy()));
    set(&mut app, &ctx, id, "errors", json!("stop"));
    assert_eq!(value(&app, id, "errorLog").as_deref(), Some(""), "Stop for Errors leaves the log field empty, greyed (#13713)");
    crate::dialogs::confirm(&mut app, &ctx, id).expect("OK writes the droplet");
    assert!(alerts(&app).is_empty(), "{:?}", alerts(&app));
    assert!(std::path::Path::new(&droplet).exists(), "the droplet is written as today");
}

/// #13715: with all three filled in OK writes the droplet as today, with no alert.
#[test]
fn ok_with_everything_filled_in_writes_the_droplet() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let dir = tempfile::tempdir().unwrap();
    let droplet = dir.path().join("Invert.pcdroplet").to_string_lossy().into_owned();
    let log = dir.path().join("logs").join("droplog.txt").to_string_lossy().into_owned();
    let id = open(&mut app);
    set(&mut app, &ctx, id, "path", json!(droplet));
    set(&mut app, &ctx, id, "output", json!(dir.path().join("out").to_string_lossy()));
    set(&mut app, &ctx, id, "errors", json!("log"));
    set(&mut app, &ctx, id, "errorLog", json!(log));
    crate::dialogs::confirm(&mut app, &ctx, id).expect("OK writes the droplet");
    assert!(alerts(&app).is_empty(), "{:?}", alerts(&app));
    let saved: Value = serde_json::from_slice(&std::fs::read(&droplet).expect("the droplet was written")).unwrap();
    assert_eq!(saved["photocraftDroplet"], json!(1), "the droplet as today");
    assert_eq!(saved["action"]["steps"], json!([["image.adjustments.invert", {}]]));
}

/// #13715: OK on the alert closes the alert only. The Create Droplet dialog is still open with every
/// value as it was, and a corrected OK then writes the droplet.
#[test]
fn the_alerts_ok_leaves_the_dialog_open_with_its_values() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let dir = tempfile::tempdir().unwrap();
    let droplet = dir.path().join("Invert.pcdroplet").to_string_lossy().into_owned();
    let id = open(&mut app);
    set(&mut app, &ctx, id, "path", json!(droplet));
    set(&mut app, &ctx, id, "output", json!(""));
    let before = values(&app, id);
    assert!(crate::dialogs::confirm(&mut app, &ctx, id).is_err(), "OK is refused");
    let alert = alert_id(&app).expect("one alert is open");
    assert_eq!(alerts(&app).len(), 1, "one alert, got {:?}", alerts(&app));
    crate::dialogs::confirm(&mut app, &ctx, alert).expect("the alert answers OK");
    assert!(alerts(&app).is_empty(), "OK closed the alert");
    assert_eq!(values(&app, id), before, "the dialog is still open with every value as it was");
    set(&mut app, &ctx, id, "output", json!(dir.path().join("out").to_string_lossy()));
    crate::dialogs::confirm(&mut app, &ctx, id).expect("the corrected OK is accepted");
    assert!(std::path::Path::new(&droplet).exists(), "the droplet is written once the folder is chosen");
}

/// #13715: OK pressed again while the alert is up adds nothing: still one alert with the same words,
/// still nothing written.
#[test]
fn a_repeated_ok_keeps_one_alert() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let dir = tempfile::tempdir().unwrap();
    let id = open(&mut app);
    set(&mut app, &ctx, id, "path", json!(""));
    set(&mut app, &ctx, id, "output", json!(dir.path().join("out").to_string_lossy()));
    for at in 0..3 {
        assert_eq!(crate::dialogs::confirm(&mut app, &ctx, id), Err(NO_DROPLET_FILE.to_string()), "OK {at} is refused");
        assert_eq!(alerts(&app), [NO_DROPLET_FILE], "still one alert after OK {at}");
        let alert = alert_id(&app).expect("the alert is open");
        crate::dialogs::confirm(&mut app, &ctx, alert).expect("OK on the alert");
    }
    assert!(written(dir.path()).is_empty(), "nothing was written: {:?}", written(dir.path()));
}

/// The check is Create Droplet's alone: another form of the same family keeps today's answer, an OK
/// that closes and runs the command. Contact Sheet II with an emptied Input row is not measured, so
/// it stays as it is.
#[test]
fn the_other_forms_of_the_family_are_not_checked() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let id = super::file_forms::open_contact_sheet(&mut app);
    set(&mut app, &ctx, id, "input", json!(""));
    crate::dialogs::confirm(&mut app, &ctx, id).expect("Create Droplet's check does not answer for another form");
    assert!(alerts(&app).is_empty(), "no alert on another form: {:?}", alerts(&app));
    assert!(app.framework.dialogs.get_by_id::<HostedParamDialog>(id).is_none(), "the dialog closed as today");
}

/// The droplet-file check asks whether a file was chosen, not what it is named: a path without an
/// extension is a chosen file, and the engine still appends `.pcdroplet` as today.
#[test]
fn a_path_without_an_extension_is_a_chosen_file() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let dir = tempfile::tempdir().unwrap();
    let droplet = dir.path().join("Invert").to_string_lossy().into_owned();
    let id = open(&mut app);
    set(&mut app, &ctx, id, "path", json!(droplet));
    set(&mut app, &ctx, id, "output", json!(dir.path().join("out").to_string_lossy()));
    crate::dialogs::confirm(&mut app, &ctx, id).expect("OK accepts the chosen path");
    assert!(std::path::Path::new(&format!("{droplet}.pcdroplet")).exists(), "the engine adds the extension, as today");
}
