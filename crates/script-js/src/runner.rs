//! One script run, from its source to its result.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use boa_engine::builtins::error::Error as ErrorData;
use boa_engine::builtins::promise::{OperationType, Promise, PromiseState};
use boa_engine::context::HostHooks;
use boa_engine::object::builtins::JsPromise;
use boa_engine::realm::Realm;
use boa_engine::{Context, JsNativeError, JsObject, JsResult, JsString, JsValue, Source, js_string};
use serde_json::Value;

use crate::bounds::Bounds;
use crate::budget::Budget;
use crate::host::Host;
use crate::jobs::Jobs;
use crate::{APP_NAME, natives, preprocess, scan, value};

/// Loop iterations allowed per call frame before the run fails (Boa counts per frame on 0.22).
const LOOP_LIMIT: u64 = 100_000_000;
/// Call-stack frames allowed before the run fails.
const RECURSION_LIMIT: usize = 512;

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
    /// The script's arguments as JSON strings (`[message, ...]`), each converted the way
    /// `String(value)` converts it, as `$.writeln` does.
    pub args: Value,
}

/// Why the run was stopped.
pub(crate) enum Stop {
    TimeLimit,
    Fault(String),
}

pub(crate) struct State {
    pub bounds: Bounds,
    pub host: Box<dyn Host>,
    pub budget: Box<dyn Budget>,
    pub stop: Option<Stop>,
    pub app_name: String,
}

pub(crate) type Shared = Rc<RefCell<State>>;

/// A script run. Not `Send`: on native targets it lives on the script thread.
pub struct Runner {
    source: String,
    script_name: String,
    unbounded_loops: bool,
    unguarded_nesting: bool,
    shared: Shared,
}

impl Runner {
    /// # Errors
    /// A preprocessing error (an unsupported directive).
    pub fn new(source: &str, host: Box<dyn Host>, budget: Box<dyn Budget>, options: RunOptions) -> Result<Self, String> {
        let source = preprocess::preprocess_with(source, !options.unguarded_nesting)?;
        let script_name = options.script_name.unwrap_or_else(|| "script".to_owned());
        let state = State { bounds: Bounds::default(), host, budget, stop: None, app_name: options.app_name.unwrap_or_else(|| APP_NAME.to_owned()) };
        Ok(Self {
            source,
            script_name,
            unbounded_loops: options.unbounded_loops,
            unguarded_nesting: options.unguarded_nesting,
            shared: Rc::new(RefCell::new(state)),
        })
    }

    /// Run the script to its end: `Ok` when it finished, else the message for the user. A dialog
    /// waits for its answer inside the host call ([`Host::modal`]).
    ///
    /// # Errors
    /// The script failed or was stopped.
    pub fn run(self) -> Result<(), String> {
        self.shared.borrow_mut().budget.resume();
        // Last-resort guard: an engine panic fails the run instead of unwinding into the caller.
        // It can't catch a native stack overflow or an aborting allocation; the script host
        // (`crate::isolate`) contains those.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.execute()))
            .unwrap_or_else(|_| Err("The script stopped unexpectedly: internal error in the script engine.".into()));
        match self.shared.try_borrow_mut() {
            Ok(mut s) => s.budget.pause(),
            Err(_) => return Err("The script stopped unexpectedly: internal error in the script runner.".into()),
        }
        result
    }

    fn execute(&self) -> Result<(), String> {
        if !self.shared.borrow_mut().budget.charge() {
            return Err(time_limit_message());
        }
        let jobs = Rc::new(Jobs::default());
        let hooks = Rc::new(Hooks { guard: !self.unguarded_nesting, ..Hooks::default() });
        let result = self.context(&jobs, &hooks).and_then(|mut ctx| {
            ctx.eval(Source::from_bytes(&self.source).with_path(Path::new(&self.script_name)))?;
            ctx.run_jobs()
        });
        let mut s = self.shared.borrow_mut();
        match s.stop.take() {
            Some(Stop::TimeLimit) => Err(time_limit_message()),
            Some(Stop::Fault(m)) => Err(m),
            None if jobs.unsupported() => Err("dynamic import() and Atomics.waitAsync are not supported in scripts".into()),
            None => match result {
                Err(e) => Err(value::error_text(&e)),
                Ok(()) if let Some(reason) = hooks.unhandled() => Err(format!("Uncaught (in promise) {reason}")),
                Ok(()) => Ok(()),
            },
        }
    }

    fn context(&self, jobs: &Rc<Jobs>, hooks: &Rc<Hooks>) -> boa_engine::JsResult<Context> {
        let mut ctx = Context::builder().job_executor(jobs.clone()).host_hooks(hooks.clone()).build()?;
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
    use super::{RunOptions, Runner};
    use crate::ModalKind;
    use crate::budget::Unlimited;
    use crate::host::{Host, HostError};
    use serde_json::Value;

    /// Answers everything with `null`, keeping nothing.
    struct Sink;

    impl Host for Sink {
        fn execute(&mut self, _: &str, _: Value) -> Result<Value, HostError> {
            Ok(Value::Null)
        }

        fn write_line(&mut self, _: &str) {}

        fn modal(&mut self, _: ModalKind, _: &Value) -> Result<Value, HostError> {
            Ok(Value::Null)
        }
    }

    fn run(src: &str) -> Result<(), String> {
        Runner::new(src, Box::new(Sink), Box::new(Unlimited), RunOptions::default()).unwrap_or_else(|e| panic!("{e}")).run()
    }

    #[test]
    fn endless_calls_are_stopped_by_their_bounds() {
        let stopped = |src: &str, why: &str| assert!(run(src).is_err_and(|m| m.starts_with("The script was stopped") && m.contains(why)), "{src}");
        stopped("for (;;) $.writeln('');", "host calls");
        stopped("for (;;) alert(1);", "host calls");
        stopped("var s = 'x'.repeat(1 << 20); for (;;) $.writeln(s);", "MiB of output");
    }
}
