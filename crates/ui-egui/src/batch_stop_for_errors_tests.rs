//! The Batch Stop For Errors question (#13667) through the real dialog: a failing file parks
//! the run and the desktop asks with the step's error, the failing file's name and Continue /
//! Stop buttons. Continue (the focused answer) goes on; Stop asks the second question, whose
//! focused Stop answer ends the run.

use std::time::Duration;

use craft::ui::dialog::StandardPrompt;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_engine::jobs::{JobId, Started};
use serde_json::json;

use crate::PhotocraftApp;

fn app_harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(8).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 320, "height": 240})).unwrap();
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default());
        app.background_jobs = true;
        app
    });
    h.run_steps(4);
    h
}

/// Step at least `min` frames and until `ready` holds; fail after 60 s.
fn frames_until(h: &mut Harness<'_, Box<PhotocraftApp>>, min: u64, ready: impl Fn(&Harness<'_, Box<PhotocraftApp>>) -> bool) {
    let t = std::time::Instant::now();
    let mut n = 0;
    while n < min || !ready(h) {
        assert!(t.elapsed() < Duration::from_secs(60), "not ready after {n} frames");
        h.step();
        n += 1;
    }
}

/// A desktop Batch worker parks on each failed input and reports both answers in its result.
fn pausing_batch(h: &mut Harness<'_, Box<PhotocraftApp>>) -> photocraft_engine::jobs::JobId {
    h.state_mut().session.set_batch_failure_questions(true);
    let Started::Job(id) = h
        .state_mut()
        .session
        .start_job(
            "file.automate.batch",
            json!({}),
            "Batch",
            false,
            |ctx| {
                let first = ctx.batch_pause_for_error("broken.png", "The command \"Invert\" is not currently available.");
                // A Stop answer ends the run: only a Continue answer reaches the next input,
                // whose failure asks the same question again.
                let second =
                    if first == Some(true) { ctx.batch_pause_for_error("also broken.png", "The command \"Levels\" is not currently available.") } else { None };
                Ok(json!({"first": first, "second": second}))
            },
            |_, result| Ok(result),
        )
        .unwrap()
    else {
        panic!("batch did not start as a job")
    };
    id
}

fn question_text(h: &Harness<'_, Box<PhotocraftApp>>) -> bool {
    h.query_all_by_label("The command \"Invert\" is not currently available.").count() >= 1
}

const STOP_CONFIRM: &str = "A command was canceled or stopped during batch processing. Continue with the next file or stop?";

#[test]
fn stop_for_errors_failure_question_offers_continue_and_stop_and_enter_goes_on() {
    let mut h = app_harness();
    let id = pausing_batch(&mut h);
    frames_until(&mut h, 1, question_text);
    assert!(h.query_all_by_label("broken.png").count() >= 1, "the question names the failing file");
    assert!(h.query_by_label("Continue").is_some() && h.query_by_label("Stop").is_some(), "the question offers Continue and Stop");
    // Continue is the focused answer: Enter answers it and the same run asks about the next file.
    h.key_press(egui::Key::Enter);
    frames_until(&mut h, 1, |h| h.query_all_by_label("The command \"Levels\" is not currently available.").count() >= 1);
    assert!(h.query_all_by_label("also broken.png").count() >= 1, "a further failure asks again");
    // Stop brings the second question; there, Stop is the focused answer.
    h.get_by_label("Stop").click();
    frames_until(&mut h, 1, |h| h.query_all_by_label(STOP_CONFIRM).count() >= 1);
    assert!(h.query_by_label("Continue").is_some() && h.query_by_label("Stop").is_some(), "the second question offers Continue and Stop");
    h.key_press(egui::Key::Enter);
    frames_until(&mut h, 1, |h| h.state().session.job(id).is_none());
    let result = h.state().session.jobs_with_recent().into_iter().find(|job| job.id == id).and_then(|job| job.result).unwrap();
    assert_eq!(result["first"], json!(true), "Enter answered the failure question with Continue");
    assert_eq!(result["second"], json!(false), "Enter answered the second question with Stop");
}

#[test]
fn stop_for_errors_stop_on_the_failure_question_can_still_continue() {
    let mut h = app_harness();
    let id = pausing_batch(&mut h);
    frames_until(&mut h, 1, question_text);
    h.get_by_label("Stop").click();
    frames_until(&mut h, 1, |h| h.query_all_by_label(STOP_CONFIRM).count() >= 1);
    h.get_by_label("Continue").click();
    // The run went on to the next input and its failure asks again.
    frames_until(&mut h, 1, |h| h.query_all_by_label("also broken.png").count() >= 1);
    h.key_press(egui::Key::Enter);
    frames_until(&mut h, 1, |h| h.state().session.job(id).is_none());
    let result = h.state().session.jobs_with_recent().into_iter().find(|job| job.id == id).and_then(|job| job.result).unwrap();
    assert_eq!(result["first"], json!(true), "the second question's Continue goes on with the next file");
    assert_eq!(result["second"], json!(true), "the next failure asked and Continue answered again");
}

#[test]
fn stop_for_errors_escape_on_the_failure_question_answers_stop() {
    let mut h = app_harness();
    let id = pausing_batch(&mut h);
    frames_until(&mut h, 1, question_text);
    h.key_press(egui::Key::Escape);
    frames_until(&mut h, 1, |h| h.query_all_by_label(STOP_CONFIRM).count() >= 1);
    // Escape on the second question answers its focused Stop, ending the run (#13667).
    h.key_press(egui::Key::Escape);
    frames_until(&mut h, 1, |h| h.state().session.job(id).is_none());
    let result = h.state().session.jobs_with_recent().into_iter().find(|job| job.id == id).and_then(|job| job.result).unwrap();
    assert_eq!(result["first"], json!(false), "the failure question's Escape answered Stop, and the second question's Escape ended the run");
    assert_eq!(result["second"], json!(null), "Stop ended the run before the next input");
}

/// One real PNG in a temp folder; the Batch's single step fails on it under Stop For Errors.
fn failing_folder_batch(dir: &std::path::Path) -> serde_json::Value {
    let image = photocraft_codecs::Image::from_u8(4, 4, photocraft_codecs::ChannelLayout::Rgba, [10u8, 200, 30, 255].repeat(16)).unwrap();
    std::fs::write(dir.join("one.png"), photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap()).unwrap();
    json!({
        "source": "folder",
        "input": dir.to_string_lossy(),
        "output": dir.join("out").to_string_lossy(),
        "format": "png",
        "stopOnError": true,
        "steps": [["layer.select", { "layer": "does not exist" }]],
    })
}

/// The control channel's own route: `ui.menu.invoke` of `file.automate.batch` with explicit
/// params runs the engine command (no form). A control client waits for the job instead of
/// answering prompts, so the run must keep stopping at the first failure (#13667, Done-when
/// "headless").
#[test]
fn stop_for_errors_a_control_channel_menu_batch_stops_at_the_first_failure() {
    let dir = std::env::temp_dir().join(format!("pc-13667-menu-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = app_harness();
    let ctx = h.ctx.clone();
    let (req, _rx) =
        crate::ControlRequest::new("ui.menu.invoke", json!({"command": "file.automate.batch", "params": failing_folder_batch(&dir), "wait": false}));
    let crate::control::Reply::Done(v) = crate::control::handle(h.state_mut(), &ctx, &req) else { panic!("wait: false replies at once") };
    let id = JobId(v["result"]["job"].as_u64().unwrap_or_else(|| panic!("the menu route started the Batch as a job: {v}")));
    let t = std::time::Instant::now();
    while h.state().session.job(id).is_some() && !h.state().session.batch_interrupt_waiting(id) && t.elapsed() < Duration::from_secs(30) {
        h.step();
    }
    let parked = h.state().session.batch_interrupt_waiting(id);
    if parked {
        h.state_mut().session.decide_batch_interrupt_job(id, false);
    }
    let t = std::time::Instant::now();
    while h.state().session.job(id).is_some() && t.elapsed() < Duration::from_secs(30) {
        h.step();
    }
    let result = h.state().session.jobs_with_recent().into_iter().find(|job| job.id == id).and_then(|job| job.result).expect("the batch reported");
    assert!(!parked, "a control-channel menu click parked the Batch on a failure question its caller cannot answer");
    assert!(result["stopped"].is_string(), "today's partial result stands: {result}");
    assert_eq!(result["errors"].as_array().map(Vec::len), Some(1), "{result}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A Batch the desktop route runs with automation input set (the state a control-channel
/// dialog answer or a synthetic pass leaves): Stop For Errors keeps stopping at the first
/// failure for headless callers, so this run never parks (#13667, Done-when "headless").
#[test]
fn stop_for_errors_an_automation_run_never_parks_and_ends_at_the_first_failure() {
    let dir = std::env::temp_dir().join(format!("pc-13667-ctl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = app_harness();
    let app = h.state_mut();
    app.automation_input = true;
    let r = app.run("file.automate.batch", failing_folder_batch(&dir)).unwrap();
    app.automation_input = false;
    let id = JobId(r["job"].as_u64().expect("the desktop's Batch runs as a background job"));
    let t = std::time::Instant::now();
    while h.state().session.job(id).is_some() && !h.state().session.batch_interrupt_waiting(id) && t.elapsed() < Duration::from_secs(30) {
        h.step();
    }
    let parked = h.state().session.batch_interrupt_waiting(id);
    if parked {
        // Release the parked worker so its thread ends before the failure reports.
        h.state_mut().session.decide_batch_interrupt_job(id, false);
    }
    let t = std::time::Instant::now();
    while h.state().session.job(id).is_some() && t.elapsed() < Duration::from_secs(30) {
        h.step();
    }
    let result = h.state().session.jobs_with_recent().into_iter().find(|job| job.id == id).and_then(|job| job.result).expect("the batch reported");
    assert!(!parked, "an automation run parked on the Stop For Errors question instead of stopping at the first failure");
    assert!(result["stopped"].is_string(), "today's partial result stands: {result}");
    assert_eq!(result["errors"].as_array().map(Vec::len), Some(1), "{result}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The same folder Batch through the plain desktop route (background jobs on, no automation or
/// playback flag): it arms and parks, so the headless gate cannot switch the person's question
/// off. Passes before and after the automation fix on purpose: it pins the desktop half of the
/// gate.
#[test]
fn stop_for_errors_the_desktop_route_arms_the_question() {
    let dir = std::env::temp_dir().join(format!("pc-13667-desk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = app_harness();
    let r = h.state_mut().run("file.automate.batch", failing_folder_batch(&dir)).unwrap();
    let id = JobId(r["job"].as_u64().expect("the desktop's Batch runs as a background job"));
    let t = std::time::Instant::now();
    while !h.state().session.batch_interrupt_waiting(id) && h.state().session.job(id).is_some() && t.elapsed() < Duration::from_secs(30) {
        h.step();
    }
    assert!(h.state().session.batch_interrupt_waiting(id), "the desktop run parks on the failure question");
    assert!(h.state_mut().session.decide_batch_interrupt_job(id, true), "the person's Continue answers through the existing channel");
    let t = std::time::Instant::now();
    while h.state().session.job(id).is_some() && t.elapsed() < Duration::from_secs(30) {
        h.step();
    }
    let result = h.state().session.jobs_with_recent().into_iter().find(|job| job.id == id).and_then(|job| job.result).expect("the batch reported");
    assert!(result.get("stopped").is_none(), "Continue went on past the failed file: {result}");
    assert_eq!(result["errors"].as_array().map(Vec::len), Some(1), "{result}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A `jobs.cancel` that ends the run while the failure question is on screen takes that prompt
/// with it: its Stop cannot open a second blocking question for a run that has already ended,
/// and no stale question is left on screen behind (#13667).
#[test]
fn stop_for_errors_a_cancelled_run_takes_its_open_prompt_with_it() {
    let mut h = app_harness();
    let id = pausing_batch(&mut h);
    frames_until(&mut h, 1, question_text);
    h.state_mut().session.cancel_job(id);
    let t = std::time::Instant::now();
    while h.state().session.job(id).is_some() && t.elapsed() < Duration::from_secs(30) {
        h.step();
    }
    assert!(h.state().session.job(id).is_none(), "the parked run ends on jobs.cancel");
    h.step();
    assert_eq!(h.query_all_by_label(STOP_CONFIRM).count(), 0, "the open Stop answer cannot ask a second question for a run that ended");
    assert!(!h.state().framework.dialogs.is_open::<StandardPrompt>(), "the question prompt left with its job");
}

/// The question's file name follows the platform's own path rules: a Unix name holding a
/// literal backslash stays whole, and a Windows path still names just its file (#13667).
#[test]
fn stop_for_errors_the_question_name_follows_the_platform_path_rules() {
    assert_eq!(crate::jobs_ui::batch_failure_name("broken.png"), "broken.png");
    #[cfg(windows)]
    assert_eq!(
        crate::jobs_ui::batch_failure_name(r"C:\Users\me\in\set\broken.png"),
        "broken.png",
        "Windows: separators split, the last component names the file"
    );
    #[cfg(unix)]
    assert_eq!(crate::jobs_ui::batch_failure_name("/in/set\\broken.png"), "set\\broken.png", "Unix: a backslash is part of the file name");
    #[cfg(unix)]
    assert_eq!(crate::jobs_ui::batch_failure_name("/in/broken.png"), "broken.png", "Unix: the separator splits");
    assert_eq!(crate::jobs_ui::batch_failure_name(""), "", "no file name: the input itself stands");
}
