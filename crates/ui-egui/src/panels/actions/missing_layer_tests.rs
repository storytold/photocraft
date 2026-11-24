//! #10769 "Playing on another document": a step that names a layer the playing document does not
//! have stops with a message asked of the person: Continue skips the step, Stop ends the playback,
//! and nothing done so far is undone. Measured in the reference application
//! (docs/atn.md › Playing on another document).

use super::stop_tests::{answer, assert_idle, count, stop_prompt};
use super::*;
use craft::ui::dialog::StandardPrompt;
use photocraft_engine::actions_cmds::ActionStep;

/// A document with layers X, Y and Z: none of the names a recording made over A, B, C refers to.
fn missing_name_app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 6, "background": "white"})).unwrap();
    for name in ["X", "Y", "Z"] {
        app.run("layer.new.layer", json!({"name": name})).unwrap();
    }
    app
}

/// `On B`: Prefix, a Select of the absent layer B, Suffix.
fn named_select_action(app: &mut PhotocraftApp) {
    app.session
        .actions_mut()
        .list
        .push(Action::new(
            "On B",
            vec![
                ActionStep::new("layer.new.layer", json!({"name": "Prefix"})),
                ActionStep::new("layer.select", json!({"name": "B"})),
                ActionStep::new("layer.new.layer", json!({"name": "Suffix"})),
            ],
        ))
        .unwrap();
}

/// How many standard prompts the app holds open.
fn prompts_open(app: &PhotocraftApp) -> usize {
    let host = &app.framework.dialogs;
    host.stack().ids().filter(|id| host.get_by_id::<StandardPrompt>(*id).is_some()).count()
}

#[test]
fn a_step_naming_a_missing_layer_asks_continue_or_stop() {
    let mut app = missing_name_app();
    named_select_action(&mut app);
    let v = play(&mut app, &json!({"action": "On B"})).unwrap();
    assert_eq!(v["failed"]["step"], 1, "{v}");
    assert_eq!(prompts_open(&app), 1, "the stopped step shows its message with the choice");
    assert_eq!(count(&app, "Prefix"), 1, "the step already applied stays applied");
    assert_eq!(count(&app, "Suffix"), 0, "playback stops at the step naming B");
    let dialog = stop_prompt(&app);
    answer(&mut app, dialog, true).unwrap();
    assert_eq!(count(&app, "Suffix"), 1, "Continue skips the missing layer's step and finishes the action");
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_idle(&app);
    // The whole Play, across the message, stays one history state (as a Stop message's does).
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Action: On B"), "{}", app.ui.status);
    app.session.undo();
    assert_eq!(count(&app, "Prefix"), 0, "one undo takes back the whole play");
    assert_eq!(count(&app, "Suffix"), 0);
}

#[test]
fn stop_on_the_message_ends_the_playback_with_nothing_undone() {
    let mut app = missing_name_app();
    named_select_action(&mut app);
    let v = play(&mut app, &json!({"action": "On B"})).unwrap();
    assert_eq!(v["failed"]["step"], 1, "{v}");
    let dialog = stop_prompt(&app);
    answer(&mut app, dialog, false).unwrap();
    assert_eq!(count(&app, "Prefix"), 1, "Stop undoes nothing");
    assert_eq!(count(&app, "Suffix"), 0);
    assert_idle(&app);
    // The edits so far stay one history state behind the message, as Continue's do: Stop undoes
    // nothing itself, and one undo takes the whole play back (as a Stop message's does).
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Action: On B"), "{}", app.ui.status);
    assert!(app.session.undo(), "one undo takes back the whole play");
    assert_eq!(count(&app, "Prefix"), 0);
}

/// A Play over a set of actions: Continue finishes the failed action, then the set goes on.
#[test]
fn continue_carries_the_rest_of_a_set_play() {
    let mut app = missing_name_app();
    named_select_action(&mut app);
    app.session.actions_mut().list.push(Action::new("Next", vec![ActionStep::new("layer.new.layer", json!({"name": "Next layer"}))])).unwrap();
    let v = play_targets(&mut app, &[0, 1], 0, 0).unwrap().unwrap();
    assert_eq!(v["failed"]["step"], 1, "{v}");
    assert_eq!(prompts_open(&app), 1);
    let dialog = stop_prompt(&app);
    answer(&mut app, dialog, true).unwrap();
    assert_eq!(count(&app, "Suffix"), 1);
    assert_eq!(count(&app, "Next layer"), 1, "the actions Play queued after it still play");
    assert_idle(&app);
}

/// A person's Play of an action that calls another: the called action's step naming a missing
/// layer asks its message, and Continue ends the child there and lets the caller go on
/// (the pause carries its callers, as a Stop message's does).
#[test]
fn a_called_actions_missing_layer_step_asks_and_continues_into_its_caller() {
    let mut app = missing_name_app();
    app.session
        .actions_mut()
        .list
        .push(Action::new("Child B", vec![ActionStep::new("layer.select", json!({"name": "B"})), ActionStep::new("layer.new.layer", json!({"name": "Tail"}))]))
        .unwrap();
    app.session
        .actions_mut()
        .list
        .push(Action::new(
            "Caller",
            vec![ActionStep::new("actions.play", json!({"action": "Child B"})), ActionStep::new("layer.new.layer", json!({"name": "After"}))],
        ))
        .unwrap();
    let v = play(&mut app, &json!({"action": "Caller"})).unwrap();
    assert!(v.get("failed").is_none(), "the caller waits at its child's message: {v}");
    assert_eq!(prompts_open(&app), 1);
    let dialog = stop_prompt(&app);
    answer(&mut app, dialog, true).unwrap();
    assert_eq!(count(&app, "Tail"), 1, "Continue skips the failed step inside the child");
    assert_eq!(count(&app, "After"), 1, "and the caller goes on");
    assert_idle(&app);
}

/// Agent Play stays headless: no message opens; the failed step is reported as it is today. Both
/// non-interactive flags are tested, not just claimed: `automation_input` for synthetic input, and
/// `programmatic_playback`, which the control channel's `engine.execute` sets around its call.
#[test]
fn automation_playback_keeps_the_headless_failure() {
    for automated in ["automation_input", "programmatic_playback"] {
        let mut app = missing_name_app();
        named_select_action(&mut app);
        match automated {
            "automation_input" => app.automation_input = true,
            _ => app.programmatic_playback = true,
        }
        let v = play(&mut app, &json!({"action": "On B"})).unwrap();
        assert_eq!(v["failed"]["step"], 1, "{automated}: {v}");
        assert_eq!(prompts_open(&app), 0, "{automated}: the agent gets the failure, not the message");
        assert_eq!(count(&app, "Suffix"), 0, "{automated}");
        assert_idle(&app);
    }
}

/// The messages, as measured: the reference reports a Select of a missing layer as the command
/// being unavailable, and other named steps as the layer object being unavailable.
#[test]
fn the_message_names_the_command_or_the_missing_layer() {
    let app = missing_name_app();
    let action = Action::new(
        "Forms",
        vec![
            ActionStep::new("layer.select", json!({"name": "B"})),
            ActionStep::new("layer.select", json!({"layer": "B"})),
            ActionStep::new("layer.setProps", json!({"visible": false, "_actionLayer": {"name": "B", "ordinal": 0}})),
            ActionStep::new("layer.translate", json!({"dx": 3.0, "dy": 10.0, "layer": "B"})),
            ActionStep::new("select.loadSelection", json!({"channel": "transparency", "layer": {"name": "B"}})),
        ],
    );
    let message =
        |step: usize| runner::missing_layer_prompt(&app, &action, &json!({"step": step, "id": action.steps[step].command, "error": "x"})).map(|(m, _)| m);
    assert_eq!(message(0).as_deref(), Some("The command 'Select' is not currently available"));
    assert_eq!(message(1).as_deref(), Some("The command 'Select' is not currently available"));
    let object = || Some("The object 'Layer \"B\"' is not currently available".to_string());
    assert_eq!(message(2), object());
    assert_eq!(message(3), object());
    assert_eq!(message(4), object(), "a source layer named as an object");
}

/// Neighbours keep today's status-bar failure: steps naming no layer (a target step, a numeric id
/// kept for an ambiguous name, a rename's new name, a layer-id list), a name the document has, and
/// a step index the action does not have.
#[test]
fn a_failure_that_names_no_layer_keeps_the_plain_report() {
    let mut app = missing_name_app();
    let plain = Action::new(
        "Plain",
        vec![
            ActionStep::new("layer.setProps", json!({"opacity": 0.5})),
            ActionStep::new("layer.delete", json!({"layer": 99})),
            ActionStep::new("layer.setProps", json!({"name": "Fresh"})),
            ActionStep::new("layer.select", json!({"layers": [1, 2]})),
            ActionStep::new("layer.select", json!({"name": "Z"})),
        ],
    );
    for step in 0..5 {
        assert_eq!(runner::missing_layer_prompt(&app, &plain, &json!({"step": step, "id": plain.steps[step].command, "error": "x"})), None, "step {step}");
    }
    assert_eq!(runner::missing_layer_prompt(&app, &plain, &json!({"step": 9, "id": "layer.select", "error": "x"})), None, "no such step");
    app.session.actions_mut().list.push(plain).unwrap();
    let v = play(&mut app, &json!({"action": "Plain"})).unwrap();
    report_play(&mut app, &v);
    assert_eq!(v["failed"]["step"], 1, "{v}");
    assert_eq!(prompts_open(&app), 0, "the numeric id of an ambiguous name keeps today's failure report");
    assert!(app.ui.status_error, "the status bar still shows the failure: {}", app.ui.status);
    assert_idle(&app);
}

/// The presence test mirrors how the engine resolves a layer param: exact name, else the same name
/// whatever its case (engine `explicit_layer`). A step naming "background" over a document whose
/// layer is "Background" does find that layer, so a failure there is unrelated to a missing layer
/// and keeps today's plain failure: no message, no skip on Continue.
#[test]
fn a_name_the_document_has_in_another_case_is_not_missing() {
    let mut app = missing_name_app();
    let action = Action::new("Pass Through", vec![ActionStep::new("layer.setProps", json!({"layer": "background", "blend": "Pass Through"}))]);
    let failed = json!({"step": 0, "id": "layer.setProps", "error": "Pass Through applies to groups only; layer 0 isn't one"});
    assert_eq!(
        runner::missing_layer_prompt(&app, &action, &failed),
        None,
        "the engine finds Background for `background`, so the ask would skip a step that can never pass"
    );
    app.session.actions_mut().list.push(action).unwrap();
    let v = play(&mut app, &json!({"action": "Pass Through"})).unwrap();
    report_play(&mut app, &v);
    assert_eq!(v["failed"]["step"], 0, "{v}");
    assert_eq!(prompts_open(&app), 0, "the setProps failure takes today's plain-failure path");
    assert!(v["failed"]["error"].as_str().is_some_and(|error| error.contains("Pass Through")), "{v}");
    assert!(app.ui.status_error, "the status bar shows the failure: {}", app.ui.status);
    assert_idle(&app);
}

/// The other direction: the step records the exact-case name of a differently cased document.
#[test]
fn an_exact_case_name_on_a_lowercase_layer_is_not_missing() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 6})).unwrap();
    app.run("layer.new.layer", json!({"name": "ink"})).unwrap();
    let action = Action::new("Hide Ink", vec![ActionStep::new("layer.setProps", json!({"visible": false, "layer": "Ink"}))]);
    let failed = json!({"step": 0, "id": "layer.setProps", "error": "x"});
    assert_eq!(runner::missing_layer_prompt(&app, &action, &failed), None, "the engine finds `ink` for `Ink` by case");
}

/// The other two forms a recorded step can name its layer by are matched EXACTLY by the engine, so
/// a name differing only in case is a layer the playing document does not have: the visibility
/// step's `_actionLayer` goes through `playback_params`, at the recorded ordinal. Over a document
/// with layer `b` the step naming `B` stops there, and the person is asked rather than getting
/// today's plain failure (#10769 review).
#[test]
fn a_recorded_visibility_step_naming_the_layer_in_another_case_still_asks() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 6, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({"name": "b"})).unwrap();
    let hide = ActionStep::new("layer.setProps", json!({"visible": false, "_actionLayer": {"name": "B", "ordinal": 0}}));
    let suffix = ActionStep::new("layer.new.layer", json!({"name": "Suffix"}));
    app.session.actions_mut().list.push(Action::new("Hide B", vec![hide, suffix])).unwrap();
    let v = play(&mut app, &json!({"action": "Hide B"})).unwrap();
    assert_eq!(v["failed"]["step"], 0, "{v}");
    assert!(v["failed"]["error"].as_str().is_some_and(|error| error.contains("recorded layer `B`")), "the engine's exact lookup is what fails: {v}");
    assert_eq!(prompts_open(&app), 1, "the name is missing for this step's kind, so the play asks");
    let dialog = stop_prompt(&app);
    answer(&mut app, dialog, true).unwrap();
    assert_eq!(count(&app, "Suffix"), 1, "Continue skips the step and finishes the action");
    let doc = app.session.active().unwrap().doc.clone();
    assert!(doc.walk().into_iter().any(|(_, _, layer)| layer.name == "b" && layer.visible), "the step never hid the differently cased layer");
    assert_idle(&app);
}

/// The same for a Select step: its `name` goes through `selection_target`, which matches the exact
/// name at `occurrence`, so over a document with `b` the step naming `B` asks with the Select
/// message instead of keeping a plain failure.
#[test]
fn a_select_step_naming_the_layer_in_another_case_still_asks() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 6, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({"name": "b"})).unwrap();
    let select = ActionStep::new("layer.select", json!({"name": "B"}));
    let suffix = ActionStep::new("layer.new.layer", json!({"name": "Suffix"}));
    app.session.actions_mut().list.push(Action::new("On B", vec![select, suffix])).unwrap();
    let v = play(&mut app, &json!({"action": "On B"})).unwrap();
    assert_eq!(v["failed"]["step"], 0, "{v}");
    assert_eq!(prompts_open(&app), 1, "the engine finds no `B` in this document, so the play asks");
    let dialog = stop_prompt(&app);
    answer(&mut app, dialog, true).unwrap();
    assert_eq!(count(&app, "Suffix"), 1, "Continue skips the Select and finishes the action");
    assert_idle(&app);
}

/// The ask stays a test of the NAME and its case, not of the recorded occurrence: a visibility step
/// naming a name the document has, at an ordinal past its last layer of that name, still fails in
/// the engine and keeps today's plain report. A pin, green before and after this change.
#[test]
fn a_name_the_document_has_at_another_occurrence_keeps_the_plain_report() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 6, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({"name": "B"})).unwrap();
    let action = Action::new("Hide second B", vec![ActionStep::new("layer.setProps", json!({"visible": false, "_actionLayer": {"name": "B", "ordinal": 1}}))]);
    let failed = json!({"step": 0, "id": "layer.setProps", "error": "recorded layer `B` does not exist"});
    assert_eq!(runner::missing_layer_prompt(&app, &action, &failed), None, "the document has the name, only the recorded occurrence is missing");
    app.session.actions_mut().list.push(action).unwrap();
    let v = play(&mut app, &json!({"action": "Hide second B"})).unwrap();
    assert_eq!(v["failed"]["step"], 0, "{v}");
    assert_eq!(prompts_open(&app), 0, "an ordinal past the last layer of that name keeps today's plain failure");
    assert_idle(&app);
}

/// A player refusal is reported at the step Play starts at, under the Play label — not under the
/// refused step's own command. When that starting step is one that names a layer, the report must
/// not be read as a missing layer: the ask only follows a failure whose id is the step's command,
/// so a refusal keeps its plain failure report and Continue is never offered on it. (The engine's
/// `playback_plan` pre-refuses a recursion or a 17th level at the calling step, so this report
/// shape is pinned at the ask rather than reproduced through a playback.)
#[test]
fn a_refusal_report_at_a_layer_naming_start_step_is_not_the_ask() {
    let mut app = missing_name_app();
    app.session
        .actions_mut()
        .list
        .push(Action::new(
            "Starts on B",
            vec![ActionStep::new("layer.select", json!({"name": "B"})), ActionStep::new("layer.new.layer", json!({"name": "Suffix"}))],
        ))
        .unwrap();
    let action = app.session.actions().list.action(0).unwrap().clone();
    let refused = json!({"step": 0, "id": "actions.play", "error": "action nesting exceeds 16 levels"});
    assert_eq!(
        runner::missing_layer_prompt(&app, &action, &refused),
        None,
        "a refusal must not be reported as a missing layer, and Continue must not skip from it"
    );
    // The step's own failure, same index, does ask: only the id changed.
    let own = json!({"step": 0, "id": "layer.select", "error": "no layer named \"B\""});
    assert_eq!(runner::missing_layer_prompt(&app, &action, &own).as_ref().map(|(m, _)| m.as_str()), Some("The command 'Select' is not currently available"));
}
