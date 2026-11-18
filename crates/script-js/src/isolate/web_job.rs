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

use super::protocol::{FromHost, MAX_FROM_HOST, MAX_LINE, MAX_MESSAGE, MAX_TO_HOST, ToHost, check_dialog_args, check_window, cut, parse_body};
use crate::commands::check_command;
use crate::tape::{MAX_ENTRIES, MAX_OUTPUT_BYTES, MAX_WINDOW_BYTES};
use crate::{Modal, ModalKind};

/// What a Worker sent, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    /// A message: a [`FromHost`] as JSON text.
    Message(String),
    /// The Worker failed (a trap: a failed allocation, a stack overflow, an engine panic), with
    /// the browser's description.
    Error(String),
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

/// One run in a Worker.
pub struct WebJob<T: Transport> {
    transport: T,
    options: WebOptions,
    queue: VecDeque<Incoming>,
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
            queue: VecDeque::new(),
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
        if let Some(t) = self.since.take() {
            self.spent += t.elapsed();
        }
    }

    /// End the run now with `result`: the Worker is stopped and nothing it sent later counts.
    fn end(&mut self, result: Result<(), String>) {
        if self.ending.is_none() && !self.finished {
            self.transport.terminate();
            self.queue.clear();
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
        if self.ending.is_none() {
            self.check_limits();
        }
        if self.ending.is_none() {
            let incoming = self.transport.take();
            self.queue.extend(incoming);
        }
        while self.ending.is_none() {
            let next = self.queue.pop_front()?;
            if let Some(event) = self.handle(next) {
                return Some(event);
            }
        }
        self.finished = true;
        self.ending.take().map(WebEvent::Finished)
    }

    fn check_limits(&mut self) {
        if self.elapsed() > self.options.time_limit {
            self.end(Err("The script was stopped: it ran longer than its time limit.".into()));
            return;
        }
        if let Some((since, dialog)) = self.waiting {
            let (what, limit) = if dialog { ("dialog", self.options.reply_limits.1) } else { ("command", self.options.reply_limits.0) };
            if since.elapsed() > limit {
                self.end(Err(format!("The script was stopped: its {what} got no answer within {} s.", limit.as_secs())));
            }
        }
    }

    fn handle(&mut self, incoming: Incoming) -> Option<WebEvent> {
        let text = match incoming {
            Incoming::Message(text) => text,
            Incoming::Error(why) => {
                self.end(Err(format!("The script stopped unexpectedly: {} (it may have run out of memory or overflowed its stack).", cut(&why, MAX_MESSAGE))));
                return None;
            }
        };
        let msg = match parse_body::<FromHost>(text.as_bytes(), MAX_FROM_HOST) {
            Ok(m) => m,
            Err(e) => {
                self.violation(&e.to_string());
                return None;
            }
        };
        if let FromHost::Finished { error } = msg {
            self.end(error.map_or(Ok(()), |m| Err(cut(&m, MAX_MESSAGE).to_owned())));
            return None;
        }
        if self.waiting.is_some() {
            self.violation("a message while it waited for an answer");
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
                self.pause();
                self.waiting = Some((Instant::now(), false));
                Some(WebEvent::Execute { id, params })
            }
            FromHost::Dialog { kind, args } => {
                if let Err(d) = check_dialog_args(kind, &args) {
                    self.violation(&d);
                    return None;
                }
                self.pause();
                self.waiting = Some((Instant::now(), true));
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
        if self.finished || self.ending.is_some() {
            return Err("the script has ended".into());
        }
        if self.waiting.map(|(_, d)| d) != Some(dialog) {
            return Err("the script is not waiting for that reply".into());
        }
        let text = serde_json::to_string(msg).map_err(|e| e.to_string())?;
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
    /// When the run waits for no command, or the answer could not be handed over (the run ends).
    pub fn reply_executed(&mut self, result: Result<Value, String>) -> Result<(), String> {
        self.reply(false, &ToHost::Executed { result })
    }

    /// Answer the dialog the run waits on.
    ///
    /// # Errors
    /// When the run waits for no dialog, or the answer could not be handed over (the run ends).
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
    }

    impl Fake {
        fn send(&self, msg: &FromHost) {
            self.inbox.borrow_mut().push_back(Incoming::Message(serde_json::to_string(msg).unwrap()));
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
            fake.inbox.borrow_mut().push_back(Incoming::Message(bad.clone()));
            let Some(WebEvent::Finished(Err(m))) = j.poll() else { panic!("{bad}") };
            assert!(m.contains("may not make"), "{m}");
            assert!(*fake.terminated.borrow());
        }
        // Asking again before the answer came is not allowed either.
        let fake = Fake::default();
        let mut j = job(&fake, options());
        fake.send(&FromHost::Execute { id: "session.inspect".into(), params: json!({}) });
        fake.send(&FromHost::Output { line: "early".into() });
        assert!(matches!(j.poll(), Some(WebEvent::Execute { .. })));
        assert!(matches!(j.poll(), Some(WebEvent::Finished(Err(m))) if m.contains("while it waited")));
    }

    #[test]
    fn the_time_limit_counts_computing_not_waiting_and_ends_the_run() {
        let fake = Fake::default();
        let mut o = options();
        o.time_limit = Duration::from_millis(100);
        let mut j = job(&fake, o);
        fake.send(&FromHost::Execute { id: "session.inspect".into(), params: json!({}) });
        assert!(matches!(j.poll(), Some(WebEvent::Execute { .. })));
        // The page takes its time answering: not the script's.
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(j.poll(), None);
        j.reply_executed(Ok(Value::Null)).unwrap();
        assert_eq!(j.poll(), None, "still within its limit");
        std::thread::sleep(Duration::from_millis(150));
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
