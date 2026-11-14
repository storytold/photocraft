//! The Modern panel enters its incremental runner through the registered Play command.
use super::{Action, PhotocraftApp, modern};
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width":8,"height":6})).unwrap();
    app
}

fn names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.walk().iter().map(|(_, _, layer)| layer.name.clone()).filter(|name| name != "Background").collect()
}

#[test]
fn panel_play_dispatches_registered_command_and_keeps_incremental_ownership() {
    let mut app = app();
    let steps = (0..12).map(|index| ("layer.new.layer".into(), json!({"name":format!("Step {index}")}))).collect();
    app.session.actions.list.push(Action::new("Stream", steps));
    let value = modern::play_library(&mut app, &[0]).unwrap();
    assert_eq!(value["playing"], true);
    assert_eq!(app.perf.last_command, "actions.play", "the top-level request passes through app command dispatch");
    assert!(names(&app).is_empty(), "queuing must not execute the action synchronously");
    assert!(app.session.actions.held_playback.is_some());
    let ctx = egui::Context::default();
    modern::tick(&mut app, &ctx);
    assert_eq!(names(&app), (0..8).map(|index| format!("Step {index}")).collect::<Vec<_>>());
    assert!(app.session.actions.held_playback.is_some(), "the same playback still owns its remaining slice");
    modern::tick(&mut app, &ctx);
    assert_eq!(names(&app), (0..12).map(|index| format!("Step {index}")).collect::<Vec<_>>());
    assert!(app.session.actions.held_playback.is_none());
    assert_eq!(modern::inspect(&app)["playing"]["done"], true);
    assert_eq!(app.run_depth, 0);
}

#[test]
fn refused_panel_dispatch_cannot_leave_a_queue_mode_on_the_next_play() {
    let mut app = app();
    app.session.actions.list.extend([
        Action::new("First", vec![("layer.new.layer".into(), json!({"name":"First"}))]),
        Action::new("Second", vec![("layer.new.layer".into(), json!({"name":"Second"}))]),
    ]);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let calls = seen.clone();
    app.services.automation_command = Some(Box::new(move |id, params| {
        calls.borrow_mut().push((id.to_string(), params.clone()));
        if id == "actions.play" { Err("panel request denied".into()) } else { Ok(()) }
    }));
    app.automation_input = true;
    assert_eq!(modern::play_library(&mut app, &[0, 1]).unwrap_err(), "panel request denied");
    assert_eq!(seen.borrow().as_slice(), &[("actions.play".to_string(), json!({"action":0}))]);
    assert!(names(&app).is_empty());
    assert!(app.session.actions.held_playback.is_none());
    assert!(modern::inspect(&app)["playing"].is_null());
    app.automation_input = false;
    let value = app.run("actions.play", json!({"action":0})).unwrap();
    assert_eq!(value["ran"], 1);
    assert!(value.get("playing").is_none());
    assert_eq!(names(&app), ["First"], "a rejected group request must not leak its tail");
    assert!(app.session.actions.held_playback.is_none());
}

#[test]
fn queued_group_dispatch_keeps_captured_policy_and_rejects_a_changed_live_gate() {
    fn captured(id: &str, params: &Value) -> photocraft_engine::Result<()> {
        if id == "layer.new.layer" && params["name"] == "Forbidden" {
            Err(photocraft_engine::EngineError::Other("captured group policy".into()))
        } else {
            Ok(())
        }
    }
    fn changed(_: &str, _: &Value) -> photocraft_engine::Result<()> {
        Ok(())
    }
    for from_service in [false, true] {
        for change_live in [false, true] {
            let mut app = app();
            app.session.actions.list.extend([
                Action::new("First", vec![("layer.new.layer".into(), json!({"name":"Allowed"}))]),
                Action::new("Second", vec![("layer.new.layer".into(), json!({"name":"Forbidden"}))]),
            ]);
            if from_service {
                app.automation_input = true;
                app.services.automation_authorize = Some(captured);
            } else {
                app.session.authorize = Some(captured);
            }
            modern::play_library(&mut app, &[0, 1]).unwrap();
            app.automation_input = false;
            app.services.automation_authorize = None;
            app.session.authorize = None;
            let ctx = egui::Context::default();
            modern::tick(&mut app, &ctx);
            assert_eq!(names(&app), ["Allowed"]);
            if change_live {
                app.session.authorize = Some(changed);
            }
            modern::tick(&mut app, &ctx);
            assert_eq!(names(&app), ["Allowed"], "the next group action must retain the captured restriction");
            assert!(app.ui.status_error);
            assert!(app.ui.status.contains(if change_live { "authorization context changed" } else { "captured group policy" }), "{}", app.ui.status);
            assert!(app.session.actions.held_playback.is_none());
            assert!(app.session.actions.playback_stack.is_empty());
        }
    }
}

#[test]
fn ordinary_registered_play_stays_synchronous_for_classic_and_automation_callers() {
    for classic in [false, true] {
        for automated in [false, true] {
            let mut app = app();
            app.ui.actions_layout = if classic { crate::state::ActionsLayout::Classic } else { crate::state::ActionsLayout::Modern };
            app.session.actions.list.push(Action::new("Ordinary", vec![("layer.new.layer".into(), json!({"name":"Immediate"}))]));
            app.automation_input = automated;
            let value = app.run("actions.play", json!({"action":0})).unwrap();
            assert_eq!(value["ran"], 1);
            assert!(value.get("playing").is_none());
            assert_eq!(names(&app), ["Immediate"]);
            assert!(app.session.actions.held_playback.is_none());
            assert!(modern::inspect(&app)["playing"].is_null());
            assert_eq!(app.run_depth, 0);
        }
    }
}
