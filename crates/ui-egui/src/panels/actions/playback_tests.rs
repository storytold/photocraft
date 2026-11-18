//! #5212: Playback Options. Step by Step and Pause For play the whole action from one Play, with a
//! frame (and the wait) between steps, in one history state when Create a single history state is
//! on; Esc and the Stop button end them keeping the steps applied. Measured in the reference
//! application 2026-11-05.
use super::*;
use photocraft_engine::actions_cmds::ActionStep;
use web_time::{Duration, Instant};

fn names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.walk().iter().map(|(_, _, l)| l.name.clone()).filter(|n| n != "Background").collect()
}

/// `Three` makes layers A, B and C.
fn app_with(performance: &str, seconds: u8, single: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 6})).unwrap();
    app.run("actions.setPlayback", json!({"performance": performance, "seconds": seconds, "singleState": single})).unwrap();
    let steps = ["A", "B", "C"].map(|n| ActionStep::new("layer.new.layer", json!({"name": n}))).to_vec();
    app.session.actions.list.push(Action::new("Three", steps)).unwrap();
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

#[test]
fn step_by_step_plays_every_step_from_one_play_with_a_frame_between() {
    for single in [true, false] {
        let mut app = app_with("step", 1, single);
        let ctx = app.ctx().clone();
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
    let ctx = app.ctx().clone();
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
        let ctx = app.ctx().clone();
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
                app.session.actions.list.push(Action::new("One", vec![ActionStep::new("layer.new.layer", json!({"name": "D"}))])).unwrap();
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

/// A frame that runs two passes gives its input to the first only: Esc ends the wait once, and the second pass
/// finds nothing waiting and plays nothing.
#[test]
fn esc_in_a_frame_of_two_passes_ends_the_wait_once() {
    let mut app = app_with("pause", 3, true);
    let ctx = app.ctx().clone();
    play(&mut app, &json!({"action": "Three"})).unwrap();
    next_step(&mut app, &ctx);
    assert_eq!(names(&app), ["A", "B"]);
    if let Some(timer) = app.paused_playbacks.timer.as_mut() {
        timer.due = Instant::now();
    }
    let raw = egui::RawInput { events: vec![esc()], ..Default::default() };
    ctx.run_ui(raw, |ui| {
        playback::tick(&mut app, ui.ctx());
        playback::tick(&mut app, ui.ctx());
    })
    .drop_without_applying_deltas();
    assert_eq!(names(&app), ["A", "B"], "the due step did not run: Esc ended the wait first");
    assert_idle(&app);
}

#[test]
fn an_edit_between_steps_ends_the_playback_without_joining_it() {
    let mut app = app_with("step", 1, true);
    let ctx = app.ctx().clone();
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
    let ctx = app.ctx().clone();
    app.session
        .actions
        .list
        .push(Action::new(
            "Caller",
            vec![
                ActionStep::new("layer.new.layer", json!({"name": "Before"})),
                ActionStep::new("actions.play", json!({"action": "Three"})),
                ActionStep::new("layer.new.layer", json!({"name": "After"})),
            ],
        ))
        .unwrap();
    let past = app.session.active().unwrap().history.past_len();
    play_targets(&mut app, &[1], 0, 0).unwrap();
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
    fast.session.actions.list.push(app.session.actions.list.actions()[1].clone()).unwrap();
    let v = play_targets(&mut fast, &[1], 0, 0).unwrap().unwrap();
    assert_eq!(v["ran"], 3, "{v}");
}

#[test]
fn a_paced_play_clears_an_earlier_playback_error() {
    let mut app = app_with("step", 1, true);
    app.ui.status = "Step 2 (layer.select) failed: no such layer".into();
    app.ui.status_error = true;
    let v = play_targets(&mut app, &[0], 0, 0).unwrap().unwrap();
    report_play(&mut app, &v);
    assert_eq!(names(&app), ["A"], "the play waits after its first step");
    assert!(app.paused_playbacks.timer.is_some());
    assert!(!app.ui.status_error, "the old failure is gone: {}", app.ui.status);
    assert!(app.ui.status.is_empty(), "{}", app.ui.status);
    // A status that isn't an error stays (a step's own message, say).
    let mut app = app_with("step", 1, true);
    app.ui.status = "Saved".into();
    let v = play_targets(&mut app, &[0], 0, 0).unwrap().unwrap();
    report_play(&mut app, &v);
    assert_eq!(app.ui.status, "Saved");
}

#[test]
fn playback_options_ok_refuses_values_automation_set_that_the_dialog_cannot_show() {
    use photocraft_engine::prefs::PlaybackPerformance;
    let control = |app: &mut PhotocraftApp, method: &str, params: Value| {
        let (request, _) = crate::control::ControlRequest::new(method, params);
        let ctx = app.ctx().clone();
        match crate::control::handle(app, &ctx, &request) {
            crate::control::Reply::Done(v) => v,
            _ => panic!("{method} answers at once"),
        }
    };
    for (field, value) in [
        ("performance", Value::Null),
        ("performance", json!("fast")),
        ("performance", json!(2)),
        ("singleState", Value::Null),
        ("singleState", json!("yes")),
        ("seconds", json!(75)),
        ("seconds", json!(2.5)),
    ] {
        let mut app = app_with("step", 4, false);
        let id = playback::open_dialog(&mut app).unwrap()["dialog"].clone();
        // The hosted dialog refuses a value its row cannot show when it is set, or else at OK.
        let set = control(&mut app, "ui.dialog.set", json!({"dialog": id, "field": field, "value": value}));
        if set["ok"] == true {
            let r = control(&mut app, "ui.dialog.confirm", json!({"dialog": id}));
            assert!(r["error"].is_string(), "{field} = {value}: {r}");
        }
        let a = &app.session.prefs().actions;
        assert_eq!(
            (a.performance, a.pause_seconds, a.create_single_history_state),
            (PlaybackPerformance::StepByStep, 4, false),
            "{field} = {value} saves nothing"
        );
    }
    // Valid values through the same path are saved.
    let mut app = app_with("step", 4, false);
    let id = playback::open_dialog(&mut app).unwrap()["dialog"].clone();
    control(&mut app, "ui.dialog.set", json!({"dialog": id, "field": "performance", "value": "accelerated"}));
    control(&mut app, "ui.dialog.set", json!({"dialog": id, "field": "singleState", "value": true}));
    assert_eq!(control(&mut app, "ui.dialog.confirm", json!({"dialog": id}))["ok"], true);
    let a = &app.session.prefs().actions;
    assert_eq!((a.performance, a.create_single_history_state), (PlaybackPerformance::Accelerated, true));
}
