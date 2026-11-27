//! Runs in a web Worker, for the web build. Each run gets its own Worker, so the page stays
//! responsive while a script computes.
//!
//! - **The Worker** runs the engine ([`serve_worker`]). It sends its requests to the page as
//!   messages, then waits for the page's answer with `Atomics.wait` on a shared mailbox, so a
//!   run continues where it left off after a command or a dialog, as on the desktop.
//! - **The page** reaches the Worker through [`WorkerTransport`], a [`Transport`] for
//!   [`WebJob`](super::web_job::WebJob). The page checks every request there.
//!
//! The mailbox is a `SharedArrayBuffer`, which a browser gives only to a cross-origin-isolated
//! page. Its layout:
//! - two `Int32` slots, the state ([`EMPTY`] or [`FULL`]) and the answer's length in bytes;
//! - then the answer's bytes, JSON text of a [`ToHost`].

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use js_sys::{Atomics, Int32Array, Object, Reflect, SharedArrayBuffer, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{DedicatedWorkerGlobalScope, ErrorEvent, Event, MessageEvent, Worker};

use super::HostAllocator;
use super::link::{self, Link};
use super::protocol::{FromHost, MAX_FROM_HOST, ToHost, parse_body};
use super::web_job::{Incoming, Transport};

/// Bytes before the answer: the state and the length.
const HEADER: u32 = 8;
/// Largest answer the mailbox holds: a command's result or a dialog's answer. Larger answers
/// end the run by name.
pub const MAILBOX_BYTES: u32 = 32 << 20;
/// The mailbox holds no answer: the Worker waits, or computes.
const EMPTY: i32 = 0;
/// The page left an answer for the Worker.
const FULL: i32 = 1;
/// What the Worker sends once it is listening, before the run starts, followed by a space and
/// when it sent it ([`shared_clock_ms`], which the page and the Worker read alike). Messages
/// that arrive before then would be lost.
const READY: &str = "ready";

/// Longest a Worker may take to load before it says it listens. A Worker whose engine can't load
/// (a missing or blocked file) reports nothing to the page, so without this the run would sit
/// until its time limit.
const START_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

/// When a Worker was started: by the page's clock, and by [`shared_clock_ms`], the clock the
/// Worker stamps its `READY` with. `overdue`: a [`WorkerTransport::take`] has seen the limit pass.
#[derive(Clone, Copy)]
struct Started {
    at: web_time::Instant,
    clock_ms: f64,
    overdue: bool,
}

/// Milliseconds on a clock the page and its Workers share: `performance.timeOrigin +
/// performance.now()`. Each context's `now()` is monotonic from its own `timeOrigin`, so unlike
/// `Date.now()` a change of the system's calendar time does not move it. `Date.now()` only where a
/// browser has no `performance`.
fn shared_clock_ms() -> f64 {
    let read = || -> Option<f64> {
        let performance = Reflect::get(&js_sys::global(), &JsValue::from_str("performance")).ok()?;
        let origin = Reflect::get(&performance, &JsValue::from_str("timeOrigin")).ok()?.as_f64()?;
        let now: js_sys::Function = Reflect::get(&performance, &JsValue::from_str("now")).ok()?.dyn_into().ok()?;
        let now = now.call0(&performance).ok()?.as_f64()?;
        Some(origin + now).filter(|ms| ms.is_finite())
    };
    read().unwrap_or_else(js_sys::Date::now)
}

/// When a `READY` message says the Worker sent it, in [`shared_clock_ms`] milliseconds.
fn ready_sent_at(text: &str) -> Option<f64> {
    text.strip_prefix(READY)?.strip_prefix(' ')?.parse::<f64>().ok().filter(|ms| ms.is_finite())
}

/// Why scripts can't run in this page.
pub const NOT_ISOLATED: &str = "Scripts need a page served with cross-origin isolation (the COOP and COEP headers); \
     this one is not, so its browser offers no shared memory for the script Worker.";

/// Whether this page (or Worker) may use shared memory.
#[must_use]
pub fn cross_origin_isolated() -> bool {
    Reflect::get(&js_sys::global(), &JsValue::from_str("crossOriginIsolated")).ok().and_then(|v| v.as_bool()).unwrap_or(false)
}

// ---------- the Worker's side ----------

/// The Worker's channel to the page: messages out, the mailbox in.
struct WorkerLink {
    scope: DedicatedWorkerGlobalScope,
    state: Int32Array,
    body: Uint8Array,
}

fn post(scope: &DedicatedWorkerGlobalScope, msg: &FromHost) -> Result<(), String> {
    let text = serde_json::to_string(msg).map_err(|e| e.to_string())?;
    if text.len() > MAX_FROM_HOST {
        return Err(format!("larger than {} MiB", MAX_FROM_HOST >> 20));
    }
    scope.post_message(&JsValue::from_str(&text)).map_err(|_| "the page is gone".to_owned())
}

impl Link for WorkerLink {
    fn send(&mut self, msg: &FromHost) -> Result<(), String> {
        post(&self.scope, msg)
    }

    fn recv(&mut self) -> Result<ToHost, String> {
        // Sleep until the page fills the mailbox; a wake-up with the mailbox still empty
        // sleeps again.
        while Atomics::load(&self.state, 0).map_err(|_| "the mailbox can't be read".to_owned())? != FULL {
            Atomics::wait(&self.state, 0, EMPTY).map_err(|_| "this browser can't wait for the page's answer".to_owned())?;
        }
        let len = Atomics::load(&self.state, 1).map_err(|_| "the mailbox can't be read".to_owned())?;
        let len = u32::try_from(len).ok().filter(|n| *n <= self.body.length()).ok_or("the page left a malformed answer")?;
        let bytes = self.body.subarray(0, len).to_vec();
        Atomics::store(&self.state, 0, EMPTY).map_err(|_| "the mailbox can't be written".to_owned())?;
        parse_body(&bytes, usize::try_from(MAILBOX_BYTES).unwrap_or(usize::MAX)).map_err(|e| e.to_string())
    }
}

/// The script Worker's entry point. Call it from the Worker executable's `main`, with the
/// allocator that executable installs. It waits for the page's start message, then runs that
/// one script.
pub fn serve_worker(heap: &'static HostAllocator) {
    let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    let listener = scope.clone();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        // One run per Worker: later messages are not starts.
        listener.set_onmessage(None);
        let result = run(&listener, heap, &event.data());
        let _ = post(&listener, &FromHost::Finished { error: result.err() });
    });
    scope.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    // The Worker's lifetime; it ends when the page terminates the Worker.
    on_message.forget();
    let _ = scope.post_message(&JsValue::from_str(&format!("{READY} {}", shared_clock_ms())));
}

fn run(scope: &DedicatedWorkerGlobalScope, heap: &'static HostAllocator, data: &JsValue) -> Result<(), String> {
    let field = |name: &str| Reflect::get(data, &JsValue::from_str(name)).ok();
    let start = field("start").and_then(|v| v.as_string()).ok_or("internal error: the page sent no start message")?;
    let mailbox: SharedArrayBuffer = field("mailbox").and_then(|v| v.dyn_into().ok()).ok_or("internal error: the page sent no mailbox")?;
    let Ok(ToHost::Start { source, app_name, script_name, memory_limit }) = serde_json::from_str(&start) else {
        return Err("internal error: the page sent a malformed start message".into());
    };
    super::heap::cap_heap(heap, memory_limit)?;
    let state = Int32Array::new_with_byte_offset_and_length(&mailbox, 0, 2);
    let body = Uint8Array::new_with_byte_offset(&mailbox, HEADER);
    let link = WorkerLink { scope: scope.clone(), state, body };
    link::run(Rc::new(RefCell::new(link)), &source, app_name, script_name)
}

// ---------- the page's side ----------

/// One run's Worker, as [`WebJob`](super::web_job::WebJob) drives it from the page.
pub struct WorkerTransport {
    worker: Worker,
    inbox: Rc<RefCell<Vec<Incoming>>>,
    /// The start message, until the Worker says it listens.
    pending: Rc<RefCell<Option<Object>>>,
    mailbox: SharedArrayBuffer,
    state: Int32Array,
    body: Uint8Array,
    /// When the Worker was started, until it says it listens ([`START_LIMIT`]). Shared with the
    /// message handler, which doesn't send the start message once the limit has passed.
    starting: Rc<Cell<Option<Started>>>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_error: Closure<dyn FnMut(Event)>,
}

impl WorkerTransport {
    /// Start a Worker from `url`: the Worker executable's loader script.
    ///
    /// # Errors
    /// The page is not cross-origin isolated ([`NOT_ISOLATED`]), or the browser refused the
    /// Worker.
    pub fn new(url: &str) -> Result<Self, String> {
        if !cross_origin_isolated() {
            return Err(NOT_ISOLATED.into());
        }
        // Allocated first: a browser can refuse 32 MiB of shared memory, and nothing is started yet
        // to clean up then.
        let mailbox: SharedArrayBuffer =
            Reflect::construct(&SharedArrayBuffer::new(0).constructor(), &js_sys::Array::of1(&JsValue::from(HEADER + MAILBOX_BYTES)))
                .map_err(|e| format!("Couldn't reserve the script mailbox: {}", describe(&e)))?
                .unchecked_into();
        let worker = Worker::new(url).map_err(|e| format!("Couldn't start the script Worker: {}", describe(&e)))?;
        let inbox: Rc<RefCell<Vec<Incoming>>> = Rc::default();
        let pending: Rc<RefCell<Option<Object>>> = Rc::default();
        let starting: Rc<Cell<Option<Started>>> = Rc::default();
        let on_message = {
            let (inbox, pending, worker, starting) = (inbox.clone(), pending.clone(), worker.clone(), starting.clone());
            Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                let text = event.data().as_string();
                if let Some(sent) = text.as_deref().and_then(ready_sent_at) {
                    // Too late by when the Worker sent it, not by when the page got to the
                    // message (a busy page handles messages late): left pending, so `take`
                    // reports the start limit.
                    if starting.get().is_some_and(|s| sent - s.clock_ms > START_LIMIT.as_secs_f64() * 1000.0) {
                        return;
                    }
                    if let Some(start) = pending.borrow_mut().take()
                        && let Err(e) = worker.post_message(&start)
                    {
                        inbox.borrow_mut().push(Incoming::Unavailable(format!("the script Worker could not be started: {}", describe(&e))));
                    }
                    return;
                }
                let incoming = match text {
                    Some(text) => Incoming::Message(text, web_time::Instant::now()),
                    None => Incoming::Error("the script Worker sent something that is not text".into()),
                };
                inbox.borrow_mut().push(incoming);
            })
        };
        let on_error = {
            let inbox = inbox.clone();
            // A script that fails to load fires a plain `Event`, an error in it an `ErrorEvent`.
            Closure::<dyn FnMut(Event)>::new(move |event: Event| {
                // Reported to the user with the run; not again in the console as uncaught.
                event.prevent_default();
                // An `ErrorEvent` is an error in the running Worker; a plain `Event`, one that never
                // loaded.
                let incoming = match event.dyn_ref::<ErrorEvent>().map(ErrorEvent::message).filter(|m| !m.is_empty()) {
                    Some(why) => Incoming::Error(why),
                    None => Incoming::Unavailable("the script Worker could not be loaded".into()),
                };
                inbox.borrow_mut().push(incoming);
            })
        };
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        let state = Int32Array::new_with_byte_offset_and_length(&mailbox, 0, 2);
        let body = Uint8Array::new_with_byte_offset(&mailbox, HEADER);
        Ok(Self { worker, inbox, pending, mailbox, state, body, starting, _on_message: on_message, _on_error: on_error })
    }
}

fn describe(e: &JsValue) -> String {
    e.as_string()
        .or_else(|| Reflect::get(e, &JsValue::from_str("message")).ok().and_then(|m| m.as_string()))
        .unwrap_or_else(|| "the browser gave no reason".into())
}

impl Transport for WorkerTransport {
    fn start(&mut self, start: &str) -> Result<(), String> {
        let msg = Object::new();
        Reflect::set(&msg, &JsValue::from_str("start"), &JsValue::from_str(start)).map_err(|e| describe(&e))?;
        Reflect::set(&msg, &JsValue::from_str("mailbox"), &self.mailbox).map_err(|e| describe(&e))?;
        // Sent when the Worker says it listens (see `READY`).
        *self.pending.borrow_mut() = Some(msg);
        self.starting.set(Some(Started { at: web_time::Instant::now(), clock_ms: shared_clock_ms(), overdue: false }));
        Ok(())
    }

    fn take(&mut self) -> Vec<Incoming> {
        let mut taken = std::mem::take(&mut *self.inbox.borrow_mut());
        if self.pending.borrow().is_none() {
            // It said it listens: the start message went out.
            self.starting.set(None);
        } else if let Some(started) = self.starting.get().filter(|s| s.at.elapsed() > START_LIMIT) {
            if started.overdue {
                self.starting.set(None);
                self.pending.borrow_mut().take();
                taken.push(Incoming::Unavailable(format!("the script Worker didn't start within {} s", START_LIMIT.as_secs())));
            } else {
                // Past the limit by the page's clock, but a `READY` the Worker sent in time may
                // still wait in the page's message queue (a page busy past the limit): it is
                // handled before the next take, which decides.
                self.starting.set(Some(Started { overdue: true, ..started }));
            }
        }
        taken
    }

    fn answer(&mut self, answer: &str) -> Result<(), String> {
        let len = u32::try_from(answer.len()).ok().filter(|n| *n <= MAILBOX_BYTES).ok_or_else(|| format!("larger than {} MiB", MAILBOX_BYTES >> 20))?;
        if Atomics::load(&self.state, 0).map_err(|e| describe(&e))? != EMPTY {
            return Err("internal error: the Worker has not taken the last answer".into());
        }
        self.body.subarray(0, len).copy_from(answer.as_bytes());
        let len = i32::try_from(len).map_err(|_| "too large")?;
        Atomics::store(&self.state, 1, len).map_err(|e| describe(&e))?;
        Atomics::store(&self.state, 0, FULL).map_err(|e| describe(&e))?;
        Atomics::notify(&self.state, 0).map_err(|e| describe(&e))?;
        Ok(())
    }

    fn answer_limit(&self) -> usize {
        MAILBOX_BYTES as usize
    }

    fn terminate(&mut self) {
        self.worker.set_onmessage(None);
        self.worker.set_onerror(None);
        self.worker.terminate();
        self.pending.borrow_mut().take();
    }
}

impl Drop for WorkerTransport {
    fn drop(&mut self) {
        self.terminate();
    }
}
