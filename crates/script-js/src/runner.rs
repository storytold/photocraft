//! One script run: attempts, pauses at modal dialogs, and the final result.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

use boa_engine::builtins::error::Error as ErrorData;
use boa_engine::builtins::promise::{OperationType, Promise, PromiseState};
use boa_engine::context::HostHooks;
use boa_engine::context::time::{Clock, JsInstant, StdClock};
use boa_engine::object::builtins::JsPromise;
use boa_engine::realm::Realm;
use boa_engine::{Context, JsNativeError, JsObject, JsResult, JsString, JsValue, Source, js_string};
use serde_json::Value;

use crate::budget::Budget;
use crate::host::Host;
use crate::jobs::Jobs;
use crate::tape::{Recorded, Tape};
use crate::{APP_NAME, natives, preprocess, scan, value};

/// Loop iterations allowed per call frame before the run fails (Boa counts per frame on 0.22).
const LOOP_LIMIT: u64 = 100_000_000;
/// Call-stack frames allowed before the run fails.
const RECURSION_LIMIT: usize = 512;
/// Golden-ratio fraction used to mix a clock seed into the `Math.random` state.
const RNG_MIX: u64 = 0x9E37_79B9_7F4A_7C15;
/// Most attempts one run may make: the first, plus one replay per answered dialog. Each replay
/// runs the script again from the start, so this bounds the work dialogs can multiply.
const MAX_ATTEMPTS: u32 = 1001;

/// Per-run settings.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// `app.name` for this run only (the first tag of a loaded script). `None` keeps the default.
    pub app_name: Option<String>,
    /// File name shown in script error messages. `None` uses `"script"`.
    pub script_name: Option<String>,
    /// Run without the engine's loop iteration, recursion and value-stack limits, for a host that bounds the
    /// run itself: its watchdog stops it at its time limit, its heap cap at its memory limit,
    /// and a native stack overflow ends its process with a named crash. Boa 0.22 drops an
    /// uncatchable error raised inside an `await` continuation (`vm/opcode/await/mod.rs`
    /// ignores the result of resuming the async function), so either limit hit after an `await`
    /// would end the run silently, as if it had finished. The script host ([`crate::isolate`])
    /// sets this. In-process runs keep both limits and that gap.
    pub unbounded_loops: bool,
    /// Parse without the bracket-nesting guard ([`crate::scan`]), for a host in a process of its
    /// own: a parser stack overflow there ends that process with a named crash and the app goes
    /// on, so the guard would only refuse deeply nested scripts (generated or minified ones) for
    /// nothing. The script host ([`crate::isolate`]) sets this; a run in the app's own process
    /// must not.
    pub unguarded_nesting: bool,
}

/// Kind of modal dialog the script opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    /// `alert(message, title, errorIcon)`.
    Alert,
    /// `confirm(message, noAsDefault, title)`.
    Confirm,
    /// `prompt(message, preset, title)`.
    Prompt,
    /// A ScriptUI dialog's `show()`: the arguments are `{"id", "window"}` with the window's
    /// controls, and the answer is the next user event (see [`crate::scriptui`]).
    Window,
    /// `File.openDialog(prompt, filter, multiSelect)`: `{"prompt", "filter", "multiple"}`;
    /// the answer is a path, a list of paths, or `null`.
    OpenFile,
    /// `File.saveDialog(prompt, filter)`: `{"prompt", "filter"}`; the answer is a path or `null`.
    SaveFile,
    /// `Folder.selectDialog(prompt)`: `{"prompt"}`; the answer is a path or `null`.
    SelectFolder,
}

/// A dialog the UI must answer before the run can continue.
#[derive(Debug, Clone, PartialEq)]
pub struct Modal {
    /// Which dialog was opened.
    pub kind: ModalKind,
    /// For `alert`, `confirm` and `prompt`, the script's arguments as JSON strings
    /// (`[message, ...]`), each converted the way `String(value)` converts it, as `$.writeln`
    /// does. For the other kinds, the object described on [`ModalKind`].
    pub args: Value,
}

/// Where a run stands after [`Runner::step`].
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// The script finished.
    Done,
    /// Waiting for [`Runner::answer`].
    Paused(Modal),
    /// The script failed or was stopped; the message is for the user.
    Failed(String),
}

/// Why an attempt was aborted.
pub(crate) enum Stop {
    Paused,
    TimeLimit,
    Fault(String),
}

pub(crate) struct State {
    pub tape: Tape,
    pub host: Box<dyn Host>,
    pub budget: Box<dyn Budget>,
    pub stop: Option<Stop>,
    pub app_name: String,
    pub attempts: u32,
    /// Where the run may write: what the user chose in its save and folder dialogs.
    pub grants: crate::grants::PathGrants,
    rng: u64,
}

pub(crate) type Shared = Rc<RefCell<State>>;

/// `Date` reads the wall clock from the tape, so a replay sees the times the first attempt saw.
struct TapeClock {
    shared: Shared,
    monotonic: StdClock,
    /// Set when the clock was read while the run state was busy; the attempt then faults
    /// instead of replaying with an untaped time.
    fault: Rc<Cell<bool>>,
}

impl Clock for TapeClock {
    fn now(&self) -> JsInstant {
        self.monotonic.now()
    }

    fn system_time_millis(&self) -> i64 {
        let live = || self.monotonic.system_time_millis();
        match self.shared.try_borrow_mut() {
            Ok(mut s) => {
                let v = s.tape.clock(live);
                // Past the reading bound, stop the attempt at its next checkpoint.
                if let Some(m) = s.tape.overflow().map(str::to_owned) {
                    s.stop.get_or_insert(Stop::Fault(m));
                }
                v
            }
            Err(_) => {
                self.fault.set(true);
                0
            }
        }
    }
}

impl State {
    /// A fresh `Math.random()` value (xorshift seeded from the clock; taped by the caller).
    pub fn next_random(&mut self) -> f64 {
        if self.rng == 0 {
            let seed = StdClock::new().system_time_millis().unsigned_abs();
            self.rng = seed.wrapping_mul(RNG_MIX) | 1;
        }
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A script run. Not `Send`: on native targets it lives on the script thread.
pub struct Runner {
    source: String,
    script_name: String,
    unbounded_loops: bool,
    unguarded_nesting: bool,
    shared: Shared,
    finished: Option<Step>,
}

impl Runner {
    /// # Errors
    /// A preprocessing error (an unsupported directive).
    pub fn new(source: &str, host: Box<dyn Host>, budget: Box<dyn Budget>, options: RunOptions) -> Result<Self, String> {
        let source = preprocess::preprocess_with(source, !options.unguarded_nesting)?;
        let script_name = options.script_name.unwrap_or_else(|| "script".to_owned());
        let state = State {
            tape: Tape::default(),
            host,
            budget,
            stop: None,
            app_name: options.app_name.unwrap_or_else(|| APP_NAME.to_owned()),
            attempts: 0,
            grants: crate::grants::PathGrants::default(),
            rng: 0,
        };
        Ok(Self {
            source,
            script_name,
            unbounded_loops: options.unbounded_loops,
            unguarded_nesting: options.unguarded_nesting,
            shared: Rc::new(RefCell::new(state)),
            finished: None,
        })
    }

    /// Attempts made so far (the first run plus one replay per answered dialog).
    pub fn attempts(&self) -> u32 {
        self.shared.borrow().attempts
    }

    /// Run until the script finishes, fails, or opens a dialog that has no answer yet.
    pub fn step(&mut self) -> Step {
        if let Some(done) = &self.finished {
            return done.clone();
        }
        if let Some(modal) = self.pending() {
            return Step::Paused(modal);
        }
        self.shared.borrow_mut().budget.resume();
        // Last-resort guard: an engine panic fails the run instead of unwinding into the caller.
        // It can't catch a native stack overflow or an aborting allocation; the script host
        // (`crate::isolate`) contains those.
        let mut step = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.attempt()))
            .unwrap_or_else(|_| Step::Failed("The script stopped unexpectedly: internal error in the script engine.".into()));
        // The budget must stop with the attempt, or time outside the run would count against it.
        match self.shared.try_borrow_mut() {
            Ok(mut s) => s.budget.pause(),
            Err(_) => step = Step::Failed("The script stopped unexpectedly: internal error in the script runner.".into()),
        }
        if !matches!(step, Step::Paused(_)) {
            self.finished = Some(step.clone());
        }
        step
    }

    /// Answer the dialog the run is paused at. `alert` takes `null`, `confirm` a boolean and
    /// `prompt` a string, or `null` for Cancel.
    ///
    /// # Errors
    /// When the run is not paused.
    pub fn answer(&mut self, value: Value) -> Result<(), String> {
        self.shared.borrow_mut().tape.answer(value)
    }

    fn pending(&self) -> Option<Modal> {
        let s = self.shared.borrow();
        let (_, e) = s.tape.pending_modal()?;
        match &e.call {
            Recorded::Modal { kind, args } => Some(Modal { kind: *kind, args: args.clone() }),
            _ => None,
        }
    }

    fn attempt(&mut self) -> Step {
        {
            let mut s = self.shared.borrow_mut();
            s.tape.rewind();
            s.stop = None;
            s.attempts += 1;
            if s.attempts > MAX_ATTEMPTS {
                return Step::Failed(format!("The script was stopped: it was resumed after a dialog more than {} times.", MAX_ATTEMPTS - 1));
            }
            if let Some(m) = s.tape.overflow() {
                return Step::Failed(m.to_owned());
            }
            if !s.budget.charge() {
                return Step::Failed(time_limit_message());
            }
        }
        let fault = Rc::new(Cell::new(false));
        let jobs = Rc::new(Jobs::default());
        let hooks = Rc::new(Hooks { guard: !self.unguarded_nesting, ..Hooks::default() });
        let result = self.fresh_context(&fault, &jobs, &hooks).and_then(|mut ctx| {
            ctx.eval(Source::from_bytes(&self.source).with_path(Path::new(&self.script_name)))?;
            ctx.run_jobs()
        });
        if fault.get() {
            return Step::Failed("internal error: the clock was read while the script host was busy".into());
        }
        let mut s = self.shared.borrow_mut();
        // A crossed tape bound outranks how the attempt ended: it may have gone on unrecorded.
        if let Some(m) = s.tape.overflow() {
            return Step::Failed(m.to_owned());
        }
        match s.stop.take() {
            Some(Stop::Paused) => {
                drop(s);
                self.pending().map_or_else(|| Step::Failed("internal error: paused without a dialog".into()), Step::Paused)
            }
            Some(Stop::TimeLimit) => Step::Failed(time_limit_message()),
            Some(Stop::Fault(m)) => Step::Failed(m),
            None if jobs.unsupported() => Step::Failed("dynamic import() and Atomics.waitAsync are not supported in scripts".into()),
            None => match result {
                Err(e) => Step::Failed(value::error_text(&e)),
                Ok(()) if let Some(reason) = hooks.unhandled() => Step::Failed(format!("Uncaught (in promise) {reason}")),
                Ok(()) if s.tape.unconsumed() > 0 => {
                    Step::Failed(format!("replay divergence: the script ended with {} recorded calls left", s.tape.unconsumed()))
                }
                Ok(()) => Step::Done,
            },
        }
    }

    fn fresh_context(&self, fault: &Rc<Cell<bool>>, jobs: &Rc<Jobs>, hooks: &Rc<Hooks>) -> boa_engine::JsResult<Context> {
        let clock = Rc::new(TapeClock { shared: self.shared.clone(), monotonic: StdClock::new(), fault: fault.clone() });
        let mut ctx = Context::builder().clock(clock).job_executor(jobs.clone()).host_hooks(hooks.clone()).build()?;
        ctx.runtime_limits_mut().set_loop_iteration_limit(if self.unbounded_loops { u64::MAX } else { LOOP_LIMIT });
        ctx.runtime_limits_mut().set_recursion_limit(if self.unbounded_loops { usize::MAX } else { RECURSION_LIMIT });
        if self.unbounded_loops {
            // The value stack's limit raises the same silent error; the heap cap bounds it instead.
            ctx.runtime_limits_mut().set_stack_size_limit(usize::MAX);
        }
        ctx.insert_data(self.shared.clone());
        natives::install(&mut ctx)?;
        Ok(ctx)
    }
}

fn time_limit_message() -> String {
    "The script was stopped: it ran longer than its time limit.".into()
}

/// Longest rejection reason a failed run reports, in characters, before it is cut.
const MAX_REASON_CHARS: usize = 2000;

/// The run's host hooks.
///
/// Promises rejected with no handler yet are kept in rejection order (HostPromiseRejectionTracker).
/// One still unhandled once the jobs have run fails the run instead of letting it end as done.
/// The same jobs run in the same order on every attempt, so the outcome replays identically.
///
/// Source compiled at run time (`eval`, `new Function`) gets the nesting check the script file
/// gets in [`Runner::new`], before Boa parses it, unless the run is unguarded.
#[derive(Default)]
struct Hooks {
    /// Rejected promises with no handler yet.
    rejections: RefCell<Vec<JsObject>>,
    /// Check the nesting of source compiled at run time.
    guard: bool,
}

impl HostHooks for Hooks {
    fn ensure_can_compile_strings(&self, _: Realm, parameters: &[JsString], body: &JsString, _: bool, _: &mut Context) -> JsResult<()> {
        if !self.guard {
            return Ok(());
        }
        // Boa parses the parameters joined with commas as one list, and the body on its own.
        let joined = parameters.iter().map(JsString::to_std_string_lossy).collect::<Vec<_>>().join(",");
        for source in [joined, body.to_std_string_lossy()] {
            if let Err(r) = scan::check_nesting(&source) {
                return Err(JsNativeError::syntax().with_message(format!("{} (line {})", r.why, r.line)).into());
            }
        }
        Ok(())
    }

    fn promise_rejection_tracker(&self, promise: &JsObject<Promise>, op: OperationType, _: &mut Context) {
        let p = promise.clone().upcast();
        let mut list = self.rejections.borrow_mut();
        match op {
            OperationType::Reject => list.push(p),
            OperationType::Handle => list.retain(|q| !JsObject::equals(q, &p)),
        }
    }
}

impl Hooks {
    /// The reason of the first rejection nobody handled, as text. Built from data only: no
    /// script code (getters, `toString`, proxy traps) runs after the body has finished.
    fn unhandled(&self) -> Option<String> {
        let first = self.rejections.borrow().first().cloned()?;
        let reason = match JsPromise::from_object(first).map(|p| p.state()) {
            Ok(PromiseState::Rejected(v)) => reason_text(&v),
            _ => String::new(),
        };
        Some(value::cut(&reason, MAX_REASON_CHARS))
    }
}

/// `"Name: message"` for an Error, the string itself, a primitive as displayed, or a fixed text
/// for any other object.
fn reason_text(v: &JsValue) -> String {
    if let Some(s) = v.as_string() {
        return s.slice(0, s.len().min(MAX_REASON_CHARS + 1)).to_std_string_lossy();
    }
    if let Some(sym) = v.as_symbol() {
        let d = sym.description().map(|d| d.slice(0, d.len().min(MAX_REASON_CHARS + 1)).to_std_string_lossy()).unwrap_or_default();
        return format!("Symbol({d})");
    }
    let Some(obj) = v.as_object() else { return v.display().to_string() };
    if !obj.is::<ErrorData>() {
        return "the promise was rejected with an object that is not an Error".to_owned();
    }
    let part =
        |key: &str| value::data_on_chain(&obj, &js_string!(key).into()).string().map(|s| s.slice(0, s.len().min(MAX_REASON_CHARS + 1)).to_std_string_lossy());
    let name = part("name").unwrap_or_else(|| "Error".to_owned());
    match part("message") {
        Some(m) if !m.is_empty() && !name.is_empty() => format!("{name}: {m}"),
        Some(m) if !m.is_empty() => m,
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::{RunOptions, Runner, Step};
    use crate::budget::Unlimited;
    use crate::host::{Host, HostError};
    use serde_json::Value;

    /// Counts output lines without keeping them.
    struct Sink;

    impl Host for Sink {
        fn execute(&mut self, _: &str, _: Value) -> Result<Value, HostError> {
            Ok(Value::Null)
        }

        fn write_line(&mut self, _: &str) {}
    }

    fn run_answering(src: &str) -> (Runner, Step) {
        let mut r = Runner::new(src, Box::new(Sink), Box::new(Unlimited), RunOptions::default()).unwrap_or_else(|e| panic!("{e}"));
        loop {
            match r.step() {
                Step::Paused(_) => r.answer(Value::Null).unwrap_or_else(|e| panic!("{e}")),
                done => return (r, done),
            }
        }
    }

    /// Size of what the tape holds, as its debug text: a fair measure of the memory it keeps.
    fn tape_size(r: &Runner) -> usize {
        format!("{:?}", r.shared.borrow().tape.entries()).len()
    }

    #[test]
    fn long_output_is_recorded_compactly() {
        // 30k lines of 1 KB, inside the run's output limit: the tape used to hold every line,
        // about 30 MB.
        let (r, step) = run_answering("var s = 'x'.repeat(1024); for (var i = 0; i < 30000; i++) $.writeln(s);");
        assert_eq!(step, Step::Done);
        assert!(tape_size(&r) < 4 << 20, "{} bytes", tape_size(&r));
    }

    #[test]
    fn fifty_dialogs_of_five_hundred_calls_stay_small() {
        let src = "var s = 'y'.repeat(1024); for (var d = 0; d < 50; d++) { for (var i = 0; i < 500; i++) $.writeln(s); alert(d); }";
        let (r, step) = run_answering(src);
        assert_eq!(step, Step::Done);
        assert_eq!(r.attempts(), 51);
        assert!(tape_size(&r) < 8 << 20, "{} bytes", tape_size(&r));
    }

    #[test]
    fn endless_calls_are_stopped_by_the_tape_bounds() {
        let (_, step) = run_answering("for (;;) $.writeln('');");
        assert_eq!(step, Step::Failed("The script was stopped: it made more than 524288 host calls (output lines, dialogs and commands).".into()));
        let (_, step) = run_answering("for (;;) Date.now();");
        assert_eq!(step, Step::Failed("The script was stopped: it read the clock or Math.random() more than 4194304 times.".into()));
        let (_, step) = run_answering("var s = 'x'.repeat(1 << 20); for (;;) prompt(s);");
        assert_eq!(step, Step::Failed("The script was stopped: its recorded host calls took more than 64 MiB.".into()));
    }

    #[test]
    fn a_run_resumed_too_often_is_stopped() {
        let mut r = Runner::new("for (;;) alert(1);", Box::new(Sink), Box::new(Unlimited), RunOptions::default()).unwrap_or_else(|e| panic!("{e}"));
        for _ in 0..1001 {
            assert!(matches!(r.step(), Step::Paused(_)));
            r.answer(Value::Null).unwrap_or_else(|e| panic!("{e}"));
        }
        assert_eq!(r.step(), Step::Failed("The script was stopped: it was resumed after a dialog more than 1000 times.".into()));
        assert_eq!(r.attempts(), 1002);
    }
}
