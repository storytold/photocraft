//! Replies to synthetic-input control requests (`ui.key`, `ui.type`, `ui.click`, `ui.move`),
//! and the refusals that input caused (#11579).
//!
//! A request's reply waits until all queued synthetic input has been handled. While a batch is
//! being handled (`automation_input`), every refusal it causes is logged: a command or menu item
//! that was refused, a disabled shortcut, a dialog OK that was refused, an unsaved-changes prompt
//! whose answer failed, and any other status-bar error raised during that pass. Each waiting
//! request owns the slice of that log from when the input queued before it had been handled to
//! the end of the pass that handled its own last event, so with several requests in flight each
//! reply carries only the refusals its own input caused. A reply with refusals is
//! `{"ok": false, "error": <first>, "errors": [...]}`; input that was handled without one still
//! replies `{"ok": true, "result": null}`.

use super::PhotocraftApp;
use crate::ControlResponse;
use std::sync::mpsc::Sender;

/// At most this many refusals are kept per settle; input that causes more keeps the first ones.
const MAX_ERRORS: usize = 64;

/// One control reply waiting for queued synthetic input.
struct Waiter {
    reply: Sender<ControlResponse>,
    /// The log length when its input began to be handled: once every request queued before it
    /// was handled (`None` until then). Its refusals start here.
    from: Option<usize>,
    /// The input ends once this many synthetic events have been taken for delivery.
    until_taken: u64,
    /// The log length once the pass that handled its last event finished.
    to: Option<usize>,
}

/// Waiting replies and the refusals logged for them.
#[derive(Default)]
pub(crate) struct InputReplies {
    waiters: Vec<Waiter>,
    /// Refusals logged while synthetic input was handled since the queue was last empty.
    errors: Vec<String>,
    /// Where the current pass's refusals start in `errors`: repeats are only folded within a pass.
    pass_start: usize,
    /// Synthetic events taken for delivery so far (`take_synthetic_step`).
    taken: u64,
    /// The status bar as the in-flight batch's pass began: an error that differs from it was
    /// raised by that pass.
    status_before: Option<(String, bool)>,
    /// A direct control request is being handled (`drain_control`): its failures are its own
    /// reply's, never the synthetic input's, even while a synthetic step is in flight.
    direct: bool,
}

impl InputReplies {
    pub(crate) fn is_empty(&self) -> bool {
        self.waiters.is_empty()
    }

    /// Log a refusal unless it repeats the one this pass logged just before it (a refusal is
    /// often reported by the command and again by the menu or shortcut that ran it). Past
    /// `MAX_ERRORS` only the first refusal of the request being handled is kept, so every refused
    /// request still fails.
    fn log(&mut self, error: &str) {
        let error = error.trim();
        if error.is_empty() || self.errors.get(self.pass_start..).and_then(<[String]>::last).is_some_and(|last| last == error) {
            return;
        }
        if self.errors.len() >= MAX_ERRORS {
            let len = self.errors.len();
            let current = self.waiters.iter().find(|w| w.to.is_none());
            let has_own = current.is_none_or(|w| w.from.is_none_or(|from| len > from));
            if has_own {
                return;
            }
        }
        self.errors.push(error.to_string());
    }

    /// Whether `status` only wraps a refusal this pass already logged ("Couldn't save: <it>").
    fn wraps_logged(&self, status: &str) -> bool {
        self.errors.get(self.pass_start..).unwrap_or_default().iter().any(|e| status == e || status.ends_with(&format!(": {e}")))
    }

    /// Where the next synthetic step must end so it carries one request's input only: the
    /// first request boundary after `taken`.
    fn next_boundary(&self) -> Option<u64> {
        self.waiters.iter().map(|w| w.until_taken).filter(|&end| end > self.taken).min()
    }

    /// The reply for refusals `errors` (none: the input was handled).
    fn reply(errors: &[String]) -> ControlResponse {
        match errors.first() {
            None => serde_json::json!({"ok": true, "result": null}),
            Some(first) => serde_json::json!({"ok": false, "error": first, "errors": errors}),
        }
    }

    /// Answer every waiting request with the refusals its input caused, and start a new log.
    fn settle(&mut self) {
        let errors = std::mem::take(&mut self.errors);
        self.pass_start = 0;
        for w in self.waiters.drain(..) {
            let to = w.to.unwrap_or(errors.len());
            let mine = errors.get(w.from.unwrap_or(to)..to).unwrap_or_default();
            let _ = w.reply.send(Self::reply(mine));
        }
        self.status_before = None;
    }

    /// The queued input was dropped: a request whose input was already handled gets its own
    /// reply (its refusals or `ok`); the others fail with `error`. Then drop the log.
    fn fail(&mut self, error: &str) {
        for w in self.waiters.drain(..) {
            let reply = match w.to {
                Some(to) => Self::reply(self.errors.get(w.from.unwrap_or(to)..to).unwrap_or_default()),
                None => serde_json::json!({"ok": false, "error": error}),
            };
            let _ = w.reply.send(reply);
        }
        self.errors.clear();
        self.pass_start = 0;
        self.status_before = None;
    }
}

impl PhotocraftApp {
    /// Reply to `reply` once the synthetic input queued so far has been handled.
    pub(crate) fn wait_for_input(&mut self, reply: Sender<ControlResponse>) {
        let r = &mut self.input_replies;
        let until_taken = r.taken.saturating_add(self.synthetic.len() as u64);
        // Input queued by earlier requests is handled first: this one's refusals start after it.
        let from = r.waiters.last().map_or(Some(r.errors.len()), |prev| prev.to);
        r.waiters.push(Waiter { reply, from, until_taken, to: None });
    }

    /// Note that `n` synthetic events were taken for delivery.
    /// Each step is its own pass for folding repeated refusals, also for hosts that feed steps
    /// themselves and never call `begin_synthetic_pass`.
    pub(super) fn note_synthetic_taken(&mut self, n: usize) {
        let r = &mut self.input_replies;
        r.taken = r.taken.saturating_add(n as u64);
        r.pass_start = r.errors.len();
    }

    /// How many of the queued synthetic events the next step may take at most: up to the end of
    /// the first request that still has input queued (`None`: no limit).
    pub(super) fn synthetic_step_limit(&self) -> Option<usize> {
        let r = &self.input_replies;
        r.next_boundary().map(|end| usize::try_from(end.saturating_sub(r.taken)).unwrap_or(usize::MAX))
    }

    /// Handle direct control requests with refusal logging off (see `InputReplies::direct`).
    pub(crate) fn set_direct_request(&mut self, direct: bool) {
        self.input_replies.direct = direct;
    }

    /// Record a refusal caused by the synthetic input being handled now, for the control reply
    /// waiting on it. A no-op outside synthetic input. The status bar is the caller's business.
    pub(crate) fn note_input_error(&mut self, error: &str) {
        if self.automation_input && !self.input_replies.direct {
            self.input_replies.log(error);
        }
    }

    /// Show `error` in the status bar and, during synthetic input, report it to the control
    /// reply waiting on that input.
    pub(crate) fn show_error(&mut self, error: impl Into<String>) {
        let error = error.into();
        self.note_input_error(&error);
        self.ui.status = error;
        self.ui.status_error = true;
    }

    /// The status bar now, to compare with after shortcut handling (`log_status_change`); `None`
    /// outside synthetic input.
    pub(crate) fn input_status(&self) -> Option<(String, bool)> {
        self.automation_input.then(|| (self.ui.status.clone(), self.ui.status_error))
    }

    /// Log a status-bar error raised since `before` (from `input_status`) that no refusal path
    /// logged. Shortcuts run in `logic`, before the UI pass's own comparison begins.
    pub(crate) fn log_status_change(&mut self, before: Option<(String, bool)>) {
        let Some(before) = before else { return };
        if self.ui.status_error && (before.0 != self.ui.status || !before.1) && !self.input_replies.direct {
            let status = self.ui.status.clone();
            if !self.input_replies.wraps_logged(&status) {
                self.input_replies.log(&status);
            }
        }
    }

    /// The in-flight batch's pass begins: remember the status bar to compare with at its end.
    pub(super) fn begin_input_errors(&mut self) {
        self.input_replies.status_before = Some((self.ui.status.clone(), self.ui.status_error));
        self.input_replies.pass_start = self.input_replies.errors.len();
    }

    /// The in-flight batch's pass finished. A status-bar error it raised that no refusal path
    /// logged is logged now, and requests whose last event this pass handled stop collecting.
    pub(super) fn finish_input_errors(&mut self) {
        let before = self.input_replies.status_before.take();
        self.log_status_change(before);
        let (taken, len) = (self.input_replies.taken, self.input_replies.errors.len());
        let mut prev_to = None;
        for w in &mut self.input_replies.waiters {
            if w.from.is_none() {
                w.from = prev_to;
            }
            if w.to.is_none() && w.until_taken <= taken {
                w.to = Some(len);
            }
            prev_to = w.to;
        }
    }

    /// All queued synthetic input has been handled: answer the waiting requests.
    pub(crate) fn settle_input_replies(&mut self) {
        self.input_replies.settle();
    }

    /// The queued input was dropped: fail the waiting requests with `error`.
    pub(super) fn fail_input_replies(&mut self, error: &str) {
        self.input_replies.fail(error);
    }
}

#[cfg(test)]
mod tests {
    use super::{InputReplies, PhotocraftApp};
    use serde_json::json;

    #[test]
    fn no_refusal_replies_ok_and_refusals_reply_the_first_with_all() {
        assert_eq!(InputReplies::reply(&[]), json!({"ok": true, "result": null}));
        let errors = ["a".to_string(), "b".to_string()];
        assert_eq!(InputReplies::reply(&errors), json!({"ok": false, "error": "a", "errors": ["a", "b"]}));
    }

    #[test]
    fn the_log_skips_blanks_and_back_to_back_repeats_and_is_capped() {
        let mut r = InputReplies::default();
        r.log("  ");
        r.log("x");
        r.log("x");
        r.log("y");
        r.log("x");
        assert_eq!(r.errors, ["x", "y", "x"]);
        for i in 0..200 {
            r.log(&i.to_string());
        }
        assert_eq!(r.errors.len(), super::MAX_ERRORS);
    }

    #[test]
    fn a_waiter_out_of_range_of_the_log_still_gets_an_answer() {
        let mut r = InputReplies::default();
        let (tx, rx) = std::sync::mpsc::channel();
        r.waiters.push(super::Waiter { reply: tx, from: Some(5), until_taken: 0, to: Some(2) });
        r.settle();
        assert_eq!(rx.recv().unwrap()["ok"], true);
    }

    /// Earlier input filled the capped log: a later refused request still fails.
    #[test]
    fn a_full_log_still_keeps_each_requests_first_refusal() {
        let mut r = InputReplies::default();
        let (first_tx, first) = std::sync::mpsc::channel();
        let (second_tx, second) = std::sync::mpsc::channel();
        r.waiters.push(super::Waiter { reply: first_tx, from: Some(0), until_taken: 1, to: None });
        r.waiters.push(super::Waiter { reply: second_tx, from: None, until_taken: 2, to: None });
        for i in 0..super::MAX_ERRORS + 10 {
            r.log(&format!("first {i}"));
        }
        assert_eq!(r.errors.len(), super::MAX_ERRORS, "the first request is capped");
        let len = r.errors.len();
        r.waiters[0].to = Some(len);
        r.waiters[1].from = Some(len);
        r.log("second refused");
        r.log("second again");
        r.settle();
        assert_eq!(first.recv().unwrap()["errors"].as_array().map(Vec::len), Some(super::MAX_ERRORS));
        assert_eq!(second.recv().unwrap(), json!({"ok": false, "error": "second refused", "errors": ["second refused"]}));
    }

    /// Later queued input is cancelled: requests whose input was already handled keep their own
    /// replies (a refusal or `ok`); only the one still waiting on input is cancelled.
    #[test]
    fn cancelling_later_input_keeps_the_replies_of_handled_requests() {
        let mut r = InputReplies::default();
        let (refused_tx, refused) = std::sync::mpsc::channel();
        let (handled_tx, handled) = std::sync::mpsc::channel();
        let (pending_tx, pending) = std::sync::mpsc::channel();
        r.waiters.push(super::Waiter { reply: refused_tx, from: Some(0), until_taken: 1, to: None });
        r.log("A refused");
        r.waiters[0].to = Some(1);
        r.waiters.push(super::Waiter { reply: handled_tx, from: Some(1), until_taken: 2, to: Some(1) });
        r.waiters.push(super::Waiter { reply: pending_tx, from: Some(1), until_taken: 3, to: None });
        r.fail("input cancelled: the document changed");
        assert_eq!(refused.recv().unwrap(), json!({"ok": false, "error": "A refused", "errors": ["A refused"]}));
        assert_eq!(handled.recv().unwrap(), json!({"ok": true, "result": null}));
        assert_eq!(pending.recv().unwrap(), json!({"ok": false, "error": "input cancelled: the document changed"}));
        assert!(r.waiters.is_empty() && r.errors.is_empty());
    }

    fn app() -> PhotocraftApp {
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    }

    /// One synthetic pass, as `ui_frame` runs it, around `during`; then the queue settles.
    fn one_pass(app: &mut PhotocraftApp, during: impl FnOnce(&mut PhotocraftApp)) -> crate::ControlResponse {
        let (tx, rx) = std::sync::mpsc::channel();
        app.wait_for_input(tx);
        app.automation_input = true;
        app.begin_input_errors();
        during(app);
        app.finish_input_errors();
        app.automation_input = false;
        app.settle_input_replies();
        rx.recv().unwrap()
    }

    /// A failed Actions playback whose message is already in the status bar is still reported.
    #[test]
    fn a_repeated_failed_playback_fails_the_input() {
        let mut app = app();
        let failed = json!({"failed": {"step": 0, "id": "layer.delete", "error": "no such layer"}});
        crate::actions::report_play(&mut app, &failed);
        assert!(app.ui.status_error, "the earlier playback's failure is showing");
        let r = one_pass(&mut app, |app| crate::actions::report_play(app, &failed));
        assert_eq!(r["ok"], false, "{r}");
        assert!(r["error"].as_str().is_some_and(|e| e.contains("no such layer")), "{r}");
    }

    /// A status error that merely contains an earlier refusal is a separate refusal; one that
    /// wraps it ("Couldn't save: <it>") is not.
    #[test]
    fn a_distinct_refusal_containing_an_earlier_one_is_kept() {
        let mut app = app();
        let r = one_pass(&mut app, |app| {
            app.note_input_error("target layer is hidden");
            app.ui.status = "Can't paint because the target layer is hidden".into();
            app.ui.status_error = true;
        });
        assert_eq!(r["errors"], json!(["target layer is hidden", "Can't paint because the target layer is hidden"]), "{r}");
        let r = one_pass(&mut app, |app| {
            app.note_input_error("disk full");
            app.ui.status = "Couldn't save: disk full".into();
            app.ui.status_error = true;
        });
        assert_eq!(r["errors"], json!(["disk full"]), "a wrapper adds nothing: {r}");
    }

    /// A host that feeds steps itself (`take_synthetic_step`, no `begin_synthetic_pass`) still
    /// reports the same refusal to each of two requests.
    #[test]
    fn identical_refusals_in_successive_steps_reach_both_requests() {
        let services = crate::Services {
            automation_command: Some(Box::new(|id, _| if id == "file.save" { Err("filesystem command denied".into()) } else { Ok(()) })),
            export: Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((Vec::new(), Vec::new())))),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let ctx = egui::Context::default();
        let mut replies = Vec::new();
        for _ in 0..2 {
            let (req, _) = crate::ControlRequest::new("ui.key", json!({"key": "S", "command": true}));
            assert!(matches!(crate::control::handle(&mut app, &ctx, &req), crate::control::Outcome::AfterInput));
            let (tx, rx) = std::sync::mpsc::channel();
            app.wait_for_input(tx);
            replies.push(rx);
        }
        while !app.synthetic.is_empty() {
            let events = app.take_synthetic_step();
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| crate::shortcuts::handle(&mut app, ui.ctx()));
            out.textures_delta.clear();
            app.finish_synthetic_pass();
            app.automation_input = false;
        }
        app.settle_input_replies();
        for rx in replies {
            assert_eq!(rx.recv().unwrap()["error"], "filesystem command denied");
        }
    }
}
