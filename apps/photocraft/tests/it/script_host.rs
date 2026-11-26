//! The desktop script service runs a script end to end in the script host PhotoCraft ships:
//! started as the app starts it, its commands answered from a session, its dialog answered, its
//! output and result reported, and a run stopped; and it says when the host is missing.

use std::path::Path;
use std::time::{Duration, Instant};

use photocraft_engine::Session;
use photocraft_ui_egui::{ScriptCommands, ScriptDialog, ScriptEvent, ScriptRun, ScriptStart};
use serde_json::{Value, json};

#[path = "../../src/scripts.rs"]
mod scripts;

/// Nextest archives relocate binaries (and name the variable with `_` for `-`); plain Cargo keeps
/// the compile-time path valid.
fn helper() -> std::path::PathBuf {
    std::env::var_os("NEXTEST_BIN_EXE_photocraft_script_host").unwrap_or_else(|| env!("CARGO_BIN_EXE_photocraft-script-host").into()).into()
}

/// The run's commands, answered by a session as the app's command path would.
struct Commands(Session);

impl ScriptCommands for Commands {
    fn execute(&mut self, id: &str, mut params: Value) -> Result<Value, String> {
        // A command naming its document runs there, as the app runs it.
        if let Some(serial) = photocraft_script_js::commands::take_document(&mut params)
            && !self.0.active().is_some_and(|d| d.serial == serial)
        {
            self.0.execute("document.activate", json!({ "serial": serial })).map_err(|e| e.to_string())?;
        }
        self.0.execute(id, params).map_err(|e| e.to_string())
    }
}

fn start(source: &str) -> Box<dyn ScriptRun> {
    let start = ScriptStart { name: "e2e.jsx".into(), source: source.into(), app_name: None };
    scripts::start_run(helper(), start).unwrap_or_else(|e| panic!("{e}"))
}

/// Pump `run` as the interface does, a frame at a time, until it shows a dialog or ends.
fn pump_until_it_asks(run: &mut dyn ScriptRun, commands: &mut Commands, lines: &mut Vec<String>) -> ScriptEvent {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        assert!(Instant::now() < deadline, "the run neither asked nor ended");
        for event in run.pump(commands, Duration::from_millis(8)) {
            match event {
                ScriptEvent::Output(line) => lines.push(line),
                other => return other,
            }
        }
    }
}

#[test]
fn the_script_service_runs_a_script_with_its_commands_and_dialog_in_the_shipped_host() {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 16, "height": 16, "name": "Card"})).unwrap();
    let mut commands = Commands(session);
    let mut run = start(
        "var n = app.documents.length + ' ' + app.activeDocument.name;
         var ok = confirm('Go on?');
         $.writeln(n, ' ', ok);",
    );
    let mut lines = Vec::new();
    let asked = pump_until_it_asks(&mut *run, &mut commands, &mut lines);
    assert_eq!(asked, ScriptEvent::Dialog { kind: ScriptDialog::Confirm, args: json!(["Go on?"]) });
    // At its dialog the run sends nothing until it is answered.
    assert!(run.pump(&mut commands, Duration::from_millis(50)).is_empty());
    run.answer(json!(true)).unwrap();
    let ended = pump_until_it_asks(&mut *run, &mut commands, &mut lines);
    assert_eq!(ended, ScriptEvent::Finished(Ok(())));
    assert_eq!(lines, ["1 Card true"]);
}

/// A run writes only where the user chose in one of its own dialogs: the app holds that, not the
/// script host, so the file the user picked is written and one the script names itself is not.
#[test]
fn a_run_writes_only_the_file_its_user_chose() {
    let dir = tempfile::tempdir().unwrap();
    let chosen = dir.path().join("chosen.png");
    let other = dir.path().join("other.png");
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 16, "height": 16, "name": "Card"})).unwrap();
    let mut commands = Commands(session);
    let source = format!(
        "var f = File.saveDialog('Save the card');
         app.activeDocument.saveAs(f, new PNGSaveOptions(), true);
         try {{ app.activeDocument.saveAs(new File({other}), new PNGSaveOptions(), true); $.writeln('wrote'); }} catch (e) {{ $.writeln('refused'); }}",
        other = serde_json::to_string(&other.to_string_lossy()).unwrap()
    );
    let mut run = start(&source);
    let mut lines = Vec::new();
    let asked = pump_until_it_asks(&mut *run, &mut commands, &mut lines);
    assert!(matches!(asked, ScriptEvent::Dialog { kind: ScriptDialog::SaveFile, .. }), "{asked:?}");
    run.answer(json!(chosen.to_string_lossy())).unwrap();
    let ended = pump_until_it_asks(&mut *run, &mut commands, &mut lines);
    assert_eq!(ended, ScriptEvent::Finished(Ok(())), "{lines:?}");
    assert_eq!(lines, ["refused"]);
    assert!(chosen.is_file(), "the chosen file was written");
    assert!(!other.exists(), "the file the script named itself was not");
}

/// A folder the user chose takes new files, but the app doesn't let the run replace one that was
/// already in it.
#[test]
fn a_run_adds_files_to_a_chosen_folder_but_does_not_replace_its_files() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old.png");
    std::fs::write(&old, b"before").unwrap();
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 16, "height": 16, "name": "Card"})).unwrap();
    let mut commands = Commands(session);
    let mut run = start(
        "var f = Folder.selectDialog('Export to');
         app.activeDocument.saveAs(new File(f.fsName + '/new.png'), new PNGSaveOptions(), true);
         try { app.activeDocument.saveAs(new File(f.fsName + '/old.png'), new PNGSaveOptions(), true); $.writeln('replaced'); } catch (e) { $.writeln('kept'); }",
    );
    let mut lines = Vec::new();
    let asked = pump_until_it_asks(&mut *run, &mut commands, &mut lines);
    assert!(matches!(asked, ScriptEvent::Dialog { kind: ScriptDialog::SelectFolder, .. }), "{asked:?}");
    run.answer(json!(dir.path().to_string_lossy())).unwrap();
    let ended = pump_until_it_asks(&mut *run, &mut commands, &mut lines);
    assert_eq!(ended, ScriptEvent::Finished(Ok(())), "{lines:?}");
    assert_eq!(lines, ["kept"]);
    assert!(dir.path().join("new.png").is_file(), "the new file was written");
    assert_eq!(std::fs::read(&old).unwrap(), b"before", "the file already there was not replaced");
}

/// The app finds a command's document by the parameter the script host names it with: the two
/// crates must spell it the same, or every edit would run on whichever document is active.
#[test]
fn the_app_and_the_script_host_name_a_commands_document_alike() {
    assert_eq!(photocraft_ui_egui::ON_DOCUMENT, photocraft_script_js::commands::ON_DOCUMENT);
}

#[test]
fn a_cancelled_run_ends_as_failed() {
    let mut commands = Commands(Session::new());
    let mut run = start("for (;;) {}");
    run.cancel();
    let ended = pump_until_it_asks(&mut *run, &mut commands, &mut Vec::new());
    assert!(matches!(ended, ScriptEvent::Finished(Err(_))), "{ended:?}");
}

#[test]
fn a_missing_script_host_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let error = scripts::helper_beside(&dir.path().join("photocraft")).unwrap_err();
    assert!(error.contains("missing"), "{error}");
    // The service looks beside its own executable, where a test binary has no host.
    let start = ScriptStart { name: "e2e.jsx".into(), source: "1".into(), app_name: None };
    let mut service = scripts::service();
    assert!(service(start).is_err_and(|e| e.contains("missing")));
}

#[test]
fn the_helper_is_found_beside_the_app() {
    let helper = helper();
    let found = scripts::helper_beside(&helper.with_file_name("photocraft")).unwrap();
    assert_eq!(found, helper);
    assert!(Path::new(&found).is_file());
}
