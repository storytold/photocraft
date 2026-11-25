//! Script runs through a fake script service: the commands they run, their dialogs, ScriptUI
//! windows and pickers, the Scripts Manager, and action recording and playback.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_engine::Session;
use serde_json::{Value, json};

use super::*;
use photocraft_engine::actions_cmds::ActionsExt as _;
use std::path::PathBuf;
use std::sync::Arc;

use craft::automation::actions::ActionStep;
use craft::host::{Picked, ScriptedFiles};

/// What a fake run does, in order.
enum Act {
    /// Run a session command (as a script's host call does).
    Run(&'static str, Value),
    /// Hand the interface an event; a dialog waits for its answer.
    Emit(ScriptEvent),
    /// Fold into the last journal entry, as typing or a brush gesture does.
    Fold(Value),
    /// Run a session command that may be refused; a refusal is written as output.
    Try(&'static str, Value),
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
                Some(Act::Run(id, params)) => {
                    commands.execute(id, params).unwrap();
                }
                Some(Act::Try(id, params)) => {
                    if let Err(e) = commands.execute(id, params) {
                        out.push(ScriptEvent::Output(e));
                    }
                }
                Some(Act::Fold(params)) => {
                    commands.execute(TEST_FOLD, params).unwrap();
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
    start(app, ScriptStart { name: name.into(), source: String::new(), app_name: None }, None).unwrap();
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
}

fn layer_names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.walk().iter().map(|(_, _, l)| l.name.clone()).filter(|n| n != "Background").collect()
}

fn dialog(kind: ScriptDialog, args: Value) -> Act {
    Act::Emit(ScriptEvent::Dialog { kind, args })
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
    assert!(start(&mut app, ScriptStart { name: "second.jsx".into(), source: String::new(), app_name: None }, None).is_err(), "one run at a time");
    settle(&mut app);
    assert!(app.scripts.running());
    assert!(app.scripts.log.iter().any(|l| l == "hello"));
    assert_eq!(app.session.documents().len(), 1);
    assert!(layer_names(&app).is_empty(), "the run waits at the alert");
    answer(&mut app, Value::Null);
    settle(&mut app);
    assert!(!app.scripts.running());
    assert_eq!(layer_names(&app), ["after the alert"]);
    assert_eq!(app.ui.status, "Ran script greet.jsx");
    assert_eq!(*log.answers.borrow(), [Value::Null]);
}

#[test]
fn a_failed_run_reports_its_error_and_cancel_stops_a_run() {
    let log = Log::default();
    let failing = vec![Act::Emit(ScriptEvent::Finished(Err("ReferenceError: nope is not defined (bad.jsx:1:1)\n    at <main>".into())))];
    let mut app = app(vec![failing, vec![dialog(ScriptDialog::Confirm, json!(["Go?"]))]], &log);
    start_named(&mut app, "bad.jsx");
    settle(&mut app);
    assert!(app.ui.status_error);
    assert_eq!(app.ui.status, "Script bad.jsx: ReferenceError: nope is not defined (bad.jsx:1:1)");
    start_named(&mut app, "slow.jsx");
    settle(&mut app);
    cancel(&mut app);
    settle(&mut app);
    assert!(!app.scripts.running());
    assert!(app.ui.status.contains("cancelled"), "{}", app.ui.status);
}

fn window_tree() -> Value {
    json!({"id": 0, "window": {"id": 0, "type": "window", "style": "dialog", "text": "Options", "children": [
        {"id": 1, "type": "statictext", "text": "Pick one"},
        {"id": 2, "type": "edittext", "text": "old"},
        {"id": 3, "type": "checkbox", "text": "Twice", "value": false},
        {"id": 4, "type": "button", "text": "OK", "default": true},
        {"id": 5, "type": "button", "text": "Cancel", "cancel": true}
    ]}})
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
fn a_scriptui_dialog_sends_the_changed_values_with_the_click() {
    let log = Log::default();
    let mut h = harness(vec![vec![dialog(ScriptDialog::Window, window_tree())]], &log, vec![]);
    start_named(h.state_mut(), "options.jsx");
    h.run_steps(4);
    h.get_by_label("Twice").click();
    h.run_steps(4);
    assert_eq!(*log.answers.borrow(), [json!({"event": "click", "id": 3, "values": [{"id": 3, "value": true}]})]);
}

#[test]
fn escape_clicks_the_cancel_button_of_a_scriptui_dialog() {
    let log = Log::default();
    let mut h = harness(vec![vec![dialog(ScriptDialog::Window, window_tree())]], &log, vec![]);
    start_named(h.state_mut(), "options.jsx");
    h.run_steps(4);
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    assert_eq!(*log.answers.borrow(), [json!({"event": "click", "id": 5, "values": []})]);
}

/// Enter takes the default answer only when it activates no button: with No focused, it's No.
#[test]
fn enter_on_a_focused_button_of_a_confirm_answers_that_button() {
    let log = Log::default();
    let mut h = harness(vec![vec![dialog(ScriptDialog::Confirm, json!(["Go?"]))]], &log, vec![]);
    start_named(h.state_mut(), "ask.jsx");
    h.run_steps(4);
    h.get_by_label("No").focus();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    assert_eq!(*log.answers.borrow(), [json!(false)]);
}

/// Enter on a prompt's focused Cancel button cancels it (`null`); Enter in the text is OK.
#[test]
fn enter_on_a_prompts_focused_cancel_button_cancels() {
    for (focus, want) in [(Some("Cancel"), Value::Null), (None, json!("typed"))] {
        let log = Log::default();
        let mut h = harness(vec![vec![dialog(ScriptDialog::Prompt, json!(["Name?", "typed"]))]], &log, vec![]);
        start_named(h.state_mut(), "ask.jsx");
        h.run_steps(4);
        if let Some(label) = focus {
            h.get_by_label(label).focus();
            h.run_steps(2);
        }
        h.key_press(egui::Key::Enter);
        h.run_steps(4);
        assert_eq!(*log.answers.borrow(), [want], "focused: {focus:?}");
    }
}

#[test]
fn pickers_answer_the_chosen_path_or_null() {
    let log = Log::default();
    let program = vec![
        dialog(ScriptDialog::OpenFile, json!({"prompt": "Pick", "filter": "Images:*.png;*.JPG", "multiple": false})),
        dialog(ScriptDialog::SelectFolder, json!({"prompt": "Where?"})),
        dialog(ScriptDialog::OpenFile, json!({"prompt": "Several", "filter": null, "multiple": true})),
    ];
    let paths = |ps: &[&str]| Picked::Paths(ps.iter().map(PathBuf::from).collect());
    let answers = vec![paths(&["/x/a.png"]), Picked::Cancelled, paths(&["/x/1.png", "/x/2.png"])];
    let mut h = harness(vec![program], &log, answers);
    start_named(h.state_mut(), "pick.jsx");
    h.run_steps(12);
    assert_eq!(*log.answers.borrow(), [json!("/x/a.png"), Value::Null, json!(["/x/1.png", "/x/2.png"])]);
    assert!(!h.state().scripts.running());
}

/// A script's filter is the file types its Open File and Save File pickers offer; without one
/// they offer every file.
#[test]
fn open_and_save_pickers_offer_the_scripts_filter() {
    for kind in [ScriptDialog::OpenFile, ScriptDialog::SaveFile] {
        let filtered = picker_request(kind, &json!({"prompt": null, "filter": "*.png"}));
        assert_eq!(filtered.filters.iter().map(|f| f.extensions.clone()).collect::<Vec<_>>(), [vec!["png".to_string()]], "{kind:?}");
        assert!(picker_request(kind, &json!({"prompt": null, "filter": null})).filters.is_empty(), "{kind:?}");
    }
}

#[test]
fn file_filters_become_extensions() {
    assert_eq!(filter_extensions(Some("Images:*.png;*.JPG")), Some(vec!["png".to_string(), "jpg".to_string()]));
    assert_eq!(filter_extensions(Some("*.txt,*.txt")), Some(vec!["txt".to_string()]));
    assert_eq!(filter_extensions(Some("All files:*.*")), None);
    assert_eq!(filter_extensions(Some("no pattern")), None);
    assert_eq!(filter_extensions(None), None);
    assert!(is_js("a/b/Tool.JSX") && is_js("x.js") && !is_js(".js") && !is_js("x.jsxbin") && !is_js("x.json"));
}

#[test]
fn a_listed_script_runs_with_its_first_tag_as_app_name_and_records_without_tags() {
    let log = Log::default();
    let mut app = app(vec![vec![Act::Run("layer.new.layer", json!({"name": "from the script"}))]], &log);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("file.scripts.load", json!({"name": "tool.jsx", "source": "// tool", "tags": "Photoshop, Other"})).unwrap();
    app.run("actions.record", json!({"name": "With a script"})).unwrap();
    run_listed(&mut app, 0).unwrap();
    settle(&mut app);
    app.run("actions.stop", json!({})).unwrap();
    let starts = log.starts.borrow();
    assert_eq!(starts[0], ScriptStart { name: "tool.jsx".into(), source: "// tool".into(), app_name: Some("Photoshop".into()) });
    assert_eq!(layer_names(&app), ["from the script"]);
    let action = app.session.actions().list.actions().iter().find(|a| a.name == "With a script").unwrap();
    assert_eq!(
        action.steps,
        [ActionStep::new("file.scripts.run", json!({"name": "tool.jsx", "source": "// tool"}))],
        "the script's own commands are not recorded, nor its tags or place in the list"
    );
}

#[test]
fn a_run_that_does_not_start_is_not_recorded() {
    let log = Log::default();
    let mut app = app(vec![], &log);
    app.services.scripts = Some(Box::new(|_| Err("the script host could not start".into())));
    app.run("actions.record", json!({"name": "Nothing"})).unwrap();
    assert!(resolve_and_start(&mut app, json!({"name": "a.js", "source": "1"}), None, None).is_err());
    app.run("actions.stop", json!({})).unwrap();
    let action = app.session.actions().list.actions().iter().find(|a| a.name == "Nothing").unwrap();
    assert!(action.steps.is_empty(), "{:?}", action.steps);
}

#[test]
fn a_file_picker_answered_after_its_run_stopped_is_dropped() {
    let log = Log::default();
    let first = vec![dialog(ScriptDialog::OpenFile, json!({"prompt": "Pick", "filter": null, "multiple": false}))];
    // The new run waits at the same kind of picker, which the late answer must not reach.
    let second = vec![dialog(ScriptDialog::OpenFile, json!({"prompt": "Pick again", "filter": null, "multiple": false}))];
    let files: Arc<dyn craft::host::Files> = Arc::new(ScriptedFiles::held());
    let mut app =
        PhotocraftApp::new(Session::new(), crate::Services { files: Some(files), scripts: Some(service(vec![first, second], &log)), ..Default::default() });
    start_named(&mut app, "pick.jsx");
    settle(&mut app);
    tick(&mut app, &egui::Context::default());
    let late = app.file_dialog.take().expect("the picker was asked for");
    cancel(&mut app);
    settle(&mut app);
    assert!(!app.scripts.running());
    start_named(&mut app, "ask.jsx");
    settle(&mut app);
    late.answer_now(&mut app, Picked::Paths(vec![PathBuf::from("/x/a.png")])).unwrap();
    assert!(log.answers.borrow().is_empty(), "{:?}", log.answers.borrow());
    assert!(app.scripts.run.as_ref().is_some_and(|a| a.dialog.as_ref().is_some_and(|d| d.kind == ScriptDialog::OpenFile)), "the new run still waits");
}

#[test]
fn script_commands_never_change_the_step_being_recorded() {
    let log = Log::default();
    let program = vec![Act::Fold(json!({"name": "changed by the script"})), Act::Run("layer.new.layer", json!({"name": "from the script"}))];
    let mut app = app(vec![program], &log);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("actions.record", json!({"name": "Kept"})).unwrap();
    app.run("layer.new.layer", json!({"name": "mine"})).unwrap();
    let before = app.session.journal.clone();
    start_named(&mut app, "s.jsx");
    settle(&mut app);
    assert_eq!(app.session.journal, before);
}

#[test]
fn an_action_waits_for_its_script_step_and_goes_on_after_it() {
    let log = Log::default();
    let ok = vec![Act::Run("layer.new.layer", json!({"name": "from the script"}))];
    let failing = vec![Act::Emit(ScriptEvent::Finished(Err("Error: stop here".into())))];
    let mut app = app(vec![ok, failing], &log);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.session
        .actions_mut()
        .list
        .push(photocraft_engine::actions_cmds::Action::new(
            "Scripted",
            vec![
                ActionStep::new("layer.new.layer", json!({"name": "before"})),
                ActionStep::new("file.scripts.run", json!({"name": "step.jsx", "source": "1"})),
                ActionStep::new("layer.new.layer", json!({"name": "after"})),
            ],
        ))
        .unwrap();
    let played = crate::actions::play(&mut app, &json!({"action": "Scripted"})).unwrap();
    assert_eq!(played["paused"]["dialog"], PLAYBACK_WAIT, "{played}");
    assert_eq!(layer_names(&app), ["before"]);
    settle(&mut app);
    assert_eq!(layer_names(&app), ["before", "from the script", "after"], "status: {}", app.ui.status);
    assert_eq!(log.starts.borrow()[0].app_name, None, "an action's script runs without tags");
    // A failing script ends the playback there.
    crate::actions::play(&mut app, &json!({"action": "Scripted"})).unwrap();
    settle(&mut app);
    assert_eq!(layer_names(&app), ["before", "from the script", "after", "before"]);
    assert_eq!(app.session.recording.playing, 0);
}

/// Step by Step waits after a script step as after any other, before the step after it; the
/// last step goes on at once.
#[test]
fn step_by_step_waits_after_a_script_step() {
    let log = Log::default();
    let mut app = app(vec![vec![Act::Run("layer.new.layer", json!({"name": "from the script"}))], vec![], vec![]], &log);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("prefs.set", json!({"path": "actions.performance", "value": "step"})).unwrap();
    app.session
        .actions_mut()
        .list
        .push(photocraft_engine::actions_cmds::Action::new(
            "Scripted",
            vec![ActionStep::new("file.scripts.run", json!({"name": "step.jsx", "source": "1"})), ActionStep::new("layer.new.layer", json!({"name": "after"}))],
        ))
        .unwrap();
    crate::actions::play(&mut app, &json!({"action": "Scripted"})).unwrap();
    settle(&mut app);
    assert_eq!(layer_names(&app), ["from the script"], "the next step waits");
    assert!(crate::actions::playback::pending(&app), "a paced wait holds it");
    // A script step that is the last one ends the playback at once.
    assert!(crate::actions::playback::stop(&mut app));
    app.session
        .actions_mut()
        .list
        .push(photocraft_engine::actions_cmds::Action::new("Last", vec![ActionStep::new("file.scripts.run", json!({"name": "last.jsx", "source": "1"}))]))
        .unwrap();
    crate::actions::play(&mut app, &json!({"action": "Last"})).unwrap();
    settle(&mut app);
    assert!(!crate::actions::playback::pending(&app));
    assert_eq!(app.session.recording.playing, 0, "the playback ended");
    // Last in an action that another calls, with a step after the call: that step waits too.
    app.session
        .actions_mut()
        .list
        .push(photocraft_engine::actions_cmds::Action::new(
            "Calls",
            vec![ActionStep::new("actions.play", json!({"action": "Last"})), ActionStep::new("layer.new.layer", json!({"name": "after the call"}))],
        ))
        .unwrap();
    crate::actions::play(&mut app, &json!({"action": "Calls"})).unwrap();
    settle(&mut app);
    assert!(crate::actions::playback::pending(&app), "the caller's next step waits");
    assert!(!layer_names(&app).contains(&"after the call".to_string()));
}

/// A playback a person started goes on after its script step as the person's, even when the
/// script ends during a pass of injected automation input: automation's gate does not reach
/// the steps after it.
#[test]
fn a_persons_playback_goes_on_as_theirs_when_its_script_ends_in_an_injected_pass() {
    let log = Log::default();
    let mut app = app(vec![vec![Act::Run("layer.new.layer", json!({"name": "from the script"}))]], &log);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.session
        .actions_mut()
        .list
        .push(photocraft_engine::actions_cmds::Action::new(
            "Scripted",
            vec![ActionStep::new("file.scripts.run", json!({"name": "step.jsx", "source": "1"})), ActionStep::new("layer.new.layer", json!({"name": "after"}))],
        ))
        .unwrap();
    let played = crate::actions::play(&mut app, &json!({"action": "Scripted"})).unwrap();
    assert_eq!(played["paused"]["dialog"], PLAYBACK_WAIT, "{played}");
    // Automation refuses every command it is asked to admit.
    app.services.automation_command = Some(Box::new(|_, _| Err("automation may not".into())));
    app.services.automation_authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("automation may not".into())));
    app.automation_input = true;
    settle(&mut app);
    app.automation_input = false;
    assert_eq!(layer_names(&app), ["from the script", "after"], "status: {}", app.ui.status);
}

/// On the direct control route a playback finishes before it answers, so a script step, which
/// needs frames to run, is refused there rather than left paused behind a reply that says the
/// action is done. No script starts.
#[test]
fn a_script_step_is_refused_where_playback_must_finish_before_it_answers() {
    let log = Log::default();
    let mut app = app(vec![vec![Act::Run("layer.new.layer", json!({"name": "from the script"}))]], &log);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.session
        .actions_mut()
        .list
        .push(photocraft_engine::actions_cmds::Action::new(
            "Scripted",
            vec![ActionStep::new("file.scripts.run", json!({"name": "step.jsx", "source": "1"})), ActionStep::new("layer.new.layer", json!({"name": "after"}))],
        ))
        .unwrap();
    app.programmatic_playback = true;
    let played = app.run("actions.play", json!({"action": "Scripted"}));
    app.programmatic_playback = false;
    let played = played.unwrap_or_else(|e| json!({"error": e}));
    assert!(played.get("paused").is_none(), "{played}");
    assert!(played.to_string().contains("script"), "the reply says why: {played}");
    assert!(log.starts.borrow().is_empty(), "no script started");
    assert!(!app.scripts.running());
    settle(&mut app);
    assert!(layer_names(&app).is_empty(), "nothing after the step ran: {:?}", layer_names(&app));
}

#[test]
fn the_menu_opens_the_manager_and_runs_scripts_by_command() {
    let log = Log::default();
    let mut app = app(vec![vec![]], &log);
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "file.scripts.manager", json!({})).unwrap();
    assert!(app.ui.scripts.open);
    crate::menus::invoke(&mut app, &ctx, "file.scripts.run", json!({"name": "cmd.js", "source": "1"})).unwrap();
    assert_eq!(log.starts.borrow()[0].name, "cmd.js");
    settle(&mut app);
    assert!(!app.scripts.running());
    // Without a script service no script runs, and the command says so.
    let mut plain = PhotocraftApp::new(Session::new(), crate::Services::default());
    let r = crate::menus::invoke(&mut plain, &ctx, "file.scripts.run", json!({"name": "cmd.js", "source": "1"}));
    assert_eq!(r.unwrap_err(), NO_SCRIPTS);
}

/// An action's script step fails where no script can run, rather than playing on as if it ran.
#[test]
fn a_script_step_fails_without_a_script_service() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.session
        .actions_mut()
        .list
        .push(photocraft_engine::actions_cmds::Action::new(
            "Scripted",
            vec![ActionStep::new("file.scripts.run", json!({"name": "step.jsx", "source": "1"})), ActionStep::new("layer.new.layer", json!({"name": "after"}))],
        ))
        .unwrap();
    let played = crate::actions::play(&mut app, &json!({"action": "Scripted"})).unwrap();
    assert!(played["failed"]["error"].as_str().is_some_and(|e| e == NO_SCRIPTS), "{played}");
    assert!(layer_names(&app).is_empty(), "the action stopped at the step: {:?}", layer_names(&app));
}

/// Editing a text field and clicking OK in the same pass sends the click, with the edit.
#[test]
fn a_click_outranks_the_change_of_the_field_it_takes_focus_from() {
    let mut edits = BTreeMap::new();
    let mut st = Draw { window: 1, edits: &mut edits, event: None, live: true, multiline_focus: false };
    st.fire("change", 2);
    st.fire("click", 3);
    assert_eq!(st.event, Some(("click", 3)));
    st.fire("change", 4);
    assert_eq!(st.event, Some(("click", 3)), "the first click stays");
}

fn deny_new_layer(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id == "layer.new.layer" { Err(photocraft_engine::EngineError::Other("new layer denied".into())) } else { Ok(()) }
}

/// An action played under automation's gate pauses at its script step: the script's own
/// commands run under that gate too.
#[test]
fn a_playback_scripts_commands_run_under_the_gate_it_paused_with() {
    let log = Log::default();
    let mut app = app(vec![vec![Act::Try("layer.new.layer", json!({"name": "from the script"}))]], &log);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.session
        .actions_mut()
        .list
        .push(photocraft_engine::actions_cmds::Action::new("Scripted", vec![ActionStep::new("file.scripts.run", json!({"name": "step.jsx", "source": "1"}))]))
        .unwrap();
    app.session.authorize = Some(deny_new_layer);
    let played = crate::actions::play(&mut app, &json!({"action": "Scripted"})).unwrap();
    app.session.authorize = None;
    assert_eq!(played["paused"]["dialog"], PLAYBACK_WAIT, "{played}");
    settle(&mut app);
    assert!(layer_names(&app).is_empty(), "the denied command did not run: {:?}", layer_names(&app));
    assert!(app.scripts.log.iter().any(|l| l.contains("new layer denied")), "{:?}", app.scripts.log);
    assert!(app.session.authorize.is_none(), "the gate is removed again");
}

#[test]
fn controls_in_a_disabled_group_take_no_input() {
    let tree = json!({"id": 0, "window": {"id": 0, "type": "window", "style": "dialog", "text": "Options", "children": [
        {"id": 1, "type": "group", "enabled": false, "children": [
            {"id": 2, "type": "checkbox", "text": "Locked", "value": false}
        ]},
        {"id": 3, "type": "button", "text": "OK", "default": true}
    ]}});
    let log = Log::default();
    let mut h = harness(vec![vec![dialog(ScriptDialog::Window, tree)]], &log, vec![]);
    start_named(h.state_mut(), "options.jsx");
    h.run_steps(4);
    h.get_by_label("Locked").click();
    h.run_steps(4);
    assert!(log.answers.borrow().is_empty(), "{:?}", log.answers.borrow());
    h.get_by_label("OK").click();
    h.run_steps(4);
    assert_eq!(*log.answers.borrow(), [json!({"event": "click", "id": 3, "values": []})]);
}

#[test]
fn a_loaded_script_that_is_not_utf8_is_refused() {
    let log = Log::default();
    let files = Arc::new(ScriptedFiles::answering([Picked::Contents(vec![("latin.jsx".into(), b"alert('caf\xe9');".to_vec())])]));
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(Session::new(), crate::Services { files: Some(files), scripts: Some(service(vec![], &log)), ..Default::default() })
    });
    load_dialog(h.state_mut()).unwrap();
    h.run_steps(4);
    assert!(h.state().session.prefs().scripts.entries.is_empty());
    assert_eq!(h.state().ui.status, "latin.jsx is not UTF-8 text");
}

#[test]
fn a_run_that_ends_while_its_dialog_is_open_ends_and_closes_the_dialog() {
    let log = Log::default();
    let mut app = app(vec![vec![dialog(ScriptDialog::Alert, json!(["hi"]))]], &log);
    start_named(&mut app, "alert.jsx");
    settle(&mut app);
    assert!(app.scripts.run.as_ref().is_some_and(|a| a.dialog.is_some()));
    // The host died at the dialog: its next pump reports the end.
    log.end_latest();
    tick(&mut app, &egui::Context::default());
    assert!(!app.scripts.running(), "the dead run and its dialog are gone");
    assert!(log.answers.borrow().is_empty());
}

/// Stopping the playback ends its script: nothing more runs under a gate it no longer has.
#[test]
fn stopping_the_playback_ends_its_script() {
    let log = Log::default();
    let program = vec![dialog(ScriptDialog::Alert, json!(["wait"])), Act::Run("layer.new.layer", json!({"name": "after the stop"}))];
    let mut app = app(vec![program], &log);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.session
        .actions_mut()
        .list
        .push(photocraft_engine::actions_cmds::Action::new("Scripted", vec![ActionStep::new("file.scripts.run", json!({"name": "step.jsx", "source": "1"}))]))
        .unwrap();
    crate::actions::play(&mut app, &json!({"action": "Scripted"})).unwrap();
    settle(&mut app);
    let token = app.scripts.resume_token().expect("the script runs for the playback");
    crate::actions::abandoned(&mut app, token);
    answer(&mut app, Value::Null);
    settle(&mut app);
    assert!(!app.scripts.running());
    assert!(layer_names(&app).is_empty(), "{:?}", layer_names(&app));
    assert_eq!(log.cancels.get(), 1);
}

/// A command naming its document runs there, whichever document is active when it arrives.
#[test]
fn a_command_naming_its_document_runs_on_that_document() {
    let log = Log::default();
    let mut app = app(vec![], &log);
    app.run("file.new", json!({"width": 16, "height": 16, "name": "First"})).unwrap();
    let first = app.session.active().unwrap().serial;
    app.run("file.new", json!({"width": 16, "height": 16, "name": "Second"})).unwrap();
    let mut budget = RunBudget::default();
    let mut commands = AppCommands { app: &mut app, automation: false, gated: false, budget: &mut budget };
    commands.execute("layer.new.layer", json!({"name": "There", ON_DOCUMENT: first})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.name, "First");
    assert_eq!(layer_names(&app)[0], "There");
    let mut commands = AppCommands { app: &mut app, automation: false, gated: false, budget: &mut budget };
    let gone = commands.execute("layer.new.layer", json!({ON_DOCUMENT: 9999})).unwrap_err();
    assert_eq!(gone, "the document is no longer open");
}

/// A run started by automation keeps automation's policy for every command, in later frames too.
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

/// A run an agent started under the agent policy doesn't go on without it: once the policy is
/// gone (the agent link stopped), its commands are refused, not run ungated.
#[test]
fn a_run_started_under_an_agent_policy_stops_when_the_policy_goes() {
    let log = Log::default();
    let mut app = app(vec![vec![Act::Try("layer.new.layer", json!({"name": "late"}))]], &log);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.services.automation_command = Some(Box::new(|_, _| Ok(())));
    app.automation_input = true;
    start_named(&mut app, "agent.jsx");
    app.automation_input = false;
    app.services.automation_command = None;
    settle(&mut app);
    assert!(layer_names(&app).iter().all(|n| n != "late"), "{:?}", layer_names(&app));
    assert!(app.scripts.log.iter().any(|l| l.contains("agent link")), "{:?}", app.scripts.log);
}

/// A document the policy won't switch to is refused with the policy's reason.
#[test]
fn a_refused_switch_to_the_target_document_keeps_its_reason() {
    let log = Log::default();
    let mut app = app(vec![], &log);
    app.run("file.new", json!({"width": 16, "height": 16, "name": "First"})).unwrap();
    let first = app.session.active().unwrap().serial;
    app.run("file.new", json!({"width": 16, "height": 16, "name": "Second"})).unwrap();
    app.services.automation_command = Some(Box::new(|id, _| if id == "document.activate" { Err("no switching".into()) } else { Ok(()) }));
    let mut budget = RunBudget::default();
    let mut commands = AppCommands { app: &mut app, automation: true, gated: true, budget: &mut budget };
    let err = commands.execute("layer.new.layer", json!({ON_DOCUMENT: first})).unwrap_err();
    assert!(err.contains("no switching"), "{err}");
}

/// The work a run asks of the app is bounded, whatever the script's own time.
#[test]
fn a_runs_work_in_the_app_is_bounded() {
    let mut budget = RunBudget { layers: MAX_RUN_LAYERS - 1, documents: MAX_RUN_DOCUMENTS, ..RunBudget::default() };
    assert_eq!(budget.charge("layer.new.layer", &Value::Null, 0), Ok(()));
    assert!(budget.charge("layer.duplicate", &Value::Null, 1).unwrap_err().contains("layers"));
    assert!(budget.charge("file.new", &Value::Null, 0).unwrap_err().contains("documents"));
    assert_eq!(budget.charge("layer.setProps", &Value::Null, 0), Ok(()), "other commands still run");
    budget.app_time = MAX_RUN_APP_TIME;
    assert!(budget.charge("layer.setProps", &Value::Null, 0).is_err());
    let mut budget = RunBudget { commands: MAX_RUN_COMMANDS, ..RunBudget::default() };
    assert!(budget.charge("session.inspect", &Value::Null, 0).is_err());
}

/// Duplicating a group or a document copies everything in it, and the run pays for each copy.
#[test]
fn duplicating_a_group_counts_every_layer_it_copies() {
    let log = Log::default();
    let mut app = app(vec![], &log);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let group = app.run("layer.new.group", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap();
    for name in ["a", "b"] {
        let l = app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap();
        app.run("layer.moveTo", json!({"layer": l, "target": group, "position": "into"})).unwrap();
    }
    let serial = app.session.active().unwrap().serial;
    let mut budget = RunBudget { layers: MAX_RUN_LAYERS - 2, ..RunBudget::default() };
    let mut commands = AppCommands { app: &mut app, automation: false, gated: false, budget: &mut budget };
    assert_eq!(commands.copies("layer.duplicate", Some(serial), &json!({"layer": group})), 3);
    let err = commands.execute("layer.duplicate", json!({"layer": group, ON_DOCUMENT: serial})).unwrap_err();
    assert!(err.contains("layers"), "{err}");
    // So does a move that copies the group; one that only moves it makes nothing.
    assert_eq!(commands.copies("layer.moveTo", Some(serial), &json!({"layer": group, "copy": true})), 3);
    assert_eq!(commands.copies("layer.moveTo", Some(serial), &json!({"layer": group})), 0, "a plain move is not walked");
    let copy = json!({"layer": group, "target": group, "position": "above", "copy": true, ON_DOCUMENT: serial});
    let err = commands.execute("layer.moveTo", copy).unwrap_err();
    assert!(err.contains("layers"), "{err}");
    assert_eq!(commands.budget.charge("layer.moveTo", &json!({"layer": group}), 3), Ok(()));
    // A duplicated document copies every layer: background, the group and its two layers.
    assert_eq!(commands.copies("image.duplicate", Some(serial), &json!({"mergedOnly": false})), 4);
    assert_eq!(commands.copies("image.duplicate", Some(serial), &json!({"mergedOnly": true})), 1);
    let err = commands.execute("image.duplicate", json!({"name": "copy", "mergedOnly": false, ON_DOCUMENT: serial})).unwrap_err();
    assert!(err.contains("layers"), "{err}");
}

/// What a duplicate is charged is what it makes: the layers the document gains, or the layers
/// of the new document.
#[test]
fn duplicates_are_charged_the_layers_they_make() {
    let log = Log::default();
    let mut app = app(vec![], &log);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let group = app.run("layer.new.group", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap();
    for name in ["a", "b"] {
        let l = app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap();
        app.run("layer.moveTo", json!({"layer": l, "target": group, "position": "into"})).unwrap();
    }
    let serial = app.session.active().unwrap().serial;
    let layers = |app: &PhotocraftApp| u32::try_from(app.session.active().unwrap().doc.layer_count()).unwrap();
    let mut budget = RunBudget::default();
    let mut commands = AppCommands { app: &mut app, automation: false, gated: false, budget: &mut budget };
    let before = layers(commands.app);
    commands.execute("layer.duplicate", json!({"layer": group, ON_DOCUMENT: serial})).unwrap();
    assert_eq!(layers(commands.app) - before, commands.budget.layers, "a group and the two layers in it");
    let (before, charged) = (layers(commands.app), commands.budget.layers);
    commands.execute("layer.moveTo", json!({"layer": group, "target": group, "position": "below", "copy": true, ON_DOCUMENT: serial})).unwrap();
    assert_eq!(layers(commands.app) - before, commands.budget.layers - charged, "a copying move: the same three");
    for merged in [false, true] {
        let charged = commands.budget.layers;
        commands.execute("image.duplicate", json!({"name": "copy", "mergedOnly": merged, ON_DOCUMENT: serial})).unwrap();
        assert_ne!(commands.app.session.active().unwrap().serial, serial, "the copy is active");
        assert_eq!(layers(commands.app), commands.budget.layers - charged, "merged: {merged}");
    }
}

/// Opening a file makes a document, which the run's document count bounds.
#[test]
fn opening_files_counts_against_the_documents_a_run_may_make() {
    let mut budget = RunBudget { documents: MAX_RUN_DOCUMENTS, ..RunBudget::default() };
    assert!(budget.charge("file.open", &Value::Null, 0).unwrap_err().contains("documents"));
}

/// Typing tags and then clicking Run is one click that does both: the tags are kept, and the
/// run starts.
#[test]
fn leaving_the_tags_field_by_clicking_run_keeps_the_tags_and_runs() {
    let log = Log::default();
    let mut h = harness(vec![vec![]], &log, vec![]);
    let app = h.state_mut();
    app.run("file.scripts.load", json!({"name": "a.jsx", "source": "1"})).unwrap();
    app.ui.scripts.open = true;
    app.ui.scripts.selected = Some(0);
    h.run_steps(4);
    let field = h.get_by_role(egui::accesskit::Role::TextInput);
    field.focus();
    h.run_steps(2);
    h.get_by_role(egui::accesskit::Role::TextInput).type_text("night");
    h.run_steps(2);
    h.get_by_label("Run").click();
    h.run_steps(6);
    assert_eq!(h.state().session.prefs().scripts.entries[0].tags, ["night"]);
    assert_eq!(log.starts.borrow().len(), 1, "the run started");
}

/// While a run is taken out to pump its commands through the app it is still running: nothing
/// reached from one of its commands can start another run in its place.
#[test]
fn no_run_starts_while_one_pumps() {
    let log = Log::default();
    let mut app = app(vec![vec![]], &log);
    app.scripts.pumping = true;
    assert!(app.scripts.running());
    assert_eq!(resolve_and_start(&mut app, json!({"name": "inner.jsx", "source": "1"}), None, None).unwrap_err(), "a script is already running");
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
    finish(&mut app, Ok(()));
    assert!(start(&mut app, ScriptStart { name: "next.jsx".into(), source: String::new(), app_name: None }, None).is_ok());
    // The new run goes on and ends well.
    settle(&mut app);
    assert!(!app.scripts.running());
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert!(app.ui.status.contains("next.jsx"), "{}", app.ui.status);
}

/// A run that panics after a command while an action records leaves that command out of the
/// recording, as a run that ends normally does.
#[test]
fn a_panicking_run_leaves_its_commands_out_of_a_recording() {
    let log = Log::default();
    let program = vec![Act::Run("layer.new.layer", json!({"name": "by the script"})), Act::Run("no.such.command", json!({}))];
    let mut app = app(vec![program], &log);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("actions.record", json!({"name": "Rec"})).unwrap();
    start_named(&mut app, "boom.jsx");
    let before = app.session.journal.len();
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| settle(&mut app))).is_err();
    assert!(panicked, "the run panics on the unknown command");
    assert!(!app.scripts.running());
    assert_eq!(app.session.journal.len(), before, "{:?}", app.session.journal.iter().map(|e| &e.0).collect::<Vec<_>>());
    assert_eq!(layer_names(&app), ["by the script"], "the command it ran stays done");
}
