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
    fn execute(&mut self, id: &str, params: Value) -> Result<Value, String> {
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
