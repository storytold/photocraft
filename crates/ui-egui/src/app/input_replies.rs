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
}

impl InputReplies {
    pub(crate) fn is_empty(&self) -> bool {
        self.waiters.is_empty()
    }

    /// Log a refusal unless it repeats the one this pass logged just before it (a refusal is
    /// often reported by the command and again by the menu or shortcut that ran it).
    fn log(&mut self, error: &str) {
        let error = error.trim();
        if error.is_empty()
            || self.errors.len() >= MAX_ERRORS
            || self.errors.get(self.pass_start..).and_then(<[String]>::last).is_some_and(|last| last == error)
        {
            return;
        }
        self.errors.push(error.to_string());
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

    /// Fail every waiting request with `error` and drop the log.
    fn fail(&mut self, error: &str) {
        for w in self.waiters.drain(..) {
            let _ = w.reply.send(serde_json::json!({"ok": false, "error": error}));
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
    pub(super) fn note_synthetic_taken(&mut self, n: usize) {
        self.input_replies.taken = self.input_replies.taken.saturating_add(n as u64);
    }

    /// Record a refusal caused by the synthetic input being handled now, for the control reply
    /// waiting on it. A no-op outside synthetic input. The status bar is the caller's business.
    pub(crate) fn note_input_error(&mut self, error: &str) {
        if self.automation_input {
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

    /// The in-flight batch's pass begins: remember the status bar to compare with at its end.
    pub(super) fn begin_input_errors(&mut self) {
        self.input_replies.status_before = Some((self.ui.status.clone(), self.ui.status_error));
        self.input_replies.pass_start = self.input_replies.errors.len();
    }

    /// The in-flight batch's pass finished. A status-bar error it raised that no refusal path
    /// logged is logged now, and requests whose last event this pass handled stop collecting.
    pub(super) fn finish_input_errors(&mut self) {
        if let Some(before) = self.input_replies.status_before.take()
            && self.ui.status_error
            && (before.0 != self.ui.status || !before.1)
        {
            let status = self.ui.status.clone();
            // A wrapped message ("Save failed: <refusal>") says nothing new.
            let r = &self.input_replies;
            let known = r.errors.get(r.pass_start..).unwrap_or_default().iter().any(|e| status.contains(e.as_str()));
            if !known {
                self.input_replies.log(&status);
            }
        }
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
    pub(super) fn settle_input_replies(&mut self) {
        self.input_replies.settle();
    }

    /// The queued input was dropped: fail the waiting requests with `error`.
    pub(super) fn fail_input_replies(&mut self, error: &str) {
        self.input_replies.fail(error);
    }
}

#[cfg(test)]
mod tests {
    use super::InputReplies;
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
}
