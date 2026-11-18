//! The app side of an isolated run: start a script host, relay its requests, enforce the
//! limits, and turn however it ends into one [`Event::Finished`].

use std::ffi::OsString;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::limits::{self, Containment, Exit, Meter, STATUS_ACCESS_VIOLATION, STATUS_STACK_BUFFER_OVERRUN, STATUS_STACK_OVERFLOW};
use super::protocol::{
    FrameError, FromHost, MAX_FROM_HOST, MAX_LINE, MAX_MESSAGE, MAX_TO_HOST, ToHost, check_command, check_dialog_args, check_window, cut, read_frame,
    write_frame,
};
use super::{SCRIPT_HOST_FLAG, host};
use crate::tape::{MAX_ENTRIES, MAX_OUTPUT_BYTES, MAX_WINDOW_BYTES};
use crate::{Modal, RunOptions};

/// Memory a host may use unless the options say otherwise: less on a 32-bit system, whose
/// address space is 2 GiB.
pub const DEFAULT_MEMORY_LIMIT: u64 = if cfg!(target_pointer_width = "64") { 2 << 30 } else { 1 << 30 };
/// The least memory limit: the script thread's stack plus room for the engine.
pub const MIN_MEMORY_LIMIT: u64 = host::STACK as u64 + (256 << 20);
// The default is a limit the host can reach, and on a 32-bit system one that fits its 2 GiB of
// address space with the app's own mappings beside it.
const _: () = assert!(DEFAULT_MEMORY_LIMIT >= MIN_MEMORY_LIMIT);
const _: () = assert!(cfg!(target_pointer_width = "64") || DEFAULT_MEMORY_LIMIT <= 3 << 29);
/// Events buffered between the host and the interface. When the interface falls behind, the
/// host blocks on its next message.
const QUEUE: usize = 64;
/// How often the watchdog looks at the host.
const TICK: Duration = Duration::from_millis(5);
/// Longest the app may take to answer one command: as long as it gives all of a run's commands
/// (`ui-egui`'s run budget), since one slow command (a large save) may take that. The run's clock
/// stops meanwhile, so without this a command handler that never answers would hold the host
/// forever.
pub const COMMAND_REPLY_LIMIT: Duration = Duration::from_secs(30 * 60);
/// How long at a time a full queue of events stops the run's clock ([`Shared::deliver`]).
pub const QUEUE_GRACE: Duration = Duration::from_secs(30);
/// Longest a dialog may stay open. A person may leave one open for a long time; one the
/// interface never shows or answers still ends.
pub const DIALOG_REPLY_LIMIT: Duration = Duration::from_secs(12 * 60 * 60);
/// How often it reads the host's resident size.
const METER_EVERY: Duration = Duration::from_millis(20);
/// Standard error kept for classifying a crash.
const STDERR_KEEP: usize = 16 << 10;

pub use super::CANCELLED;

/// How to start a script host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperCommand {
    pub program: PathBuf,
    /// Arguments; [`HelperCommand::new`] passes [`SCRIPT_HOST_FLAG`].
    pub args: Vec<OsString>,
    /// Environment variables set for the host, beside the app's own.
    pub envs: Vec<(OsString, OsString)>,
}

impl HelperCommand {
    /// `program` started with [`SCRIPT_HOST_FLAG`].
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self { program: program.into(), args: vec![SCRIPT_HOST_FLAG.into()], envs: Vec::new() }
    }

    /// This executable, which serves runs itself when started with [`SCRIPT_HOST_FLAG`].
    ///
    /// # Errors
    /// The path of this executable is unknown.
    pub fn current_exe() -> io::Result<Self> {
        std::env::current_exe().map(Self::new)
    }
}

/// Per-run settings.
#[derive(Debug, Clone)]
pub struct IsolateOptions {
    pub run: RunOptions,
    /// Time the run may spend executing, replays included; time at an open dialog is not counted.
    pub time_limit: Duration,
    /// Bytes the host may use (at least [`MIN_MEMORY_LIMIT`]).
    pub memory_limit: u64,
    /// History step key shared by the run's commands; unique per run.
    pub coalesce: String,
    pub helper: HelperCommand,
    /// Longest the app may take to answer one command, and one dialog
    /// ([`COMMAND_REPLY_LIMIT`], [`DIALOG_REPLY_LIMIT`]).
    pub reply_limits: (Duration, Duration),
    /// How long at a time a full queue of events stops the run's clock ([`QUEUE_GRACE`]).
    pub queue_grace: Duration,
}

/// What a run needs from the interface next. The same events as the in-process job.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Run this session command, then [`Reply::Executed`]. The params carry the run's
    /// `coalesce` key, so the run's edits share history steps.
    Execute { id: String, params: Value },
    /// A line the script wrote (`$.writeln`). Needs no reply.
    Output(String),
    /// Show this dialog, then [`Reply::Dialog`] with the answer (`null` for an alert or a
    /// cancelled prompt, a boolean for confirm, a string for prompt).
    Dialog(Modal),
    /// Show or update (`Some`, the window's controls) or hide (`None`) a ScriptUI window that
    /// does not wait for the user. Needs no reply.
    Window { id: u32, window: Option<Value> },
    /// The run ended. After this, close any dialog of the run that is still open.
    Finished(Result<(), Failure>),
}

/// The interface's answer to the last [`Event::Execute`] or [`Event::Dialog`].
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Executed(Result<Value, String>),
    Dialog(Value),
}

/// Why a run failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureKind {
    /// The script failed or threw (the engine's message).
    Script,
    /// It ran longer than its time limit and was stopped.
    Timeout,
    /// It used more than its memory limit and was stopped.
    MemoryLimit,
    /// The host process ended without a result.
    Crashed(Crash),
    /// The host sent something it may not send and was stopped.
    HostViolation,
    /// [`IsolatedJob::cancel`].
    Cancelled,
    /// The host could not be started.
    Unavailable,
}

/// How a host process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crash {
    /// The exit code; on Windows the full status, e.g. `0xC00000FD`.
    pub code: Option<u32>,
    /// The signal that ended it (Unix).
    pub signal: Option<i32>,
    pub cause: Option<CrashCause>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashCause {
    StackOverflow,
    AccessViolation,
    Aborted,
    /// Killed from outside the app (for example by the system when memory ran out).
    Killed,
}

/// A failed run: the kind, and the message for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    pub message: String,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Failure {}

impl Failure {
    fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }

    fn timeout() -> Self {
        Self::new(FailureKind::Timeout, "The script was stopped: it ran longer than its time limit.")
    }

    fn memory(limit: u64) -> Self {
        Self::new(FailureKind::MemoryLimit, format!("The script was stopped: it used more than its memory limit ({} MiB).", limit >> 20))
    }

    fn crashed(crash: Crash) -> Self {
        let what = match crash.cause {
            Some(CrashCause::StackOverflow) => "the script host ran out of stack space (stack overflow)",
            Some(CrashCause::AccessViolation) => "the script host made an invalid memory access (access violation)",
            Some(CrashCause::Aborted) => "the script host aborted",
            Some(CrashCause::Killed) => "the script host was killed from outside the app",
            None => "the script host ended without a result",
        };
        let how = match (crash.signal, crash.code) {
            (Some(s), _) => format!(", signal {s}"),
            (None, Some(c)) if c >= 0xC000_0000 => format!(", exit code 0x{c:08X}"),
            (None, Some(c)) => format!(", exit code {c}"),
            (None, None) => String::new(),
        };
        Self::new(FailureKind::Crashed(crash), format!("The script stopped unexpectedly: {what}{how}."))
    }
}

/// Why the app stopped the host.
#[derive(Debug, Clone)]
enum Verdict {
    Timeout,
    Memory,
    Cancelled,
    Violation(String),
    /// It closed its output but did not exit.
    Lingered,
    /// The script's start message was over [`MAX_TO_HOST`] bytes.
    TooLarge(usize),
    /// The app did not answer a command (`false`) or a dialog (`true`) in time.
    NoReply(bool),
    /// An answer could not be sent to the host.
    Unsent(String),
}

/// Time the run spent executing.
struct Clock {
    spent: Duration,
    since: Option<Instant>,
}

impl Clock {
    fn elapsed(&self) -> Duration {
        self.spent + self.since.map_or(Duration::ZERO, |t| t.elapsed())
    }

    fn pause(&mut self) {
        if let Some(t) = self.since.take() {
            self.spent += t.elapsed();
        }
    }

    fn resume(&mut self) {
        if self.since.is_none() {
            self.since = Some(Instant::now());
        }
    }
}

struct Watch {
    clock: Clock,
    /// Since when the host has waited for the app's answer, and whether to a dialog.
    waiting: Option<(Instant, bool)>,
    /// Since when the reader has been blocked handing an event to a full queue while the run's
    /// clock was stopped for it.
    queued: Option<Instant>,
    verdict: Option<Verdict>,
    exit: Option<Exit>,
    peak: u64,
    /// The host is gone or being stopped: its pending requests are void.
    dead: bool,
}

#[derive(Default)]
struct Tail {
    text: String,
    alloc_failed: bool,
    stack_overflow: bool,
    /// The host's standard error reached its end: every marker it wrote is in.
    done: bool,
}

/// How long to wait for the host's standard error to end after the host exited, before a crash
/// is classified. The pipe ends when the host's last writer closes it; a grandchild that keeps
/// it open may outlive the host, so the wait is bounded.
const STDERR_GRACE: Duration = Duration::from_millis(500);

struct Shared {
    watch: Mutex<Watch>,
    child: Mutex<Child>,
    stderr: Mutex<Tail>,
    /// Set from a forwarded request until the interface replies.
    awaiting: AtomicBool,
    /// The reader is blocked reading the host's output (not handing an event to the interface).
    reading: AtomicBool,
    /// [`Event::Finished`] was sent; it is sent once, by the reader or by the watchdog.
    reported: AtomicBool,
    /// Bytes of output lines relayed so far.
    output: std::sync::atomic::AtomicUsize,
    /// Bytes of window updates relayed so far.
    windows: std::sync::atomic::AtomicUsize,
    /// Output lines and window updates relayed so far.
    relayed: std::sync::atomic::AtomicUsize,
    time_limit: Duration,
    memory_limit: u64,
    /// See [`IsolateOptions::reply_limits`].
    reply_limits: (Duration, Duration),
    /// See [`IsolateOptions::queue_grace`].
    queue_grace: Duration,
    containment: Containment,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    /// Stop the host for `why` (the first reason wins; a host that already exited keeps its
    /// own exit status).
    fn kill(&self, why: Verdict) {
        {
            let mut w = lock(&self.watch);
            if w.verdict.is_none() && w.exit.is_none() {
                w.verdict = Some(why);
            }
            w.dead = true;
        }
        // Anything the host started too: one of them may hold its output open.
        self.containment.end(&mut lock(&self.child));
    }

    /// Send the run's [`Event::Finished`] unless it was sent already.
    fn finish(&self, events: &SyncSender<Event>, result: Result<(), Failure>) {
        if !self.reported.swap(true, Ordering::SeqCst) {
            let _ = events.send(Event::Finished(result));
        }
    }

    /// Count one relayed host call: a command, a dialog, an output line or a window update.
    /// Past the bound the host made more than a run may, and is stopped: an empty line costs no
    /// bytes and a dialog no output, so the byte bound alone would not hold them.
    fn relay_one(&self) -> bool {
        if self.relayed.fetch_add(1, Ordering::SeqCst) >= MAX_ENTRIES {
            self.kill(Verdict::Violation(format!("more than {MAX_ENTRIES} commands, dialogs, output lines and window updates")));
            return false;
        }
        true
    }

    /// The host now waits for the app's answer to a command or a `dialog`: the run's clock stops,
    /// and the wait's own clock starts.
    fn await_reply(&self, dialog: bool) {
        let mut w = lock(&self.watch);
        w.clock.pause();
        w.waiting = Some((Instant::now(), dialog));
    }

    /// Hand `event` to the interface. When its queue is full the interface is behind: the host
    /// soon blocks on its output too, and the wait is the app's, not the script's, up to
    /// [`QUEUE_GRACE`] at a time. Past that the time counts again: the host may keep computing
    /// until its output pipe fills, so a stalled interface must not stop the clock for good.
    fn deliver(&self, events: &SyncSender<Event>, event: Event) -> bool {
        match events.try_send(event) {
            Ok(()) => true,
            Err(mpsc::TrySendError::Disconnected(_)) => false,
            Err(mpsc::TrySendError::Full(event)) => {
                let running = {
                    let mut w = lock(&self.watch);
                    let running = w.waiting.is_none();
                    if running {
                        w.clock.pause();
                        w.queued = Some(Instant::now());
                    }
                    running
                };
                let sent = events.send(event).is_ok();
                if running {
                    let mut w = lock(&self.watch);
                    if let Some(blocked) = w.queued.take() {
                        w.clock.spent += blocked.elapsed().saturating_sub(self.queue_grace);
                    }
                    if w.waiting.is_none() {
                        w.clock.resume();
                    }
                }
                sent
            }
        }
    }

    /// How a run that the app stopped ended, if it did: a cancel, timeout or memory verdict
    /// outranks whatever result the host had sent before it was stopped.
    fn stopped(&self) -> Option<Failure> {
        // The guard is dropped at the end of the statement, before the waits below.
        lock(&self.watch).verdict.as_ref()?;
        self.wait_exit();
        self.wait_stderr();
        Some(self.classify())
    }

    /// Whether the host's requests are void: it is gone, being stopped, done, or exited just now.
    fn dead(&self) -> bool {
        if lock(&self.watch).dead {
            return true;
        }
        // The watchdog reaps on its next tick; a host that already exited is gone now.
        let status = self.containment.try_wait(&mut lock(&self.child));
        let exit = match status {
            Ok(None) => return false,
            Ok(Some(status)) => Exit::of(status),
            Err(_) => Exit { code: None, signal: None },
        };
        let mut w = lock(&self.watch);
        w.exit.get_or_insert(exit);
        w.dead = true;
        true
    }

    /// Wait until the watchdog reaped the host, killing it if it lingers.
    fn wait_exit(&self) {
        let start = Instant::now();
        let mut killed = false;
        while lock(&self.watch).exit.is_none() {
            if !killed && start.elapsed() > Duration::from_secs(2) {
                // The app's own kill: classified as such, not as a kill from outside.
                self.kill(Verdict::Lingered);
                killed = true;
            }
            if start.elapsed() > Duration::from_secs(10) {
                return;
            }
            std::thread::sleep(TICK);
        }
    }

    /// Wait, up to [`STDERR_GRACE`], for the host's standard error to end, so the markers it
    /// wrote while dying ("memory allocation of … failed", "has overflowed its stack") are read
    /// before the crash is classified. The host may exit, and be reaped, before the reader
    /// thread has drained the pipe.
    fn wait_stderr(&self) {
        let start = Instant::now();
        while !lock(&self.stderr).done && start.elapsed() < STDERR_GRACE {
            std::thread::sleep(TICK);
        }
    }

    /// How a host that sent no result ended. Call after [`Self::wait_exit`] and
    /// [`Self::wait_stderr`].
    fn classify(&self) -> Failure {
        let w = lock(&self.watch);
        match &w.verdict {
            Some(Verdict::Timeout) => return Failure::timeout(),
            Some(Verdict::Memory) => return Failure::memory(self.memory_limit),
            Some(Verdict::Cancelled) => return Failure::new(FailureKind::Cancelled, CANCELLED),
            Some(Verdict::Violation(d)) => {
                return Failure::new(FailureKind::HostViolation, format!("The script was stopped: the script host made a request it may not make ({d})."));
            }
            Some(Verdict::TooLarge(n)) => {
                return Failure::new(
                    FailureKind::Script,
                    format!("The script is too large to run: {} MiB, over the limit of {} MiB.", n.div_ceil(1 << 20), MAX_TO_HOST >> 20),
                );
            }
            Some(Verdict::NoReply(dialog)) => {
                let (what, limit) = if *dialog { ("dialog", self.reply_limits.1) } else { ("command", self.reply_limits.0) };
                return Failure::new(FailureKind::Timeout, format!("The script was stopped: its {what} got no answer within {} s.", limit.as_secs()));
            }
            Some(Verdict::Unsent(why)) => return Failure::new(FailureKind::Script, format!("The script was stopped: {why}.")),
            Some(Verdict::Lingered) => {
                return Failure::new(FailureKind::HostViolation, "The script was stopped: the script host kept running after it closed its output.");
            }
            None => {}
        }
        let tail = lock(&self.stderr);
        if tail.alloc_failed || w.peak >= self.memory_limit / 10 * 9 {
            return Failure::memory(self.memory_limit);
        }
        let exit = w.exit.unwrap_or(Exit { code: None, signal: None });
        Failure::crashed(Crash { code: exit.code, signal: exit.signal, cause: cause(exit, &tail) })
    }
}

fn cause(exit: Exit, tail: &Tail) -> Option<CrashCause> {
    // Unix signal numbers as Linux and macOS use them.
    const SIGABRT: i32 = 6;
    const SIGKILL: i32 = 9;
    const SIGSEGV: i32 = 11;
    if tail.stack_overflow || exit.code == Some(STATUS_STACK_OVERFLOW) {
        return Some(CrashCause::StackOverflow);
    }
    match (exit.signal, exit.code) {
        (Some(SIGSEGV), _) | (None, Some(STATUS_ACCESS_VIOLATION)) => Some(CrashCause::AccessViolation),
        (Some(SIGABRT), _) | (None, Some(STATUS_STACK_BUFFER_OVERRUN)) => Some(CrashCause::Aborted),
        (Some(SIGKILL), _) => Some(CrashCause::Killed),
        _ => None,
    }
}

/// A run in its own script host process. Dropping the job stops the host.
pub struct IsolatedJob {
    events: Receiver<Event>,
    to_host: Option<Sender<ToHost>>,
    shared: Arc<Shared>,
    cancel: Arc<AtomicBool>,
    waiting: Option<Waiting>,
    finished: bool,
    coalesce: String,
    pid: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Waiting {
    Execute,
    Dialog,
}

impl IsolatedJob {
    /// Start a host and the run in it. Any thread may start a run; the host lives until the run
    /// ends, the job is dropped, or the app exits.
    ///
    /// # Errors
    /// The host could not be started.
    pub fn spawn(source: String, options: IsolateOptions) -> Result<Self, Failure> {
        let unavailable = |e: String| Failure::new(FailureKind::Unavailable, format!("Couldn't start the script host: {e}"));
        let mut cmd = Command::new(&options.helper.program);
        cmd.args(&options.helper.args)
            .envs(options.helper.envs.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            /// `CREATE_NO_WINDOW`: no console window for the host.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Its own process group, so stopping it stops anything it started (see `limits`).
            cmd.process_group(0);
        }
        let mut child = cmd.spawn().map_err(|e| unavailable(e.to_string()))?;
        let containment = limits::contain(&child);
        let pid = child.id();
        let pipes = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let (Some(input), Some(output), Some(errors)) = pipes else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(unavailable("its pipes could not be opened".into()));
        };
        let memory_limit = options.memory_limit.max(MIN_MEMORY_LIMIT);
        let shared = Arc::new(Shared {
            watch: Mutex::new(Watch {
                clock: Clock { spent: Duration::ZERO, since: Some(Instant::now()) },
                waiting: None,
                queued: None,
                verdict: None,
                exit: None,
                peak: 0,
                dead: false,
            }),
            child: Mutex::new(child),
            stderr: Mutex::default(),
            awaiting: AtomicBool::new(false),
            reading: AtomicBool::new(false),
            reported: AtomicBool::new(false),
            output: std::sync::atomic::AtomicUsize::new(0),
            windows: std::sync::atomic::AtomicUsize::new(0),
            relayed: std::sync::atomic::AtomicUsize::new(0),
            time_limit: options.time_limit,
            memory_limit,
            reply_limits: options.reply_limits,
            queue_grace: options.queue_grace,
            containment,
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let (to_host, host_rx) = mpsc::channel();
        let (events_tx, events) = mpsc::sync_channel(QUEUE);
        let started = start_threads(&shared, &cancel, input, output, errors, host_rx, events_tx, options.coalesce.clone(), pid);
        if let Err(e) = started {
            shared.kill(Verdict::Cancelled);
            // No watchdog may be there to reap it: reap it here, so it leaves no zombie.
            let _ = lock(&shared.child).wait();
            return Err(unavailable(e.to_string()));
        }
        let start = ToHost::Start { source, app_name: options.run.app_name, script_name: options.run.script_name, memory_limit };
        let _ = to_host.send(start);
        Ok(Self { events, to_host: Some(to_host), shared, cancel, waiting: None, finished: false, coalesce: options.coalesce, pid })
    }

    /// The next event, waiting up to `timeout` for one. `None` while the run is busy, waits for
    /// a [`Self::reply`], or has finished. A run that ends while it waits for a reply still
    /// reports [`Event::Finished`]. A request still queued when the host's output ended, when it
    /// sent its result, when it exited or when the app stopped it is dropped here; one returned
    /// before that is a request of the running host and the interface may still carry it out.
    pub fn poll(&mut self, timeout: Duration) -> Option<Event> {
        if self.finished {
            return None;
        }
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(left) {
                Ok(Event::Finished(result)) => {
                    self.finished = true;
                    self.waiting = None;
                    self.to_host = None;
                    return Some(Event::Finished(result));
                }
                Ok(e @ (Event::Execute { .. } | Event::Dialog(_))) => {
                    if self.shared.dead() {
                        continue;
                    }
                    self.waiting = Some(if matches!(e, Event::Dialog(_)) { Waiting::Dialog } else { Waiting::Execute });
                    return Some(e);
                }
                Ok(e) => return Some(e),
                Err(RecvTimeoutError::Timeout) => return None,
                Err(RecvTimeoutError::Disconnected) => {
                    self.finished = true;
                    self.waiting = None;
                    self.to_host = None;
                    return Some(Event::Finished(Err(Failure::crashed(Crash { code: None, signal: None, cause: None }))));
                }
            }
        }
    }

    /// Answer the event the run waits on.
    ///
    /// # Errors
    /// When the run waits for no reply, or for a different kind.
    pub fn reply(&mut self, reply: Reply) -> Result<(), String> {
        let msg = match (self.waiting, reply) {
            (Some(Waiting::Execute), Reply::Executed(result)) => {
                let mut w = lock(&self.shared.watch);
                w.waiting = None;
                w.clock.resume();
                ToHost::Executed { result }
            }
            (Some(Waiting::Dialog), Reply::Dialog(answer)) => {
                let mut w = lock(&self.shared.watch);
                w.waiting = None;
                w.clock.resume();
                ToHost::Dialog { answer }
            }
            _ => return Err("the script is not waiting for that reply".into()),
        };
        self.waiting = None;
        self.shared.awaiting.store(false, Ordering::SeqCst);
        if let Some(tx) = &self.to_host {
            let _ = tx.send(msg);
        }
        Ok(())
    }

    /// Stop the run now (the host is killed). Its [`Event::Finished`] still arrives, with
    /// [`FailureKind::Cancelled`].
    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.waiting = None;
        self.shared.kill(Verdict::Cancelled);
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// The run's history step key (the one added to every command).
    pub fn coalesce(&self) -> &str {
        &self.coalesce
    }

    /// The host's process id.
    pub fn host_id(&self) -> u32 {
        self.pid
    }
}

impl Drop for IsolatedJob {
    fn drop(&mut self) {
        if !self.finished {
            self.cancel();
        }
        self.to_host = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn start_threads(
    shared: &Arc<Shared>,
    cancel: &Arc<AtomicBool>,
    input: ChildStdin,
    output: ChildStdout,
    errors: ChildStderr,
    host_rx: Receiver<ToHost>,
    events: SyncSender<Event>,
    coalesce: String,
    pid: u32,
) -> io::Result<()> {
    let s = shared.clone();
    let c = cancel.clone();
    let e = events.clone();
    std::thread::Builder::new().name("script-watchdog".into()).spawn(move || watchdog(&s, &c, pid, &e))?;
    let s = shared.clone();
    std::thread::Builder::new().name("script-stderr".into()).spawn(move || read_stderr(errors, &s))?;
    let s = shared.clone();
    std::thread::Builder::new().name("script-to-host".into()).spawn(move || write_host(input, &host_rx, &s))?;
    let s = shared.clone();
    let c = cancel.clone();
    std::thread::Builder::new().name("script-from-host".into()).spawn(move || read_host(output, &s, &c, &events, &coalesce))?;
    Ok(())
}

/// How long the reader may stay blocked on the host's output after the host exited before the
/// watchdog reports the run itself. Everything the host wrote can be read without waiting once it
/// exited; a reader still blocked after this waits on a pipe something else holds open.
const READER_GRACE: Duration = Duration::from_secs(1);

fn watchdog(shared: &Shared, cancel: &AtomicBool, pid: u32, events: &SyncSender<Event>) {
    let mut meter = Meter::new(pid);
    let mut metered: Option<Instant> = None;
    loop {
        let status = shared.containment.try_wait(&mut lock(&shared.child));
        let exit = match status {
            Ok(None) => None,
            Ok(Some(status)) => Some(Exit::of(status)),
            Err(_) => Some(Exit { code: None, signal: None }),
        };
        if let Some(exit) = exit {
            {
                let mut w = lock(&shared.watch);
                w.exit = Some(exit);
                w.dead = true;
            }
            report_if_reader_stalls(shared, events);
            return;
        }
        if cancel.load(Ordering::SeqCst) {
            shared.kill(Verdict::Cancelled);
        } else {
            let (elapsed, waiting) = {
                let w = lock(&shared.watch);
                // A block on a full queue counts past its grace while it lasts, not only once it
                // ends: an interface that stopped taking events must not hold the run forever.
                let queued = w.queued.map_or(Duration::ZERO, |t| t.elapsed().saturating_sub(shared.queue_grace));
                (w.clock.elapsed() + queued, w.waiting)
            };
            if elapsed > shared.time_limit {
                shared.kill(Verdict::Timeout);
            } else if let Some((since, dialog)) = waiting
                && since.elapsed() > if dialog { shared.reply_limits.1 } else { shared.reply_limits.0 }
            {
                shared.kill(Verdict::NoReply(dialog));
            }
        }
        if metered.is_none_or(|t| t.elapsed() >= METER_EVERY) {
            metered = Some(Instant::now());
            if let Some(resident) = meter.resident() {
                let over = {
                    let mut w = lock(&shared.watch);
                    w.peak = w.peak.max(resident);
                    resident > shared.memory_limit
                };
                if over {
                    shared.kill(Verdict::Memory);
                }
            }
        }
        std::thread::sleep(TICK);
    }
}

/// After the host exited: if the reader stays blocked reading its output for [`READER_GRACE`]
/// (a process the host started holds the pipe open), report the run from here. A reader that
/// is handing events to a slow interface is not stalled and is left to report.
fn report_if_reader_stalls(shared: &Shared, events: &SyncSender<Event>) {
    let mut stalled: Option<Instant> = None;
    while !shared.reported.load(Ordering::SeqCst) {
        if shared.reading.load(Ordering::SeqCst) {
            if stalled.get_or_insert_with(Instant::now).elapsed() >= READER_GRACE {
                // Stop whatever holds the pipe, so the reader thread ends too.
                shared.containment.end(&mut lock(&shared.child));
                let result = shared.stopped().unwrap_or_else(|| {
                    shared.wait_stderr();
                    shared.classify()
                });
                shared.finish(events, Err(result));
                return;
            }
        } else {
            stalled = None;
        }
        std::thread::sleep(TICK);
    }
}

fn read_stderr(mut errors: ChildStderr, shared: &Shared) {
    let mut buf = [0u8; 4096];
    // The end of the previous chunk, so a marker split across two reads is still found.
    let mut carry = String::new();
    loop {
        let n = match errors.read(&mut buf) {
            Ok(0) | Err(_) => {
                lock(&shared.stderr).done = true;
                return;
            }
            Ok(n) => n,
        };
        let chunk = String::from_utf8_lossy(buf.get(..n).unwrap_or_default());
        let window = format!("{carry}{chunk}");
        let mut tail = lock(&shared.stderr);
        // Rust's messages for a failed allocation and for a thread that overran its stack.
        tail.alloc_failed |= window.contains("memory allocation of ");
        tail.stack_overflow |= window.contains("has overflowed its stack");
        if tail.text.len() < STDERR_KEEP {
            let room = STDERR_KEEP - tail.text.len();
            let add = cut(&chunk, room).to_owned();
            tail.text.push_str(&add);
        }
        drop(tail);
        let keep = window.len().saturating_sub(64);
        let mut start = keep;
        while !window.is_char_boundary(start) {
            start += 1;
        }
        carry = window.get(start..).unwrap_or_default().to_owned();
    }
}

/// A size `msg` is known to exceed once encoded, from its text alone: a script or a text answer
/// longer than the frame limit. Such a message is refused before it is encoded, which for a few
/// hundred MiB takes long enough on a loaded machine to run into the run's time limit first.
fn known_too_large(msg: &ToHost) -> Option<usize> {
    let text = match msg {
        ToHost::Start { source, .. } => Some(source.len()),
        ToHost::Dialog { answer } => answer.as_str().map(str::len),
        ToHost::Executed { result: Ok(value) } => value.as_str().map(str::len),
        ToHost::Executed { result: Err(e) } => Some(e.len()),
    };
    text.filter(|n| *n > MAX_TO_HOST)
}

fn write_host(mut input: ChildStdin, rx: &Receiver<ToHost>, shared: &Shared) {
    for msg in rx {
        let first = match known_too_large(&msg) {
            Some(n) => Err(FrameError::TooLarge(n)),
            None => write_frame(&mut input, &msg, MAX_TO_HOST),
        };
        let written = match first {
            Err(FrameError::TooLarge(n)) if matches!(msg, ToHost::Start { .. }) => {
                // Nothing was sent: the host never sees the script, and the run fails by name.
                shared.kill(Verdict::TooLarge(n));
                return;
            }
            Err(FrameError::TooLarge(n)) if matches!(msg, ToHost::Dialog { .. }) => {
                // Nothing was sent: the host is still waiting, so the run ends here with the reason.
                shared.kill(Verdict::Unsent(format!(
                    "a dialog's answer could not be sent to the script host ({} MiB, over the limit of {} MiB)",
                    n.div_ceil(1 << 20),
                    MAX_TO_HOST >> 20
                )));
                return;
            }
            Err(FrameError::TooLarge(n)) if matches!(msg, ToHost::Executed { .. }) => {
                let result = Err(format!("the command's result is too large for a script ({} MiB)", n >> 20));
                write_frame(&mut input, &ToHost::Executed { result }, MAX_TO_HOST)
            }
            r => r,
        };
        if let Err(e) = written {
            // A host that is gone is classified by how it ended; one still running waits for an
            // answer it will never get, so the run ends here with the reason.
            let what = match msg {
                ToHost::Start { .. } => "the script",
                ToHost::Executed { .. } => "a command's result",
                ToHost::Dialog { .. } => "a dialog's answer",
            };
            // A host that just died closes its pipe a moment before it shows as exited: give it
            // that moment, so it is classified by how it ended rather than by the broken pipe.
            let start = Instant::now();
            let exited = loop {
                if matches!(shared.containment.try_wait(&mut lock(&shared.child)), Ok(Some(_))) || lock(&shared.watch).exit.is_some() {
                    break true;
                }
                if start.elapsed() >= Duration::from_millis(250) {
                    break false;
                }
                std::thread::sleep(TICK);
            };
            if !exited {
                shared.kill(Verdict::Unsent(format!("{what} could not be sent to the script host ({e})")));
            }
            return;
        }
    }
    // Dropping `input` closes the pipe: the host exits.
}

/// Relay the host's messages, checking each, then report how the run ended.
fn read_host(mut output: ChildStdout, shared: &Shared, cancel: &AtomicBool, events: &SyncSender<Event>, coalesce: &str) {
    let reported = relay(&mut output, shared, events, coalesce);
    // The host sent its result or its output ended: whatever it still had queued is void, even
    // before it has exited.
    {
        lock(&shared.watch).dead = true;
    }
    let result = match (shared.stopped(), reported) {
        // The app stopped the run: that is how it ended, even if the host's result was queued.
        (Some(failure), _) => Err(failure),
        (None, Some(Ok(()))) => Ok(()),
        (None, Some(Err(_))) if cancel.load(Ordering::SeqCst) => Err(Failure::new(FailureKind::Cancelled, CANCELLED)),
        (None, Some(Err(m))) => Err(Failure::new(FailureKind::Script, m)),
        (None, None) => {
            shared.wait_exit();
            shared.wait_stderr();
            Err(shared.classify())
        }
    };
    // The run is over either way: nothing of the host is needed any more.
    shared.containment.end(&mut lock(&shared.child));
    shared.finish(events, result);
}

/// `Some` with the host's own result, `None` when it ended without one (or was stopped).
fn relay(output: &mut ChildStdout, shared: &Shared, events: &SyncSender<Event>, coalesce: &str) -> Option<Result<(), String>> {
    loop {
        shared.reading.store(true, Ordering::SeqCst);
        let read = read_frame::<_, FromHost>(output, MAX_FROM_HOST);
        shared.reading.store(false, Ordering::SeqCst);
        let msg = match read {
            Ok(Some(m)) => m,
            // The host ended, or died in the middle of a message.
            Ok(None) | Err(FrameError::Truncated | FrameError::Io(_)) => return None,
            Err(e @ (FrameError::TooLarge(_) | FrameError::Malformed(_))) => {
                shared.kill(Verdict::Violation(e.to_string()));
                return None;
            }
        };
        if let FromHost::Finished { error } = msg {
            return Some(error.map_or(Ok(()), |m| Err(cut(&m, MAX_MESSAGE).to_owned())));
        }
        if shared.awaiting.load(Ordering::SeqCst) {
            shared.kill(Verdict::Violation("a message while it waited for an answer".into()));
            return None;
        }
        let event = match msg {
            FromHost::Execute { id, mut params } => {
                if let Err(d) = check_command(&id, &params) {
                    shared.kill(Verdict::Violation(d));
                    return None;
                }
                // The host's commands always join the run's own history step.
                if let Value::Object(map) = &mut params {
                    map.insert("coalesce".into(), Value::String(coalesce.to_owned()));
                }
                if !shared.relay_one() {
                    return None;
                }
                shared.awaiting.store(true, Ordering::SeqCst);
                // The time the app takes to answer (a busy frame, a minimized window that draws
                // no frames) is not the script's; the watchdog bounds it on its own.
                shared.await_reply(false);
                Event::Execute { id, params }
            }
            FromHost::Dialog { kind, args } => {
                if let Err(d) = check_dialog_args(kind, &args) {
                    shared.kill(Verdict::Violation(d));
                    return None;
                }
                if !shared.relay_one() {
                    return None;
                }
                shared.awaiting.store(true, Ordering::SeqCst);
                shared.await_reply(true);
                Event::Dialog(Modal { kind: kind.into(), args })
            }
            FromHost::Output { line } => {
                if !shared.relay_one() {
                    return None;
                }
                let line = cut(&line, MAX_LINE + 64).to_owned();
                // The interface keeps every line: hold the total to what a run may write, whatever
                // the host claims to have counted.
                let total = shared.output.fetch_add(line.len(), Ordering::SeqCst).saturating_add(line.len());
                if total > MAX_OUTPUT_BYTES {
                    shared.kill(Verdict::Violation(format!("more than {} MiB of output", MAX_OUTPUT_BYTES >> 20)));
                    return None;
                }
                Event::Output(line)
            }
            FromHost::Window { id, window } => {
                if !shared.relay_one() {
                    return None;
                }
                if let Some(Err(d)) = window.as_ref().map(check_window) {
                    shared.kill(Verdict::Violation(d));
                    return None;
                }
                // Bounded on their own: the interface handles every update, keeping the latest
                // controls of each window.
                let len = window.as_ref().map_or(0, |w| w.to_string().len());
                let total = shared.windows.fetch_add(len, Ordering::SeqCst).saturating_add(len);
                if total > MAX_WINDOW_BYTES {
                    shared.kill(Verdict::Violation(format!("more than {} MiB of window updates", MAX_WINDOW_BYTES >> 20)));
                    return None;
                }
                Event::Window { id, window }
            }
            FromHost::Finished { .. } => return None,
        };
        if !shared.deliver(events, event) {
            // The job was dropped.
            shared.kill(Verdict::Cancelled);
            return None;
        }
    }
}
