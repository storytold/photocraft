use super::*;

use craft::automation::actions::{ActionList, ActionStep};
use craft::automation::ui::actions::{ActionSelection, ActionsEvent, ActionsHost};
use craft::text::ui::spelling::CheckSpellingDialog;
use craft::ui::canvas::{ViewCommands, ViewRequest};
use craft::ui::dialog::DialogId;

use super::host::PhotoActions;
use super::runner::token_of;

/// The steps for `(command, params)` pairs.
fn steps(pairs: Vec<(String, Value)>) -> Vec<ActionStep> {
    pairs.into_iter().map(ActionStep::from).collect()
}

/// An action named `name` whose steps are `(command, params)` pairs.
fn new_action(name: &str, pairs: Vec<(String, Value)>) -> Action {
    Action::new(name, steps(pairs))
}

/// Add `action` to the session's list.
fn add_action(app: &mut PhotocraftApp, action: Action) {
    app.session.actions.list.push(action).expect("a name the list does not hold yet");
}

/// The steps of action `index` as `(command, params)` pairs, the form the journal and the engine's
/// results keep.
fn pairs(app: &PhotocraftApp, index: usize) -> Vec<(String, Value)> {
    app.session.actions.list.actions()[index].steps.iter().map(|s| (s.command.clone(), s.params.clone())).collect()
}

/// The commands of action `index`'s steps.
fn commands(app: &PhotocraftApp, index: usize) -> Vec<String> {
    app.session.actions.list.actions()[index].steps.iter().map(|s| s.command.clone()).collect()
}

/// Edit the session's actions as a plain vector, to reach states the list's own edits do not
/// (steps replaced under a running playback, an action inserted at a place). The edited list
/// replaces the old one through [`ActionList::replace`], so the revision only moves forward; a
/// name taken twice is renamed, as for any load.
fn edit_actions(app: &mut PhotocraftApp, edit: impl FnOnce(&mut Vec<Action>)) {
    let mut file = app.session.actions.list.to_file();
    edit(&mut file.actions);
    app.session.actions.list.replace(ActionList::from_file(file));
}

/// Press the footer's Play on the selected action (or `action` from step `from`), as the panel
/// reports it, and report the result in the status bar.
fn play_event(app: &mut PhotocraftApp, action: usize, from: usize) {
    super::apply_actions_events(app, vec![ActionsEvent::play(action, from)]);
}

/// The open dialog whose id has `value`, the number a playback report's `paused.dialog` carries.
fn open_id(app: &PhotocraftApp, value: u64) -> Option<DialogId> {
    app.framework.dialogs.open_dialogs().into_iter().map(|dialog| dialog.id).find(|id| id.value() == value)
}

/// One `logic` pass of the app: the dialog host's answers reach the code that finishes their dialogs.
// PLAN: assumes the host's answers are applied in `PhotocraftApp::logic` (dialogs-w19 owns the answer path).
fn settle(app: &mut PhotocraftApp, ctx: &egui::Context) {
    use eframe::App as _;
    let mut frame = eframe::Frame::_new_kittest();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    let input = egui::RawInput { screen_rect: Some(screen), focused: true, ..Default::default() };
    let _ = ctx.run_logic(&input, |ctx| app.logic(ctx, &mut frame));
}

/// Done on Check Spelling: the app's own close, on the app's context.
fn close_spelling(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    crate::type_panels_ui::close_spelling(app, &ctx);
}

/// How many misspellings the open Check Spelling dialog lists.
fn spelling_items(app: &PhotocraftApp) -> usize {
    app.framework.dialogs.get::<CheckSpellingDialog>().map_or(0, |dialog| dialog.items().len())
}

/// Select the set `set` and press the footer's Play on it.
fn play_set(app: &mut PhotocraftApp, set: &str) {
    app.ui.actions.panel.selected = Some(ActionSelection::Set(set.into()));
    super::apply_actions_events(app, vec![ActionsEvent::PlaySet(set.into())]);
}

/// The value of the dialog opened last.
fn last_dialog(app: &PhotocraftApp) -> u64 {
    app.framework.dialogs.open_dialogs().last().map(|dialog| dialog.id.value()).expect("a dialog is open")
}

/// Press OK on the open dialog `value` through the app's OK path, which answers a held OK at once. `Err` is the
/// failure the dialog's finish left in the status bar, or a dialog that is not open or refused OK.
fn answer_ok(app: &mut PhotocraftApp, value: u64) -> Result<(), String> {
    let id = open_id(app, value).ok_or_else(|| format!("no dialog {value}"))?;
    let ctx = app.ctx().clone();
    app.ui.status.clear();
    app.ui.status_error = false;
    crate::dialogs::confirm(app, &ctx, id)?;
    settle(app, &ctx);
    if app.ui.status_error { Err(app.ui.status.clone()) } else { Ok(()) }
}

/// Cancel (Esc, the close box) the open dialog `value` through the dialog host.
fn answer_cancel(app: &mut PhotocraftApp, value: u64) -> Result<(), String> {
    let id = open_id(app, value).ok_or_else(|| format!("no dialog {value}"))?;
    let ctx = app.ctx().clone();
    crate::dialogs::cancel(app, &ctx, id)?;
    settle(app, &ctx);
    Ok(())
}

/// The token of the playback paused at the open dialog `value`.
fn token(app: &PhotocraftApp, value: u64) -> u64 {
    token_of(app, open_id(app, value).expect("the dialog is open")).expect("a playback is paused at the dialog")
}

/// The fields of the open dialog `value`, as automation lists them. The framework keeps every number
/// as a float; a whole one reads as the integer a parameter is written with.
fn dialog_fields(app: &PhotocraftApp, value: u64) -> Value {
    fn whole(v: Value) -> Value {
        match v {
            Value::Number(n) => match n.as_f64() {
                Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 9.0e15 => json!(f as i64),
                _ => Value::Number(n),
            },
            Value::Array(items) => Value::Array(items.into_iter().map(whole).collect()),
            Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, whole(v))).collect()),
            other => other,
        }
    }
    let id = open_id(app, value).expect("the dialog is open");
    whole(app.framework.dialogs.open_dialogs().into_iter().find(|dialog| dialog.id == id).and_then(|dialog| dialog.fields).unwrap_or(Value::Null))
}

/// Set `field` of the open dialog `value`, as `ui.dialog.set` does.
fn set_field(app: &mut PhotocraftApp, value: u64, field: &str, to: Value) -> Result<(), String> {
    let id = open_id(app, value).ok_or_else(|| format!("no dialog {value}"))?;
    let ctx = app.ctx().clone();
    let (request, _reply) = crate::control::ControlRequest::new("ui.dialog.set", json!({"dialog": id, "field": field, "value": to}));
    match crate::control::handle(app, &ctx, &request) {
        crate::control::Reply::Done(reply) => {
            if reply["ok"] == true {
                Ok(())
            } else {
                Err(reply["error"].as_str().unwrap_or("refused").to_string())
            }
        }
        _ => Err("ui.dialog.set answers at once".into()),
    }
}

/// Dialog confirmation, worker completion and cancellation retain the original action boundary.
#[test]
fn single_history_dialog_confirmation_and_cancel() {
    for (single, confirm, background) in [(true, true, false), (true, true, true), (true, false, false), (false, true, false)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":4,"height":4,"background":"#804020"})).unwrap();
        app.run("actions.setHistory", json!({"singleState":single})).unwrap();
        app.background_jobs = background;
        let before = app.session.active().unwrap().doc.clone();
        let count = app.session.active().unwrap().history.past_len();
        let mut action = new_action(
            "Dialog",
            vec![
                ("layer.renameLayer".into(), json!({"name":"Prefix"})),
                ("filter.blur.gaussianBlur".into(), json!({"radius":1})),
                ("layer.renameLayer".into(), json!({"name":"Suffix"})),
            ],
        );
        action.dialogs.insert(1);
        add_action(&mut app, action);
        let paused = play(&mut app, &json!({"action":"Dialog"})).unwrap();
        let dialog = paused["paused"]["dialog"].as_u64().unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), count + 1);
        if confirm {
            answer_ok(&mut app, dialog).unwrap();
        } else {
            answer_cancel(&mut app, dialog).unwrap();
        }
        let st = app.session.active().unwrap();
        let after = st.doc.clone();
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, if confirm { "Suffix" } else { "Prefix" });
        if single {
            assert_eq!(st.history.past_len(), count + 1);
            assert_eq!(st.history.undo_label(), Some("Action: Dialog"));
        } else {
            assert!(st.history.past_len() >= count + 2);
        }
        assert!(app.session.undo());
        if single {
            assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
        } else {
            assert_ne!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
        }
        assert!(app.session.redo());
        assert_eq!(app.session.active().unwrap().doc.as_ref(), after.as_ref());
        assert!(app.paused_playbacks.paused.is_empty());
        assert_eq!(app.session.recording.playing, 0);
        let count = app.session.active().unwrap().history.past_len();
        app.session.execute("layer.renameLayer", json!({"name":"Outside"})).unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), count + 1);
        assert!(app.session.undo());
        assert_eq!(app.session.active().unwrap().doc.as_ref(), after.as_ref());
    }
}

/// A stale dialog must reject its own edit before it can change an unrelated document/history.
#[test]
fn single_history_dialog_refuses_intervening_edit_or_document_switch() {
    for (single_state, switch) in [(true, false), (true, true), (false, false), (false, true)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":4,"height":4,"background":"#804020"})).unwrap();
        app.run("actions.setHistory", json!({"singleState":single_state})).unwrap();
        let mut action =
            new_action("Bound", vec![("layer.renameLayer".into(), json!({"name":"Prefix"})), ("filter.blur.gaussianBlur".into(), json!({"radius":1}))]);
        action.dialogs.insert(1);
        add_action(&mut app, action);
        let paused = play(&mut app, &json!({"action":0})).unwrap();
        let dialog = paused["paused"]["dialog"].as_u64().unwrap();
        if switch {
            app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        } else {
            app.session.execute("layer.renameLayer", json!({"name":"Unrelated"})).unwrap();
        }
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.entries();
        assert!(answer_ok(&mut app, dialog).is_err());
        assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &before));
        assert_eq!(app.session.active().unwrap().history.entries(), history);
        assert!(app.filter_preview.is_none());
        assert!(app.paused_playbacks.paused.is_empty());
        assert_eq!(app.session.recording.playing, 0);
    }
}

/// Prepare a real paused spelling step with a known document and history boundary, under either grouping policy.
fn paused_history_spelling(single: bool, gate: Option<Gate>) -> (Box<PhotocraftApp>, std::sync::Arc<photocraft_doc::Document>, usize) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":300,"height":100})).unwrap();
    app.run("type.create", json!({"x":10,"y":50,"text":"Teh brwon"})).unwrap();
    app.run("actions.setHistory", json!({"singleState":single})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let past = app.session.active().unwrap().history.past_len();
    let mut action = new_action(
        "Spell",
        vec![
            ("layer.renameLayer".into(), json!({"name":"Prefix"})),
            ("edit.checkSpelling".into(), json!({"allLayers":false})),
            ("layer.renameLayer".into(), json!({"name":"Tail"})),
        ],
    );
    action.dialogs.insert(1);
    add_action(&mut app, action);
    app.session.authorize = gate;
    let paused = play(&mut app, &json!({"action":"Spell"})).unwrap();
    assert_eq!(paused["paused"]["dialog"], crate::type_panels_ui::SPELLING, "{paused}");
    (app, before, past)
}

#[test]
/// Repeated corrections share one owned history entry; a rejected correction cannot lose earlier admitted edits.
fn paused_spelling_repeated_changes_and_errors_keep_one_owned_history() {
    for single in [false, true] {
        let (mut app, before, past) = paused_history_spelling(single, None);
        let paused_doc = app.session.active().unwrap().doc.clone();
        let rows = app.session.active().unwrap().history.entries();
        assert!(spelling_change(&mut app, json!({"action":"change","start":0,"end":3,"word":"wrong","replace":"The"})).is_err());
        assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &paused_doc));
        assert_eq!(app.session.active().unwrap().history.entries(), rows);
        for params in
            [json!({"action":"change","start":0,"end":3,"word":"Teh","replace":"The"}), json!({"action":"changeAll","word":"brwon","replace":"brown"})]
        {
            spelling_change(&mut app, params).unwrap();
            let token = app.ui.type_panels.spell.as_ref().unwrap().resume.unwrap();
            assert!(app.paused_playbacks.paused[&token].history.is_some());
            assert_eq!(app.session.recording.playing, 0);
        }
        let st = app.session.active().unwrap();
        let texts: Vec<_> = st
            .doc
            .walk()
            .into_iter()
            .filter_map(|(_, _, layer)| match &layer.content {
                photocraft_doc::LayerContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["The brown"]);
        let before_tail = st.doc.clone();
        close_spelling(&mut app);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert!(app.paused_playbacks.paused.is_empty());
        let st = app.session.active().unwrap();
        assert_eq!(st.history.past_len(), past + if single { 1 } else { 4 });
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "Tail");
        if single {
            assert_eq!(st.history.undo_label(), Some("Action: Spell"));
        }
        let after = st.doc.clone();
        assert!(app.session.undo());
        assert_eq!(app.session.active().unwrap().doc.as_ref(), if single { before.as_ref() } else { before_tail.as_ref() });
        assert!(app.session.redo());
        assert_eq!(app.session.active().unwrap().doc.as_ref(), after.as_ref());
        app.run("layer.renameLayer", json!({"name":"Outside"})).unwrap();
        assert!(app.session.undo());
        assert_eq!(app.session.active().unwrap().doc.as_ref(), after.as_ref());
    }
}

/// A playback paused at Check Spelling holds the library: Clear All waits until Done.
#[test]
fn spelling_playback_blocks_library_edits_until_done() {
    let (mut app, _, _) = paused_history_spelling(false, None);
    assert_eq!(app.session.recording.playing, 0);
    let error = app.run("actions.clearAll", json!({"confirm": true})).unwrap_err();
    assert_eq!(app.ui.status, error);
    assert!(app.ui.status_error);
    assert_eq!(app.session.actions.list.actions()[0].name, "Spell");
    close_spelling(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "Tail");
    app.run("actions.clearAll", json!({"confirm": true})).unwrap();
    assert!(app.session.actions.list.actions().is_empty());
}

fn deny_second_spelling_change(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    if id == "edit.checkSpelling" && params["word"] == "brwon" { Err(photocraft_engine::EngineError::Other("original spelling gate".into())) } else { Ok(()) }
}

#[test]
/// Reject a spelling continuation after document/history drift or captured-policy denial before any correction.
fn paused_spelling_rejects_external_edit_switch_and_original_gate_before_change() {
    for single in [false, true] {
        for scenario in ["edit", "switch", "gate"] {
            let gate = (scenario == "gate").then_some(deny_second_spelling_change as Gate);
            let (mut app, _, _) = paused_history_spelling(single, gate);
            spelling_change(&mut app, json!({"action":"change","start":0,"end":3,"word":"Teh","replace":"The"})).unwrap();
            let corrected = app.session.active().unwrap().doc.clone();
            if scenario == "edit" {
                let past = app.session.active().unwrap().history.past_len();
                app.run("layer.renameLayer", json!({"name":"External"})).unwrap();
                assert_eq!(app.session.active().unwrap().history.past_len(), past + 1, "idle edits have their own history");
            } else if scenario == "switch" {
                app.run("file.new", json!({"width":4,"height":4})).unwrap();
            }
            // The original playback gate still applies even if the live gate has gone away.
            app.session.authorize = None;
            let before: Vec<_> = app.session.documents().iter().map(|st| (st.doc.clone(), st.history.entries())).collect();
            let error = spelling_change(&mut app, json!({"action":"changeAll","word":"brwon","replace":"brown"})).unwrap_err();
            if scenario == "gate" {
                assert!(error.contains("original spelling gate"), "{error}");
            }
            for (state, (doc, rows)) in app.session.documents().iter().zip(&before) {
                assert!(std::sync::Arc::ptr_eq(&state.doc, doc), "{scenario}");
                assert_eq!(&state.history.entries(), rows);
            }
            close_spelling(&mut app);
            assert!(app.paused_playbacks.paused.is_empty());
            assert_eq!(app.session.recording.playing, 0);
            if scenario == "gate" {
                assert_eq!(app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().name, "Tail");
            } else {
                assert!(!app.session.documents().iter().any(|st| st.doc.walk().iter().any(|(_, _, layer)| layer.name == "Tail")));
                assert!(app.ui.status_error, "{}", app.ui.status);
                if scenario == "edit" {
                    assert!(app.session.undo());
                    assert_eq!(app.session.active().unwrap().doc.as_ref(), corrected.as_ref());
                } else {
                    assert!(app.session.documents().iter().any(|st| std::sync::Arc::ptr_eq(&st.doc, &corrected)));
                }
            }
        }
    }
}

#[test]
fn shell_playback_resolves_recorded_eye_target_on_new_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.run("layer.new.layer", json!({"name": "Target"})).unwrap();
    app.run("actions.record", json!({"name": "Hide"})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.setProps", json!({"layer": id.0, "visible": false})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.run("layer.new.layer", json!({"name": "Target"})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let result = play(&mut app, &json!({"action": "Hide"})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    assert!(!app.session.active().unwrap().doc.layer(id).unwrap().visible);
}

#[test]
fn playback_errors_are_visible_in_status_bar() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    add_action(&mut app, new_action("Bad action", vec![("not.a.command".into(), json!({}))]));
    // The footer's Play on the selected action reports Play { action: 0, from: 0 }.
    play_event(&mut app, 0, 0);
    assert!(app.ui.status_error);
    assert!(app.ui.status.contains("not.a.command") || app.ui.status.contains("unknown command"));
}

/// Shell playback retains nested failures and Batch file outcomes without running later steps.
#[test]
fn nested_shell_actions_include_view_steps_and_propagate_failures() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":40,"height":40})).unwrap();
    add_action(&mut app, new_action("Child", vec![("layer.new.layer".into(), json!({})), ("view.fitOnScreen".into(), json!({}))]));
    app.run("actions.record", json!({"name":"Parent"})).unwrap();
    app.run("actions.play", json!({"action":"Child"})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(pairs(&app, 1), [("actions.play".into(), json!({"action":"Child"}))]);
    assert_eq!(PhotoActions::new(&app.session, false).step_label(&ActionStep::new("actions.play", json!({"action":"Child"}))), "Play Action: Child");
    app.run("view.actualPixels", json!({})).unwrap();
    let r = app.run("actions.play", json!({"action":"Parent"})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
    assert_eq!(app.ui.views[0].pending.last(), Some(&ViewRequest::Command(ViewCommands::FitOnScreen)), "the nested Fit on Screen is asked of the canvas");
    app.session.authorize =
        Some(|id, _| if id == "view.fitOnScreen" { Err(photocraft_engine::EngineError::Other("denied nested view".into())) } else { Ok(()) });
    let r = app.run("actions.play", json!({"action":"Parent"})).unwrap();
    assert!(r["failed"]["error"].as_str().unwrap().contains("denied nested view"));
    assert!(app.session.recording.playback_stack.is_empty());
    assert_eq!(app.session.recording.playing, 0);

    app.session.authorize = None;
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("valid.png");
    let corrupt = dir.path().join("corrupt.png");
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "png", &Default::default()).unwrap().bytes;
    std::fs::write(&input, bytes).unwrap();
    std::fs::write(&corrupt, b"not a PNG").unwrap();
    let before = app.session.documents().len();
    for mixed in [true, false] {
        let output = dir.path().join(if mixed { "mixed" } else { "success" });
        let inputs = if mixed { vec![&input, &corrupt] } else { vec![&input] };
        let batch = json!({"input": inputs, "output": output, "steps": [], "format": "png"});
        let index = app.session.actions.list.actions().len();
        let name = if mixed { "Mixed Batch" } else { "Successful Batch" };
        add_action(&mut app, new_action(name, vec![("file.automate.batch".into(), batch), ("file.new".into(), json!({"width": 2, "height": 2}))]));
        let result = app.run("actions.play", json!({"action": index})).unwrap();
        if mixed {
            assert_eq!(result["ran"], 0, "{result}");
            let batch = &result["failed"]["result"];
            let files = batch["files"].as_array().unwrap();
            assert_eq!(files.len(), 1);
            assert_eq!(std::path::Path::new(files[0].as_str().unwrap()), output.join("valid.png").as_path());
            assert_eq!(batch["errors"].as_array().unwrap().len(), 1);
            assert_eq!(batch["errors"][0]["file"], json!(corrupt));
            report_play(&mut app, &result);
            assert!(app.ui.status_error);
            assert!(app.ui.status.contains("1 file(s) failed") && app.ui.status.contains("1 saved") && app.ui.status.contains("corrupt.png"));
            assert_eq!(app.session.documents().len(), before, "a failed Batch stops before file.new");
        } else {
            assert!(result.get("failed").is_none(), "{result}");
            assert_eq!(result["ran"], 2);
            assert_eq!(app.session.documents().len(), before + 1, "a successful Batch continues");
        }
        assert!(output.join("valid.png").exists(), "successful files remain available");
    }
}

/// A played action's view steps are asked of the active document's canvas when they play, in order: after a step that
/// opens a document they act on that document, and an action recorded around the playback records only the call.
#[test]
fn played_view_steps_act_on_the_document_the_steps_before_them_opened() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
    app.sync_views();
    app.ui.views[0].clear_pending();
    let steps = [("file.new", json!({"width": 80, "height": 60})), ("view.zoomIn", json!({})), ("view.zoomIn", json!({})), ("view.fitOnScreen", json!({}))];
    add_action(&mut app, new_action("Open and fit", steps.iter().map(|(id, p)| (id.to_string(), p.clone())).collect()));
    app.run("actions.record", json!({"name": "Around"})).unwrap();
    let r = app.run("actions.play", json!({"action": "Open and fit"})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
    app.run("actions.stop", json!({})).unwrap();
    let opened = app.session.active_index().unwrap();
    assert_ne!(opened, 0);
    assert!(app.ui.views[opened].pending.ends_with(&[ViewCommands::ZoomIn, ViewCommands::ZoomIn, ViewCommands::FitOnScreen].map(ViewRequest::Command)));
    assert!(app.ui.views[0].pending.is_empty(), "the document open before is left alone");
    let around = app.session.actions.list.actions().iter().position(|a| a.name == "Around").unwrap();
    assert_eq!(commands(&app, around), ["actions.play"], "the view steps are not recorded a second time");
}

#[test]
fn view_menu_commands_record_append_and_replay_after_resize() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 160, "height": 100})).unwrap();
    super::begin_recording(&mut app, None);
    app.run("image.canvasSize", json!({"width": 80, "height": 60})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    super::begin_recording(&mut app, Some(0));
    crate::menus::invoke(&mut app, &ctx, "view.fitOnScreen", json!({})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list.actions().len(), 1);
    assert_eq!(commands(&app, 0), ["image.canvasSize", "view.fitOnScreen"]);
    assert_eq!(label_of("view.fitOnScreen"), "Fit on Screen");
    // Persisted actions retain the menu step, and playback fits the new document.
    let saved = serde_json::to_string(&app.session.actions.list.to_file()).unwrap();
    let file = serde_json::from_str(&saved).unwrap();
    app.session.actions.list.replace(ActionList::from_file(file));
    app.run("file.new", json!({"width": 200, "height": 140})).unwrap();
    app.run("view.actualPixels", json!({})).unwrap();
    let result = app.run("actions.play", json!({"action": 0})).unwrap();
    assert_eq!(result["ran"], 2);
    assert!(result.get("failed").is_none(), "{result}");
    let idx = app.session.active_index().unwrap();
    assert_eq!(app.ui.views[idx].pending.last(), Some(&ViewRequest::Command(ViewCommands::FitOnScreen)));
    // The commands asked, as the canvas runs them on a 100 x 100 canvas: the fit leaves its margin around the resized
    // document.
    let target = crate::canvas::view_target(&app, idx);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 100.0));
    let mut camera = app.ui.views[idx].canvas_view(rect, 1.0, false, 1.0);
    for request in &app.ui.views[idx].pending {
        request.command().apply(&mut camera, &target, None, 1.0);
    }
    app.ui.views[idx].set_from(&camera, 1.0);
    assert_eq!(app.ui.views[idx].zoom, 0.75);
    assert_eq!(app.ui.views[idx].center, [40.0, 30.0]);
}

#[test]
fn view_action_failure_resume_and_authorization() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    add_action(&mut app, new_action("Fit", vec![("view.fitOnScreen".into(), json!({})), ("view.actualPixels".into(), json!({}))]));
    let result = app.run("actions.play", json!({"action": "Fit"})).unwrap();
    assert_eq!(result["ran"], 0);
    assert_eq!(result["failed"]["step"], 0);
    assert_eq!(app.session.recording.playing, 0);
    app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
    app.session.authorize =
        Some(
            |id, _| {
                if id == "view.fitOnScreen" { Err(photocraft_engine::EngineError::BadParams { cmd: id.into(), msg: "denied view".into() }) } else { Ok(()) }
            },
        );
    let result = app.run("actions.play", json!({"action": 0})).unwrap();
    assert_eq!(result["ran"], 0);
    assert!(result["failed"]["error"].as_str().unwrap().contains("denied view"));
    let result = app.run("actions.play", json!({"action": 0, "from": 1})).unwrap();
    assert_eq!(result["ran"], 1);
    assert_eq!(app.ui.views[0].pending.last(), Some(&ViewRequest::Command(ViewCommands::ActualPixels)), "played from step 1: no fit after 100%");
    assert!(app.run("actions.play", json!({"action": 0, "from": -1})).is_err());
    app.session.recording.playing = 1;
    assert!(app.run("actions.play", json!({"action": 0})).is_err());
}

#[test]
fn record_and_replay_on_another_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
    app.sync_views();
    app.run("actions.record", json!({})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let _ = app.run("document.pixel", json!({"x": 1, "y": 1}));
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(commands(&app, 0), ["layer.new.layer", "select.rect", "edit.fill"]);
    app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
    app.sync_views();
    let played = app.run("actions.play", json!({"action": 0})).unwrap();
    assert_eq!(played["ran"], 3);
    assert!(played.get("failed").is_none(), "{played}");
    let d = &app.session.active().unwrap().doc;
    assert_eq!(d.layers.len(), 2);
    assert_eq!(d.layers[1].surface().unwrap().pixel(5, 5), vec![1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn recorded_save_as_and_save_replay_and_write_the_file() {
    let written = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let log = written.clone();
    let services = crate::Services {
        export: Some(Box::new(|doc, path, _| photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string()))),
        write: Some(Box::new(move |path, _| {
            log.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("actions.record", json!({})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": "out/result.psd"})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(
        pairs(&app, 0),
        [("layer.new.layer".into(), json!({})), ("file.saveAs".into(), json!({"path": "out/result.psd"})), ("file.save".into(), json!({}))]
    );
    written.borrow_mut().clear();
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let played = app.run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(*written.borrow(), ["out/result.psd", "out/result.psd"]);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("out/result.psd"));
}

/// An app with the real importer/exporter and writer whose file picks go to `files`, the test's
/// scripted file service (the test keeps its own `Arc` to queue answers and read the requests).
pub(super) fn disk_app(files: std::sync::Arc<craft::host::ScriptedFiles>) -> Box<PhotocraftApp> {
    let services = crate::Services {
        import: Some(Box::new(|name, bytes, opts| {
            photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
        })),
        export: Some(Box::new(|doc, path, _| photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string()))),
        write: Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string()))),
        files: Some(files),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.background_jobs = false;
    app
}

/// Records Invert + File › Save on the file `a`, then plays it on the file `b` (given a
/// new layer first when `layered`). Returns the played result and whether `b` / `a`
/// changed on disk.
fn record_save_on_a_play_on_b(ext: &str, layered: bool) -> (Value, bool, bool) {
    use craft::host::{Picked, ScriptedFiles};
    let dir = std::env::temp_dir().join(format!("photocraft-rec-save-{ext}-{layered}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = |n: &str| dir.join(format!("{n}.{ext}")).to_string_lossy().into_owned();
    let (a, b) = (path("a"), path("b"));
    // The dialog confirms the suggested name, which is the document's own file.
    let saves = std::sync::Arc::new(ScriptedFiles::answering([Picked::Paths(vec![a.clone().into()]), Picked::Paths(vec![a.clone().into()])]));
    let mut app = disk_app(saves);
    let ctx = app.ctx().clone();
    for p in [&a, &b] {
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.save_as(Some(p.clone())).unwrap();
    }
    let (before_a, before_b) = (std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    app.open_path(&a).unwrap();
    app.run("actions.record", json!({})).unwrap();
    app.run("image.adjustments.invert", json!({})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog();
    app.run("actions.stop", json!({})).unwrap();
    std::fs::write(&a, &before_a).unwrap();
    app.open_path(&b).unwrap();
    if layered {
        app.run("layer.new.layer", json!({})).unwrap();
    }
    let played = app.run("actions.play", json!({"action": 0})).unwrap();
    let r = (played, std::fs::read(&b).unwrap() != before_b, std::fs::read(&a).unwrap() != before_a);
    let _ = std::fs::remove_dir_all(&dir);
    r
}

#[test]
fn recorded_save_overwrites_the_layered_file_the_action_is_played_on() {
    let (played, b_changed, a_changed) = record_save_on_a_play_on_b("psd", false);
    assert!(played.get("failed").is_none(), "{played}");
    assert!(b_changed && !a_changed);
}

#[test]
fn recorded_save_overwrites_the_flat_file_the_action_is_played_on() {
    let (played, b_changed, a_changed) = record_save_on_a_play_on_b("png", false);
    assert!(played.get("failed").is_none(), "{played}");
    assert!(b_changed, "Save did not overwrite the file the action was played on");
    assert!(!a_changed, "Save wrote the file the action was recorded on");
}

#[test]
fn recorded_save_never_flattens_layers_over_a_flat_file() {
    // The played-on document has layers its PNG can't keep: the step fails, nothing is written (#3806).
    let (played, b_changed, a_changed) = record_save_on_a_play_on_b("png", true);
    assert!(played.get("failed").is_some(), "{played}");
    assert!(!b_changed && !a_changed);
}

/// Loads a Photoshop set holding Invert + a Save As into `dest` (in `out`; with `in_folder`,
/// Photoshop's "save in directory", `out` itself) and plays it on each PSD in `sources`.
/// Returns the played results, whether any PSD changed on disk and the files in `out`.
fn play_imported_save_as(dest: &str, in_folder: bool, sources: &[&str]) -> (Vec<Value>, bool, Vec<String>) {
    use craft_psd::atn::{AtnAction, AtnEvent, AtnFile, AtnItem};
    use craft_psd::descriptor::{Descriptor, UnicodeString, Value as D};
    let dir = std::env::temp_dir().join(format!("photocraft-atn-save-as-{dest}-{in_folder}-{}", std::process::id()));
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let path = |text: &str| {
        let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let mut b = b"txtu".to_vec();
        b.extend_from_slice(&((12 + units.len() * 2) as u32).to_le_bytes());
        b.extend_from_slice(&(units.len() as u32).to_le_bytes());
        units.iter().for_each(|u| b.extend_from_slice(&u.to_le_bytes()));
        D::Path(b)
    };
    let mut save = Descriptor::new("null").with("As  ", D::Descriptor(Descriptor::new("PNGF")));
    save = if in_folder {
        save.with("In  ", path(&out.to_string_lossy())).with("DIDr", D::Boolean(true))
    } else {
        save.with("In  ", path(&out.join(dest).to_string_lossy()))
    };
    let save = save.with("DocI", D::Integer(53));
    let item = |code: &[u8; 4], label: &str, d: Option<Descriptor>| AtnItem {
        event: AtnEvent::Code(*code),
        dictionary: label.into(),
        descriptor: d,
        ..Default::default()
    };
    let set = AtnFile {
        name: UnicodeString::new_nul("Export"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Invert and save"),
            items: vec![item(b"Invr", "Invert", None), item(b"save", "Save", Some(save))],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut app = disk_app(std::sync::Arc::new(craft::host::ScriptedFiles::new()));
    let sources: Vec<String> = sources.iter().map(|n| dir.join(format!("{n}.psd")).to_string_lossy().into_owned()).collect();
    let mut before = Vec::new();
    for src in &sources {
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.save_as(Some(src.clone())).unwrap();
        before.push(std::fs::read(src).unwrap());
    }
    app.run("actions.load", json!({"data": photocraft_engine::paint::tile::b64_encode(&craft_psd::atn::write(&set).unwrap())})).unwrap();
    let mut played = Vec::new();
    for src in &sources {
        app.open_path(src).unwrap();
        played.push(app.run("actions.play", json!({"action": 0})).unwrap());
    }
    let mut written: Vec<String> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    written.sort();
    let changed = sources.iter().zip(&before).any(|(src, before)| std::fs::read(src).unwrap() != *before);
    let _ = std::fs::remove_dir_all(&dir);
    (played, changed, written)
}

#[test]
fn imported_save_as_writes_the_recorded_file_not_the_played_one() {
    // #7102: the Save As imported as File › Save, which wrote over src.psd.
    let (played, src_changed, written) = play_imported_save_as("result.png", false, &["src"]);
    assert!(played[0].get("failed").is_none(), "{}", played[0]);
    assert!(!src_changed, "the Save As wrote over the file the action was played on");
    assert_eq!(written, ["result.png"]);
}

#[test]
fn imported_save_as_to_a_file_without_an_extension_stops_before_writing() {
    // A destination that isn't a file of the recorded format stays a kept step (#7102):
    // playback stops on it, nothing is saved.
    let (played, src_changed, written) = play_imported_save_as("folder", false, &["src"]);
    assert!(played[0].get("failed").is_some(), "{}", played[0]);
    assert!(!src_changed);
    assert!(written.is_empty(), "{written:?}");
}

#[test]
fn imported_save_as_into_a_folder_saves_each_document_under_its_own_name() {
    // #5305: Photoshop's "save in directory" Save As, played on two documents, writes one file
    // for each, named after it, in the recorded folder, and leaves the played files alone.
    let (played, src_changed, written) = play_imported_save_as("", true, &["first", "second"]);
    for p in &played {
        assert!(p.get("failed").is_none(), "{p}");
    }
    assert!(!src_changed);
    assert_eq!(written, ["first.png", "second.png"]);
}

/// #5305: a Save As recorded with the document's own name (the one Save As suggests) keeps
/// only the folder, so playing the action saves each document under its own name instead of
/// over the recorded file; a name typed in is recorded as it is.
#[test]
fn recorded_save_as_with_the_suggested_name_saves_each_document_under_its_own() {
    let written = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let log = written.clone();
    let services = crate::Services {
        export: Some(Box::new(|doc, path, _| photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string()))),
        write: Some(Box::new(move |path, _| {
            log.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.background_jobs = false;
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 16, "height": 16, "name": "first"})).unwrap();
    app.run("actions.record", json!({})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": "out/first.psd"})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": "out/renamed.psd"})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(
        pairs(&app, 0),
        [("file.saveAs".into(), json!({"folder": "out", "extension": "psd"})), ("file.saveAs".into(), json!({"path": "out/renamed.psd"}))]
    );
    // Played on two other documents: each under its own name in the recorded folder.
    app.session.actions.list.delete_step(0, 1).unwrap();
    written.borrow_mut().clear();
    let mut expected = Vec::new();
    for name in ["second", "third"] {
        app.run("file.new", json!({"width": 16, "height": 16, "name": name})).unwrap();
        let played = app.run("actions.play", json!({"action": 0})).unwrap();
        assert!(played.get("failed").is_none(), "{played}");
        let own = std::path::Path::new("out").join(format!("{name}.psd")).to_string_lossy().into_owned();
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some(own.as_str()));
        expected.push(own);
    }
    assert_eq!(*written.borrow(), expected);
    // A folder step needs its extension.
    assert!(app.replay_save("file.saveAs", &json!({"folder": "out"})).is_err());
}

fn layer_names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.walk().iter().map(|(_, _, l)| l.name.clone()).filter(|n| n != "Background").collect()
}

/// #10459 fix-up: a set whose first action pauses at step dialogs plays the rest of the set
/// after OK, and Cancel stops the whole set, as it stops one action.
#[test]
fn a_set_paused_at_a_step_dialog_plays_its_other_actions_after_ok() {
    for ok in [true, false] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let mut a = new_action(
            "A",
            vec![
                ("layer.new.layer".into(), json!({"name": "a"})),
                ("filter.blur.gaussianBlur".into(), json!({"radius": 1})),
                ("filter.blur.gaussianBlur".into(), json!({"radius": 2})),
                ("layer.new.layer".into(), json!({"name": "a2"})),
            ],
        );
        a.set = Some("S".into());
        a.dialogs.extend([1, 2]);
        let mut b = new_action("B", vec![("layer.new.layer".into(), json!({"name": "b"}))]);
        b.set = Some("S".into());
        add_action(&mut app, a);
        add_action(&mut app, b);
        play_set(&mut app, "S");
        assert_eq!(layer_names(&app), ["a"]);
        assert!(app.ui.status.contains("dialog"), "{}", app.ui.status);
        let first = last_dialog(&app);
        if !ok {
            answer_cancel(&mut app, first).unwrap();
            assert_eq!(layer_names(&app), ["a"], "Cancel stops the set");
            continue;
        }
        let first_token = token(&app, first);
        answer_ok(&mut app, first).unwrap();
        // The second dialog of A pauses again (a dialog of one command keeps its id); the rest of the set waits behind it.
        let second = last_dialog(&app);
        assert_ne!(first_token, token(&app, second), "the second dialog holds a new continuation");
        answer_ok(&mut app, second).unwrap();
        let mut names = layer_names(&app);
        names.sort();
        assert_eq!(names, ["a", "a2", "b"], "B played after A's dialogs");
        // CodeRabbit on batch 11: the count covers the whole Play, also the steps before each pause.
        assert_eq!(app.ui.status, "Played 5 steps");
    }
}

/// CodeRabbit on batch 11: the rest of a paused set is kept by name, so an action moved into the
/// middle of the list while the dialog is open neither plays in its place nor skips one of the set.
#[test]
fn a_set_paused_at_a_dialog_plays_its_own_actions_after_a_move() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for (name, set, layer) in [("A", "S", "a"), ("B", "S", "b"), ("C", "S", "c"), ("X", "Other", "x")] {
        let mut a = new_action(name, vec![("layer.new.layer".into(), json!({"name": layer}))]);
        a.set = Some(set.into());
        if name == "A" {
            a.steps.push(ActionStep::new("filter.blur.gaussianBlur", json!({"radius": 1})));
            a.dialogs.insert(1);
        }
        add_action(&mut app, a);
    }
    play_set(&mut app, "S");
    assert_eq!(layer_names(&app), ["a"]);
    let dialog = last_dialog(&app);
    app.run("actions.move", json!({"action": "X", "to": 1})).unwrap();
    answer_ok(&mut app, dialog).unwrap();
    let mut names = layer_names(&app);
    names.sort();
    assert_eq!(names, ["a", "b", "c"]);
}

/// CodeRabbit on batch 11: an action of the paused set deleted and made again under its name while
/// the dialog is open is a different action, so the set ends there instead of playing it.
#[test]
fn a_set_paused_at_a_dialog_ends_at_an_action_replaced_under_its_name() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for (name, layer) in [("A", "a"), ("B", "b"), ("C", "c")] {
        let mut a = new_action(name, vec![("layer.new.layer".into(), json!({"name": layer}))]);
        a.set = Some("S".into());
        if name == "A" {
            a.steps.push(ActionStep::new("filter.blur.gaussianBlur", json!({"radius": 1})));
            a.dialogs.insert(1);
        }
        add_action(&mut app, a);
    }
    play_set(&mut app, "S");
    assert_eq!(layer_names(&app), ["a"]);
    let dialog = last_dialog(&app);
    app.run("actions.delete", json!({"action": "B"})).unwrap();
    let mut b = new_action("B", vec![("layer.new.layer".into(), json!({"name": "new b"}))]);
    b.set = Some("S".into());
    edit_actions(&mut app, |actions| actions.insert(1, b));
    answer_ok(&mut app, dialog).unwrap();
    assert_eq!(layer_names(&app), ["a"], "neither the new B nor C after it played");
}

/// #10459 fix-up: a set whose action pauses at a dialog inside an action it calls stops there:
/// the called action resumes alone, so the next action of the set must not run on a caller that
/// never finished.
#[test]
fn a_set_paused_inside_a_called_action_continues_into_its_caller_then_the_set() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let count = app.session.active().unwrap().history.past_len();
    let mut a = new_action("A", vec![("actions.play".into(), json!({"action": "C"})), ("layer.new.layer".into(), json!({"name": "a-tail"}))]);
    a.set = Some("S".into());
    let mut b = new_action("B", vec![("layer.new.layer".into(), json!({"name": "b"}))]);
    b.set = Some("S".into());
    let mut c = new_action("C", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1})), ("layer.new.layer".into(), json!({"name": "c"}))]);
    c.dialogs.insert(0);
    for action in [a, b, c] {
        add_action(&mut app, action);
    }
    play_set(&mut app, "S");
    let dialog = last_dialog(&app);
    answer_ok(&mut app, dialog).unwrap();
    // #8386, measured: confirming a dialog inside a called action continues the caller after
    // the call, and the set's next action after that.
    assert_eq!(layer_names(&app), ["c", "a-tail", "b"], "C finished, then A's tail, then B");
    // One history state per played action: A with its call into C, then B.
    assert_eq!(app.session.active().unwrap().history.past_len(), count + 2);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Action: B"));
    let after = app.session.active().unwrap().doc.clone();
    assert!(app.session.undo());
    assert_eq!(layer_names(&app), ["c", "a-tail"]);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Action: A"));
    assert!(app.session.undo());
    assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
    assert!(app.session.redo());
    assert!(app.session.redo());
    assert_eq!(app.session.active().unwrap().doc.as_ref(), after.as_ref());
}

#[test]
fn dialog_worker_failure_stops_action_before_following_steps() {
    for (background, locked) in [(true, true), (false, true), (true, false)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 6, "background": "#203040"})).unwrap();
        let mut action = new_action(
            "Worker Result",
            vec![
                ("layer.setProps".into(), json!({"name": "Prefix", "locks": {"pixels": locked}})),
                ("filter.blur.gaussianBlur".into(), json!({"radius": 1.5})),
                ("layer.new.layer".into(), json!({"name": "Suffix"})),
            ],
        );
        action.dialogs.insert(1);
        add_action(&mut app, action);
        app.background_jobs = background;
        let paused = play(&mut app, &json!({"action": "Worker Result"})).unwrap();
        let dialog = paused["paused"]["dialog"].as_u64().unwrap();
        let result = answer_ok(&mut app, dialog);
        let suffix = app.session.active().unwrap().doc.walk().iter().any(|(_, _, layer)| layer.name == "Suffix");

        assert_eq!(suffix, !locked, "background={background}, locked={locked}: {result:?}");
        if locked && background {
            assert!(result.as_ref().is_err_and(|e| e.contains("locked")), "failed worker must fail the dialog confirmation: {result:?}");
            assert!(app.ui.status_error, "the action must report the worker failure");
            assert!(app.ui.status.contains("locked"), "{}", app.ui.status);
            assert_eq!(app.session.jobs_with_recent()[0].state, "failed");
        } else if locked {
            assert!(result.is_err(), "the foreground command rejects the locked layer");
        } else {
            assert!(result.is_ok(), "the successful worker lets the action continue: {result:?}");
        }
        assert_eq!(app.session.recording.playing, 0);
        assert!(app.session.recording.playback_stack.is_empty());
    }
}

#[test]
fn a_check_spelling_step_with_its_dialog_opens_it_and_done_continues() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
    app.run("type.create", json!({"x": 10, "y": 50, "text": "Teh cat"})).unwrap();
    let mut a = new_action("Spell", vec![("edit.checkSpelling".into(), json!({"allLayers": false})), ("layer.new.layer".into(), json!({"name": "after"}))]);
    a.dialogs.insert(0);
    add_action(&mut app, a);
    let v = play(&mut app, &json!({"action": "Spell"})).unwrap();
    assert_eq!((v["ran"].as_u64(), v["paused"]["step"].as_u64()), (Some(0), Some(0)), "{v}");
    assert_eq!(v["paused"]["dialog"], crate::type_panels_ui::SPELLING);
    let d = app.ui.type_panels.spell.clone().expect("Check Spelling is open");
    assert!(!d.all_layers, "the dialog opens with Check All Layers as recorded");
    assert_eq!(spelling_items(&app), 1);
    assert!(!layer_names(&app).contains(&"after".to_string()), "nothing after the dialog ran yet");
    close_spelling(&mut app);
    assert!(layer_names(&app).contains(&"after".to_string()), "Done played the rest");
    assert!(app.ui.status.starts_with("Played"), "{}", app.ui.status);
    // With its dialog off, the step checks without opening anything and changes nothing.
    app.session.actions.list.set_dialog(0, 0, false).unwrap();
    let v = play(&mut app, &json!({"action": "Spell"})).unwrap();
    assert_eq!(v["ran"], 2, "{v}");
    assert!(app.ui.type_panels.spell.is_none());
    let st = app.session.active().unwrap();
    let texts: Vec<String> = st
        .doc
        .walk()
        .into_iter()
        .filter_map(|(_, _, l)| match &l.content {
            photocraft_doc::LayerContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["Teh cat"]);
}

/// Check Spelling opened by hand keeps its place: an action's Check Spelling step fails instead
/// of replacing it.
#[test]
fn a_check_spelling_step_fails_while_the_dialog_is_open_by_hand() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
    app.run("type.create", json!({"x": 10, "y": 50, "text": "Teh cat"})).unwrap();
    let mut a = new_action("Spell", vec![("edit.checkSpelling".into(), json!({"allLayers": false})), ("layer.new.layer".into(), json!({"name": "after"}))]);
    a.dialogs.insert(0);
    add_action(&mut app, a);
    crate::type_panels_ui::open_spelling(&mut app, true).unwrap();
    let v = play(&mut app, &json!({"action": "Spell"})).unwrap();
    assert_eq!(v["failed"]["error"], "Check Spelling is already open", "{v}");
    let d = app.ui.type_panels.spell.as_ref().expect("the dialog stays open");
    assert!(d.all_layers && d.resume.is_none(), "the hand-opened dialog is kept");
    assert!(!layer_names(&app).contains(&"after".to_string()));
}

/// A correction step (`action` set) runs as it did before the dialog toggle applied to Check
/// Spelling: its dialog switch opens nothing.
#[test]
fn a_check_spelling_correction_with_its_dialog_on_runs_without_opening_it() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
    app.run("type.create", json!({"x": 10, "y": 50, "text": "Teh cat"})).unwrap();
    let change = json!({"action": "change", "start": 0, "end": 3, "word": "Teh", "replace": "The"});
    let mut a = new_action("Fix", vec![("edit.checkSpelling".into(), change), ("layer.new.layer".into(), json!({"name": "after"}))]);
    a.dialogs.insert(0);
    add_action(&mut app, a);
    let v = play(&mut app, &json!({"action": "Fix"})).unwrap();
    assert_eq!(v["ran"], 2, "{v}");
    assert!(app.ui.type_panels.spell.is_none(), "no dialog opened");
    let st = app.session.active().unwrap();
    let texts: Vec<String> = st
        .doc
        .walk()
        .into_iter()
        .filter_map(|(_, _, l)| match &l.content {
            photocraft_doc::LayerContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["The cat"]);
}

/// Done after another document became active ends the waiting action there: its remaining steps
/// don't run on the other document.
#[test]
fn check_spelling_done_on_another_document_ends_the_waiting_action() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
    app.run("type.create", json!({"x": 10, "y": 50, "text": "Teh cat"})).unwrap();
    let mut a = new_action("Spell", vec![("edit.checkSpelling".into(), json!({"allLayers": false})), ("layer.new.layer".into(), json!({"name": "after"}))]);
    a.dialogs.insert(0);
    add_action(&mut app, a);
    let v = play(&mut app, &json!({"action": "Spell"})).unwrap();
    assert_eq!(v["paused"]["dialog"], crate::type_panels_ui::SPELLING, "{v}");
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    close_spelling(&mut app);
    for st in app.session.documents() {
        assert!(!st.doc.walk().iter().any(|(_, _, l)| l.name == "after"), "a step after the dialog ran");
    }
    assert!(app.paused_playbacks.paused.is_empty());
    assert!(app.ui.status_error, "{}", app.ui.status);
    assert!(app.ui.status.contains("document"), "{}", app.ui.status);
}

fn deny_spelling(id: &str, p: &Value) -> photocraft_engine::Result<()> {
    if id == "edit.checkSpelling" && p.get("action").is_none() { Err(photocraft_engine::EngineError::Other("spelling denied".into())) } else { Ok(()) }
}

/// Done records the step and continues the waiting action: when recording it fails, the status
/// keeps that error next to the playback's outcome.
#[test]
fn check_spelling_done_keeps_a_recording_error_beside_the_playback_status() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
    app.run("type.create", json!({"x": 10, "y": 50, "text": "Teh cat"})).unwrap();
    let mut a = new_action("Spell", vec![("edit.checkSpelling".into(), json!({"allLayers": false})), ("layer.new.layer".into(), json!({"name": "after"}))]);
    a.dialogs.insert(0);
    add_action(&mut app, a);
    let v = play(&mut app, &json!({"action": "Spell"})).unwrap();
    assert_eq!(v["paused"]["dialog"], crate::type_panels_ui::SPELLING, "{v}");
    app.run("actions.record", json!({"name": "Rec"})).unwrap();
    app.session.authorize = Some(deny_spelling);
    close_spelling(&mut app);
    app.session.authorize = None;
    assert!(layer_names(&app).contains(&"after".to_string()), "Done played the rest");
    assert!(app.ui.status_error, "{}", app.ui.status);
    assert!(app.ui.status.contains("spelling denied"), "{}", app.ui.status);
    assert!(app.ui.status.contains("Played"), "{}", app.ui.status);
}

#[test]
fn check_spelling_opened_again_while_an_action_waits_keeps_its_continuation() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
    app.run("type.create", json!({"x": 10, "y": 50, "text": "Teh cat"})).unwrap();
    let mut a = new_action("Spell", vec![("edit.checkSpelling".into(), json!({"allLayers": false})), ("layer.new.layer".into(), json!({"name": "after"}))]);
    a.dialogs.insert(0);
    add_action(&mut app, a);
    let v = play(&mut app, &json!({"action": "Spell"})).unwrap();
    assert_eq!(v["paused"]["dialog"], crate::type_panels_ui::SPELLING, "{v}");
    let token = app.ui.type_panels.spell.as_ref().and_then(|d| d.resume);
    assert!(token.is_some());
    // Edit › Check Spelling… while the action waits.
    crate::type_panels_ui::open_spelling(&mut app, true).unwrap();
    assert_eq!(app.ui.type_panels.spell.as_ref().and_then(|d| d.resume), token, "the reopened dialog keeps the token");
    // A second playback that reaches a Check Spelling step with its dialog fails there.
    let v = play(&mut app, &json!({"action": "Spell"})).unwrap();
    assert_eq!(v["failed"]["step"], 0, "{v}");
    assert_eq!(app.ui.type_panels.spell.as_ref().and_then(|d| d.resume), token, "the waiting action keeps the dialog");
    assert!(!layer_names(&app).contains(&"after".to_string()));
    close_spelling(&mut app);
    let after = layer_names(&app).iter().filter(|n| *n == "after").count();
    assert_eq!(after, 1, "Done continued the waiting action once");
    assert!(app.paused_playbacks.paused.is_empty());
}

#[test]
fn a_dialog_toggled_step_pauses_playback_and_resumes_on_ok() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut a = new_action(
        "Blur",
        vec![("layer.new.layer".into(), json!({})), ("filter.blur.gaussianBlur".into(), json!({"radius": 3})), ("select.all".into(), json!({}))],
    );
    a.dialogs.insert(1);
    add_action(&mut app, a);
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    assert_eq!(v["ran"], 1, "{v}");
    assert_eq!(v["paused"]["step"], 1);
    let id = v["paused"]["dialog"].as_u64().unwrap();
    assert_eq!(dialog_fields(&app, id)["radius"], 3, "the dialog opens on the recorded values");
    token(&app, id);
    assert!(app.session.active().unwrap().doc.selection.is_none(), "nothing after the dialog ran yet");
    report_play(&mut app, &v);
    assert!(app.ui.status.contains("dialog"), "{}", app.ui.status);
    // OK runs the blur with the dialog's values and plays the rest.
    set_field(&mut app, id, "radius", json!(5)).unwrap();
    answer_ok(&mut app, id).unwrap();
    assert!(!app.dialog_open());
    assert!(app.session.active().unwrap().doc.selection.is_some(), "the step after the dialog ran");
    assert!(app.ui.status.starts_with("Played"), "{}", app.ui.status);
    // Cancel stops the action there.
    app.run("select.deselect", json!({})).unwrap();
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    let id = v["paused"]["dialog"].as_u64().unwrap();
    answer_cancel(&mut app, id).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_none());
    assert!(app.ui.status.contains("stopped"), "{}", app.ui.status);
    // With background jobs (the desktop default) the resumed steps wait for the dialog's
    // filter to land instead of meeting a locked document.
    app.background_jobs = true;
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    let id = v["paused"]["dialog"].as_u64().unwrap();
    answer_ok(&mut app, id).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_some(), "the step after the background blur ran");
    // The job's completion, polled by the frame the answer settles with, names itself in the status bar after the
    // action's "Played" line: the action is over and has not failed.
    assert!(app.paused_playbacks.paused.is_empty() && !app.ui.status_error, "{}", app.ui.status);
    app.background_jobs = false;
    // Agents play headlessly: no pause, the recorded values run.
    app.automation_input = true;
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    assert_eq!(v["ran"], 3, "{v}");
    assert!(v.get("paused").is_none());
}

#[test]
fn equalize_step_with_a_selection_opens_its_choices() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 16, "height": 8})).unwrap();
    app.run("edit.fill", json!({"color": [0.4, 0.4, 0.4]})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 8, "width": 16, "height": 8})).unwrap();
    app.run("edit.fill", json!({"color": [0.5, 0.5, 0.5]})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 16})).unwrap();
    let mut action = new_action("Equalize", vec![(crate::equalize_ui::COMMAND.into(), json!({}))]);
    action.dialogs.insert(0);
    add_action(&mut app, action);
    let gray = |app: &PhotocraftApp, x: i32, y: i32| {
        let st = app.session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)[0]
    };

    let result = play(&mut app, &json!({"action": "Equalize"})).unwrap();
    let id = result["paused"]["dialog"].as_u64().unwrap();
    assert_eq!(dialog_fields(&app, id)["selectedAreaOnly"], "selected");
    assert!((gray(&app, 12, 1) - 0.4).abs() < 0.01, "nothing changes before OK");

    // OK with the entire image chosen maps the pixels outside the selection too.
    set_field(&mut app, id, "selectedAreaOnly", json!("entire")).unwrap();
    answer_ok(&mut app, id).unwrap();
    assert!(gray(&app, 12, 1) < 0.05 && gray(&app, 12, 12) > 0.95, "outside the selection too");
}

/// An 8×6 document, a Child action whose first step (a blur) shows its dialog and whose
/// second step adds a "Suffix" layer, and recording into Owner started.
fn recording_with_dialog_child() -> Box<PhotocraftApp> {
    recording_with_dialog_child_using(crate::Services::default())
}

/// [`recording_with_dialog_child`] with the app built on `services`: the file service is the framework's from
/// construction on.
fn recording_with_dialog_child_using(services: crate::Services) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 8, "height": 6, "background": "#203040"})).unwrap();
    let mut child =
        new_action("Child", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1.5})), ("layer.new.layer".into(), json!({"name": "Suffix"}))]);
    child.dialogs.insert(0);
    add_action(&mut app, child);
    app.run("actions.record", json!({"name": "Owner"})).unwrap();
    app
}

fn suffixes(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.walk().iter().filter(|(_, _, l)| l.name == "Suffix").count()
}

fn child_call() -> Vec<(String, Value)> {
    vec![("actions.play".to_string(), json!({"action": "Child"}))]
}

/// Play Child and return the id of its step dialog.
fn pause_child(app: &mut PhotocraftApp) -> u64 {
    let v = play(app, &json!({"action": "Child"})).unwrap();
    v["paused"]["dialog"].as_u64().unwrap()
}

#[test]
fn a_recorded_call_paused_by_a_dialog_records_one_call_and_replays_once() {
    let mut app = recording_with_dialog_child();
    let refusal = crate::i18n::t("Confirm or cancel the open action step dialog first").to_string();

    // Confirm: the dialog command and the resumed rest stay inside the one named call.
    let live = app.session.journal.len();
    let id = pause_child(&mut app);
    assert!(app.session.recording.target.is_none(), "recording stays suspended while the step dialog is open");
    let names = |app: &PhotocraftApp| app.session.actions.list.actions().iter().map(|a| a.name.clone()).collect::<Vec<_>>();
    let before = names(&app);
    assert_eq!(app.run("actions.record", json!({"name": "Intruder"})), Err(refusal.clone()), "no second recording while the call is suspended");
    assert_eq!(app.run("actions.delete", json!({"action": "Owner"})), Err(refusal.clone()), "the suspended owner can't be deleted");
    assert_eq!(app.run("actions.move", json!({"action": "Owner", "to": 0})), Err(refusal.clone()), "the suspended owner can't be moved");
    assert_eq!(names(&app), before, "the refused commands leave the list alone");
    answer_ok(&mut app, id).unwrap();
    assert!(app.session.recording.target.is_some(), "recording comes back when the call completes");
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(pairs(&app, 1), child_call());
    assert_eq!(suffixes(&app), 1);

    // Replaying the recording repeats the call once.
    app.run("file.new", json!({"width": 8, "height": 6})).unwrap();
    let start = app.session.journal.len();
    app.automation_input = true;
    let v = play(&mut app, &json!({"action": "Owner"})).unwrap();
    app.automation_input = false;
    assert!(v.get("failed").is_none(), "{v}");
    assert_eq!(suffixes(&app), 1, "replay creates the suffix once");
    let edits = |from: usize| -> Vec<(String, Value)> {
        app.session.journal[from..].iter().filter(|(id, _)| !id.starts_with("actions.") && id != "file.new").cloned().collect()
    };
    let expected = vec![("filter.blur.gaussianBlur".to_string(), json!({"radius": 1.5})), ("layer.new.layer".to_string(), json!({"name": "Suffix"}))];
    assert_eq!(edits(start), expected, "replay journals the blur and the suffix once each, in order");
    assert_eq!(edits(live), expected[..].iter().chain(&expected).cloned().collect::<Vec<_>>(), "live then replay");

    // Cancel: the call ends at the dialog and is recorded once.
    app.run("actions.record", json!({"name": "Cancelled"})).unwrap();
    let id = pause_child(&mut app);
    answer_cancel(&mut app, id).unwrap();
    assert!(app.session.recording.target.is_some(), "cancel restores recording");
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(pairs(&app, 2), child_call());
}

#[test]
fn a_failing_step_dialog_command_ends_the_recorded_call_once() {
    let mut app = recording_with_dialog_child();
    let id = pause_child(&mut app);
    assert!(app.session.recording.target.is_none(), "recording stays suspended while the step dialog is open");
    // The dialog takes only values its row accepts, so the command fails on the document: the layer it filters is locked.
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    assert!(answer_ok(&mut app, id).is_err(), "the dialog command fails");
    assert!(app.session.recording.target.is_some(), "recording comes back when the dialog command fails");
    assert_eq!(suffixes(&app), 0, "the action stops at the failed step");
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(pairs(&app, 1), child_call());
}

#[test]
fn an_abandoned_step_dialog_leaves_no_paused_playback() {
    let mut app = recording_with_dialog_child();
    let id = pause_child(&mut app);
    // The dialog takes only values its row accepts, so the command fails on the document: the layer it filters is locked.
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    assert!(answer_ok(&mut app, id).is_err(), "the dialog command fails");
    assert!(app.paused_playbacks.paused.is_empty(), "the failed dialog ends its pause");
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(pairs(&app, 1), child_call(), "the call is recorded once");
    app.run("actions.record", json!({"name": "Next"})).unwrap();
}

#[test]
fn a_failing_resumed_playback_ends_the_recorded_call_once() {
    let mut app = recording_with_dialog_child();
    let id = pause_child(&mut app);
    // The rest of the call can't be found any more: resuming it fails.
    app.session.actions.list.rename(0, "Renamed", |_, _, _| {}).unwrap();
    assert!(answer_ok(&mut app, id).is_err(), "the resumed playback fails");
    assert!(app.session.recording.target.is_some(), "recording comes back when the resumed playback fails");
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(pairs(&app, 1), child_call());
}

#[test]
fn a_suspended_call_is_recorded_into_its_action_after_the_list_changes() {
    let mut app = recording_with_dialog_child();
    let id = pause_child(&mut app);
    // Something else puts an action where Owner was.
    edit_actions(&mut app, |actions| actions.insert(1, new_action("Bystander", vec![])));
    answer_ok(&mut app, id).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    assert!(pairs(&app, 1).is_empty(), "the action now at the old index gets nothing");
    assert_eq!(app.session.actions.list.actions()[2].name, "Owner");
    assert_eq!(pairs(&app, 2), child_call());
}

#[test]
fn escape_on_a_step_dialog_ends_the_recorded_call_once() {
    let app = recording_with_dialog_child();
    let mut h = egui_kittest::Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui_state(|ui, app| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(2);
    pause_child(h.state_mut());
    h.run_steps(4);
    assert!(h.state().session.recording.target.is_none(), "recording stays suspended while the step dialog is open");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(!h.state().dialog_open(), "Esc closes the step dialog");
    assert!(h.state().session.recording.target.is_some(), "Esc restores recording");
    let app = h.state_mut();
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(pairs(app, 1), child_call());
}

#[test]
fn a_step_dialog_keeps_its_continuation_token() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for (name, steps) in [
        ("First", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1}))]),
        // Another command, so the two step dialogs are two dialogs: a dialog of a command is the one open.
        ("Second", vec![("filter.blur.boxBlur".into(), json!({"radius": 1})), ("select.all".into(), json!({}))]),
    ] {
        let mut action = new_action(name, steps);
        action.dialogs.insert(0);
        add_action(&mut app, action);
    }
    let first = play(&mut app, &json!({"action": "First"})).unwrap()["paused"]["dialog"].as_u64().unwrap();
    let second = play(&mut app, &json!({"action": "Second"})).unwrap()["paused"]["dialog"].as_u64().unwrap();
    let first_token = token(&app, first);
    let second_token = token(&app, second);
    assert_ne!(first_token, second_token);
    // The token lives in the app's paused playbacks, bound to the dialog's id: no field of the
    // dialog carries it, so no value set on a dialog can rebind it.
    for value in [json!(second_token), Value::Null, json!("invalid"), json!({"action": "Second", "from": 1})] {
        assert!(set_field(&mut app, first, "resume", value).is_err(), "no dialog field carries the continuation");
        assert_eq!(token(&app, first), first_token);
        assert_eq!(token(&app, second), second_token);
        assert_eq!(app.paused_playbacks.paused.len(), 2);
    }
    answer_ok(&mut app, first).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_none(), "First must not run Second's remaining step");
    answer_ok(&mut app, second).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_some());
    assert!(app.paused_playbacks.paused.is_empty());
}

/// [`recording_with_dialog_child`] over a Displace step, whose map picker answers `answers` in turn.
fn recording_with_displace_child(answers: impl IntoIterator<Item = craft::host::Picked>) -> Box<PhotocraftApp> {
    let services = crate::Services { files: Some(std::sync::Arc::new(craft::host::ScriptedFiles::answering(answers))), ..Default::default() };
    let mut app = recording_with_dialog_child_using(services);
    app.background_jobs = false;
    app.session.actions.list.replace_step(0, 0, ActionStep::new("filter.distort.displace", json!({}))).unwrap();
    app
}

fn assert_displace_call_finished(app: &mut PhotocraftApp, suffix_count: usize) {
    assert!(!app.file_dialog_open());
    assert!(!app.dialog_open());
    assert!(app.paused_playbacks.paused.is_empty());
    assert!(app.session.recording.target.is_some());
    assert_eq!(suffixes(app), suffix_count);
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(pairs(app, 1), child_call(), "record the child call exactly once");
    app.run("actions.record", json!({"name": "Next"})).unwrap();
}

#[test]
fn a_displace_step_restores_recording_when_the_map_picker_fails_or_is_cancelled() {
    use craft::host::Picked;
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.png");
    for answer in [
        Picked::Cancelled,
        Picked::Unavailable("no picker available".into()),
        Picked::Paths(Vec::new()),
        Picked::Contents(vec![("map.png".into(), Vec::new())]),
        Picked::Paths(vec![missing]),
    ] {
        let mut app = recording_with_displace_child([answer]);
        let id = pause_child(&mut app);
        answer_ok(&mut app, id).unwrap();
        assert!(app.file_dialog_open());
        assert!(app.session.recording.target.is_none());
        app.poll_file_dialog();
        assert_displace_call_finished(&mut app, 0);
    }
}

#[test]
fn a_displace_step_resumes_and_records_its_call_once_after_choosing_a_map() {
    let dir = tempfile::tempdir().unwrap();
    let map = dir.path().join("map.png");
    let mut app = recording_with_displace_child([craft::host::Picked::Paths(vec![map.clone()])]);
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "png", &Default::default()).unwrap().bytes;
    std::fs::write(&map, bytes).unwrap();
    let id = pause_child(&mut app);
    answer_ok(&mut app, id).unwrap();
    app.poll_file_dialog();
    assert_displace_call_finished(&mut app, 1);
}

/// Recorded dialogs use sanitized public parameters and retain server-owned command and playback context.
#[test]
fn a_step_dialog_takes_only_the_steps_values() {
    for (command, key, value) in
        [("filter.blur.gaussianBlur", "radius", 2), ("image.adjustments.curves", "smooth", 2), ("image.adjustments.levels", "gamma", 2)]
    {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let mut params = json!({"__command": "select.all", "__label": "Other", "__adjust": "invert", "__resumeAction": 999999, "__insertStop": true, "__smartFilter": {"layer": 1}});
        params[key] = json!(value);
        let mut a = new_action("Step", vec![(command.into(), params)]);
        a.dialogs.insert(0);
        add_action(&mut app, a);
        let v = play(&mut app, &json!({"action": "Step"})).unwrap();
        let id = v["paused"]["dialog"].as_u64().unwrap();
        // The dialog holds the step's own values; its command and the playback it belongs to are
        // the host's and the app's (the dialog's type, the paused playback's token), not fields.
        let fields = dialog_fields(&app, id);
        assert_eq!(fields[key], value);
        for private in ["__command", "__label", "__adjust", "__insertStop", "__smartFilter", "__resumeAction"] {
            assert!(fields.get(private).is_none(), "the step's `{private}` stays out of the dialog: {fields}");
        }
        token(&app, id);
        answer_ok(&mut app, id).unwrap();
        assert!(!app.dialog_open());
        assert!(app.paused_playbacks.paused.is_empty());
    }
}

fn deny_blur(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id == "filter.blur.gaussianBlur" { Err(photocraft_engine::EngineError::Other("blur denied".into())) } else { Ok(()) }
}

/// Permit the recorded small blur but reject an edited radius, so admission is tested against final dialog parameters.
fn deny_wide_blur(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    if id == "filter.blur.gaussianBlur" && params["radius"].as_f64().is_some_and(|radius| radius > 1.0) {
        Err(photocraft_engine::EngineError::Other("edited blur radius denied".into()))
    } else {
        Ok(())
    }
}

/// Build a two-tone raster whose blur changes pixels, with a real action step configured to open its dialog.
fn gated_blur_fixture(single: bool, background: bool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16, "background": "#000000"})).unwrap();
    app.run("select.rect", json!({"x": 8, "y": 0, "width": 8, "height": 16})).unwrap();
    app.run("edit.fill", json!({"color": "#ffffff"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("actions.setHistory", json!({"singleState": single})).unwrap();
    app.background_jobs = background;
    let mut action = new_action("Gated blur", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1}))]);
    action.dialogs.insert(0);
    add_action(&mut app, action);
    app
}

/// Simulate a temporary control gate: capture it at the pause and release the session gate before later confirmation.
fn pause_gated_blur(app: &mut PhotocraftApp, gate: Option<Gate>) -> u64 {
    app.session.authorize = gate;
    let paused = play(app, &json!({"action": "Gated blur"})).unwrap();
    app.session.authorize = None; // The temporary control gate ends before a later human OK.
    paused["paused"]["dialog"].as_u64().unwrap()
}

#[test]
/// Denied edited parameters must change neither pixels nor history and must admit no background worker.
fn a_paused_dialog_authorizes_edited_params_before_history_or_worker_admission() {
    for (single, background) in [(true, false), (true, true), (false, false)] {
        let mut app = gated_blur_fixture(single, background);
        let dialog = pause_gated_blur(&mut app, Some(deny_wide_blur));
        set_field(&mut app, dialog, "radius", json!(4)).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        let count = app.session.active().unwrap().history.past_len();
        let error = answer_ok(&mut app, dialog).unwrap_err();
        assert!(error.contains("edited blur radius denied"), "{error}");
        let state = app.session.active().unwrap();
        assert_eq!(state.doc.as_ref(), before.as_ref(), "denial changes no pixels");
        assert_eq!(state.revision, revision);
        assert_eq!(state.history.past_len(), count);
        assert!(app.session.job_on(state.doc.id).is_none(), "no worker was admitted");
        assert!(app.session.authorize.is_none());
        assert!(app.paused_playbacks.paused.is_empty());
        assert_eq!(app.session.recording.playing, 0);
        app.run("layer.renameLayer", json!({"name": "Outside"})).unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), count + 1, "no action scope leaked");
    }
}

#[test]
/// Allowed captured edits and unrelated manual dialogs retain their normal synchronous and worker behavior.
fn an_allowed_paused_edit_and_an_ordinary_manual_dialog_still_run() {
    for (paused, background) in [(true, false), (true, true), (false, false)] {
        let mut app = gated_blur_fixture(true, background);
        let before = app.session.active().unwrap().doc.clone();
        let dialog = if paused {
            pause_gated_blur(&mut app, Some(deny_wide_blur))
        } else {
            crate::dialogs::open_command_dialog(&mut app, "filter.blur.gaussianBlur", "Gaussian Blur").unwrap().value()
        };
        set_field(&mut app, dialog, "radius", json!(if paused { 1 } else { 4 })).unwrap();
        answer_ok(&mut app, dialog).unwrap();
        let state = app.session.active().unwrap();
        assert_ne!(state.doc.layers[0].surface().unwrap().pixel(7, 8), before.layers[0].surface().unwrap().pixel(7, 8));
        assert!(app.session.job_on(state.doc.id).is_none(), "the resumed action waited for its worker");
        assert!(app.session.authorize.is_none(), "the transient captured gate was restored");
        assert!(app.paused_playbacks.paused.is_empty());
        assert_eq!(app.session.recording.playing, 0);
    }
}

#[test]
/// A paused action's captured permission cannot bypass a stricter live policy on confirmation.
fn a_stricter_current_gate_still_denies_the_captured_dialog_edit() {
    for service in [false, true] {
        let mut app = gated_blur_fixture(true, true);
        let dialog = pause_gated_blur(&mut app, Some(deny_wide_blur));
        let before = app.session.active().unwrap().doc.clone();
        if service {
            app.automation_input = true;
            app.services.automation_authorize = Some(deny_blur);
        } else {
            app.session.authorize = Some(deny_blur);
        }
        let error = answer_ok(&mut app, dialog).unwrap_err();
        assert!(error.contains("blur denied"), "{error}");
        assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
        if service {
            assert!(app.session.authorize.is_none());
        } else {
            assert!(app.session.authorize.unwrap()("filter.blur.gaussianBlur", &json!({"radius": 1})).is_err(), "the live gate was restored");
        }
        assert!(app.paused_playbacks.paused.is_empty());
    }
}

#[test]
/// Spelling corrections consult live Session and automation-service policies before resuming the owned history.
fn paused_spelling_checks_the_live_gate_before_resuming_history() {
    fn deny_correction(id: &str, p: &Value) -> photocraft_engine::Result<()> {
        if id == "edit.checkSpelling" && p["action"] == "changeAll" { Err(photocraft_engine::EngineError::Other("live spelling gate".into())) } else { Ok(()) }
    }
    for service in 0..3 {
        let (mut app, _, _) = paused_history_spelling(true, Some(deny_second_spelling_change));
        let before = app.session.active().unwrap().doc.clone();
        let rows = app.session.active().unwrap().history.entries();
        let token = app.ui.type_panels.spell.as_ref().unwrap().resume.unwrap();
        if service != 1 {
            app.session.authorize = None;
        }
        match service {
            0 => app.session.authorize = Some(deny_correction),
            1 => {
                app.automation_input = true;
                app.services.automation_authorize = Some(deny_correction);
            }
            _ => {
                app.automation_input = true;
                app.services.automation_command = Some(Box::new(|id, _| if id == "edit.checkSpelling" { Err("live spelling gate".into()) } else { Ok(()) }));
            }
        }
        assert!(spelling_change(&mut app, json!({"action":"changeAll","word":"Teh","replace":"The"})).is_err());
        assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
        assert_eq!(app.session.active().unwrap().history.entries(), rows);
        assert!(app.paused_playbacks.paused.get(&token).unwrap().history.is_some(), "denial must leave the bound continuation untouched");
        assert!(app.session.execute("actions.setHistory", json!({"singleState":false})).is_err());
        super::runner::abandoned(&mut app, token);
        app.session.execute("actions.setHistory", json!({"singleState":false})).unwrap();
    }
}

#[test]
/// A changed authority context cancels both direct and nested tails while keeping the admitted prefix undoable.
fn a_changed_live_gate_cannot_be_replaced_for_the_remaining_action() {
    fn deny_tail(id: &str, _: &Value) -> photocraft_engine::Result<()> {
        if id == "layer.renameLayer" { Err(photocraft_engine::EngineError::Other("live tail denied".into())) } else { Ok(()) }
    }
    for service in [false, true] {
        for nested in [false, true] {
            let (mut app, before, past) = paused_history_spelling(true, Some(deny_wide_blur));
            if nested {
                add_action(&mut app, new_action("Nested tail", vec![("layer.renameLayer".into(), json!({"name":"Forbidden tail"}))]));
                app.session.actions.list.replace_step(0, 2, ActionStep::new("actions.play", json!({"action":"Nested tail"}))).unwrap();
            }
            spelling_change(&mut app, json!({"action":"change","start":0,"end":3,"word":"Teh","replace":"The"})).unwrap();
            let corrected = app.session.active().unwrap().doc.clone();
            if service {
                app.automation_input = true;
                app.services.automation_authorize = Some(deny_tail);
            } else {
                app.session.authorize = Some(deny_tail);
            }
            close_spelling(&mut app);
            assert!(app.ui.status_error && app.ui.status.contains("authorization context changed"), "{}", app.ui.status);
            assert_eq!(app.session.active().unwrap().doc.as_ref(), corrected.as_ref(), "the admitted prefix remains, with no tail edit");
            assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
            assert!(app.session.undo());
            assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
            assert!(app.paused_playbacks.paused.is_empty());
            assert_eq!(app.session.recording.playing, 0);
        }
    }
}

#[test]
/// Distinct captured/live gates fail before applying the dialog or starting a worker, even if the edit is allowed.
fn a_changed_context_is_rejected_before_dialog_or_worker_admission() {
    fn allow_all(_: &str, _: &Value) -> photocraft_engine::Result<()> {
        Ok(())
    }
    for background in [false, true] {
        let mut app = gated_blur_fixture(true, background);
        let dialog = pause_gated_blur(&mut app, Some(deny_wide_blur));
        let before = app.session.active().unwrap().doc.clone();
        let rows = app.session.active().unwrap().history.entries();
        app.session.authorize = Some(allow_all);
        assert!(answer_ok(&mut app, dialog).is_err_and(|e| e.contains("authorization context changed")));
        assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
        assert_eq!(app.session.active().unwrap().history.entries(), rows);
        assert!(app.session.job_on(before.id).is_none());
    }
}

#[test]
/// The same gate, a released temporary gate and a newly installed gate permit supported continuation paths.
fn unchanged_and_released_gates_allow_the_action_tail() {
    for scenario in ["released", "same", "new"] {
        let mut app = gated_blur_fixture(true, false);
        app.session.actions.list.insert_step(0, 1, ActionStep::new("layer.renameLayer", json!({"name":"Allowed tail"}))).unwrap();
        let dialog = pause_gated_blur(&mut app, (scenario != "new").then_some(deny_wide_blur as Gate));
        match scenario {
            "same" | "new" => app.session.authorize = Some(deny_wide_blur),
            _ => {}
        }
        answer_ok(&mut app, dialog).unwrap();
        let state = app.session.active().unwrap();
        assert_eq!(state.doc.layer(state.active_layer.unwrap()).unwrap().name, "Allowed tail");
    }
}

#[test]
/// Distinct configured policies permit manual confirmation but cannot be combined for automated dispatch.
/// The identical-gate automated control proves the supported composed-policy path still completes and undoes once.
fn configured_gates_require_one_composed_policy_for_automated_resume() {
    fn deny_file_open(id: &str, _: &Value) -> photocraft_engine::Result<()> {
        if id == "file.open" { Err(photocraft_engine::EngineError::Other("file open denied".into())) } else { Ok(()) }
    }
    assert!(!std::ptr::fn_addr_eq(deny_wide_blur as Gate, deny_file_open as Gate));
    for (automated, identical) in [(false, false), (true, false), (true, true)] {
        let mut app = gated_blur_fixture(true, false);
        app.session.actions.list.insert_step(0, 1, ActionStep::new("layer.renameLayer", json!({"name":"Allowed tail"}))).unwrap();
        app.session.authorize = Some(deny_wide_blur);
        let service: Gate = if identical { deny_wide_blur } else { deny_file_open };
        app.services.automation_authorize = Some(service);
        // The service is configured before the real local pause, but becomes effective only
        // for the automated OK. Both callbacks allow this edit and its remaining command.
        for gate in [deny_wide_blur as Gate, deny_file_open as Gate] {
            gate("filter.blur.gaussianBlur", &json!({"radius":1})).unwrap();
            gate("layer.renameLayer", &json!({"name":"Allowed tail"})).unwrap();
        }
        let dialog = play(&mut app, &json!({"action":"Gated blur"})).unwrap()["paused"]["dialog"].as_u64().unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let rows = app.session.active().unwrap().history.past_len();
        app.automation_input = automated;
        let result = answer_ok(&mut app, dialog);
        let state = app.session.active().unwrap();
        if automated && !identical {
            assert!(result.is_err_and(|error| error.contains("policies that cannot be combined")));
            assert_eq!(state.doc.as_ref(), before.as_ref());
            assert_eq!(state.history.past_len(), rows);
            assert!(app.session.job_on(before.id).is_none());
        } else {
            result.unwrap();
            assert_eq!(state.doc.layer(state.active_layer.unwrap()).unwrap().name, "Allowed tail");
            assert_eq!(state.history.past_len(), rows + 1);
            assert!(app.session.undo());
            assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
        }
        assert!(std::ptr::fn_addr_eq(app.session.authorize.unwrap(), deny_wide_blur as Gate));
        assert!(std::ptr::fn_addr_eq(app.services.automation_authorize.unwrap(), service));
        assert!(app.paused_playbacks.paused.is_empty());
        assert_eq!(app.session.recording.playing, 0);
    }
}

#[test]
/// A policy change after the final spelling step must not discard completed edits when no tail remains to dispatch.
fn a_changed_gate_after_the_last_spelling_step_keeps_completed_edits() {
    let (mut app, before, past) = paused_history_spelling(true, Some(deny_wide_blur));
    app.session.actions.list.delete_step(0, 2).unwrap();
    spelling_change(&mut app, json!({"action":"change","start":0,"end":3,"word":"Teh","replace":"The"})).unwrap();
    let corrected = app.session.active().unwrap().doc.clone();
    app.session.authorize = Some(|_, _| Ok(()));
    close_spelling(&mut app);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().doc.as_ref(), corrected.as_ref());
    assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
    assert!(app.session.undo());
    assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
}

#[test]
/// Implicit layer-owned adjustments reject a receiver switch; explicit receivers change only their intended surface.
/// An omitted target separately inherits the mask captured when its dialog opens.
fn paused_adjustments_bind_implicit_pixel_and_mask_receivers() {
    for targeting in ["implicit", "null", "explicit"] {
        for mask in [false, true] {
            let mut app = gated_blur_fixture(true, false);
            let a = app.session.active().unwrap().active_layer.unwrap();
            app.run("edit.fill", json!({"color":"#808080"})).unwrap();
            app.run("layer.layerMask.revealAll", json!({})).unwrap();
            app.run("edit.fill", json!({"color":"#808080","target":"mask"})).unwrap();
            let b = photocraft_doc::LayerId(app.run("layer.new.layer", json!({"name":"B"})).unwrap()["layer"].as_u64().unwrap());
            // Native mask eligibility checks the active layer, so keep both receivers eligible.
            // The assertions below still require the explicitly named A mask alone to change.
            app.run("layer.layerMask.revealAll", json!({})).unwrap();
            app.session.select_layer(a).unwrap();
            let mut params = json!({"brightness":40,"target":"pixels"});
            if mask {
                params["target"] = json!("mask");
            }
            if targeting == "explicit" {
                params["layer"] = json!(a.0);
            } else if targeting == "null" {
                params["layer"] = Value::Null;
            }
            app.session.actions.list.replace_step(0, 0, ActionStep::new("image.adjustments.brightnessContrast", params)).unwrap();
            let dialog = pause_gated_blur(&mut app, None);
            app.session.select_layer(b).unwrap();
            let before = app.session.active().unwrap().doc.clone();
            let result = answer_ok(&mut app, dialog);
            if targeting == "explicit" {
                result.unwrap();
                let after = &app.session.active().unwrap().doc;
                assert_eq!(after.layer(b).unwrap(), before.layer(b).unwrap());
                if mask {
                    assert_ne!(after.layer(a).unwrap().mask, before.layer(a).unwrap().mask);
                    assert_eq!(after.layer(a).unwrap().surface(), before.layer(a).unwrap().surface());
                } else {
                    assert_ne!(after.layer(a).unwrap().surface(), before.layer(a).unwrap().surface());
                    assert_eq!(after.layer(a).unwrap().mask, before.layer(a).unwrap().mask);
                }
            } else {
                assert!(result.is_err_and(|e| e.contains("receiver changed")));
                assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
            }
        }
    }

    // Batch 16 review: a dialog paused on a document target (an alpha channel) and switched to
    // the layer's pixels checks the layer that was active at the pause.
    {
        let mut app = gated_blur_fixture(true, false);
        let a = app.session.active().unwrap().active_layer.unwrap();
        let b = photocraft_doc::LayerId(app.run("layer.new.layer", json!({"name":"B"})).unwrap()["layer"].as_u64().unwrap());
        app.session.select_layer(a).unwrap();
        app.session
            .actions
            .list
            .replace_step(0, 0, ActionStep::new("image.adjustments.brightnessContrast", json!({"brightness":40,"target":{"channel":0}})))
            .unwrap();
        let dialog = pause_gated_blur(&mut app, None);
        set_field(&mut app, dialog, "target", json!("pixels")).unwrap();
        app.session.select_layer(b).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let result = answer_ok(&mut app, dialog);
        assert!(result.is_err_and(|e| e.contains("receiver changed")));
        assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
    }

    // An omitted target inherits the mask selected when the paused dialog opens. This is
    // distinct from native playback without a dialog, which resolves the current channel.
    let mut app = gated_blur_fixture(true, false);
    let a = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.run("edit.fill", json!({"color":"#808080","target":"mask"})).unwrap();
    assert!(app.layer_mask_targeted());
    app.session.actions.list.replace_step(0, 0, ActionStep::new("image.adjustments.brightnessContrast", json!({"brightness":40}))).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let rows = app.session.active().unwrap().history.past_len();
    let dialog = pause_gated_blur(&mut app, None);
    assert_eq!(dialog_fields(&app, dialog)["target"], "mask");
    answer_ok(&mut app, dialog).unwrap();
    let state = app.session.active().unwrap();
    assert_ne!(state.doc.layer(a).unwrap().mask, before.layer(a).unwrap().mask);
    assert_eq!(state.doc.layer(a).unwrap().surface(), before.layer(a).unwrap().surface());
    assert_eq!(state.history.past_len(), rows + 1);
    assert!(app.session.undo());
    assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
}

#[test]
/// Document-owned alpha channels remain valid targets when the active layer changes during the pause.
fn a_document_channel_adjustment_survives_an_active_layer_change() {
    let mut app = gated_blur_fixture(true, false);
    let a = app.session.active().unwrap().active_layer.unwrap();
    let b = photocraft_doc::LayerId(app.run("layer.new.layer", json!({"name":"B"})).unwrap()["layer"].as_u64().unwrap());
    app.run("channel.new", json!({"name":"Alpha","fill":"black"})).unwrap();
    app.run("edit.fill", json!({"color":"#808080","target":{"channel":0}})).unwrap();
    app.session.select_layer(a).unwrap();
    app.session
        .actions
        .list
        .replace_step(0, 0, ActionStep::new("image.adjustments.brightnessContrast", json!({"brightness":40,"target":{"channel":0}})))
        .unwrap();
    let dialog = pause_gated_blur(&mut app, None);
    app.session.select_layer(b).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    answer_ok(&mut app, dialog).unwrap();
    let after = &app.session.active().unwrap().doc;
    assert_eq!(after.layers, before.layers);
    assert_ne!(after.channels, before.channels);
}

#[test]
/// A public Session layer switch invalidates an implicit receiver while explicit-layer playback remains usable.
fn an_implicit_paused_filter_refuses_a_public_api_receiver_change() {
    for single in [false, true] {
        for explicit in [false, true] {
            let mut app = gated_blur_fixture(single, false);
            let a = app.session.active().unwrap().active_layer.unwrap();
            let b = photocraft_doc::LayerId(app.run("layer.new.layer", json!({"name": "B"})).unwrap()["layer"].as_u64().unwrap());
            app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 16})).unwrap();
            app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
            app.run("select.deselect", json!({})).unwrap();
            app.session.select_layer(a).unwrap();
            if explicit {
                let mut step = app.session.actions.list.actions()[0].steps[0].clone();
                step.params["layer"] = json!(a.0);
                app.session.actions.list.replace_step(0, 0, step).unwrap();
            }
            let dialog = pause_gated_blur(&mut app, None);
            // Supported host/session API, not a modal-blocked native menu/control command.
            app.session.select_layer(b).unwrap();
            let before = app.session.active().unwrap().doc.clone();
            let result = answer_ok(&mut app, dialog);
            let after = &app.session.active().unwrap().doc;
            if explicit {
                result.unwrap();
                assert_ne!(after.layer(a).unwrap().surface().unwrap().pixel(7, 8), before.layer(a).unwrap().surface().unwrap().pixel(7, 8));
                assert_eq!(after.layer(b).unwrap().surface().unwrap().pixel(7, 8), before.layer(b).unwrap().surface().unwrap().pixel(7, 8));
            } else {
                assert!(result.is_err_and(|e| e.contains("receiver changed")));
                assert_eq!(after.as_ref(), before.as_ref(), "neither layer may be edited after stale implicit targeting");
            }
            assert!(app.paused_playbacks.paused.is_empty());
            assert_eq!(app.session.recording.playing, 0);
        }
    }
}

#[test]
/// Changes to other selected layers do not invalidate the unchanged active receiver captured by a filter dialog.
fn an_implicit_filter_keeps_its_receiver_when_only_other_selected_layers_change() {
    let mut app = gated_blur_fixture(true, false);
    let a = app.session.active().unwrap().active_layer.unwrap();
    let b = photocraft_doc::LayerId(app.run("layer.new.layer", json!({"name": "Other"})).unwrap()["layer"].as_u64().unwrap());
    app.session.select_layer(a).unwrap();
    let dialog = pause_gated_blur(&mut app, None);
    app.session.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    app.session.execute("layer.select", json!({"layer": a.0, "mode": "add"})).unwrap();
    assert_eq!(app.session.active().unwrap().selected_layers().len(), 2);
    let before = app.session.active().unwrap().doc.clone();
    answer_ok(&mut app, dialog).unwrap();
    let after = &app.session.active().unwrap().doc;
    assert_ne!(after.layer(a).unwrap().surface().unwrap().pixel(7, 8), before.layer(a).unwrap().surface().unwrap().pixel(7, 8));
    assert_eq!(after.layer(b).unwrap().surface().unwrap().content_bounds(), before.layer(b).unwrap().surface().unwrap().content_bounds());
}

fn deny_displace_file(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    if id == "filter.distort.displace" && params.get("mapPath").is_some() {
        Err(photocraft_engine::EngineError::Other("chosen displacement map denied".into()))
    } else {
        Ok(())
    }
}

#[test]
/// Authorize a deferred displacement-map path before file access, worker admission or continuation history changes.
fn a_deferred_displace_map_is_authorized_before_reading_or_resuming_history() {
    let mut app = recording_with_displace_child([craft::host::Picked::Paths(vec!["denied-map.png".into()])]);
    app.background_jobs = true;
    app.session.authorize = Some(deny_displace_file);
    let dialog = pause_child(&mut app);
    app.session.authorize = None;
    let before = app.session.active().unwrap().doc.clone();
    let count = app.session.active().unwrap().history.past_len();
    answer_ok(&mut app, dialog).unwrap();
    assert!(app.file_dialog_open());
    app.poll_file_dialog();
    assert!(app.ui.status.contains("chosen displacement map denied"), "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
    assert_eq!(app.session.active().unwrap().history.past_len(), count);
    assert!(app.session.authorize.is_none());
    assert!(app.session.job_on(before.id).is_none());
    assert_displace_call_finished(&mut app, 0);
}

#[test]
fn a_step_the_gate_denies_opens_no_dialog() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut a = new_action("Blur", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1}))]);
    a.dialogs.insert(0);
    add_action(&mut app, a);
    app.session.authorize = Some(deny_blur);
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    app.session.authorize = None;
    assert!(v["failed"]["error"].as_str().is_some_and(|e| e.contains("blur denied")), "{v}");
    assert!(!app.dialog_open(), "no step dialog opened");
    assert!(app.paused_playbacks.paused.is_empty());
}

fn deny_select_all(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id == "select.all" { Err(photocraft_engine::EngineError::Other("select.all denied".into())) } else { Ok(()) }
}

/// An action an automation client plays (`control::handle` installs the gate, then removes
/// it) pauses at a step dialog; confirming the dialog later, here with synthetic Enter or
/// OK, runs the remaining steps under the same gate.
#[test]
fn a_paused_playback_keeps_its_automation_gate_when_the_dialog_is_confirmed() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut a = new_action("Seeded", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1})), ("select.all".into(), json!({}))]);
    a.dialogs.insert(0);
    add_action(&mut app, a);
    app.session.authorize = Some(deny_select_all);
    let v = play(&mut app, &json!({"action": "Seeded"})).unwrap();
    app.session.authorize = None;
    let id = v["paused"]["dialog"].as_u64().unwrap();
    app.automation_input = true;
    let _ = answer_ok(&mut app, id);
    app.automation_input = false;
    assert!(app.session.active().unwrap().doc.selection.is_none(), "the denied step did not run");
    assert!(app.ui.status.contains("select.all denied"), "{}", app.ui.status);
}

/// An action automation wrote, bound to a key, plays under the desktop's automation policy
/// when the user presses the key: its file step is refused, not run with the user's access.
#[test]
fn a_users_key_plays_an_action_automation_wrote_under_its_gate() {
    /// The automation policy this test stands in for: every `file.` command is refused.
    fn deny_file(id: &str, _: &Value) -> photocraft_engine::Result<()> {
        if id.starts_with("file.") { Err(photocraft_engine::EngineError::Other(format!("automation command `{id}` is disabled"))) } else { Ok(()) }
    }
    let services = crate::Services { automation_authorize: Some(deny_file), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let mut planted = new_action("Backup", vec![("file.openAs".into(), json!({"path": "/outside/secret.png"}))]);
    planted.automation = true;
    add_action(&mut app, planted);
    assign_shortcut(&mut app, "Backup", "F9").unwrap();
    let ctx = app.ctx().clone();
    crate::shortcut_dispatch::dispatch(&mut app, &ctx, &shortcut_id("Backup"));
    assert!(app.session.documents().is_empty());
    assert!(app.ui.status.contains("disabled"), "the refusal is reported: {}", app.ui.status);
    assert!(app.session.authorize.is_none(), "the kept gate is removed after playback");
}

/// CodeRabbit on #9783: the kept gate of an action automation wrote stays with that action. A set
/// the user plays pauses at the marked action's step dialog; after OK the marked action's tail
/// still plays under the gate (its refused step is refused), and once that tail passes, the
/// user's own action after it in the set keeps the user's access.
#[test]
fn a_kept_gate_does_not_carry_into_the_rest_of_a_set_after_a_pause() {
    /// The automation policy this test stands in for: it refuses only layers named `a2` and `b`.
    fn deny_a2_and_b(id: &str, p: &Value) -> photocraft_engine::Result<()> {
        match p.get("name").and_then(Value::as_str) {
            Some(name @ ("a2" | "b")) if id == "layer.new.layer" => Err(photocraft_engine::EngineError::Other(format!("layer {name} denied"))),
            _ => Ok(()),
        }
    }
    let services = crate::Services { automation_authorize: Some(deny_a2_and_b), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let mut a = new_action(
        "A",
        vec![
            ("layer.new.layer".into(), json!({"name": "a"})),
            ("filter.blur.gaussianBlur".into(), json!({"radius": 1})),
            ("layer.new.layer".into(), json!({"name": "a2"})),
        ],
    );
    a.set = Some("S".into());
    a.dialogs.insert(1);
    a.automation = true;
    let mut b = new_action("B", vec![("layer.new.layer".into(), json!({"name": "b"}))]);
    b.set = Some("S".into());
    add_action(&mut app, a);
    add_action(&mut app, b);

    // The marked action's tail after OK: its `a2` step meets the kept gate, and the set stops.
    play_set(&mut app, "S");
    assert_eq!(layer_names(&app), ["a"]);
    let dialog = last_dialog(&app);
    let _confirmed = answer_ok(&mut app, dialog);
    assert_eq!(layer_names(&app), ["a"], "a2 was refused after OK: {}", app.ui.status);
    assert!(app.ui.status.contains("a2 denied"), "the refusal is reported: {}", app.ui.status);
    assert!(app.session.authorize.is_none(), "no gate is left installed");

    // With that step off, the tail passes, and the user's own action after it plays with the
    // user's access.
    app.session.actions.list.set_step(0, 2, false).unwrap();
    play_set(&mut app, "S");
    let dialog = last_dialog(&app);
    answer_ok(&mut app, dialog).unwrap();
    let names = layer_names(&app);
    assert!(names.iter().any(|n| n == "b"), "B played with the user's access: {names:?} {}", app.ui.status);
    assert!(!names.iter().any(|n| n == "a2"), "{names:?}");
    assert!(app.session.authorize.is_none(), "no gate is left installed");
}

/// The dialog is bound to its pause by id: no field of it can name another action to resume.
#[test]
fn a_step_dialog_resumes_only_the_playback_that_paused_it() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut a = new_action("Blur", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1}))]);
    a.dialogs.insert(0);
    add_action(&mut app, a);
    add_action(&mut app, new_action("Seeded", vec![("select.all".into(), json!({}))]));
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    let id = v["paused"]["dialog"].as_u64().unwrap();
    assert!(set_field(&mut app, id, "resume", json!({"action": "Seeded", "from": 0})).is_err(), "no dialog field names a playback");
    answer_ok(&mut app, id).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_none(), "the other action did not play");
}

/// An imported Close with Save: Yes plays as Save then Close. A Save Document script event
/// that closes the saved document stops playback, so the Close never reaches another dirty
/// document (#7367).
#[test]
fn save_hook_closing_the_target_stops_an_imported_close_with_save() {
    use craft_psd::atn::{self, AtnEvent, AtnItem};
    use craft_psd::descriptor::{Descriptor, Id, Value as D};
    use std::{cell::RefCell, rc::Rc};
    photocraft_engine::install().unwrap();
    // A one-step .atn: Close (`Cls `) with Save: Yes.
    let seed = new_action("Seed", vec![("layer.new.layer".into(), json!({}))]);
    let (bytes, _) = photocraft_engine::atn_cmds::export("Set", &[&seed], &[], false).unwrap();
    let mut f = atn::parse(&bytes).unwrap();
    f.actions[0].items = vec![AtnItem {
        event: AtnEvent::Code(*b"Cls "),
        dictionary: "Close".into(),
        descriptor: Some(Descriptor::new("null").with("Svng", D::Enumerated { type_id: Id::new("YsN "), value: Id::new("Ys  ") })),
        ..Default::default()
    }];
    let imported = photocraft_engine::atn_cmds::import(&atn::write(&f).unwrap()).unwrap();
    let close_with_save = imported.actions[0].clone();
    assert_eq!(close_with_save.steps, [ActionStep::new("file.save", json!({})), ActionStep::new("file.close", json!({}))]);
    for hook in ["none", "activate", "close"] {
        let writes = Rc::new(RefCell::new(Vec::<String>::new()));
        let w = writes.clone();
        let services = crate::Services {
            export: Some(Box::new(|doc, _, _| Ok((doc.name.clone().into_bytes(), Vec::new())))),
            write: Some(Box::new(move |path, _| {
                w.borrow_mut().push(path.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        // B is dirty with an unsaved KeepB layer; A is active and has its own file.
        for name in ["B", "A"] {
            app.run("file.new", json!({"name": name, "width": 2, "height": 2})).unwrap();
        }
        assert!(app.session.set_active(0));
        app.run("layer.new.layer", json!({"name": "KeepB"})).unwrap();
        assert!(app.session.set_active(1));
        app.session.active_mut().unwrap().path = Some("A.psd".into());
        add_action(&mut app, close_with_save.clone());
        let steps = match hook {
            "activate" => json!([["document.activate", {"document": 0}]]),
            "close" => json!([["file.close", {}]]),
            _ => Value::Null,
        };
        if !steps.is_null() {
            app.run("file.scripts.scriptEventsManager", json!({"add": {"event": "saveDocument", "steps": steps}})).unwrap();
        }
        let r = app.run("actions.play", json!({"action": 0})).unwrap();
        assert_eq!(*writes.borrow(), ["A.psd"], "hook={hook}: A is saved once");
        let names: Vec<Vec<String>> = app.session.documents().iter().map(|d| d.doc.layers.iter().map(|l| l.name.clone()).collect()).collect();
        assert_eq!(app.session.documents().len(), 1, "hook={hook}: only A closes; result={r}");
        assert_eq!(app.session.active().unwrap().doc.name, "B", "hook={hook}");
        assert_eq!(names, [["Background", "KeepB"]], "hook={hook}: B stays open with its unsaved layer");
        if hook == "close" {
            assert_eq!(r["failed"]["step"], 0, "{r}");
            assert_eq!(r["failed"]["id"], "file.save", "{r}");
            assert!(r["failed"]["error"].as_str().unwrap().contains("closed during the save"), "{r}");
        } else {
            assert!(r.get("failed").is_none(), "hook={hook}: {r}");
        }
        assert_eq!(app.session.recording.playing, 0);
        assert!(app.session.recording.playback_stack.is_empty());
    }
}
