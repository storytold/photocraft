//! The Batch Stop For Errors question (#13667) through the real dialog: a failing file parks
//! the run and the desktop asks with the step's error, the failing file's name and Continue /
//! Stop buttons. Continue (the focused answer) goes on; Stop asks the second question, whose
//! focused Stop answer ends the run.

use std::time::Duration;

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_engine::jobs::Started;
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
