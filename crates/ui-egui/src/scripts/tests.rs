//! Script runs through a fake script service: the commands they run, their dialogs, Stop, their
//! output, and the menu.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_engine::Session;
use serde_json::{Value, json};

use super::*;
use crate::control::{ControlRequest, Reply, handle};
use craft::host::{Picked, ScriptedFiles};

/// What a fake run does, in order.
enum Act {
    /// Run a session command (as a script's host call does).
    Run(&'static str, Value),
    /// Hand the interface an event; a dialog waits for its answer.
    Emit(ScriptEvent),
    /// Run a session command that may be refused; a refusal is written as output.
    Try(&'static str, Value),
    /// Keep running: never end on its own.
    Spin,
}

#[derive(Clone, Default)]
struct Log {
    starts: Rc<RefCell<Vec<ScriptStart>>>,
    answers: Rc<RefCell<Vec<Value>>>,
    /// How many times a run was cancelled.
    cancels: Rc<Cell<usize>>,
    /// The latest run's own stop flag, for a test to end that run as a dying host would.
    latest: Rc<RefCell<Option<Rc<Cell<bool>>>>>,
}

impl Log {
    /// End the latest run on its next pump, as a host that died would.
    fn end_latest(&self) {
        if let Some(stopped) = self.latest.borrow().as_ref() {
            stopped.set(true);
        }
    }
}

struct Fake {
    acts: VecDeque<Act>,
    waiting: bool,
    log: Log,
    /// This run's own: a cancelled run doesn't end the next one.
    stopped: Rc<Cell<bool>>,
}

impl ScriptRun for Fake {
    fn pump(&mut self, commands: &mut dyn ScriptCommands, _: Duration) -> Vec<ScriptEvent> {
        if self.stopped.get() {
            return vec![ScriptEvent::Finished(Err("The script was cancelled.".into()))];
        }
        let mut out = Vec::new();
        while !self.waiting {
            match self.acts.pop_front() {
                None => {
                    out.push(ScriptEvent::Finished(Ok(())));
                    break;
                }
                Some(Act::Spin) => {
                    self.acts.push_front(Act::Spin);
                    break;
                }
                Some(Act::Run(id, params)) => {
                    commands.execute(id, params).unwrap();
                }
                Some(Act::Try(id, params)) => {
                    if let Err(e) = commands.execute(id, params) {
                        out.push(ScriptEvent::Output(e));
                    }
                }
                Some(Act::Emit(e)) => {
                    let ends = matches!(e, ScriptEvent::Finished(_));
                    self.waiting = matches!(e, ScriptEvent::Dialog { .. });
                    out.push(e);
                    if ends {
                        break;
                    }
                }
            }
        }
        out
    }

    fn answer(&mut self, answer: Value) -> Result<(), String> {
        self.waiting = false;
        self.log.answers.borrow_mut().push(answer);
        Ok(())
    }

    fn cancel(&mut self) {
        self.stopped.set(true);
        self.log.cancels.set(self.log.cancels.get() + 1);
    }
}

/// A script service whose runs do `programs` in turn.
fn service(programs: Vec<Vec<Act>>, log: &Log) -> StartScriptFn {
    let mut programs: VecDeque<Vec<Act>> = programs.into();
    let log = log.clone();
    Box::new(move |start| {
        log.starts.borrow_mut().push(start);
        let stopped = Rc::new(Cell::new(false));
        *log.latest.borrow_mut() = Some(stopped.clone());
        Ok(Box::new(Fake { acts: programs.pop_front().unwrap_or_default().into(), waiting: false, log: log.clone(), stopped }) as Box<dyn ScriptRun>)
    })
}

fn app(programs: Vec<Vec<Act>>, log: &Log) -> Box<PhotocraftApp> {
    PhotocraftApp::new(Session::new(), crate::Services { scripts: Some(service(programs, log)), ..Default::default() })
}

fn start_named(app: &mut PhotocraftApp, name: &str) {
    start(app, ScriptStart { name: name.into(), source: String::new(), app_name: None }).unwrap();
}

/// Pump until the run waits at a dialog or ends.
fn settle(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    for _ in 0..16 {
        tick(app, &ctx);
        if app.scripts.run.as_ref().is_none_or(|a| a.dialog.is_some()) {
            return;
        }
    }
    panic!("the run neither ended nor waited at a dialog");
}

/// The dialog the run waits at.
fn open_id(app: &PhotocraftApp) -> DialogId {
    app.scripts.run.as_ref().and_then(|a| a.dialog).expect("the run waits at a dialog").id
}

/// A control-channel request, answered at once.
fn call(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let ctx = egui::Context::default();
    let (req, _rx) = ControlRequest::new(method, params);
    match handle(app, &ctx, &req) {
        Reply::Done(v) => v,
        _ => panic!("{method}: expected an immediate reply"),
    }
}

fn layer_names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.walk().iter().map(|(_, _, l)| l.name.clone()).filter(|n| n != "Background").collect()
}

fn dialog(kind: ScriptDialog, args: Value) -> Act {
    Act::Emit(ScriptEvent::Dialog { kind, args })
}

fn harness(programs: Vec<Vec<Act>>, log: &Log, answers: Vec<Picked>) -> Harness<'static, Box<PhotocraftApp>> {
    let files = Arc::new(ScriptedFiles::answering(answers));
    let scripts = service(programs, log);
    Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(Session::new(), crate::Services { files: Some(files), scripts: Some(scripts), ..Default::default() })
    })
}

#[test]
fn a_run_executes_its_commands_waits_at_a_dialog_and_reports_the_end() {
    let log = Log::default();
    let program = vec![
        Act::Run("file.new", json!({"width": 16, "height": 16})),
        Act::Emit(ScriptEvent::Output("hello".into())),
        dialog(ScriptDialog::Alert, json!(["Hi"])),
        Act::Run("layer.new.layer", json!({"name": "after the alert"})),
    ];
    let mut app = app(vec![program], &log);
    start_named(&mut app, "greet.jsx");
    assert!(start(&mut app, ScriptStart { name: "second.jsx".into(), source: String::new(), app_name: None }).is_err(), "one run at a time");
    settle(&mut app);
    assert!(app.scripts.running());
    assert!(app.scripts.log.iter().any(|l| l == "hello"));
    assert!(app.scripts.output_open, "output shows as it comes");
    assert_eq!(app.session.documents().len(), 1);
    assert!(layer_names(&app).is_empty(), "the run waits at the alert");
    let id = open_id(&app);
    crate::dialogs::confirm(&mut app, &egui::Context::default(), id).unwrap();
    settle(&mut app);
    assert!(!app.scripts.running());
    assert_eq!(layer_names(&app), ["after the alert"]);
    assert_eq!(app.ui.status, "Ran script greet.jsx");
    assert_eq!(*log.answers.borrow(), [Value::Null]);
}

#[test]
fn a_failed_run_reports_its_error_in_the_status_and_the_output() {
    let log = Log::default();
    let failing = vec![Act::Emit(ScriptEvent::Finished(Err("ReferenceError: nope is not defined (bad.jsx:1:1)\n    at <main>".into())))];
    let mut app = app(vec![failing], &log);
    start_named(&mut app, "bad.jsx");
    settle(&mut app);
    assert!(app.ui.status_error);
    assert_eq!(app.ui.status, "Script bad.jsx: ReferenceError: nope is not defined (bad.jsx:1:1)");
    assert!(app.scripts.output_open && app.scripts.log.iter().any(|l| l.contains("at <main>")), "{:?}", app.scripts.log);
}

/// A run busy without a dialog offers Stop; Stop cancels it, and it ends as cancelled.
#[test]
fn stop_cancels_a_busy_run() {
    let log = Log::default();
    let mut h = harness(vec![vec![Act::Spin]], &log, vec![]);
    start_named(h.state_mut(), "slow.jsx");
    h.state_mut().scripts.run.as_mut().unwrap().started_ms -= STOP_AFTER_MS;
    h.run_steps(4);
    h.get_by_label("Stop").click();
    h.run_steps(4);
    assert_eq!(log.cancels.get(), 1);
    assert!(!h.state().scripts.running());
    assert!(h.state().ui.status.contains("cancelled"), "{}", h.state().ui.status);
}

/// While a script's dialog is open the app runs no other command, as under any modal dialog:
/// the control channel's menu clicks included.
#[test]
fn a_scripts_dialog_holds_back_other_commands() {
    let log = Log::default();
    let mut app = app(vec![vec![dialog(ScriptDialog::Confirm, json!(["Go?"]))]], &log);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let add = craft::keymap::CommandRef::new("layer.new.layer");
    assert!(crate::menus::allows(&app, &add), "allowed before the dialog");
    start_named(&mut app, "ask.jsx");
    settle(&mut app);
    assert!(!crate::menus::allows(&app, &add), "held back while it is open");
    let id = open_id(&app);
    crate::dialogs::confirm(&mut app, &egui::Context::default(), id).unwrap();
    settle(&mut app);
    assert!(crate::menus::allows(&app, &add), "allowed again once answered");
}

/// While the app is in another dialog, a run's next command waits, as a click would; it runs
/// once the dialog is gone.
#[test]
fn a_runs_commands_wait_while_another_dialog_is_open() {
    let log = Log::default();
    let mut app = app(vec![vec![Act::Run("layer.new.layer", json!({"name": "later"}))]], &log);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let other = app.framework.dialogs.open(StandardPrompt::custom("Busy").button(PromptChoice::Proceed, "OK"));
    start_named(&mut app, "wait.jsx");
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    tick(&mut app, &ctx);
    assert!(layer_names(&app).is_empty(), "the command waits under the dialog");
    assert!(app.scripts.running());
    app.framework.dialogs.cancel_by_id(&ctx, other);
    settle(&mut app);
    assert_eq!(layer_names(&app), ["later"]);
}

/// A script's dialog always says that a script opened it, and which: a title the script gives
/// can't make it pass for one of the app's own.
#[test]
fn a_scripts_dialog_names_its_script_whatever_its_title() {
    let log = Log::default();
    let mut h = harness(vec![vec![dialog(ScriptDialog::Alert, json!(["Your session expired.", "Security notice"]))]], &log, vec![]);
    start_named(h.state_mut(), "phish.jsx");
    h.run_steps(4);
    assert!(h.query_all_by_label("Script: phish.jsx – Security notice").next().is_some(), "the title names the script");
    assert!(h.query_by_label("Security notice").is_none(), "the script's title never stands alone");
    let _ = h.get_by_label("Your session expired.");
    h.get_by_label("OK").click();
    h.run_steps(4);
    assert_eq!(*log.answers.borrow(), [Value::Null]);
}

/// Automation answers a script's dialogs as any other: a confirmation's default (No when the
/// script asks for it), a prompt's text, a cancelled prompt.
#[test]
fn automation_answers_a_scripts_dialogs() {
    let cases = [
        (dialog(ScriptDialog::Confirm, json!(["Go?"])), None, "ui.dialog.confirm", json!(true)),
        (dialog(ScriptDialog::Confirm, json!(["Go?", "true"])), None, "ui.dialog.confirm", json!(false)),
        (dialog(ScriptDialog::Prompt, json!(["Name?", "first"])), Some("typed"), "ui.dialog.confirm", json!("typed")),
        (dialog(ScriptDialog::Prompt, json!(["Name?", "first"])), None, "ui.dialog.cancel", Value::Null),
    ];
    for (asked, text, method, want) in cases {
        let log = Log::default();
        let mut app = app(vec![vec![asked]], &log);
        start_named(&mut app, "ask.jsx");
        settle(&mut app);
        let inspect = call(&mut app, "ui.inspect", json!({}));
        let id = inspect["result"]["dialogs"][0]["id"].clone();
        assert!(!id.is_null(), "listed: {inspect}");
        if let Some(text) = text {
            assert_eq!(call(&mut app, "ui.dialog.set", json!({"dialog": id, "field": "text", "value": text}))["ok"], true);
        }
        assert_eq!(call(&mut app, method, json!({"dialog": id}))["ok"], true, "{method}");
        assert_eq!(*log.answers.borrow(), std::slice::from_ref(&want), "{method} {want}");
        settle(&mut app);
        assert!(!app.scripts.running() && !app.dialog_open());
    }
}

/// An agent whose policy may not run scripts may not answer a script's dialog either: that would
/// let the script go on.
#[test]
fn an_agent_that_may_not_run_scripts_may_not_answer_their_dialogs() {
    let log = Log::default();
    let mut app = app(vec![vec![dialog(ScriptDialog::Confirm, json!(["Go?"]))]], &log);
    start_named(&mut app, "ask.jsx");
    settle(&mut app);
    app.services.automation_command = Some(Box::new(|id, _| if id == "file.scripts.run" { Err("not for agents".into()) } else { Ok(()) }));
    let id = open_id(&app);
    let reply = call(&mut app, "ui.dialog.confirm", json!({"dialog": id.value()}));
    assert_ne!(reply["ok"], true, "{reply}");
    assert!(log.answers.borrow().is_empty());
    assert_eq!(waits_at(&app), Some(id), "the run still waits");
}

/// File › Scripts › Browse… runs the JavaScript file picked; other files stay the engine's.
#[test]
fn browse_runs_the_picked_script() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hello.jsx");
    std::fs::write(&path, "\u{feff}alert('hi')").unwrap();
    let log = Log::default();
    let mut h = harness(vec![vec![]], &log, vec![Picked::Paths(vec![path])]);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.scripts.browse", json!({})).unwrap();
    h.run_steps(4);
    let starts = log.starts.borrow();
    assert_eq!((starts[0].name.as_str(), starts[0].source.as_str()), ("hello.jsx", "alert('hi')"));
}

/// `file.scripts.run` starts a run however it is reached: the menu, or `engine.execute`'s path
/// through the app; so does `file.scripts.browse` of a `.jsx`, and one PhotoCraft can't run is
/// refused with its reason. Without a script service it says no script can run.
#[test]
fn the_run_command_starts_a_script_and_says_when_none_can_run() {
    let dir = tempfile::tempdir().unwrap();
    let (jsx, uxp) = (dir.path().join("browsed.jsx"), dir.path().join("panel.psjs"));
    std::fs::write(&jsx, "1").unwrap();
    std::fs::write(&uxp, "1").unwrap();
    let log = Log::default();
    let mut app = app(vec![vec![], vec![], vec![]], &log);
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "file.scripts.run", json!({"name": "menu.js", "source": "1"})).unwrap();
    settle(&mut app);
    app.run("file.scripts.run", json!({"name": "direct.js", "source": "1"})).unwrap();
    settle(&mut app);
    app.run("file.scripts.browse", json!({"path": jsx})).unwrap();
    settle(&mut app);
    let names: Vec<String> = log.starts.borrow().iter().map(|s| s.name.clone()).collect();
    assert_eq!(names, ["menu.js", "direct.js", "browsed.jsx"]);
    let refused = app.run("file.scripts.browse", json!({"path": uxp})).unwrap_err();
    assert!(refused.contains("UXP"), "{refused}");
    let mut plain = PhotocraftApp::new(Session::new(), crate::Services::default());
    let r = crate::menus::invoke(&mut plain, &ctx, "file.scripts.run", json!({"name": "cmd.js", "source": "1"}));
    assert_eq!(r.unwrap_err(), NO_SCRIPTS);
}

/// An agent's policy decides whether it may start a script at all.
#[test]
fn an_agents_policy_decides_whether_it_starts_a_script() {
    let log = Log::default();
    let mut app = app(vec![vec![]], &log);
    app.services.automation_command = Some(Box::new(|id, _| if id == "file.scripts.run" { Err("not for agents".into()) } else { Ok(()) }));
    app.automation_input = true;
    let r = app.run("file.scripts.run", json!({"name": "agent.js", "source": "1"}));
    app.automation_input = false;
    assert_eq!(r.unwrap_err(), "not for agents");
    assert!(log.starts.borrow().is_empty());
}

#[test]
fn a_run_that_ends_while_its_dialog_is_open_ends_and_closes_the_dialog() {
    let log = Log::default();
    let mut app = app(vec![vec![dialog(ScriptDialog::Alert, json!(["hi"]))]], &log);
    start_named(&mut app, "alert.jsx");
    settle(&mut app);
    assert!(app.dialog_open());
    // The host died at the dialog: its next pump reports the end.
    log.end_latest();
    tick(&mut app, &egui::Context::default());
    assert!(!app.scripts.running(), "the dead run is gone");
    assert!(!app.dialog_open(), "and its dialog");
    assert!(log.answers.borrow().is_empty());
}

#[test]
fn a_run_started_by_automation_keeps_its_gate() {
    let log = Log::default();
    let mut app = app(vec![vec![Act::Try("layer.new.layer", json!({"name": "late"}))]], &log);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.services.automation_command = Some(Box::new(|id, _| if id == "layer.new.layer" { Err("not for automation".into()) } else { Ok(()) }));
    app.automation_input = true;
    start_named(&mut app, "agent.jsx");
    app.automation_input = false;
    settle(&mut app);
    assert!(layer_names(&app).iter().all(|n| n != "late"), "{:?}", layer_names(&app));
    assert!(app.scripts.log.iter().any(|l| l.contains("not for automation")), "{:?}", app.scripts.log);
    assert!(!app.automation_input);
}

/// A run started under the engine's gate (the control channel's) runs its commands under it,
/// though the gate is gone by the time they run.
#[test]
fn a_run_started_under_the_engines_gate_keeps_it() {
    fn refuse_layers(id: &str, _: &Value) -> photocraft_engine::Result<()> {
        if id == "layer.new.layer" { Err(photocraft_engine::EngineError::Other("not under this gate".into())) } else { Ok(()) }
    }
    let log = Log::default();
    let mut app = app(vec![vec![Act::Try("layer.new.layer", json!({"name": "late"}))]], &log);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.session.authorize = Some(refuse_layers);
    start_named(&mut app, "gated.jsx");
    app.session.authorize = None;
    settle(&mut app);
    assert!(layer_names(&app).iter().all(|n| n != "late"), "{:?}", layer_names(&app));
    assert!(app.scripts.log.iter().any(|l| l.contains("not under this gate")), "{:?}", app.scripts.log);
    assert!(app.session.authorize.is_none(), "the gate is put back");
}

/// A run a control request starts is automation's: the app's command policy judges its commands
/// as it judged the request.
#[test]
fn a_run_a_control_request_starts_keeps_the_command_policy() {
    fn allow(_: &str, _: &Value) -> photocraft_engine::Result<()> {
        Ok(())
    }
    let log = Log::default();
    let mut app = app(vec![vec![Act::Try("layer.new.layer", json!({"name": "late"}))]], &log);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.services.automation_authorize = Some(allow);
    app.services.automation_command = Some(Box::new(|id, _| if id == "layer.new.layer" { Err("not for automation".into()) } else { Ok(()) }));
    let started = call(&mut app, "engine.execute", json!({"id": "file.scripts.run", "params": {"name": "control.js", "source": "1"}}));
    assert_eq!(started["ok"], true, "{started}");
    settle(&mut app);
    assert!(layer_names(&app).iter().all(|n| n != "late"), "{:?}", layer_names(&app));
    assert!(app.scripts.log.iter().any(|l| l.contains("not for automation")), "{:?}", app.scripts.log);
}

/// A failed run's message keeps every line of its stack, one output row each.
#[test]
fn a_multiline_message_is_one_output_row_a_line() {
    let log = Log::default();
    let mut app = app(vec![vec![Act::Emit(ScriptEvent::Finished(Err("Error: boom\n    at f\n    at <main>".into())))]], &log);
    start_named(&mut app, "trace.jsx");
    settle(&mut app);
    let tail: Vec<&str> = app.scripts.log.iter().rev().take(3).map(String::as_str).collect();
    assert_eq!(tail, ["    at <main>", "    at f", "Error: boom"]);
}

#[test]
fn no_run_starts_while_one_pumps() {
    let log = Log::default();
    let mut app = app(vec![vec![]], &log);
    app.scripts.pumping = true;
    assert!(app.scripts.running());
    assert_eq!(resolve_and_start(&mut app, &json!({"name": "inner.jsx", "source": "1"})).unwrap_err(), ALREADY_RUNNING);
    assert!(log.starts.borrow().is_empty());
}

/// A run whose pump panics ends; the next script can still start.
#[test]
fn a_run_that_panics_while_pumping_leaves_scripts_usable() {
    let log = Log::default();
    let mut app = app(vec![vec![Act::Run("no.such.command", json!({}))], vec![]], &log);
    start_named(&mut app, "boom.jsx");
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| settle(&mut app))).is_err();
    assert!(panicked, "the run panics on the unknown command");
    assert!(!app.scripts.pumping, "the pump is over");
    assert!(!app.scripts.running(), "and the run with it");
    assert!(start(&mut app, ScriptStart { name: "next.jsx".into(), source: String::new(), app_name: None }).is_ok());
    // The new run goes on and ends well.
    settle(&mut app);
    assert!(!app.scripts.running());
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert!(app.ui.status.contains("next.jsx"), "{}", app.ui.status);
}
