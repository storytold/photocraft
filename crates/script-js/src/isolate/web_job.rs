//! The page's side of a run in a web Worker: what `IsolatedJob` is on the desktop.
//!
//! The Worker holds the engine; the page keeps the session. Every request the
//! Worker sends is checked here exactly as the desktop supervisor checks a script host's (the
//! commands and their parameters, dialogs, windows, the per-run bounds), because the Worker
//! runs the script and is not trusted. The Worker waits for the page's answer in shared memory,
//! so a run continues where it was after a command or a dialog, as on the desktop.
//!
//! The limits: the run's time is counted here, with the clock stopped while the Worker waits for
//! an answer (a command, a dialog), and the Worker is terminated when it passes; its heap is
//! capped inside the Worker (`super::heap`); an answer that does not come within its own limit
//! ends the run too. The page needs no thread for any of this: [`WebJob::poll`] is called every
//! frame.
//!
//! The transport (the Worker itself) is behind [`Transport`], so everything here is tested on
//! any platform with a fake one.

use std::collections::VecDeque;

use serde_json::Value;
use web_time::{Duration, Instant};

use super::protocol::{
    FromHost, MAX_FRAME_DEPTH, MAX_FRAME_VALUES, MAX_FROM_HOST, MAX_LINE, MAX_MESSAGE, MAX_TO_HOST, ToHost, check_dialog_args, check_window, cut, parse_body,
    too_deep, values_in,
};
use crate::commands::check_command;
use crate::tape::{MAX_ENTRIES, MAX_OUTPUT_BYTES, MAX_WINDOW_BYTES};
use crate::{Modal, ModalKind};

/// What a Worker sent, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    /// A message: a [`FromHost`] as JSON text, and when it arrived. A request stops the run's
    /// clock from then, not from when the page gets to it (a hidden tab may draw no frames for
    /// a while).
    Message(String, Instant),
    /// The Worker failed (a trap: a failed allocation, a stack overflow, an engine panic), with
    /// the browser's description.
    Error(String),
    /// The Worker could not be started or reached (its files failed to load, it never said it
    /// listens), with why.
    Unavailable(String),
}

/// The Worker, as the job sees it.
pub trait Transport {
    /// Start the run: send the Worker its start message (a [`ToHost::Start`] as JSON).
    ///
    /// # Errors
    /// The Worker could not be reached.
    fn start(&mut self, start: &str) -> Result<(), String>;
    /// Everything the Worker sent since the last call.
    fn take(&mut self) -> Vec<Incoming>;
    /// Hand the Worker the answer it waits for (a [`ToHost`] as JSON).
    ///
    /// # Errors
    /// The answer could not be handed over (larger than the shared memory holds).
    fn answer(&mut self, answer: &str) -> Result<(), String>;
    /// The most bytes [`Self::answer`] takes. A command's result past it reaches the script as
    /// the command's error, as one past the protocol's own limit does.
    fn answer_limit(&self) -> usize {
        MAX_TO_HOST
    }
    /// Stop the Worker now.
    fn terminate(&mut self);
}

/// Per-run settings.
#[derive(Debug, Clone)]
pub struct WebOptions {
    /// `app.name` for the run.
    pub app_name: Option<String>,
    /// File name shown in script error messages.
    pub script_name: Option<String>,
    /// Time the run may compute, not counting the time it waits for answers.
    pub time_limit: Duration,
    /// Bytes the Worker's heap may hold.
    pub memory_limit: u64,
    /// History step key of the run's commands.
    pub coalesce: String,
    /// Longest the page may take to answer one command, and one dialog.
    pub reply_limits: (Duration, Duration),
}

/// What the run needs from the page next.
#[derive(Debug, Clone, PartialEq)]
pub enum WebEvent {
    /// Run this session command, then [`WebJob::reply_executed`]. The params carry the run's
    /// history key.
    Execute { id: String, params: Value },
    /// A line of script output.
    Output(String),
    /// Show this dialog, then [`WebJob::reply_dialog`].
    Dialog(Modal),
    /// Show, update or hide a ScriptUI window that does not wait.
    Window { id: u32, window: Option<Value> },
    /// The run ended: `Ok` when the script finished, else the message for the user.
    Finished(Result<(), String>),
}

/// How many of the Worker's messages one [`WebJob::poll`] checks at most.
const CHECKS_PER_POLL: usize = 64;

/// How long one [`WebJob::poll`] goes on checking the Worker's messages, well inside the page's
/// frame budget: past it the rest wait for the next poll.
const CHECK_TIME: Duration = Duration::from_millis(2);

/// One run in a Worker.
pub struct WebJob<T: Transport> {
    transport: T,
    options: WebOptions,
    /// Events checked and counted as they arrived, not yet handed out. Every message is checked
    /// when it is taken from the Worker, so what is held here is already within the run's
    /// bounds (calls, output and window bytes) however fast the Worker sends.
    events: VecDeque<WebEvent>,
    /// Messages taken from the Worker and not checked yet: each poll checks at most
    /// [`CHECKS_PER_POLL`], for at most [`CHECK_TIME`], so a large batch is spread over the page's
    /// polls (and frames) rather than parsed in one. Each keeps its arrival time.
    backlog: VecDeque<Incoming>,
    /// Time computed before the current stretch, and since when it computes (`None` while it
    /// waits for an answer).
    spent: Duration,
    since: Option<Instant>,
    /// Since when it waits for an answer, and whether to a dialog.
    waiting: Option<(Instant, bool)>,
    /// The run's end, not yet handed out.
    ending: Option<Result<(), String>>,
    finished: bool,
    calls: usize,
    output: usize,
    windows: usize,
}

impl<T: Transport> WebJob<T> {
    /// Start `source` in the Worker behind `transport`.
    ///
    /// # Errors
    /// The script is too large to send, or the Worker could not be reached.
    pub fn spawn(mut transport: T, source: String, options: WebOptions) -> Result<Self, String> {
        let start = ToHost::Start { source, app_name: options.app_name.clone(), script_name: options.script_name.clone(), memory_limit: options.memory_limit };
        let start = serde_json::to_string(&start).map_err(|e| e.to_string())?;
        if start.len() > MAX_TO_HOST {
            return Err(format!("The script is too large to run: {} MiB, over the limit of {} MiB.", start.len().div_ceil(1 << 20), MAX_TO_HOST >> 20));
        }
        transport.start(&start)?;
        Ok(Self {
            transport,
            options,
            events: VecDeque::new(),
            backlog: VecDeque::new(),
            spent: Duration::ZERO,
            since: Some(Instant::now()),
            waiting: None,
            ending: None,
            finished: false,
            calls: 0,
            output: 0,
            windows: 0,
        })
    }

    fn elapsed(&self) -> Duration {
        self.spent + self.since.map_or(Duration::ZERO, |t| t.elapsed())
    }

    fn pause(&mut self) {
        self.pause_at(Instant::now());
    }

    /// Stop the clock as of `at`.
    fn pause_at(&mut self, at: Instant) {
        if let Some(t) = self.since.take() {
            self.spent += at.saturating_duration_since(t);
        }
    }

    /// End the run now with `result`: the Worker is stopped, and nothing it sent that was not
    /// handed out yet counts.
    fn end(&mut self, result: Result<(), String>) {
        if self.ending.is_none() && !self.finished {
            self.events.clear();
            self.backlog.clear();
            self.stop(result);
        }
    }

    /// The Worker reported its end: what it sent before still comes first.
    fn stop(&mut self, result: Result<(), String>) {
        if self.ending.is_none() && !self.finished {
            self.transport.terminate();
            self.waiting = None;
            self.pause();
            self.ending = Some(result);
        }
    }

    fn violation(&mut self, why: &str) {
        self.end(Err(format!("The script was stopped: the script Worker made a request it may not make ({why}).")));
    }

    /// Count one host call; `false` past the bound (the run is ended).
    fn count_call(&mut self) -> bool {
        self.calls += 1;
        if self.calls > MAX_ENTRIES {
            self.violation(&format!("more than {MAX_ENTRIES} commands, dialogs, output lines and window updates"));
            return false;
        }
        true
    }

    /// The next event, or `None` while the run computes, waits for a reply, or has finished.
    pub fn poll(&mut self) -> Option<WebEvent> {
        if self.finished {
            return None;
        }
        // What arrived is taken first: a request that came in time stopped the clock when it
        // came, however late this poll is.
        if self.ending.is_none() {
            self.backlog.extend(self.transport.take());
            // Every message but the last (the run's end) is a host call: more than the run may
            // make is refused at once, which bounds the backlog and how far it lags.
            if self.calls.saturating_add(self.backlog.len()) > MAX_ENTRIES + 1 {
                self.backlog.clear();
                self.violation(&format!("more than {MAX_ENTRIES} commands, dialogs, output lines and window updates"));
            }
            // Bounded by count and by time, checked between messages: a few large window
            // updates stop a poll as surely as many small lines (one message is always checked,
            // so a poll always gets on).
            let started = Instant::now();
            for n in 0..CHECKS_PER_POLL {
                if self.ending.is_some() || (n > 0 && started.elapsed() >= CHECK_TIME) {
                    break;
                }
                let Some(incoming) = self.backlog.pop_front() else { break };
                if let Some(event) = self.handle(incoming) {
                    self.events.push_back(event);
                }
            }
        }
        // With messages still unchecked the clock can't be read yet: one of them may be a request
        // that stopped it. Each is checked against the time limit by its arrival time instead.
        if self.ending.is_none() && self.backlog.is_empty() {
            self.check_limits();
        }
        if let Some(event) = self.events.pop_front() {
            // The page sees the request now: its time to answer starts now.
            if let (WebEvent::Execute { .. } | WebEvent::Dialog(_), Some((since, _))) = (&event, self.waiting.as_mut()) {
                *since = Instant::now();
            }
            return Some(event);
        }
        self.ending.as_ref()?;
        self.finished = true;
        self.ending.take().map(WebEvent::Finished)
    }

    /// A request taken from the Worker but not handed out to the page yet. Nothing is queued
    /// after a request (the Worker waits, and a message meanwhile ends the run), so only the last
    /// event can be one.
    fn request_queued(&self) -> bool {
        matches!(self.events.back(), Some(WebEvent::Execute { .. } | WebEvent::Dialog(_)))
    }

    fn over_time(&mut self) {
        self.end(Err("The script was stopped: it ran longer than its time limit.".into()));
    }

    fn check_limits(&mut self) {
        if self.elapsed() > self.options.time_limit {
            self.over_time();
            return;
        }
        // The page's time to answer starts when it is handed the request.
        if let Some((since, dialog)) = self.waiting.filter(|_| !self.request_queued()) {
            let (what, limit) = if dialog { ("dialog", self.options.reply_limits.1) } else { ("command", self.options.reply_limits.0) };
            if since.elapsed() > limit {
                self.end(Err(format!("The script was stopped: its {what} got no answer within {} s.", limit.as_secs())));
            }
        }
    }

    fn handle(&mut self, incoming: Incoming) -> Option<WebEvent> {
        let (text, at) = match incoming {
            Incoming::Message(text, at) => (text, at),
            Incoming::Error(why) => {
                self.end(Err(format!("The script stopped unexpectedly: {} (it may have run out of memory or overflowed its stack).", cut(&why, MAX_MESSAGE))));
                return None;
            }
            Incoming::Unavailable(why) => {
                self.end(Err(format!("The script couldn't run: {}.", cut(&why, MAX_MESSAGE))));
                return None;
            }
        };
        // Whatever came after the time limit passed does not count, its end included: a page that
        // polls late (a hidden tab) must not let a run past its limit finish as a success.
        if let Some(t) = self.since
            && self.spent + at.saturating_duration_since(t) > self.options.time_limit
        {
            self.over_time();
            return None;
        }
        let msg = match parse_body::<FromHost>(text.as_bytes(), MAX_FROM_HOST) {
            Ok(m) => m,
            Err(e) => {
                self.violation(&e.to_string());
                return None;
            }
        };
        // A Worker waits for each answer; it sends nothing meanwhile, not even its end.
        if self.waiting.is_some() {
            self.violation("a message while it waited for an answer");
            return None;
        }
        if let FromHost::Finished { error } = msg {
            self.stop(error.map_or(Ok(()), |m| Err(cut(&m, MAX_MESSAGE).to_owned())));
            return None;
        }
        if !self.count_call() {
            return None;
        }
        match msg {
            FromHost::Execute { id, mut params } => {
                if let Err(d) = check_command(&id, &params) {
                    self.violation(&d);
                    return None;
                }
                // The Worker's commands always join the run's own history step.
                if let Value::Object(map) = &mut params {
                    map.insert("coalesce".into(), Value::String(self.options.coalesce.clone()));
                }
                self.pause_at(at);
                self.waiting = Some((at, false));
                Some(WebEvent::Execute { id, params })
            }
            FromHost::Dialog { kind, args } => {
                if let Err(d) = check_dialog_args(kind, &args) {
                    self.violation(&d);
                    return None;
                }
                self.pause_at(at);
                self.waiting = Some((at, true));
                Some(WebEvent::Dialog(Modal { kind: ModalKind::from(kind), args }))
            }
            FromHost::Output { line } => {
                let line = cut(&line, MAX_LINE + 64).to_owned();
                self.output = self.output.saturating_add(line.len());
                if self.output > MAX_OUTPUT_BYTES {
                    self.violation(&format!("more than {} MiB of output", MAX_OUTPUT_BYTES >> 20));
                    return None;
                }
                Some(WebEvent::Output(line))
            }
            FromHost::Window { id, window } => {
                if let Some(Err(d)) = window.as_ref().map(check_window) {
                    self.violation(&d);
                    return None;
                }
                let len = window.as_ref().map_or(0, |w| w.to_string().len());
                self.windows = self.windows.saturating_add(len);
                if self.windows > MAX_WINDOW_BYTES {
                    self.violation(&format!("more than {} MiB of window updates", MAX_WINDOW_BYTES >> 20));
                    return None;
                }
                Some(WebEvent::Window { id, window })
            }
            FromHost::Finished { .. } => None,
        }
    }

    fn reply(&mut self, dialog: bool, msg: &ToHost) -> Result<(), String> {
        // An answer after the reply limit is too late: the run ends as if none came.
        if self.ending.is_none() {
            self.check_limits();
        }
        if self.finished || self.ending.is_some() {
            return Err("the script has ended".into());
        }
        if self.waiting.map(|(_, d)| d) != Some(dialog) || self.request_queued() {
            return Err("the script is not waiting for that reply".into());
        }
        // Anything the Worker sent that is here before this answer was sent while it should have
        // waited: refused now, before the answer goes out (with `waiting` set, `handle` ends
        // the run on the first one). A message still on its way to the page can't be told from
        // one sent after the answer; the order is the Worker's to keep, and every message is
        // checked and bounded either way.
        if let Some(early) = self.backlog.pop_front().or_else(|| self.transport.take().into_iter().next()) {
            let _ = self.handle(early);
        }
        if self.ending.is_some() {
            return Err("the script has ended".into());
        }
        let mut text = serde_json::to_string(msg).map_err(|e| e.to_string())?;
        // The Worker's limits, checked here as the desktop app checks the script host's: an
        // answer it would refuse can't be told from a broken link there.
        let limit = MAX_TO_HOST.min(self.transport.answer_limit());
        let too_big = if text.len() > limit {
            Some(format!("{} MiB, over the limit of {} MiB", text.len().div_ceil(1 << 20), limit >> 20))
        } else if values_in(text.as_bytes()) > MAX_FRAME_VALUES {
            Some(format!("more than {MAX_FRAME_VALUES} values"))
        } else if too_deep(text.as_bytes()) {
            Some(format!("nested more than {MAX_FRAME_DEPTH} deep"))
        } else {
            None
        };
        if let Some(why) = too_big {
            if dialog {
                self.end(Err(format!("The script was stopped: a dialog's answer could not be sent to the script Worker: {why}.")));
                return Err(why);
            }
            // A command's result: the script gets the reason as the command's error.
            let result: Result<Value, String> = Err(format!("the command's result is too large for a script: {why}"));
            text = serde_json::to_string(&ToHost::Executed { result }).map_err(|e| e.to_string())?;
        }
        self.waiting = None;
        self.since = Some(Instant::now());
        if let Err(e) = self.transport.answer(&text) {
            let what = if dialog { "a dialog's answer" } else { "a command's result" };
            self.end(Err(format!("The script was stopped: {what} could not be sent to the script Worker ({e}).")));
            return Err(e);
        }
        Ok(())
    }

    /// Answer the command the run waits on.
    ///
    /// # Errors
    /// When the run waits for no command, the run ended (the Worker broke the protocol, or the
    /// answer came past the reply limit), or the answer could not be handed over (the run ends).
    pub fn reply_executed(&mut self, result: Result<Value, String>) -> Result<(), String> {
        self.reply(false, &ToHost::Executed { result })
    }

    /// Answer the dialog the run waits on.
    ///
    /// # Errors
    /// When the run waits for no dialog, the run ended (the Worker broke the protocol, or the
    /// answer came past the reply limit), or the answer could not be handed over (the run ends).
    pub fn reply_dialog(&mut self, answer: Value) -> Result<(), String> {
        self.reply(true, &ToHost::Dialog { answer })
    }

    /// Stop the run now. Its [`WebEvent::Finished`] still arrives.
    pub fn cancel(&mut self) {
        self.end(Err(super::CANCELLED.into()));
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use serde_json::json;

    use super::*;

    /// A Worker played by the test: what it will send, and what it was given.
    #[derive(Default, Clone)]
    struct Fake {
        inbox: Rc<RefCell<VecDeque<Incoming>>>,
        started: Rc<RefCell<Vec<String>>>,
        answers: Rc<RefCell<Vec<Value>>>,
        terminated: Rc<RefCell<bool>>,
        room: Option<usize>,
        /// Whether the job is told `room` ([`Transport::answer_limit`]).
        limit_known: bool,
    }

    impl Fake {
        fn send(&self, msg: &FromHost) {
            self.inbox.borrow_mut().push_back(Incoming::Message(serde_json::to_string(msg).unwrap(), Instant::now()));
        }
    }

    impl Transport for Fake {
        fn start(&mut self, start: &str) -> Result<(), String> {
            self.started.borrow_mut().push(start.to_owned());
            Ok(())
        }
        fn take(&mut self) -> Vec<Incoming> {
            self.inbox.borrow_mut().drain(..).collect()
        }
        fn answer(&mut self, answer: &str) -> Result<(), String> {
            if self.room.is_some_and(|r| answer.len() > r) {
                return Err("too large".into());
            }
            self.answers.borrow_mut().push(serde_json::from_str(answer).unwrap());
            Ok(())
        }
        fn answer_limit(&self) -> usize {
            if self.limit_known { self.room.unwrap_or(MAX_TO_HOST) } else { MAX_TO_HOST }
        }
        fn terminate(&mut self) {
            *self.terminated.borrow_mut() = true;
        }
    }

    fn options() -> WebOptions {
        WebOptions {
            app_name: None,
            script_name: Some("t.jsx".into()),
            time_limit: Duration::from_secs(60),
            memory_limit: 1 << 30,
            coalesce: "script:web:1".into(),
            reply_limits: (Duration::from_secs(60), Duration::from_secs(60)),
        }
    }

    fn job(fake: &Fake, options: WebOptions) -> WebJob<Fake> {
        WebJob::spawn(fake.clone(), "1".into(), options).unwrap()
    }

    /// Poll until the run ends, at most a bounded number of times: the messages that end it may
    /// be checked over several polls (each has a time budget, so a slow machine can split them),
    /// and what an earlier poll checked is handed out first.
    fn until_end(j: &mut WebJob<Fake>) -> Result<(), String> {
        for _ in 0..1000 {
            if let Some(WebEvent::Finished(r)) = j.poll() {
                return r;
            }
        }
        panic!("the run did not end")
    }

    #[test]
    fn a_run_relays_commands_output_and_dialogs_and_takes_their_answers() {
        let fake = Fake::default();
        let mut j = job(&fake, options());
        let start: Value = serde_json::from_str(&fake.started.borrow()[0]).unwrap();
        assert_eq!(start["type"], "start");
        fake.send(&FromHost::Output { line: "hi".into() });
        fake.send(&FromHost::Execute { id: "session.inspect".into(), params: json!({}) });
        assert_eq!(j.poll(), Some(WebEvent::Output("hi".into())));
        assert_eq!(j.poll(), Some(WebEvent::Execute { id: "session.inspect".into(), params: json!({"coalesce": "script:web:1"}) }));
        assert_eq!(j.poll(), None, "it waits for the answer");
        j.reply_executed(Ok(json!({"documents": []}))).unwrap();
        assert_eq!(fake.answers.borrow()[0], json!({"type": "executed", "result": {"Ok": {"documents": []}}}));
        fake.send(&FromHost::Dialog { kind: ModalKind::Alert.into(), args: json!(["x"]) });
        assert!(matches!(j.poll(), Some(WebEvent::Dialog(m)) if m.kind == ModalKind::Alert));
        assert!(j.reply_executed(Ok(json!(1))).is_err(), "a dialog is answered as a dialog");
        j.reply_dialog(Value::Null).unwrap();
        fake.send(&FromHost::Finished { error: None });
        assert_eq!(j.poll(), Some(WebEvent::Finished(Ok(()))));
        assert_eq!(j.poll(), None);
        assert!(j.is_finished());
    }

    #[test]
    fn requests_a_script_may_not_make_end_the_run() {
        for bad in [
            serde_json::to_string(&FromHost::Execute { id: "file.saveAs".into(), params: json!({}) }).unwrap(),
            "{not json".to_owned(),
            serde_json::to_string(&FromHost::Dialog { kind: ModalKind::Window.into(), args: json!({"id": u64::from(u32::MAX) + 1}) }).unwrap(),
        ] {
            let fake = Fake::default();
            let mut j = job(&fake, options());
            fake.inbox.borrow_mut().push_back(Incoming::Message(bad.clone(), Instant::now()));
            let Err(m) = until_end(&mut j) else { panic!("{bad}") };
            assert!(m.contains("may not make"), "{m}");
            assert!(*fake.terminated.borrow());
        }
        // Asking again before the answer came is not allowed either.
        let fake = Fake::default();
        let mut j = job(&fake, options());
        fake.send(&FromHost::Execute { id: "session.inspect".into(), params: json!({}) });
        fake.send(&FromHost::Output { line: "early".into() });
        assert!(matches!(until_end(&mut j), Err(m) if m.contains("while it waited")));
    }

    /// A flood is stopped at the run's bounds: polls hand out no more than the bound allows and
    /// then the run's end, each poll checking only its share (the frame budget).
    #[test]
    fn a_flood_of_messages_is_stopped_at_the_bounds() {
        let fake = Fake::default();
        let mut j = job(&fake, options());
        let line = "x".repeat(1 << 20);
        for _ in 0..(MAX_OUTPUT_BYTES >> 20) + 8 {
            fake.send(&FromHost::Output { line: line.clone() });
        }
        let mut handed = 0usize;
        let mut end = None;
        // Bounded: every poll checks at least one of the messages sent.
        for _ in 0..(MAX_OUTPUT_BYTES >> 20) * 4 + 64 {
            match j.poll() {
                Some(WebEvent::Output(l)) => handed += l.len(),
                Some(WebEvent::Finished(r)) => {
                    end = Some(r);
                    break;
                }
                Some(other) => panic!("unexpected {other:?}"),
                None => {}
            }
        }
        let end = end.expect("the run ended");
        assert!(matches!(&end, Err(m) if m.contains("of output")), "{end:?}");
        assert!(handed <= MAX_OUTPUT_BYTES, "{handed}");
        assert!(*fake.terminated.borrow());
        assert!(j.events.is_empty() && j.backlog.is_empty());
    }

    /// Output sent just before the end still comes before it.
    #[test]
    fn output_sent_before_the_end_comes_first() {
        let fake = Fake::default();
        let mut j = job(&fake, options());
        fake.send(&FromHost::Output { line: "last words".into() });
        fake.send(&FromHost::Finished { error: None });
        assert_eq!(j.poll(), Some(WebEvent::Output("last words".into())));
        assert_eq!(j.poll(), Some(WebEvent::Finished(Ok(()))));
    }

    /// A request that arrived in time is not lost to a page that polls late.
    #[test]
    fn a_request_waiting_for_a_late_poll_does_not_count_as_compute_time() {
        let fake = Fake::default();
        let mut o = options();
        o.time_limit = Duration::from_millis(500);
        let mut j = job(&fake, o);
        fake.send(&FromHost::Execute { id: "session.inspect".into(), params: json!({}) });
        // The page draws no frame for a while (a hidden tab).
        std::thread::sleep(Duration::from_millis(600));
        assert!(matches!(j.poll(), Some(WebEvent::Execute { .. })));
        j.reply_executed(Ok(Value::Null)).unwrap();
        assert_eq!(j.poll(), None, "still within its limit");
    }

    /// A Worker that ends while a request of its waits for an answer broke the protocol.
    #[test]
    fn an_end_while_a_request_waits_is_refused_with_the_request() {
        let fake = Fake::default();
        let mut j = job(&fake, options());
        fake.send(&FromHost::Execute { id: "session.inspect".into(), params: json!({}) });
        fake.send(&FromHost::Finished { error: None });
        assert!(matches!(until_end(&mut j), Err(m) if m.contains("while it waited")));
    }

    /// A message the Worker sent while it should have waited, already at the page when the page
    /// answers, is refused even though no poll took it first.
    #[test]
    fn a_message_at_the_page_before_the_answer_is_refused() {
        let fake = Fake::default();
        let mut j = job(&fake, options());
        fake.send(&FromHost::Execute { id: "session.inspect".into(), params: json!({}) });
        assert!(matches!(j.poll(), Some(WebEvent::Execute { .. })));
        // At the page before the answer, not taken by a poll.
        fake.send(&FromHost::Finished { error: None });
        assert!(j.reply_executed(Ok(Value::Null)).is_err());
        assert!(fake.answers.borrow().is_empty(), "no answer goes to a Worker that broke the protocol");
        assert!(matches!(j.poll(), Some(WebEvent::Finished(Err(m))) if m.contains("while it waited")));
    }

    /// What arrives after the time limit passed does not count, even when the page polls late.
    #[test]
    fn an_end_that_arrives_past_the_time_limit_is_not_a_success() {
        let fake = Fake::default();
        let mut o = options();
        o.time_limit = Duration::from_millis(100);
        let mut j = job(&fake, o);
        std::thread::sleep(Duration::from_millis(150));
        fake.send(&FromHost::Output { line: "late".into() });
        fake.send(&FromHost::Finished { error: None });
        assert!(matches!(until_end(&mut j), Err(m) if m.contains("time limit")));
    }

    /// A request not handed out yet can't be answered, and its time to answer starts when the
    /// page gets it.
    #[test]
    fn a_request_is_answered_only_once_the_page_has_it() {
        let fake = Fake::default();
        let mut o = options();
        o.reply_limits = (Duration::from_millis(500), Duration::from_millis(500));
        let mut j = job(&fake, o);
        fake.send(&FromHost::Output { line: "first".into() });
        fake.send(&FromHost::Execute { id: "session.inspect".into(), params: json!({}) });
        assert_eq!(j.poll(), Some(WebEvent::Output("first".into())));
        assert!(j.reply_executed(Ok(Value::Null)).is_err(), "the page has not seen the command");
        std::thread::sleep(Duration::from_millis(600));
        assert!(matches!(j.poll(), Some(WebEvent::Execute { .. })), "its time to answer had not started");
        j.reply_executed(Ok(Value::Null)).unwrap();
    }

    /// A result the Worker would refuse (more values than it reads) reaches the script as the
    /// command's error, as on the desktop, not as a broken link.
    #[test]
    fn a_result_with_too_many_values_is_the_command_error() {
        let fake = Fake::default();
        let mut j = job(&fake, options());
        fake.send(&FromHost::Execute { id: "session.inspect".into(), params: json!({}) });
        assert!(matches!(j.poll(), Some(WebEvent::Execute { .. })));
        j.reply_executed(Ok(Value::Array(vec![json!(0); MAX_FRAME_VALUES + 1]))).unwrap();
        let answer = &fake.answers.borrow()[0];
        assert!(answer["result"]["Err"].as_str().is_some_and(|e| e.contains("values")), "{answer}");
        assert!(!j.is_finished());
    }

    /// So does a result nested deeper than the Worker parses.
    #[test]
    fn a_result_nested_too_deep_is_the_command_error() {
        let fake = Fake::default();
        let mut j = job(&fake, options());
        fake.send(&FromHost::Execute { id: "session.inspect".into(), params: json!({}) });
        assert!(matches!(j.poll(), Some(WebEvent::Execute { .. })));
        j.reply_executed(Ok((0..200).fold(json!(0), |v, _| json!([v])))).unwrap();
        let answer = &fake.answers.borrow()[0];
        assert!(answer["result"]["Err"].as_str().is_some_and(|e| e.contains("nested")), "{answer}");
        assert!(!j.is_finished());
    }

    /// A batch with more messages than a run may send ends the run on the poll it arrives, not
    /// after the page has worked through it share by share.
    #[test]
    fn a_batch_over_the_call_bound_ends_the_run_at_once() {
        let fake = Fake::default();
        let mut j = job(&fake, options());
        for _ in 0..MAX_ENTRIES + 2 {
            fake.send(&FromHost::Output { line: String::new() });
        }
        assert!(matches!(j.poll(), Some(WebEvent::Finished(Err(m))) if m.contains("commands, dialogs, output lines")));
        assert!(j.backlog.is_empty());
    }

    /// One poll checks a bounded share of a large batch, so the page's frame budget applies
    /// between them; the rest is checked by later polls, in order.
    #[test]
    fn a_large_batch_is_checked_over_several_polls() {
        let fake = Fake::default();
        let mut j = job(&fake, options());
        for i in 0..CHECKS_PER_POLL * 3 {
            fake.send(&FromHost::Output { line: i.to_string() });
        }
        assert_eq!(j.poll(), Some(WebEvent::Output("0".into())));
        // At most one share (fewer when the time cap ends the poll first).
        assert!(j.backlog.len() >= CHECKS_PER_POLL * 2, "only one share checked: {}", j.backlog.len());
        let mut lines = vec!["0".to_owned()];
        while let Some(WebEvent::Output(line)) = j.poll() {
            lines.push(line);
        }
        assert_eq!(lines, (0..CHECKS_PER_POLL * 3).map(|i| i.to_string()).collect::<Vec<_>>());
    }

    /// Output queued before a request doesn't make each poll look through the whole queue for
    /// it: a hundred thousand lines and then a dialog are handed out in time linear in the lines.
    #[test]
    fn output_queued_before_a_request_is_handed_out_in_linear_time() {
        let fake = Fake::default();
        let mut j = job(&fake, options());
        for _ in 0..100_000 {
            fake.send(&FromHost::Output { line: String::new() });
        }
        fake.send(&FromHost::Dialog { kind: ModalKind::Alert.into(), args: json!(["done"]) });
        // All checked up front, as when the batch arrived while the page was busy.
        for incoming in j.transport.take() {
            if let Some(event) = j.handle(incoming) {
                j.events.push_back(event);
            }
        }
        assert_eq!(j.events.len(), 100_001);
        let start = Instant::now();
        let mut outputs = 0;
        loop {
            match j.poll() {
                Some(WebEvent::Output(_)) => outputs += 1,
                Some(WebEvent::Dialog(_)) => break,
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(outputs, 100_000);
        assert!(start.elapsed() < Duration::from_secs(3), "took {:?}", start.elapsed());
    }

    /// An answer that comes after the reply limit is not taken.
    #[test]
    fn a_late_answer_ends_the_run() {
        let fake = Fake::default();
        let mut o = options();
        o.reply_limits = (Duration::from_millis(50), Duration::from_millis(50));
        let mut j = job(&fake, o);
        fake.send(&FromHost::Execute { id: "session.inspect".into(), params: json!({}) });
        assert!(matches!(j.poll(), Some(WebEvent::Execute { .. })));
        std::thread::sleep(Duration::from_millis(80));
        assert!(j.reply_executed(Ok(Value::Null)).is_err());
        assert!(fake.answers.borrow().is_empty());
        assert!(matches!(j.poll(), Some(WebEvent::Finished(Err(m))) if m.contains("got no answer")));
    }

    #[test]
    fn the_time_limit_counts_computing_not_waiting_and_ends_the_run() {
        let fake = Fake::default();
        let mut o = options();
        o.time_limit = Duration::from_millis(500);
        let mut j = job(&fake, o);
        fake.send(&FromHost::Execute { id: "session.inspect".into(), params: json!({}) });
        assert!(matches!(j.poll(), Some(WebEvent::Execute { .. })));
        // The page takes its time answering: not the script's.
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(j.poll(), None);
        j.reply_executed(Ok(Value::Null)).unwrap();
        assert_eq!(j.poll(), None, "still within its limit");
        std::thread::sleep(Duration::from_millis(600));
        assert!(matches!(j.poll(), Some(WebEvent::Finished(Err(m))) if m.contains("time limit")));
        assert!(*fake.terminated.borrow());
    }

    #[test]
    fn an_unanswered_request_a_crash_or_an_answer_too_large_ends_the_run() {
        let fake = Fake::default();
        let mut o = options();
        o.reply_limits = (Duration::from_millis(50), Duration::from_millis(50));
        let mut j = job(&fake, o);
        fake.send(&FromHost::Execute { id: "session.inspect".into(), params: json!({}) });
        assert!(matches!(j.poll(), Some(WebEvent::Execute { .. })));
        std::thread::sleep(Duration::from_millis(80));
        assert!(matches!(j.poll(), Some(WebEvent::Finished(Err(m))) if m.contains("got no answer")));

        let fake = Fake::default();
        let mut j = job(&fake, options());
        fake.inbox.borrow_mut().push_back(Incoming::Error("RuntimeError: unreachable".into()));
        assert!(matches!(j.poll(), Some(WebEvent::Finished(Err(m))) if m.contains("unreachable") && m.contains("memory")));

        let fake = Fake { room: Some(16), ..Fake::default() };
        let mut j = job(&fake, options());
        fake.send(&FromHost::Dialog { kind: ModalKind::Prompt.into(), args: json!(["Name?", ""]) });
        assert!(matches!(j.poll(), Some(WebEvent::Dialog(_))));
        assert!(j.reply_dialog(json!("a long answer that does not fit")).is_err());
        assert!(matches!(j.poll(), Some(WebEvent::Finished(Err(m))) if m.contains("could not be sent")));
    }

    /// A command's result past what the transport takes (the Worker's mailbox) reaches the
    /// script as the command's error, and the run goes on; one that fits is sent as it is.
    #[test]
    fn a_result_past_the_mailbox_is_the_commands_error() {
        let fake = Fake { room: Some(400), limit_known: true, ..Fake::default() };
        let mut j = job(&fake, options());
        for (result, sent) in [(json!("x".repeat(1000)), false), (json!("fits"), true)] {
            fake.send(&FromHost::Execute { id: "session.inspect".into(), params: json!({}) });
            assert!(matches!(j.poll(), Some(WebEvent::Execute { .. })));
            j.reply_executed(Ok(result.clone())).unwrap();
            let answer = fake.answers.borrow().last().cloned().unwrap();
            if sent {
                assert_eq!(answer, json!({"type": "executed", "result": {"Ok": result}}));
            } else {
                assert!(answer["result"]["Err"].as_str().is_some_and(|m| m.contains("too large for a script")), "{answer}");
            }
            assert!(!j.is_finished());
        }
    }

    /// A Worker that can't be started says so, not that the script ran out of memory.
    #[test]
    fn a_worker_that_cannot_start_says_so() {
        let fake = Fake::default();
        let mut j = job(&fake, options());
        fake.inbox.borrow_mut().push_back(Incoming::Unavailable("the script Worker could not be loaded".into()));
        let ended = j.poll();
        assert!(
            matches!(&ended, Some(WebEvent::Finished(Err(m))) if m.contains("couldn't run: the script Worker could not be loaded") && !m.contains("memory")),
            "{ended:?}"
        );
        assert!(*fake.terminated.borrow());
    }

    #[test]
    fn a_cancel_ends_the_run_once() {
        let fake = Fake::default();
        let mut j = job(&fake, options());
        j.cancel();
        fake.send(&FromHost::Output { line: "late".into() });
        assert_eq!(j.poll(), Some(WebEvent::Finished(Err(crate::isolate::CANCELLED.into()))));
        assert_eq!(j.poll(), None);
    }
}
