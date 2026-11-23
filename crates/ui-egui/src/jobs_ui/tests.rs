//! UI tests for background jobs (#210): the whole app (eframe harness, CPU canvas) keeps
//! rendering frames while a slow job runs, shows the status-bar progress and the blocking progress
//! overlay, and Esc / Cancel leave the document unchanged.

use photocraft_engine::color_cmds::ColorExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use craft::behavior::kurbo::Point;
use craft::behavior::{EditEvent, Mixed};
use craft::draw::ui::transform::TransformEdit;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::jobs::{JobId, JobOutcome, Started};
use serde_json::{Value, json};

use crate::PhotocraftApp;

mod pending_open_view;

fn app_harness() -> Harness<'static, Box<PhotocraftApp>> {
    app_harness_with(crate::Services::default())
}

/// [`app_harness`] over the given services (the beep test records the hook's calls).
fn app_harness_with(services: crate::Services) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(8).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 320, "height": 240})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#3366cc"})).unwrap();
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), s, services);
        app.background_jobs = true;
        app
    });
    h.run_steps(4);
    h
}

/// A fake document job that runs until `gate` opens (or it is cancelled), reporting 40 %.
fn slow_job(app: &mut PhotocraftApp, gate: &Arc<AtomicBool>) -> JobId {
    let gate = gate.clone();
    let r = app
        .session
        .start_job(
            "test.slow",
            json!({}),
            "Slow Filter",
            true,
            move |ctx| {
                ctx.progress(0.4, "Filtering tiles");
                while !gate.load(Ordering::Relaxed) {
                    ctx.check()?;
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(())
            },
            |s, ()| {
                s.edit("Slow Filter", |_, _| Ok(()))?;
                Ok(json!({"slow": true}))
            },
        )
        .unwrap();
    match r {
        Started::Job(id) => id,
        Started::Done(v) => panic!("ran inline: {v}"),
    }
}

/// Step frames for `ms` of wall time; returns (frames, slowest frame in ms).
fn frames_for(h: &mut Harness<'_, Box<PhotocraftApp>>, ms: u64) -> (u64, f64) {
    frames_for_at_least(h, ms, 0)
}

/// Prove the frame loop advances while a gated worker stays blocked, without assuming a
/// minimum frame rate on software-rendered CI machines. Also wait for the progress modal.
fn frames_for_at_least(h: &mut Harness<'_, Box<PhotocraftApp>>, ms: u64, min_frames: u64) -> (u64, f64) {
    let t = Instant::now();
    let (mut n, mut worst) = (0, 0.0f64);
    while t.elapsed() < Duration::from_millis(ms) || n < min_frames {
        assert!(t.elapsed() < Duration::from_secs(30), "frame observation timed out after {n} frames");
        let f = Instant::now();
        h.step();
        worst = worst.max(f.elapsed().as_secs_f64() * 1000.0);
        n += 1;
    }
    (n, worst)
}

/// Step at least `min` frames and until `ready` holds; returns (frames, slowest frame in ms).
/// The test fails when `ready` still does not hold after 60 s.
fn frames_until(h: &mut Harness<'_, Box<PhotocraftApp>>, min: u64, ready: impl Fn(&Harness<'_, Box<PhotocraftApp>>) -> bool) -> (u64, f64) {
    let t = Instant::now();
    let (mut n, mut worst) = (0, 0.0f64);
    while n < min || !ready(h) {
        assert!(t.elapsed() < Duration::from_secs(60), "not ready after {n} frames");
        let f = Instant::now();
        h.step();
        worst = worst.max(f.elapsed().as_secs_f64() * 1000.0);
        n += 1;
    }
    (n, worst)
}

#[test]
fn a_job_started_while_drawing_requests_the_follow_up_poll() {
    let mut h = app_harness();
    let gate = Arc::new(AtomicBool::new(false));
    let job = slow_job(h.state_mut(), &gate);
    let repaints = Arc::new(AtomicUsize::new(0));
    let seen = repaints.clone();
    h.ctx.set_request_repaint_callback(move |_| {
        seen.fetch_add(1, Ordering::Relaxed);
    });

    // A job may start from a menu or dialog after the frame's initial `jobs_ui::tick`.
    // The UI must still schedule a poll, or an idle Wayland event loop leaves the document
    // locked after the worker finishes and blocks Edit > Undo / Redo.
    assert!(crate::jobs_ui::keep_polling(h.state(), &h.ctx));
    assert_eq!(repaints.load(Ordering::Relaxed), 1);

    crate::jobs_ui::cancel(h.state_mut(), job);
    h.run_steps(2);
}

#[test]
fn a_finished_job_lands_as_one_undo_step_and_the_dialog_cancel_button_works() {
    let mut h = app_harness();
    let gate = Arc::new(AtomicBool::new(false));
    let steps = h.state().session.active().unwrap().history.past_len();
    slow_job(h.state_mut(), &gate);
    frames_for(&mut h, 320);
    gate.store(true, Ordering::Relaxed);
    let t = Instant::now();
    while h.state().session.has_jobs() && t.elapsed() < Duration::from_secs(10) {
        h.step();
    }
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);

    // A second job, cancelled from the overlay's Cancel button (it closes as `Requested`).
    let gate = Arc::new(AtomicBool::new(false));
    let job = slow_job(h.state_mut(), &gate);
    frames_for(&mut h, 320);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().session.job(job).is_none());
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
}

#[test]
fn filters_from_the_ui_run_in_the_background() {
    let mut h = app_harness();
    let steps = h.state().session.active().unwrap().history.past_len();
    let r = h.state_mut().run("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
    assert_eq!(r["pending"], true, "{r}");
    let t = Instant::now();
    while h.state().session.has_jobs() && t.elapsed() < Duration::from_secs(30) {
        h.step();
    }
    h.run_steps(2);
    let st = h.state().session.active().unwrap();
    assert_eq!(st.history.past_len(), steps + 1, "one step for the blur");
    assert_eq!(h.state().ui.status, "Gaussian Blur");
}

#[test]
fn control_requests_wait_for_jobs_unless_asked_not_to() {
    let mut h = app_harness();
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::ControlRequest::new("engine.execute", json!({"command": "filter.blur.gaussianBlur", "params": {"radius": 2}}));
    let job = match crate::control::handle(h.state_mut(), &ctx, &req) {
        crate::control::Reply::AfterJob(job) => JobId(job),
        _ => panic!("expected a deferred reply"),
    };
    assert!(h.state().session.jobs().iter().any(|j| j.id == job) || h.state().session.jobs_with_recent().iter().any(|j| j.id == job));
    // The app's frame loop answers the request when the job lands.
    let (req, rx) = crate::ControlRequest::new("engine.execute", json!({"command": "jobs.list"}));
    let _ = crate::control::handle(h.state_mut(), &ctx, &req);
    drop(rx);
    let reply = job_reply(&mut h, job);
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(reply["result"]["filter"].is_object(), "{reply}");

    // `wait: false` replies with the job id at once.
    let (req, _rx) = crate::ControlRequest::new("engine.execute", json!({"command": "filter.blur.gaussianBlur", "params": {"radius": 2}, "wait": false}));
    let crate::control::Reply::Done(v) = crate::control::handle(h.state_mut(), &ctx, &req) else { panic!("expected an immediate reply") };
    let id = v["result"]["job"].as_u64().unwrap();
    let (req, _rx) = crate::ControlRequest::new("jobs.cancel", json!({"job": id}));
    let crate::control::Reply::Done(v) = crate::control::handle(h.state_mut(), &ctx, &req) else { panic!() };
    assert_eq!(v["ok"], true, "{v}");
    let (req, _rx) = crate::ControlRequest::new("jobs.cancel", json!({"job": 99999}));
    let crate::control::Reply::Done(v) = crate::control::handle(h.state_mut(), &ctx, &req) else { panic!() };
    assert_eq!(v["ok"], false);
}

#[test]
fn accepted_control_job_applies_after_its_dispatch_deadline() {
    let mut h = app_harness();
    let ctx = h.ctx.clone();
    let steps = h.state().session.active().unwrap().history.past_len();
    let deadline = Instant::now() + Duration::from_secs(1);
    let (mut req, reply) = crate::ControlRequest::new("engine.execute", json!({"command": "filter.blur.gaussianBlur", "params": {"radius": 2}}));
    req.deadline = Some(deadline);
    let (tx, rx) = std::sync::mpsc::channel();
    h.state_mut().control = Some(craft::automation::ControlChannel::new(rx));
    tx.send(req).unwrap();
    h.state_mut().drain_control(&ctx);
    assert_eq!(h.state().control.as_ref().map(craft::automation::ControlChannel::waiting), Some(1), "the accepted request waits for its job");
    let job = h.state().session.jobs().first().expect("the filter runs as a job").id;
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "the job has not been applied yet");
    assert!(reply.try_recv().is_err());
    // No frames poll the worker until the transport's deadline has elapsed.
    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
    assert!(Instant::now() >= deadline);
    drop(reply);
    let result = job_reply(&mut h, job);
    assert_eq!(result["ok"], true, "{result}");
    assert!(result["result"]["filter"].is_object(), "{result}");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
}

/// A control request waiting for `job` to end, as a waiting `engine.execute` leaves one.
fn wait_for_job(app: &mut PhotocraftApp, job: JobId) -> std::sync::mpsc::Receiver<Value> {
    let (req, reply) = crate::ControlRequest::new("engine.execute", json!({}));
    let channel = app.control.get_or_insert_with(|| craft::automation::ControlChannel::new(std::sync::mpsc::channel().1));
    channel.settle(req, crate::control::Reply::AfterJob(job.0), None, 0.0);
    reply
}

/// The control reply for `job`, once the app's frame loop has applied it.
fn job_reply(h: &mut Harness<'static, Box<PhotocraftApp>>, job: JobId) -> Value {
    let rx = wait_for_job(h.state_mut(), job);
    let t = Instant::now();
    loop {
        h.step();
        if let Ok(v) = rx.try_recv() {
            return v;
        }
        assert!(t.elapsed() < Duration::from_secs(30));
    }
}

/// #511: a filter started by a menu item (Blur More has no dialog) or by a filter dialog's OK
/// replies with its result once applied, like `engine.execute`; `wait: false` replies at once.
#[test]
fn menu_and_dialog_requests_wait_for_jobs_unless_asked_not_to() {
    let mut h = app_harness();
    let ctx = h.ctx.clone();
    let control = |h: &mut Harness<'static, Box<PhotocraftApp>>, method: &str, params: Value| {
        let (req, _rx) = crate::ControlRequest::new(method, params);
        crate::control::handle(h.state_mut(), &ctx, &req)
    };
    // The job a request started: deferred while waiting, `{job, pending}` at once otherwise.
    // Lets it land and checks the filter's result.
    let land = |h: &mut Harness<'static, Box<PhotocraftApp>>, what: &str, outcome: crate::control::Reply, wait: bool| {
        let job = match (outcome, wait) {
            (crate::control::Reply::AfterJob(job), true) => JobId(job),
            (crate::control::Reply::Done(v), false) => {
                assert_eq!(v["result"]["pending"], true, "{what}: {v}");
                JobId(v["result"]["job"].as_u64().unwrap())
            }
            _ => panic!("{what}, wait {wait}: wrong reply kind"),
        };
        let reply = job_reply(h, job);
        assert!(reply["result"]["filter"].is_object(), "{what}: {reply}");
    };
    let steps = |h: &Harness<'static, Box<PhotocraftApp>>| h.state().session.active().unwrap().history.past_len();
    for wait in [true, false] {
        let before = steps(&h);
        let outcome = control(&mut h, "ui.menu.invoke", json!({"id": "filter.blur.blurMore", "wait": wait}));
        land(&mut h, "Blur More", outcome, wait);
        let crate::control::Reply::Done(v) = control(&mut h, "ui.menu.invoke", json!({"id": "filter.blur.gaussianBlur"})) else { panic!("dialog") };
        let dialog = v["result"]["dialog"].as_str().expect("Gaussian Blur opens its dialog (its id is the string ui.inspect lists)");
        control(&mut h, "ui.dialog.set", json!({"dialog": dialog, "field": "radius", "value": 3}));
        let outcome = control(&mut h, "ui.dialog.confirm", json!({"dialog": dialog, "wait": wait}));
        land(&mut h, "Gaussian Blur", outcome, wait);
        assert_eq!(steps(&h), before + 2, "both filters applied");
    }
}

fn psd_bytes() -> Vec<u8> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let doc = (*s.active().unwrap().doc).clone();
    photocraft_io::export(&doc, "x.psd", &Default::default()).unwrap().bytes
}

#[test]
fn files_open_in_the_background_with_a_tab_and_cancel_closes_it() {
    let mut h = app_harness();
    let bytes = psd_bytes();
    h.state_mut().open_file("/tmp/photocraft-test/bg.psd", &bytes).unwrap();
    // The tab is there before the document.
    assert_eq!(h.state().jobs.opens.len(), 1);
    assert_eq!(h.state().session.documents().len(), 1);
    let t = Instant::now();
    while !h.state().jobs.opens.is_empty() && t.elapsed() < Duration::from_secs(30) {
        h.step();
    }
    h.run_steps(2);
    let st = h.state().session.active().unwrap();
    assert_eq!(st.doc.size.width, 64);
    assert_eq!(st.path.as_deref(), Some("/tmp/photocraft-test/bg.psd"));
    assert_eq!(h.state().ui.recent_files.first().map(String::as_str), Some("/tmp/photocraft-test/bg.psd"));

    // Cancel: the half-open tab closes, no document is added.
    h.state_mut().open_file("/tmp/photocraft-test/bg2.psd", &bytes).unwrap();
    let job = h.state().jobs.opens[0].job;
    crate::jobs_ui::cancel(h.state_mut(), job);
    h.run_steps(3);
    assert!(h.state().jobs.opens.is_empty());
    assert_eq!(h.state().session.documents().len(), 2);
    assert!(h.state().jobs.focus.is_none());
}

#[test]
fn templates_open_untitled_in_the_background() {
    let mut h = app_harness();
    h.state_mut().open_file("/tmp/photocraft-test/card.psdt", &psd_bytes()).unwrap();
    assert_eq!(h.state().jobs.opens[0].name, "Untitled-1");
    let t = Instant::now();
    while !h.state().jobs.opens.is_empty() && t.elapsed() < Duration::from_secs(30) {
        h.step();
    }
    h.run_steps(2);
    let st = h.state().session.active().unwrap();
    assert_eq!((st.doc.name.as_str(), st.path.as_deref()), ("Untitled-1", None));
    assert_eq!(h.state().ui.recent_files.first().map(String::as_str), Some("/tmp/photocraft-test/card.psdt"));
}

/// Gates for a fake save: it encodes until `encode` opens and writes until `write` opens
/// (polling for cancellation only while encoding, as the real one does).
#[derive(Clone, Default)]
struct Gates {
    encode: Arc<AtomicBool>,
    write: Arc<AtomicBool>,
    written: Arc<std::sync::Mutex<Vec<String>>>,
}

impl Gates {
    fn open() -> Self {
        let g = Gates::default();
        g.encode.store(true, Ordering::Relaxed);
        g.write.store(true, Ordering::Relaxed);
        g
    }

    /// A `save_file` service held by these gates that records the path written, or fails with
    /// `fail` once encoded.
    fn save(&self, fail: Option<&'static str>) -> crate::SaveFileFn {
        let g = self.clone();
        Arc::new(move |_, path, _, ctl| {
            ctl.progress(0.0, "Encoding");
            while !g.encode.load(Ordering::Relaxed) {
                ctl.check()?;
                std::thread::sleep(Duration::from_millis(1));
            }
            if let Some(e) = fail {
                return Err(e.into());
            }
            ctl.commit()?;
            ctl.progress(0.0, "Writing");
            while !g.write.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1));
            }
            g.written.lock().unwrap().push(path.to_string());
            Ok(vec!["a warning".into()])
        })
    }

    fn written(&self) -> Vec<String> {
        self.written.lock().unwrap().clone()
    }
}

/// A notice (or toast card) carrying `text` is up after the harness's passes.
fn noticed(h: &Harness<'_, Box<PhotocraftApp>>, text: &str) -> bool {
    crate::notices::shown(h.state(), &h.ctx).iter().any(|(title, lines, _)| title.contains(text) || lines.iter().any(|l| l.contains(text)))
}

fn step_until(h: &mut Harness<'_, Box<PhotocraftApp>>, done: impl Fn(&PhotocraftApp) -> bool) {
    let t = Instant::now();
    while !done(h.state()) && t.elapsed() < Duration::from_secs(30) {
        h.step();
    }
    h.run_steps(2);
}

#[test]
fn saves_run_in_the_background_and_are_recorded_when_they_end() {
    let mut h = app_harness();
    let gates = Gates { write: Arc::new(AtomicBool::new(true)), ..Gates::default() };
    h.state_mut().services.save_file = Some(gates.save(None));
    let r = h.state_mut().save_as(Some("/tmp/photocraft-test/big.psd".into())).unwrap();
    assert_eq!(r["pending"], true);
    assert_eq!(r["job"].as_u64(), h.state().jobs.saves.first().map(|s| s.job.0), "control clients get the job to wait on");
    assert!(h.state().saving());

    // The window keeps drawing; the document stays unsaved and can't be edited meanwhile.
    let (frames, _) = frames_for_at_least(&mut h, 400, 4);
    assert!(frames >= 4, "only {frames} frames while saving");
    let st = h.state().session.active().unwrap();
    assert!(st.is_dirty() && st.path.is_none());
    assert!(h.state_mut().run("layer.new.layer", json!({})).is_err(), "the saving document is locked");
    // Save in Background is on by default: no blocking overlay, so the rest of the app stays usable
    // (#6207); the status bar's progress is named "Saving" and shows the percentage itself (#3389).
    assert!(h.query_by_label("Esc to cancel").is_none() && h.query_by_label("Encoding").is_none(), "no progress overlay");
    assert!(h.query_all_by_label("Saving").count() >= 1, "the status bar names the save");
    assert!(crate::jobs_ui::foreground_save(h.state()).is_none());

    gates.encode.store(true, Ordering::Relaxed);
    step_until(&mut h, |a| !a.saving());
    assert_eq!(gates.written(), ["/tmp/photocraft-test/big.psd"]);
    let st = h.state().session.active().unwrap();
    assert!(!st.is_dirty());
    assert_eq!(st.path.as_deref(), Some("/tmp/photocraft-test/big.psd"));
    assert_eq!(st.doc.name, "big.psd", "the document takes the saved file's name, as a synchronous Save As does");
    assert!(noticed(&h, "a warning"), "the export warnings are shown");
    assert!(h.state_mut().run("layer.new.layer", json!({})).is_ok(), "unlocked again");
}

/// Save in Background off (#6207): the same worker writes the file, but the app waits for it
/// like Photoshop's foreground save. The progress dialog shows at once, shortcuts don't reach the
/// app, and a close asked for meanwhile happens only once the file is complete.
#[test]
fn save_in_background_off_blocks_the_app_until_the_file_is_written() {
    let mut h = app_harness();
    h.state_mut().run("prefs.set", json!({"path": "fileHandling.saveInBackground", "value": false})).unwrap();
    let gates = Gates { write: Arc::new(AtomicBool::new(true)), ..Gates::default() };
    h.state_mut().services.save_file = Some(gates.save(None));
    h.state_mut().ui.tool = crate::state::Tool::Move;
    let r = h.state_mut().save_as(Some("/tmp/photocraft-test/fg.psd".into())).unwrap();
    assert_eq!(r["pending"], true, "the same background job does the work");
    h.run_steps(1);
    // The overlay's card is up from the first frame (no delay), titled with the save, then its stage. The overlay's
    // own node (named for the save) is in the accessibility tree a pass before the card's body, so the wait is for the
    // body's hint.
    let t = Instant::now();
    while (h.query_all_by_label("Saving fg.psd").count() == 0 || h.query_by_label("Esc to cancel").is_none()) && t.elapsed() < Duration::from_secs(30) {
        h.step();
    }
    h.get_by_label("Esc to cancel");
    while h.query_by_label("Encoding").is_none() && t.elapsed() < Duration::from_secs(30) {
        h.step();
    }
    h.get_by_label("Encoding");
    assert!(crate::jobs_ui::foreground_save(h.state()).is_some());
    h.key_press(egui::Key::B);
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, crate::state::Tool::Move, "no shortcut reaches the app while it saves");
    assert!(h.state_mut().run("layer.new.layer", json!({})).is_err(), "the saving document is locked");
    // Quitting now waits for the file.
    let st = h.state_mut().session.active_mut().unwrap();
    st.saved_revision = st.revision;
    assert!(press_window_close(h.state_mut()), "the close waits for the save");
    assert!(h.state().jobs.close_after_saves);

    gates.encode.store(true, Ordering::Relaxed);
    let t = Instant::now();
    while h.state().jobs.close_after_saves && t.elapsed() < Duration::from_secs(30) {
        h.step();
        if h.state().jobs.close_after_saves {
            assert!(h.query_by_label("Esc to cancel").is_some(), "the overlay stays up until the save ends");
        }
    }
    assert_eq!(gates.written(), ["/tmp/photocraft-test/fg.psd"], "the file is complete before the close goes on");
    assert!(!h.state().jobs.close_after_saves, "then the held close is repeated");
    h.run_steps(2);
    assert!(h.query_by_label("Esc to cancel").is_none(), "the overlay is gone");
    let st = h.state().session.active().unwrap();
    assert_eq!(st.path.as_deref(), Some("/tmp/photocraft-test/fg.psd"));
    assert!(!st.is_dirty());
    h.key_press(egui::Key::B);
    for _ in 0..2 {
        h.step();
    }
    assert_eq!(h.state().ui.tool, crate::state::Tool::Brush, "shortcuts work again");
}

/// A failing foreground save reports its error, closes its dialog and leaves the document
/// unsaved, as a background one does; Esc still cancels one that is encoding.
#[test]
fn a_failed_or_cancelled_foreground_save_leaves_the_document_unsaved() {
    let mut h = app_harness();
    h.state_mut().run("prefs.set", json!({"path": "fileHandling.saveInBackground", "value": false})).unwrap();
    let gates = Gates::open();
    h.state_mut().services.save_file = Some(gates.save(Some("disk full")));
    h.state_mut().save_as(Some("/tmp/photocraft-test/f.psd".into())).unwrap();
    step_until(&mut h, |a| !a.saving());
    let st = h.state().session.active().unwrap();
    assert!(st.is_dirty() && st.path.is_none());
    assert!(noticed(&h, "disk full"), "the failure is reported");
    assert!(h.query_by_label("Esc to cancel").is_none(), "the overlay is gone");

    gates.encode.store(false, Ordering::Relaxed);
    h.state_mut().services.save_file = Some(gates.save(None));
    h.state_mut().save_as(Some("/tmp/photocraft-test/g.psd".into())).unwrap();
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    step_until(&mut h, |a| !a.saving());
    assert!(gates.written().is_empty(), "nothing written");
    let st = h.state().session.active().unwrap();
    assert!(st.is_dirty() && st.path.is_none());
    assert_eq!(h.state().ui.status, "Canceled Saving g.psd");
    assert!(crate::jobs_ui::foreground_save(h.state()).is_none());
}

#[test]
fn a_failed_save_or_one_cancelled_while_encoding_leaves_the_document_unsaved() {
    let mut h = app_harness();
    let gates = Gates::open();
    h.state_mut().services.save_file = Some(gates.save(Some("disk full")));
    h.state_mut().save_as(Some("/tmp/photocraft-test/a.psd".into())).unwrap();
    step_until(&mut h, |a| !a.saving());
    let st = h.state().session.active().unwrap();
    assert!(st.is_dirty() && st.path.is_none());
    assert!(noticed(&h, "disk full"), "the failure is reported");

    gates.encode.store(false, Ordering::Relaxed);
    h.state_mut().services.save_file = Some(gates.save(None));
    h.state_mut().save_as(Some("/tmp/photocraft-test/b.psd".into())).unwrap();
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    step_until(&mut h, |a| !a.saving());
    assert!(gates.written().is_empty(), "nothing written");
    let st = h.state().session.active().unwrap();
    assert!(st.is_dirty() && st.path.is_none());
    assert_eq!(h.state().ui.status, "Canceled Saving b.psd");
}

/// Step until the save in `h` has started writing.
fn until_writing(h: &mut Harness<'_, Box<PhotocraftApp>>) {
    let t = Instant::now();
    while h.state().session.jobs().first().is_none_or(|j| j.message != "Writing") && t.elapsed() < Duration::from_secs(30) {
        h.step();
    }
}

#[test]
fn a_save_that_has_started_writing_cannot_be_cancelled() {
    let mut h = app_harness();
    let gates = Gates { encode: Arc::new(AtomicBool::new(true)), ..Gates::default() };
    h.state_mut().services.save_file = Some(gates.save(None));
    h.state_mut().save_as(Some("/tmp/photocraft-test/w.psd".into())).unwrap();
    until_writing(&mut h);
    let job = h.state().jobs.saves[0].job;
    crate::jobs_ui::cancel(h.state_mut(), job);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().ui.status, "Saving w.psd is writing the file and can no longer be canceled");
    assert!(h.state().session.job(job).is_some(), "still running, the document still locked");

    gates.write.store(true, Ordering::Relaxed);
    step_until(&mut h, |a| !a.saving());
    assert_eq!(gates.written(), ["/tmp/photocraft-test/w.psd"]);
    assert!(!h.state().session.active().unwrap().is_dirty(), "the written save is recorded");
}

/// A `save_file` service held by `gates` that really replaces the file: it encodes until
/// `encode` opens, commits, then writes `new` once `write` opens.
fn file_save(gates: &Gates) -> crate::SaveFileFn {
    let g = gates.clone();
    Arc::new(move |_, path, _, ctl| {
        ctl.progress(0.0, "Encoding");
        while !g.encode.load(Ordering::Relaxed) {
            ctl.check()?;
            std::thread::sleep(Duration::from_millis(1));
        }
        ctl.commit()?;
        ctl.progress(0.0, "Writing");
        while !g.write.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(1));
        }
        std::fs::write(path, b"new").map_err(|e| e.to_string())?;
        Ok(Vec::new())
    })
}

/// A file holding `old`, in a fresh directory named for the test.
fn old_file(test: &str) -> String {
    let dir = std::env::temp_dir().join(format!("photocraft-jobs-ui-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("w.psd");
    std::fs::write(&path, b"old").unwrap();
    path.to_string_lossy().into_owned()
}

fn control(h: &mut Harness<'static, Box<PhotocraftApp>>, method: &str, params: Value) -> Value {
    let (req, _rx) = crate::ControlRequest::new(method, params);
    let ctx = h.ctx.clone();
    let crate::control::Reply::Done(v) = crate::control::handle(h.state_mut(), &ctx, &req) else { panic!("{method} replies at once") };
    v
}

#[test]
fn jobs_cancel_before_a_save_writes_stops_it_and_leaves_the_target_untouched() {
    let mut h = app_harness();
    let path = old_file("cancel-before-write");
    let gates = Gates::default();
    h.state_mut().services.save_file = Some(file_save(&gates));
    h.state_mut().save_as(Some(path.clone())).unwrap();
    h.run_steps(2);
    let job = h.state().jobs.saves[0].job;
    let v = control(&mut h, "jobs.cancel", json!({"job": job.0}));
    assert_eq!(v["result"], json!({"cancelled": 1}), "{v}");
    // The worker reaches its commit point only now, and must give up there.
    gates.encode.store(true, Ordering::Relaxed);
    gates.write.store(true, Ordering::Relaxed);
    step_until(&mut h, |a| !a.saving());
    assert_eq!(std::fs::read(&path).unwrap(), b"old", "target untouched");
    let st = h.state().session.active().unwrap();
    assert!(st.is_dirty() && st.path.is_none(), "the document is not marked saved");
    assert_eq!(h.state().ui.status, "Canceled Saving w.psd");
}

#[test]
fn jobs_cancel_during_the_write_reports_the_save_as_not_cancelled_and_it_finishes() {
    let mut h = app_harness();
    let path = old_file("cancel-during-write");
    let gates = Gates { encode: Arc::new(AtomicBool::new(true)), ..Gates::default() };
    h.state_mut().services.save_file = Some(file_save(&gates));
    h.state_mut().save_as(Some(path.clone())).unwrap();
    let saved_doc = h.state().session.active().unwrap().doc.id;
    until_writing(&mut h);
    let save = h.state().jobs.saves[0].job;
    // A second document with a job that can still be cancelled.
    h.state_mut().session.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    let slow = slow_job(h.state_mut(), &Arc::new(AtomicBool::new(false)));

    let refused = json!([{"job": save.0, "reason": "committing"}]);
    let v = control(&mut h, "jobs.cancel", json!({"job": save.0}));
    assert_eq!(v["result"], json!({"cancelled": 0, "notCancelled": refused}), "{v}");
    assert_eq!(h.state().ui.status, "Saving w.psd is writing the file and can no longer be canceled");
    // Cancel everything: the other job stops, the save is reported as left running.
    let v = control(&mut h, "jobs.cancel", json!({}));
    assert_eq!(v["result"], json!({"cancelled": 1, "notCancelled": refused}), "{v}");
    assert!(h.state().session.job(slow).is_none());
    assert!(h.state().session.job(save).is_some(), "the save still runs and keeps its document locked");

    gates.write.store(true, Ordering::Relaxed);
    step_until(&mut h, |a| !a.saving());
    assert_eq!(std::fs::read(&path).unwrap(), b"new");
    let st = h.state().session.documents().iter().find(|st| st.doc.id == saved_doc).unwrap();
    assert!(!st.is_dirty() && st.path.as_deref() == Some(path.as_str()), "the save that happened is recorded");
}

/// One frame with the window's close button pressed; whether the close path cancelled the close.
fn press_window_close(app: &mut PhotocraftApp) -> bool {
    let mut info = egui::ViewportInfo::default();
    info.events.push(egui::ViewportEvent::Close);
    let mut input = egui::RawInput::default();
    input.viewports.insert(egui::ViewportId::ROOT, info);
    let mut out = crate::app::framework::close_path_frame(app, input);
    out.textures_delta.clear();
    out.viewport_output.remove(&egui::ViewportId::ROOT).is_some_and(|o| o.commands.iter().any(|c| matches!(c, egui::ViewportCommand::CancelClose)))
}

/// A clean document (so the close prompt has nothing to ask) saved as a copy in the background.
fn save_a_copy(h: &mut Harness<'static, Box<PhotocraftApp>>, gates: &Gates, path: &str) {
    h.state_mut().services.save_file = Some(gates.save(None));
    let st = h.state_mut().session.active_mut().unwrap();
    st.saved_revision = st.revision;
    assert_eq!(h.state_mut().write_document(path.into(), &crate::ExportSettings::default(), true), Ok(None));
    h.run_steps(2);
}

#[test]
fn quitting_waits_for_a_background_save_to_finish() {
    let mut h = app_harness();
    let gates = Gates { write: Arc::new(AtomicBool::new(true)), ..Gates::default() };
    save_a_copy(&mut h, &gates, "/tmp/photocraft-test/copy.psd");
    assert!(press_window_close(h.state_mut()), "the close waits for the save");
    assert!(h.state().jobs.close_after_saves);

    gates.encode.store(true, Ordering::Relaxed);
    step_until(&mut h, |a| !a.saving());
    assert_eq!(gates.written(), ["/tmp/photocraft-test/copy.psd"]);
    assert!(!h.state().jobs.close_after_saves, "the close is repeated once the save ends");
}

#[test]
fn a_failed_save_does_not_close_the_window_over_its_error() {
    let mut h = app_harness();
    let gates = Gates::default();
    h.state_mut().services.save_file = Some(gates.save(Some("disk full")));
    let st = h.state_mut().session.active_mut().unwrap();
    st.saved_revision = st.revision;
    h.state_mut().write_document("/tmp/photocraft-test/copy.psd".into(), &crate::ExportSettings::default(), true).unwrap();
    assert!(press_window_close(h.state_mut()));
    gates.encode.store(true, Ordering::Relaxed);
    step_until(&mut h, |a| !a.saving());
    assert!(noticed(&h, "disk full"));
    assert!(!h.state().jobs.close_after_saves, "the window stays open to show the error");
}

#[test]
fn a_cancelled_save_still_counts_until_its_worker_returns() {
    let mut h = app_harness();
    // A worker that keeps encoding after the cancel, as the real encoder does (it checks only
    // before writing).
    let (encode, release) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let (e, r) = (encode.clone(), release.clone());
    h.state_mut().services.save_file = Some(Arc::new(move |_, _, _, ctl| {
        while !r.load(Ordering::Relaxed) {
            e.store(true, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(1));
        }
        ctl.commit()?;
        Ok(Vec::new())
    }));
    let st = h.state_mut().session.active_mut().unwrap();
    st.saved_revision = st.revision;
    h.state_mut().write_document("/tmp/photocraft-test/c.psd".into(), &crate::ExportSettings::default(), true).unwrap();
    while !encode.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(1));
    }
    let job = h.state().jobs.saves[0].job;
    crate::jobs_ui::cancel(h.state_mut(), job);
    h.run_steps(3);
    assert!(h.state().session.job(job).is_none(), "the job ended");
    assert!(h.state().saving(), "but its worker is still running");
    assert!(press_window_close(h.state_mut()), "quitting waits for the worker");
    assert!(
        h.state_mut().save_as(Some("/tmp/photocraft-test/c.psd".into())).unwrap_err().contains("still running"),
        "a second save to the same file waits for it too"
    );

    release.store(true, Ordering::Relaxed);
    step_until(&mut h, |a| !a.saving());
    assert!(!h.state().jobs.close_after_saves, "the held close is repeated once the worker returns");
}

#[test]
fn cancelled_batch_with_partial_results_reports_cancellation_without_beeping() {
    static BEEPS: AtomicUsize = AtomicUsize::new(0);
    fn count_beep() {
        BEEPS.fetch_add(1, Ordering::Relaxed);
    }
    BEEPS.store(0, Ordering::Relaxed);
    let mut h = app_harness_with(crate::Services { beep: Some(count_beep), ..Default::default() });
    h.state_mut().run("prefs.set", json!({"path": "general.beepWhenDone", "value": true})).unwrap();
    let Started::Job(id) = h
        .state_mut()
        .session
        .start_job(
            "file.automate.batch",
            json!({}),
            "Batch",
            false,
            |ctx| {
                while !ctx.cancelled() {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(json!({"files": ["done.png"], "cancelled": true}))
            },
            |_, result| Ok(result),
        )
        .unwrap()
    else {
        panic!("batch did not start as a job")
    };
    assert!(h.state_mut().session.cancel_job(id));
    let started = Instant::now();
    let event = loop {
        if let Some(event) = h.state_mut().session.poll_jobs().into_iter().find(|event| event.id == id) {
            break event;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "batch did not finish");
        std::thread::sleep(Duration::from_millis(1));
    };
    let ctx = h.ctx.clone();
    let reply_rx = wait_for_job(h.state_mut(), id);
    crate::jobs_ui::on_event(h.state_mut(), &ctx, event);
    assert!(h.state().ui.status.contains("Canceled"), "{}", h.state().ui.status);
    assert_eq!(BEEPS.load(Ordering::Relaxed), 0);
    let reply = reply_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"], "cancelled");
    assert_eq!(reply["result"]["files"], json!(["done.png"]));
}

#[test]
fn interactive_batch_shows_its_read_only_input_and_asks_after_escape() {
    let mut h = app_harness();
    let original = h.state().session.active().unwrap().doc.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let Started::Job(id) = h
        .state_mut()
        .session
        .start_job(
            "file.automate.batch",
            json!({}),
            "Batch",
            false,
            move |ctx| {
                let mut scratch = photocraft_engine::Session::new();
                scratch.execute("file.new", json!({"name": "input.png", "width": 16, "height": 12, "background": "#00ff00"}))?;
                ctx.publish_batch_preview("/source/input.png", scratch.active().map(|state| state.doc.clone()));
                let _ = ready_tx.send(());
                while !ctx.batch_take_interrupt() {
                    ctx.check()?;
                    std::thread::sleep(Duration::from_millis(1));
                }
                let continued = ctx.batch_wait_for_decision();
                ctx.publish_batch_preview("/source/input.png", None);
                Ok(json!({"continued": continued, "files": []}))
            },
            |_, result| Ok(result),
        )
        .unwrap()
    else {
        panic!("batch did not start as a job")
    };
    ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    frames_until(&mut h, 1, |h| h.query_by_label("Batch preview").is_some());
    assert!(h.query_all_by_label("input.png").count() >= 1, "the current input's header and picture appear in the canvas");
    assert!(h.query_by_label("Batch").is_none(), "the Batch progress readout is hidden while its input is visible");
    assert!(Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc), "preview cannot replace the editable document");
    h.key_press(egui::Key::Escape);
    frames_until(&mut h, 1, |h| h.query_by_label("Continue").is_some());
    assert!(h.state().session.batch_interrupt_waiting(id));
    h.get_by_label("Continue").click();
    frames_until(&mut h, 1, |h| h.state().session.job(id).is_none());
    let result = h.state().session.jobs_with_recent().into_iter().find(|job| job.id == id).and_then(|job| job.result).unwrap();
    assert_eq!(result["continued"], true);
    assert!(h.query_by_label("Batch preview").is_none(), "the input closes after the decision");
    assert!(Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
}

#[test]
fn printing_keeps_drawing_and_reports_only_the_final_spooler_result() {
    for success in [true, false] {
        let mut h = app_harness();
        let gate = Arc::new(AtomicBool::new(false));
        let worker_gate = gate.clone();
        let Started::Job(id) = h
            .state_mut()
            .session
            .start_job(
                "file.print",
                json!({}),
                "Print",
                false,
                move |_| {
                    while !worker_gate.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    if success {
                        Ok(json!({"sent": true, "spooler": "request id is Office-42", "pdf": null}))
                    } else {
                        Err(photocraft_engine::EngineError::Other("queue refused".into()))
                    }
                },
                |_, result| Ok(result),
            )
            .unwrap()
        else {
            panic!("must run on a worker")
        };
        let frame = h.state().frame;
        h.run_steps(4);
        assert!(h.state().frame > frame);
        assert!(!h.state().ui.status.contains("Sent to the printer"));
        // The bar's cancel is refused for a submitted print, with its reason as the tooltip; a click does nothing.
        let cancel = h.get_by_label("Cancel (Esc)");
        assert!(cancel.accesskit_node().is_disabled(), "a submitted print cannot be cancelled from the bar");
        cancel.click();
        h.run_steps(2);
        assert!(h.state().session.job(id).is_some() && !h.state().ui.status.contains("Canceled"), "the refused cancel changes nothing");
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(h.state().session.job(id).is_some(), "Esc cannot retract a submitted job");
        let (request, _) = crate::control::ControlRequest::new("jobs.cancel", json!({"job": id.0}));
        let ctx = h.ctx.clone();
        let crate::control::Reply::Done(reply) = crate::control::handle(h.state_mut(), &ctx, &request) else { panic!("cancel replies immediately") };
        assert_eq!(reply["result"]["cancelled"], 0, "control must not falsely report cancellation: {reply}");
        let st = h.state_mut().session.active_mut().unwrap();
        st.saved_revision = st.revision;
        assert!(press_window_close(h.state_mut()), "quit waits for print submission");
        assert!(h.state().jobs.close_after_saves);
        gate.store(true, Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(10);
        while h.state().session.has_jobs() && Instant::now() < deadline {
            h.step();
        }
        assert!(!h.state().session.has_jobs());
        assert!(!h.state().jobs.close_after_saves, "close repeats on success or is cleared for a visible error");
        if success {
            assert!(h.state().ui.status.contains("Office-42"), "{}", h.state().ui.status);
            assert!(!h.state().ui.status_error);
        } else {
            assert!(h.state().ui.status.contains("queue refused"), "{}", h.state().ui.status);
            assert!(h.state().ui.status_error);
        }
    }
    // The job and its status do not ask the system spooler for its printers.
    assert_eq!(photocraft_engine::print_cmds::system_printer_listings(), 0, "a test asked the system spooler for its printers");
}

fn layer_count(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

/// Free Transform the active layer to the right half of the canvas, then ↩.
fn transform_and_commit(h: &mut Harness<'static, Box<PhotocraftApp>>, copy: bool) {
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    if copy { crate::transform_tool::begin_copy(app, &ctx) } else { crate::transform_tool::begin(app, &ctx) }.unwrap();
    move_box(app, [[160.0, 0.0], [320.0, 0.0], [320.0, 120.0], [160.0, 120.0]]);
    crate::transform_tool::commit(app);
}

/// Sets the open box's corners as one edit of the session.
fn move_box(app: &mut PhotocraftApp, corners: [[f64; 2]; 4]) {
    let session = &mut app.ui.transform.as_mut().unwrap().session;
    let before = *session.state();
    let mut after = before;
    after.quad = corners.map(|[x, y]| Point::new(x, y));
    session.apply(TransformEdit::Box(EditEvent::set(Mixed::Same(before), after, "Free Transform".into(), None)), 0.0);
}

#[test]
fn a_free_transform_commit_runs_in_the_background_and_keeps_its_preview() {
    let mut h = app_harness();
    let (layers, steps) = (layer_count(h.state()), h.state().session.active().unwrap().history.past_len());
    transform_and_commit(&mut h, true);
    let app = h.state();
    // The warp runs as a job; the moving pixels stay on screen until it lands, and the box's
    // history is closed meanwhile.
    let job = crate::transform_tool::committing(app).expect("the commit runs as a job");
    assert!(app.session.job(job).is_some_and(|j| j.label == "Free Transform"));
    assert!(app.ui.transform.is_some() && app.transform_preview.is_some());
    assert_eq!(crate::transform_tool::is_enabled(app, "edit.undo"), Some(false));
    assert!(h.state_mut().run("edit.undo", json!({})).is_err());
    crate::transform_tool::commit(h.state_mut());
    let t = Instant::now();
    while h.state().session.has_jobs() && t.elapsed() < Duration::from_secs(10) {
        h.step();
    }
    h.run_steps(2);
    let app = h.state();
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "the session ends with the job");
    // The copy and its transform are one step, as when the commit ran inline.
    let st = app.session.active().unwrap();
    assert_eq!((layer_count(app), st.history.past_len()), (layers + 1, steps + 1));
    assert_eq!(st.history.undo_label(), Some("Free Transform"));
    let moved = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
    assert_eq!((moved.x0, moved.y0, moved.x1, moved.y1), (160, 0, 320, 120));
}

#[test]
fn esc_cancels_a_background_free_transform_commit() {
    let mut h = app_harness();
    let (layers, steps) = (layer_count(h.state()), h.state().session.active().unwrap().history.past_len());
    let before = h.state().session.active().unwrap().doc.clone();
    transform_and_commit(&mut h, true);
    let job = crate::transform_tool::committing(h.state()).expect("the commit runs as a job");
    // Esc reaches the transform tool first (shortcuts), which cancels the job.
    crate::transform_tool::cancel(h.state_mut());
    h.run_steps(3);
    let app = h.state();
    assert!(app.session.job(job).is_none());
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
    // The copy is taken back with the transform, leaving nothing to redo.
    let st = app.session.active().unwrap();
    assert_eq!((layer_count(app), st.history.past_len()), (layers, steps));
    assert!(!st.history.can_redo());
    assert_eq!(st.doc.layers.len(), before.layers.len());
}

/// Warp the active layer into an arc, then ↩.
fn warp_and_commit(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    crate::transform_tool::begin(app, &ctx).unwrap();
    let session = &mut app.ui.transform.as_mut().unwrap().session;
    let rect = session.rect();
    let warp = photocraft_geom::warp::Warp::preset(photocraft_geom::warp::WarpStyle::Arc, 40.0, [rect.x0, rect.y0, rect.x1, rect.y1]);
    session.apply(TransformEdit::Warp(crate::state::SessionWarp::new(warp, false)), 0.0);
    crate::transform_tool::commit(app);
}

#[test]
fn a_warp_commit_runs_in_the_background_and_keeps_its_preview() {
    let mut h = app_harness();
    let steps = h.state().session.active().unwrap().history.past_len();
    warp_and_commit(&mut h);
    let app = h.state();
    let job = crate::transform_tool::committing(app).expect("the commit runs as a job");
    assert!(app.session.job(job).is_some_and(|j| j.label == "Warp"));
    assert!(app.ui.transform.is_some() && app.transform_preview.is_some());
    assert_eq!(h.state_mut().run("edit.undo", json!({})).err().as_deref(), Some("Warp is still being applied"));
    let t = Instant::now();
    while h.state().session.has_jobs() && t.elapsed() < Duration::from_secs(10) {
        h.step();
    }
    h.run_steps(2);
    let app = h.state();
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "the session ends with the job");
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), steps + 1);
    assert_eq!(st.history.undo_label(), Some("Warp"));
    // The arc bulges the layer past its old edges.
    let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
    assert_ne!(b, photocraft_geom::Rect::new(0, 0, 320, 240), "the layer was warped");
}

#[test]
fn esc_cancels_a_background_warp_commit() {
    let mut h = app_harness();
    let steps = h.state().session.active().unwrap().history.past_len();
    let before = h.state().session.active().unwrap().doc.clone();
    warp_and_commit(&mut h);
    let job = crate::transform_tool::committing(h.state()).expect("the commit runs as a job");
    crate::transform_tool::cancel(h.state_mut());
    h.run_steps(3);
    let app = h.state();
    assert!(app.session.job(job).is_none());
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), steps);
    let id = st.active_layer.unwrap();
    let px = |d: &photocraft_doc::Document| d.layer(id).unwrap().surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 320, 240));
    assert_eq!(px(&st.doc), px(&before), "the layer is untouched");
}

/// #4559: a background open (the desktop app's) also warns, when its document arrives, about
/// type in a font that isn't installed.
#[test]
fn background_open_warns_about_missing_fonts() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 40, "height": 20})).unwrap();
    s.execute("type.create", json!({"text": "Hi", "x": 2, "y": 12, "size": 10, "font": "NoSuchFamily4559"})).unwrap();
    let doc = (*s.active().unwrap().doc).clone();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    // What the open job leaves behind: the decoded document in the session, then its result.
    let (index, _) = app.session.open_document(doc, None);
    let id = app.session.documents()[index].doc.id;
    super::finish_open(&mut app, "poster.psd", None, &json!({"document": index, "docId": id.0, "warnings": []}), None, None).unwrap();
    let want = vec!["font 'NoSuchFamily4559' is not installed; text may render using a fallback face".to_string()];
    assert!(app.ui.notices.last().is_some_and(|n| n.title.contains("poster.psd") && n.lines == want), "{:?}", app.ui.notices);
}

fn identity_app() -> Box<PhotocraftApp> {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    session.execute("layer.new.layer", json!({})).unwrap();
    session.execute("edit.fill", json!({"color": "#336699"})).unwrap();
    session.active_mut().unwrap().path = Some("accepted-control.pcraft".into());
    assert!(session.active().unwrap().is_dirty());
    PhotocraftApp::new(session, crate::Services::default())
}

fn identity_png(profile: Option<craft::cms::Builtin>) -> Vec<u8> {
    let app = identity_app();
    let mut doc = (*app.session.active().unwrap().doc).clone();
    doc.icc_profile = profile.map(|profile| profile.profile().to_bytes());
    photocraft_io::export(&doc, "synthetic.png", &Default::default()).unwrap().bytes
}

/// Complete the real decoder/apply path without delivering its result to the UI yet. This is
/// the same boundary as a poll that applies all ready jobs before handling their UI events.
fn completed_identity_open(app: &mut PhotocraftApp, name: &str, bytes: Vec<u8>) -> Value {
    match app.session.start_open(name, photocraft_engine::jobs::OpenSource::Bytes(Arc::new(bytes))).unwrap() {
        Started::Done(result) => result,
        Started::Job(job) => app.session.wait_job(job).unwrap(),
    }
}

struct IdentityControl {
    doc: Arc<photocraft_doc::Document>,
    path: Option<String>,
    revision: u64,
    saved_revision: u64,
    history: usize,
    redo: bool,
}

impl IdentityControl {
    fn capture(app: &PhotocraftApp) -> Self {
        let state = app.session.active().unwrap();
        Self {
            doc: state.doc.clone(),
            path: state.path.clone(),
            revision: state.revision,
            saved_revision: state.saved_revision,
            history: state.history.past_len(),
            redo: state.history.can_redo(),
        }
    }

    fn assert_untouched(&self, app: &PhotocraftApp) {
        let state = app.session.documents().iter().find(|state| state.doc.id == self.doc.id).expect("the accepted control remains open");
        assert!(Arc::ptr_eq(&state.doc, &self.doc), "the accepted control's pixels and document data must not change");
        assert_eq!(state.path, self.path, "an import must not rebind another document's save path");
        assert_eq!(
            (state.revision, state.saved_revision, state.history.past_len(), state.history.can_redo()),
            (self.revision, self.saved_revision, self.history, self.redo)
        );
        assert!(state.is_dirty(), "the unrelated document must still need saving");
    }
}

/// Exercise the consumer separately from the engine result-schema regression: the identity is
/// observed from the real imported document, then supplied even on the old producer.
fn identified_result(app: &PhotocraftApp, mut result: Value) -> (Value, photocraft_doc::DocId) {
    let id = app.session.documents()[result["document"].as_u64().unwrap() as usize].doc.id;
    result["docId"] = json!(id.0);
    (result, id)
}

#[test]
fn open_identity_survives_tab_reordering() {
    let mut app = identity_app();
    let control = IdentityControl::capture(&app);
    let result = completed_identity_open(&mut app, "imported.png", identity_png(None));
    let (result, imported) = identified_result(&app, result);
    let old_index = result["document"].as_u64().unwrap() as usize;
    app.session.move_document(old_index, 0).unwrap();
    assert_eq!(app.session.documents()[old_index].doc.id, control.doc.id);
    super::finish_open(&mut app, "imported.png", Some("imported.png"), &result, None, None).unwrap();
    control.assert_untouched(&app);
    assert_eq!(app.session.active().unwrap().doc.id, imported);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("imported.png"));
}

#[test]
fn open_identity_survives_an_earlier_tab_closing() {
    let mut app = identity_app();
    let control = IdentityControl::capture(&app);
    app.session.execute("file.new", json!({"width": 2, "height": 2})).unwrap();
    let result = completed_identity_open(&mut app, "imported.png", identity_png(None));
    let (result, imported) = identified_result(&app, result);
    app.session.close(1).unwrap();
    app.session.set_active(0);
    super::finish_open(&mut app, "imported.png", Some("imported.png"), &result, Some(0), None).unwrap();
    control.assert_untouched(&app);
    assert_eq!(app.session.active().unwrap().doc.id, imported);
    assert_eq!(app.session.documents()[0].doc.id, imported, "the requested destination slot applies to the correct document");
}

#[test]
fn open_identity_rejects_closed_missing_and_invalid_identities() {
    for identity in [None, Some(Value::Null), Some(json!("not-an-id")), Some(json!(-1)), Some(json!(u64::MAX)), Some(json!(1.5))] {
        let mut app = identity_app();
        let control = IdentityControl::capture(&app);
        let result = completed_identity_open(&mut app, "closed.png", identity_png(None));
        let (mut result, imported) = identified_result(&app, result);
        app.session.close(result["document"].as_u64().unwrap() as usize).unwrap();
        // A reused/stale index must never become permission to finish another document.
        result["document"] = json!(0);
        match identity {
            None => {
                result.as_object_mut().unwrap().remove("docId");
            }
            Some(value) => result["docId"] = value,
        }
        assert!(super::finish_open(&mut app, "closed.png", Some("closed.png"), &result, Some(0), None).is_err(), "invalid identity was accepted: {result}");
        control.assert_untouched(&app);
        assert_eq!(app.session.active().unwrap().doc.id, control.doc.id);
        assert_eq!(app.session.documents().len(), 1);
        result["docId"] = json!(imported.0);
        assert!(
            super::finish_open(&mut app, "closed.png", Some("closed.png"), &result, None, None).is_err(),
            "a closed identity must not fall back to the old tab index"
        );
        control.assert_untouched(&app);
    }
}

#[test]
fn open_identity_keeps_same_poll_results_in_the_profile_queue() {
    let mut app = identity_app();
    let control = IdentityControl::capture(&app);
    app.session.execute("edit.colorSettings", json!({"askOnMismatch": true, "askOnMissing": false, "policyRgb": "preserve"})).unwrap();
    let mismatched = identity_png(craft::cms::Builtin::from_id("adobe-rgb-compat"));
    let prompt = completed_identity_open(&mut app, "profile.png", mismatched);
    assert!(prompt["held"].as_u64().is_some());
    let first = completed_identity_open(&mut app, "first.png", identity_png(None));
    let (first, first_id) = identified_result(&app, first);
    let second = completed_identity_open(&mut app, "second.png", identity_png(None));
    let (second, second_id) = identified_result(&app, second);
    // All engine applies have finished. Handling the first ordinary result removes its tab,
    // shifting the second result before its UI callback runs.
    super::finish_open(&mut app, "profile.png", Some("profile.png"), &prompt, None, None).unwrap();
    super::finish_open(&mut app, "first.png", Some("first.png"), &first, None, None).unwrap();
    super::finish_open(&mut app, "second.png", Some("second.png"), &second, None, None).unwrap();
    assert_eq!(app.held_opens.iter().map(|held| held.name.as_str()).collect::<Vec<_>>(), ["profile.png", "first.png", "second.png"]);
    assert_eq!(app.held_opens.iter().skip(1).map(|held| held.doc.id).collect::<Vec<_>>(), [first_id, second_id]);
    assert_eq!(app.session.documents().len(), 1, "both completed imports wait behind the profile prompt");
    control.assert_untouched(&app);
    assert_eq!(crate::open_prompt::answer(&mut app, None).unwrap()["cancelled"], 3);
    assert!(app.held_opens.is_empty());
    control.assert_untouched(&app);
}

#[test]
fn asynchronous_place_completion_displays_result_warnings() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
    let doc = app.session.active().unwrap().doc.id;
    let warning = "SVG: 1 embedded image was omitted";
    let place = super::PlaceJob { job: JobId(1), name: "omitted.svg".into(), doc };
    // A frame placement bypasses transform startup but must still display its warnings.
    super::finish_place(&mut app, &egui::Context::default(), place, photocraft_engine::jobs::JobOutcome::Done(json!({"frame":1,"warnings":[warning]})));
    assert!(app.ui.notices.iter().any(|notice| notice.lines.iter().any(|line| line == warning)));
}

/// Both Open entry points own the opening card immediately, then focus the admitted
/// document rather than the tab that was active while the worker was running.
#[test]
fn background_ordinary_and_smart_opens_reset_home_error_and_focus_their_result() {
    for smart in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("focused.psd");
        let bytes = psd_bytes();
        std::fs::write(&path, &bytes).unwrap();
        let path = path.to_str().unwrap();
        let mut session = photocraft_engine::Session::new();
        session.color_mut().settings.ask_on_missing = false;
        session.color_mut().settings.ask_on_mismatch = false;
        session.execute("file.new", json!({"width": 2, "height": 1, "name": "Previous tab"})).unwrap();
        let previous = session.active().unwrap().doc.id;
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        app.background_jobs = true;
        app.ui.chrome.home = Some(0);
        app.ui.status = "An earlier operation failed".into();
        app.ui.status_error = true;
        if smart {
            super::start_open_as_smart_object(&mut app, path, None).unwrap();
        } else {
            app.open_path(path).unwrap();
        }
        assert_eq!(app.jobs.opens.len(), 1, "smart={smart}: tracked real worker");
        let job = app.jobs.opens[0].job;
        assert_eq!(app.jobs.focus, Some(job));
        assert_eq!(app.ui.chrome.home, None);
        assert!(!app.ui.status_error);
        assert_eq!(app.ui.status, crate::i18n::fmt(tl!("Opening {name}…"), &[("name", "focused.psd")]));
        assert_eq!(app.session.active().unwrap().doc.id, previous, "not installed before polling");
        let started = Instant::now();
        while !app.jobs.opens.is_empty() {
            assert!(started.elapsed() < Duration::from_secs(30), "smart={smart}: worker timed out");
            let ctx = app.ctx().clone();
            crate::jobs_ui::tick(&mut app, &ctx);
            std::thread::yield_now();
        }
        assert_eq!(app.session.documents().len(), 2);
        let opened = app.session.active().unwrap();
        assert_ne!(opened.doc.id, previous);
        assert_eq!((opened.doc.size.width, opened.doc.size.height), (64, 48));
        assert_eq!(opened.path.as_deref(), (!smart).then_some(path));
        assert_eq!(app.jobs.focus, None);
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

/// Actual Place work is completed before its UI event, matching the engine's all-ready
/// publication boundary. The fixture records which tab was focused before that poll.
struct ReadyPlace {
    app: Box<PhotocraftApp>,
    target: Arc<photocraft_doc::Document>,
    unrelated: Arc<photocraft_doc::Document>,
    events: Vec<photocraft_engine::jobs::JobEvent>,
    focused: photocraft_doc::DocId,
}

fn ready_place(different_tab: bool) -> ReadyPlace {
    ready_place_with_services(different_tab, crate::Services::default())
}

fn ready_place_with_services(different_tab: bool, services: crate::Services) -> ReadyPlace {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width":16,"height":16,"name":"Place target"})).unwrap();
    let target = session.active().unwrap().doc.clone();
    session.execute("file.new", json!({"width":16,"height":16,"name":"Unrelated tab"})).unwrap();
    let unrelated = session.active().unwrap().doc.clone();
    session.set_active(0);
    let mut app = PhotocraftApp::new(session, services);
    app.background_jobs = true;
    app.sync_views();
    let bytes = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#ff0011"/></svg>"##;
    let result = app.place_bytes("placed.svg", bytes.to_vec(), None).unwrap();
    assert!(super::track_place(&mut app, "placed.svg", &result));
    if different_tab {
        app.session.set_active(1);
    }
    let focused = app.session.active().unwrap().doc.id;
    let job = app.jobs.place.as_ref().unwrap().job;
    app.session.wait_job(job).unwrap();
    let events = app.session.poll_jobs();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, job);
    assert_eq!(app.session.active().unwrap().doc.id, focused, "Place's engine apply preserves the pre-poll focus");
    ReadyPlace { app, target, unrelated, events, focused }
}

fn deferred_native_open(ready: &mut ReadyPlace, open_first: bool) -> photocraft_doc::DocId {
    use photocraft_io::{NativeImportTarget, NativeInitialView};
    let mut source = photocraft_engine::Session::new();
    source.execute("file.new", json!({"width":8,"height":6})).unwrap();
    source.execute("layer.new.layer", json!({"name":"Imported chosen"})).unwrap();
    let chosen = source.active().unwrap().active_layer().unwrap();
    source.execute("layer.layerMask.revealAll", json!({})).unwrap();
    source.execute("layer.new.layer", json!({"name":"Imported other"})).unwrap();
    source.execute("channel.new", json!({})).unwrap();
    let mut doc = (*source.active().unwrap().doc).clone();
    doc.name = "Deferred native.xcf".into();
    for name in ["Saved first", "Saved chosen"] {
        doc.paths.push(photocraft_doc::vector::NamedPath { name: name.into(), ..Default::default() });
    }
    let view = NativeInitialView {
        selected_layers: vec![chosen],
        active_layer: Some(chosen),
        target: NativeImportTarget::LayerMask,
        show_layer_mask: true,
        visible_alpha_channels: vec![0],
        samplers: vec![[2., 3.]],
        selected_path: Some(1),
        selected_path_identity: Some(photocraft_io::initial_view::path_identity(&doc.paths[1])),
    };
    let (index, warning) = ready.app.session.admit_import_with_initial_view(doc, Some("/imports/deferred.xcf".into()), true, Some(view));
    assert!(warning.is_none());
    let id = ready.app.session.documents()[index].doc.id;
    let job = JobId(10_000);
    ready.app.jobs.opens.push(super::OpenTab { job, name: "Deferred native.xcf".into(), path: Some("/imports/deferred.xcf".into()), slot: None, order: None });
    ready.events.push(photocraft_engine::jobs::JobEvent {
        id: job,
        command: super::OPEN_JOB.into(),
        label: "Opening Deferred native.xcf".into(),
        document: None,
        outcome: JobOutcome::Done(json!({"document":index,"docId":id.0,"warnings":[]})),
        error: None,
    });
    if open_first {
        ready.events.reverse();
    }
    id
}

fn present_place_and_open(ready: &mut ReadyPlace) {
    let ctx = ready.app.ctx().clone();
    super::finish_ready_events(&mut ready.app, &ctx, std::mem::take(&mut ready.events), Some(ready.focused));
}

fn assert_deferred_native_view(app: &PhotocraftApp, id: photocraft_doc::DocId) {
    use photocraft_engine::channel_cmds::ChannelTarget;
    let state = app.session.active().unwrap();
    assert_eq!(state.doc.id, id);
    assert_eq!(state.doc.name, "Deferred native.xcf");
    let chosen = state.doc.layers.iter().find(|layer| layer.name == "Imported chosen").unwrap().id;
    assert_eq!(state.active_layer(), Some(chosen));
    assert_eq!(state.selection(), &[chosen]);
    assert_eq!(state.channel_view.target, ChannelTarget::LayerMask);
    assert!(state.channel_view.layer_mask.is_some());
    assert!(state.channel_view.alpha_shown(0));
    assert_eq!(state.color_samplers, vec![[2., 3.]]);
    assert_eq!(state.initial_path_selection, Some(1));
    assert_eq!(state.initial_path_identity, Some(photocraft_io::initial_view::path_identity(&state.doc.paths[1])));
    assert_eq!(state.protected_import_source.as_deref(), Some("/imports/deferred.xcf"));
    assert!(state.source_read_only && state.path.is_none() && !state.is_dirty());
    assert_eq!(state.history.past_len(), 0);
}

#[test]
fn same_poll_native_open_waits_for_place_transform_in_both_completion_orders() {
    for open_first in [false, true] {
        for commit in [false, true] {
            let mut ready = ready_place(false);
            let imported = deferred_native_open(&mut ready, open_first);
            present_place_and_open(&mut ready);
            assert_eq!(ready.app.session.active().unwrap().doc.id, ready.target.id);
            assert_eq!(ready.app.held_opens.len(), 1);
            assert!(ready.app.ui.transform.as_ref().is_some_and(|transform| transform.made == Some(crate::state::MadeLayer::Place)));
            if commit {
                crate::transform_tool::commit(&mut ready.app);
            } else {
                crate::transform_tool::cancel(&mut ready.app);
            }
            let started = Instant::now();
            while ready.app.session.has_jobs() {
                assert!(started.elapsed() < Duration::from_secs(30), "Place transform timed out");
                let ctx = ready.app.ctx().clone();
                super::tick(&mut ready.app, &ctx);
                std::thread::yield_now();
            }
            let ctx = ready.app.ctx().clone();
            crate::open_prompt::take(&mut ready.app, &ctx);
            assert_deferred_native_view(&ready.app, imported);
            assert!(ready.app.held_opens.is_empty());
            let target = ready.app.session.documents().iter().find(|state| state.doc.id == ready.target.id).unwrap();
            assert_eq!(target.doc.layers.len(), ready.target.layers.len() + usize::from(commit));
            assert_eq!(target.history.past_len(), usize::from(commit));
        }
    }
}

#[test]
fn same_poll_native_open_never_starts_place_transform_over_an_unrelated_tab() {
    for open_first in [false, true] {
        let mut ready = ready_place(true);
        let imported = deferred_native_open(&mut ready, open_first);
        present_place_and_open(&mut ready);
        assert!(ready.app.ui.transform.is_none());
        assert_deferred_native_view(&ready.app, imported);
        let unrelated = &ready.unrelated;
        let unchanged = ready.app.session.documents().iter().find(|state| state.doc.id == unrelated.id).unwrap();
        assert!(Arc::ptr_eq(&unchanged.doc, unrelated));
        assert_eq!(unchanged.history.past_len(), 0);
        let target = ready.app.session.documents().iter().find(|state| state.doc.id == ready.target.id).unwrap();
        assert_eq!(target.doc.layers.len(), ready.target.layers.len() + 1);
    }
}

#[test]
fn background_place_without_open_keeps_an_unrelated_tab_active() {
    let mut ready = ready_place(true);
    present_place_and_open(&mut ready);
    assert_eq!(ready.app.session.active().unwrap().doc.id, ready.focused);
    assert!(ready.app.ui.transform.is_none());
    let unrelated = &ready.unrelated;
    assert!(Arc::ptr_eq(&ready.app.session.active().unwrap().doc, unrelated));
}

type SmartPickerSources = [(std::path::PathBuf, Vec<u8>); 2];

fn smart_picker_sources() -> (tempfile::TempDir, SmartPickerSources) {
    let directory = tempfile::tempdir().unwrap();
    let sources = [("image2.png", "#ff0000"), ("image10.png", "#00ff00")].map(|(name, color)| {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width":2,"height":1,"background":color})).unwrap();
        let bytes = photocraft_io::export(&session.active().unwrap().doc, name, &Default::default()).unwrap().bytes;
        let path = directory.path().join(name);
        std::fs::write(&path, &bytes).unwrap();
        (path, bytes)
    });
    (directory, sources)
}

/// Install the scripted native picker in the host before any job or dialog starts.
fn ready_smart_picker(different_tab: bool) -> (ReadyPlace, Arc<craft::host::ScriptedFiles>) {
    let files = Arc::new(craft::host::ScriptedFiles::held());
    // Framework captures the Files provider at construction; replacing only Services
    // afterwards does not replace the host's picker or its answer queue.
    let ready = ready_place_with_services(different_tab, crate::Services { files: Some(files.clone()), ..Default::default() });
    (ready, files)
}

/// Drive the native File menu and Files service, rather than bypassing the picker with
/// the starter. Its returned order is intentionally different from natural name order.
fn pick_smart_opens(app: &mut PhotocraftApp, files: &craft::host::ScriptedFiles, paths: Vec<std::path::PathBuf>) -> Vec<JobId> {
    app.session.color_mut().settings.ask_on_missing = false;
    app.session.color_mut().settings.ask_on_mismatch = false;
    let ctx = app.ctx().clone();
    crate::menus::invoke(app, &ctx, "file.openAsSmartObject", json!({})).unwrap();
    assert_eq!(files.open(), 1);
    assert_eq!(files.requests().last().unwrap().kind, craft::host::PickKind::OpenMany);
    files.answer(craft::host::Picked::Paths(paths)).unwrap();
    app.poll_file_dialog();
    assert_eq!(files.open(), 0);
    app.jobs.opens.iter().map(|open| open.job).collect()
}

fn assert_picked_smart_sources(app: &PhotocraftApp, sources: &SmartPickerSources) {
    let opened: Vec<_> = app.session.documents().iter().skip(2).collect();
    assert_eq!(opened.iter().map(|state| state.doc.name.as_str()).collect::<Vec<_>>(), ["image2-1", "image10-1"]);
    for ((state, (path, bytes)), expected) in opened.iter().zip(sources).zip([[1., 0., 0., 1.], [0., 1., 0., 1.]]) {
        assert_eq!(state.doc.size, photocraft_geom::Size::new(2, 1));
        assert!(state.path.is_none() && !state.is_dirty(), "Smart Object Open is untitled");
        assert_eq!(state.doc.layers.len(), 1);
        let photocraft_doc::LayerContent::Smart(object) = &state.doc.layers[0].content else { panic!("embedded Smart Object") };
        let photocraft_doc::SmartSource::Embedded { file_name, bytes: embedded } = &object.source else { panic!("embedded source") };
        assert_eq!(file_name, path.file_name().unwrap().to_str().unwrap());
        assert_eq!(embedded.as_slice(), bytes.as_slice());
        assert_eq!(std::fs::read(path).unwrap(), *bytes);
        let pixel = object.cache.as_ref().unwrap().pixel(0, 0);
        for (actual, expected) in pixel.iter().copied().zip(expected) {
            assert!((actual - expected).abs() < 1e-6, "{pixel:?}");
        }
        assert_eq!(state.history.entries(), ["Open As Smart Object"]);
    }
}

fn complete_smart_picker_before_shell(ready: &mut ReadyPlace, files: &craft::host::ScriptedFiles, sources: &SmartPickerSources, open_first: bool) {
    let jobs = pick_smart_opens(&mut ready.app, files, vec![sources[1].0.clone(), sources[0].0.clone()]);
    assert_eq!(jobs.len(), 2);
    assert_eq!(ready.app.jobs.opens.iter().map(|open| open.name.as_str()).collect::<Vec<_>>(), ["image2.png", "image10.png"]);
    // Reverse actual Smart Object completion to prove tab order comes from the
    // picker's captured OpenOrder rather than worker completion order.
    for job in jobs.iter().rev() {
        ready.app.session.wait_job(*job).unwrap();
    }
    ready.events.extend(ready.app.session.poll_jobs());
    assert_eq!(ready.events.len(), 3);
    if open_first {
        ready.events.rotate_left(1);
    }
}

#[test]
fn native_smart_picker_completions_wait_for_place_and_keep_source_order_in_either_event_order() {
    let (_directory, sources) = smart_picker_sources();
    for open_first in [false, true] {
        for commit in [false, true] {
            let (mut ready, files) = ready_smart_picker(false);
            complete_smart_picker_before_shell(&mut ready, &files, &sources, open_first);
            present_place_and_open(&mut ready);
            let ctx = ready.app.ctx().clone();
            super::tick(&mut ready.app, &ctx);
            assert!(ready.app.jobs.focus.is_none(), "ended Smart Open cards release focus");
            assert_eq!(ready.app.session.active().unwrap().doc.id, ready.target.id);
            assert_eq!(ready.app.held_opens.len(), 2);
            assert!(ready.app.ui.transform.as_ref().is_some_and(|transform| transform.made == Some(crate::state::MadeLayer::Place)));
            if commit {
                crate::transform_tool::commit(&mut ready.app);
            } else {
                crate::transform_tool::cancel(&mut ready.app);
            }
            let started = Instant::now();
            while ready.app.session.has_jobs() {
                assert!(started.elapsed() < Duration::from_secs(30), "Place transform timed out");
                let ctx = ready.app.ctx().clone();
                super::tick(&mut ready.app, &ctx);
                std::thread::yield_now();
            }
            let ctx = ready.app.ctx().clone();
            crate::open_prompt::take(&mut ready.app, &ctx);
            assert_picked_smart_sources(&ready.app, &sources);
            assert!(ready.app.held_opens.is_empty());
            assert!(ready.app.ui.recent_files.is_empty(), "Open as Smart Object does not bind the source as an opened file");
            let target = ready.app.session.documents().iter().find(|state| state.doc.id == ready.target.id).unwrap();
            assert_eq!(target.doc.layers.len(), ready.target.layers.len() + usize::from(commit));
            let unrelated = ready.app.session.documents().iter().find(|state| state.doc.id == ready.unrelated.id).unwrap();
            assert!(Arc::ptr_eq(&unrelated.doc, &ready.unrelated));
        }
    }
}

#[test]
fn cancelling_a_native_picked_smart_open_preserves_place_focus_and_never_publishes_it() {
    let (_directory, sources) = smart_picker_sources();
    let (mut ready, files) = ready_smart_picker(false);
    let jobs = pick_smart_opens(&mut ready.app, &files, vec![sources[0].0.clone()]);
    assert_eq!(jobs.len(), 1);
    assert!(super::cancel(&mut ready.app, jobs[0]));
    ready.app.session.join_cancelled_jobs();
    assert!(matches!(ready.app.session.wait_job(jobs[0]), Err(photocraft_engine::EngineError::Cancelled)));
    ready.events.extend(ready.app.session.poll_jobs());
    present_place_and_open(&mut ready);
    let ctx = ready.app.ctx().clone();
    super::tick(&mut ready.app, &ctx);
    assert!(ready.app.jobs.focus.is_none());
    assert_eq!(ready.app.session.documents().len(), 2);
    assert_eq!(ready.app.session.active().unwrap().doc.id, ready.target.id);
    assert!(ready.app.jobs.opens.is_empty() && ready.app.held_opens.is_empty());
    assert!(ready.app.ui.transform.as_ref().is_some_and(|transform| transform.made == Some(crate::state::MadeLayer::Place)));
    crate::transform_tool::cancel(&mut ready.app);
    assert_eq!(ready.app.session.active().unwrap().doc.layers.len(), ready.target.layers.len());
    assert_eq!(std::fs::read(&sources[0].0).unwrap(), sources[0].1);
}

#[test]
fn native_smart_picker_completions_do_not_start_place_transform_over_an_unrelated_tab() {
    let (_directory, sources) = smart_picker_sources();
    for open_first in [false, true] {
        let (mut ready, files) = ready_smart_picker(true);
        complete_smart_picker_before_shell(&mut ready, &files, &sources, open_first);
        present_place_and_open(&mut ready);
        let ctx = ready.app.ctx().clone();
        super::tick(&mut ready.app, &ctx);
        assert!(ready.app.jobs.focus.is_none());
        assert!(ready.app.ui.transform.is_none());
        assert_picked_smart_sources(&ready.app, &sources);
        let unrelated = ready.app.session.documents().iter().find(|state| state.doc.id == ready.unrelated.id).unwrap();
        assert!(Arc::ptr_eq(&unrelated.doc, &ready.unrelated));
        let target = ready.app.session.documents().iter().find(|state| state.doc.id == ready.target.id).unwrap();
        assert_eq!(target.doc.layers.len(), ready.target.layers.len() + 1);
    }
}
