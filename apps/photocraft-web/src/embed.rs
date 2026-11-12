//! The embedding API (#1614): `window.photocraft` for the page the app runs in, the same document
//! calls over `postMessage` for a page that embeds it in a cross-origin iframe (`?embed=<origin>`).
//! The Photopea-compatible mode is refused in this build (`PhotopeaConfig::DISABLED`).
//!
//! ```js
//! photocraft.info()                       // {app, format, doc: {id, name} | null}
//! photocraft.isDirty()                    // unsaved changes in an open document?
//! await photocraft.open(bytes, "a.psd")   // ArrayBuffer, typed array or Blob, ≤ 64 MiB →
//!                                        //   {warnings, doc: {id, name}} once the document is in the session
//! await photocraft.open("/api/asset/7")   // or a URL string on the ?embed= origin: the app
//!                                        //   fetches it itself, with the page's session cookies
//! await photocraft.save("psd")            // the active document as a Uint8Array (default "pcraft"),
//!                                        //   its export warnings on it as `.warnings`
//! await photocraft.request("ui.inspect")  // a desktop control-channel method → its result
//! window.addEventListener("photocraft-dirty", e => e.detail)   // true / false as it changes
//! ```
//!
//! An open settles only once a document of the very name it asked for has entered the session
//! — new since its import started, matched by exact name and nothing else — reported back as
//! `doc` and by `info()`, so a save asked after it can't take an older document. One open is in
//! flight at a time, and its window is taken as the open's first step — before the string is
//! parsed as a URL or a byte of the data is read: a second while the first is unsettled (its
//! bytes being fetched or read, its import running, or waiting for its document behind a
//! dialog in the app) rejects `protocol::OPEN_BUSY` at once, and that is what keeps two opens
//! from ever answering with each other's; there is no pool to draw a fallback from and no
//! claim a cancelled open could make on the next one's document. An import still decoding, or
//! held behind a dialog in the app, keeps the promise waiting, up to
//! `protocol::OPEN_TIMEOUT_MS` (30 s: one at a time makes the deadline also the lock on every
//! later open). At that deadline the underlying import is stopped too — through the app's own
//! Cancel, `open_prompt::answer`, the same thing the Cancel button does — so a document of a
//! timed-out open can never land later, into a following same-name open's claim; the one
//! prompt whose answer isn't public (the damaged-file confirmation) can't be cancelled that
//! way, and its open holds the busy window as an observer until the app is done with the
//! file: the window only ever opens once nothing can still land in it. Before the deadline, a
//! dialog cancelled in the app ends the wait at once — the pass reads the app's open prompts,
//! and once it held this file and holds nothing now (no prompt, nothing queued, no job of the
//! name still opening), with no document of the name arrived, the file was disposed of. A
//! rejected request claims nothing, and whatever document shows up later stays in the session
//! untouched. A document of another name is never taken — a `.psdt`, which the app opens as
//! "Untitled-N", rejects at its deadline, its document staying in the session. A failed import
//! rejects at once, as an argument that can't be read as bytes rejects with
//! `protocol::OPEN_READ`, and a real byte shape over `protocol::MAX_OPEN_BYTES` with
//! `protocol::OPEN_TOO_BIG` — gated on its true length before any view of it is made.
//!
//! A *string* argument is the URL form: resolved against the app's own location and let
//! through only as an `http(s)` URL on the `?embed=` origin (`protocol::OPEN_URL` otherwise —
//! decided under the one-open window, before anything is queued or fetched), the app fetches
//! it itself, same-origin, with the page's session cookies; **no redirects at all** — any 3xx
//! refuses the fetch (`redirect: "error"`, and the answer's `redirected` re-checked as the
//! belt) — and anything but a 2xx answer rejects `protocol::OPEN_FETCH`. The body a 2xx
//! answers with is read as a *stream*, counted against `protocol::MAX_OPEN_BYTES` as it
//! arrives (a declared `Content-Length` over it refused before a byte is read): the same
//! budget, the same `protocol::OPEN_TOO_BIG`, that a handed-over buffer answers to — enforced
//! while the bytes arrive, so an unbounded or lying-length body is never buffered whole. The
//! one-open window is taken before the URL is even parsed, so `protocol::OPEN_BUSY` covers
//! the fetching phase too, and the fetch has no timer of its own: it lives inside the same
//! 30 s deadline every open gets — a fetch still running at the deadline is aborted with the
//! promise. Without a `name`, the open takes the URL's last path segment
//! (`protocol::file_name`), "document" when it has none.
//!
//! A save
//! marks the document clean only after its bytes have reached the caller — the promise resolved
//! here, or the reply's transfer finished over the bridge — so an export, a reply or a transfer
//! that fails leaves the document dirty.
//!
//! Over `postMessage` (only from the `?embed=` origin, replies only to it): send
//! `{photocraft: id, method: "open" | "save" | "isDirty" | "info", …}` (`open`: `data` an
//! ArrayBuffer — the bytes ride the message's structured clone and are never read through the
//! JSON face — or a same-origin URL string the app fetches itself, and `name`; `save`:
//! `format`), get `{photocraft: id, ok, result | error}` back
//! (`save`'s result is an ArrayBuffer, transferred, with the export `warnings` beside it). A
//! message is weighed on the clone before anything decodes it — over `protocol::MAX_CALL_BYTES`
//! outside its `data`, nested deeper than `protocol::MAX_CALL_DEPTH`, or carrying binary data
//! (a typed array, a DataView, an ArrayBuffer) anywhere outside `data`, it is answered
//! `protocol::CALL_TOO_BIG` / `protocol::CALL_BINARY` and the JSON face never starts on it. At
//! most `protocol::MAX_PENDING_CALLS` calls may wait for their reply at once; the next is
//! answered `protocol::BUSY`. The app posts `{photocraft: "ready", info}` once it is up and
//! `{photocraft: "dirty", dirty}` when that changes. `request` stays with the app's own page:
//! the embedding page names itself in `?embed=`, so that origin check keeps strangers from
//! reading or replying to the app's messages but can't tell a trusted embedder from any other
//! site (`frame-ancestors` on the app's host does that).
//!
//! A save the page takes counts as a save: a PSD, PSB or `.pcraft` leaves the document clean, as
//! File › Save does.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};

use photocraft_ui_egui::{ControlRequest, ControlResponse, ExportSettings, PhotocraftApp};
use serde_json::{Value, json};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::embed_protocol::{self as protocol, Call, PhotopeaConfig};

type Settle = (js_sys::Function, js_sys::Function);

/// An open's byte source, shared by its queue entry and its byte-acquisition task (a URL
/// fetch or a read of handed-over bytes): the task writes its one answer here — bytes that
/// already passed every byte gate, or the fixed refusal — and the frame pass takes it. While a
/// URL fetch is in flight the cell also carries its abort handle, created before the task
/// starts, so the entry's deadline can always reach the transfer and kill it. An answer that
/// arrives after the entry's deadline settled it is read by no one (the entry is gone) and is
/// dropped with this.
#[derive(Default)]
struct SourceFetch {
    answer: Option<Result<Vec<u8>, String>>,
    /// The URL fetch's abort handle; a byte read has none.
    abort: Option<web_sys::AbortController>,
}

/// The one open waiting for its document: the import runs on a frame of the app's loop, and
/// the promise settles only once a document of its exact name is in the session (or the wait
/// gives up). One at a time — a second, taken while this one is unsettled, is refused at the
/// queue with `protocol::OPEN_BUSY`.
struct PendingOpen {
    name: String,
    bytes: Vec<u8>,
    /// The open's byte source while its bytes are still on their way ([`SourceFetch`]): a URL
    /// open's fetch, or a byte open's read of its data. `None` once the bytes have landed —
    /// from there the entry is exactly a bytes-in-hand open's. While it is `Some`, nothing of
    /// the entry runs: only its deadline can end it (the fetch's timeout is the open's own,
    /// no second timer).
    source: Option<Rc<RefCell<SourceFetch>>>,
    /// The document ids in the session when this import started: only a document new since
    /// then can answer this request — by exact name ([`protocol::landed_doc`]), never an
    /// older one that happens to carry the same name.
    before: Vec<u64>,
    /// The import warnings, once it ran.
    warnings: Vec<String>,
    /// The import error, when it failed.
    failed: Option<String>,
    /// When the wait gives up, in milliseconds since the epoch.
    deadline: f64,
    /// Whether the import has run (a wait carried across frames doesn't run twice).
    ran: bool,
    /// Whether the app has been seen holding this file behind an open prompt (a damage,
    /// profile or high-memory question — or a queue place behind another file's): once that
    /// is over and no document of the name has arrived, the file was disposed of in the app
    /// and the wait ends at once, not at the deadline. Never set for an import the app took
    /// straight through (a decode on a worker): its document simply arrives or doesn't.
    held: bool,
    /// A timed-out open whose import could not be cancelled (the damaged-file confirmation
    /// is the one prompt the app's public Cancel refuses): the entry stays in the queue,
    /// settling nothing, holding the one-open window until the app is done with the file —
    /// only then may the window open, so nothing of this open can land into a later
    /// same-name open's claim.
    observing: bool,
    settle: Settle,
}

impl PendingOpen {
    /// An entry whose bytes are already in hand (their gates passed where they were read):
    /// from its first pass it is exactly an import-and-wait.
    fn with_bytes(name: String, bytes: Vec<u8>, settle: Settle) -> Self {
        Self {
            deadline: js_sys::Date::now() + protocol::OPEN_TIMEOUT_MS,
            name,
            bytes,
            source: None,
            before: Vec::new(),
            warnings: Vec::new(),
            failed: None,
            ran: false,
            held: false,
            observing: false,
            settle,
        }
    }

    /// An entry whose bytes are still on their way: the task writes them (or the refusal)
    /// into `source`, and until it does nothing of the entry runs.
    fn with_source(name: String, source: Rc<RefCell<SourceFetch>>, settle: Settle) -> Self {
        Self { source: Some(source), ..Self::with_bytes(name, Vec::new(), settle) }
    }
}

/// A queued save, and who it answers to once the bytes are made.
enum QueuedSave {
    /// `window.photocraft.save`: a promise settled with the bytes.
    Window { settle: Settle },
    /// The `postMessage` bridge: call `id`, replied to once the bytes can be posted to the parent.
    /// The call's bridge slot rides the entry and is given back when the reply has been posted —
    /// or its failure — on any arm, by the guard's drop. (Named `_slot`: it is held for its
    /// drop, never read.)
    Post { id: Value, _slot: CallSlot },
}

/// What a save made: the bytes and warnings to hand over, and what marking it saved would be.
struct SaveOutcome {
    bytes: Vec<u8>,
    /// The export warnings (an adjustment layer that didn't fit the format, rasterized text…):
    /// what the app shows the user for the same save.
    warnings: Vec<String>,
    /// A layered format (PSD, PSB, `.pcraft`): taking the save counts as saving, as File › Save.
    in_place: bool,
    /// The document that was exported, and at which revision.
    doc: u64,
    revision: u64,
}

thread_local! {
    static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
    static CONTROL: RefCell<Option<Sender<ControlRequest>>> = const { RefCell::new(None) };
    /// The open waiting for the app, with how to settle it (see [`PendingOpen`]); empty when
    /// none is in flight — which is also the one-open rule: a new open is refused while this
    /// holds anything.
    static OPENS: RefCell<Vec<PendingOpen>> = const { RefCell::new(Vec::new()) };
    static SAVES: RefCell<Vec<(String, QueuedSave)>> = const { RefCell::new(Vec::new()) };
    /// Control requests sent, waiting for the app's reply.
    static WAITING: RefCell<Vec<(Receiver<ControlResponse>, Settle)>> = const { RefCell::new(Vec::new()) };
    static DIRTY: Cell<bool> = const { Cell::new(false) };
    /// The active document, for `info()` (refreshed every frame).
    static ACTIVE_DOC: RefCell<Option<protocol::DocInfo>> = const { RefCell::new(None) };
    /// Bridge calls taken and not yet answered, capped at `protocol::MAX_PENDING_CALLS`.
    static INFLIGHT: Cell<usize> = const { Cell::new(0) };
    /// The `?embed=` origin, when the `postMessage` bridge is on.
    static ORIGIN: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn repaint() {
    CTX.with(|c| {
        if let Some(ctx) = c.borrow().as_ref() {
            ctx.request_repaint();
        }
    });
}

/// Keeps frames coming while something is still on its way (an open waiting for its document).
fn repaint_soon() {
    CTX.with(|c| {
        if let Some(ctx) = c.borrow().as_ref() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    });
}

/// Sets up `window.photocraft` and, with an `?embed=` origin in `query`, the `postMessage`
/// bridge. Returns the control channel's receiver for [`PhotocraftApp::with_control`].
pub fn install(query: &str, ctx: egui::Context) -> Receiver<ControlRequest> {
    let (tx, rx) = std::sync::mpsc::channel();
    CONTROL.with(|c| *c.borrow_mut() = Some(tx));
    CTX.with(|c| *c.borrow_mut() = Some(ctx));
    if let Some(window) = web_sys::window() {
        if let Err(e) = js_sys::Reflect::set(&window, &"photocraft".into(), &api_object()) {
            log::error!("embed: couldn't set window.photocraft: {e:?}");
        }
        if let Some(origin) = protocol::allowed_origin(query) {
            listen(&window, &origin);
            ORIGIN.with(|o| *o.borrow_mut() = Some(origin));
            post(&json!({ "photocraft": "ready", "info": info_value() }), None);
        }
    }
    rx
}

/// Every frame, with the app: opens and saves what was asked for, settles answered control
/// requests, reports Photopea uploads that failed, and tells the page when the unsaved state
/// changes (after the saves, so `await save(); isDirty()` sees the save).
pub fn process(app: &mut PhotocraftApp) {
    settle_opens(app);
    settle_saves(app);
    WAITING.with(|w| {
        w.borrow_mut().retain(|(rx, (resolve, reject))| match rx.try_recv() {
            Ok(v) => {
                if v.get("ok").and_then(Value::as_bool) == Some(true) {
                    resolve.call1(&JsValue::NULL, &to_js(v.get("result").unwrap_or(&Value::Null))).ok();
                } else {
                    let msg = v.get("error").and_then(Value::as_str).unwrap_or("error");
                    reject.call1(&JsValue::NULL, &js_sys::Error::new(msg)).ok();
                }
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                reject.call1(&JsValue::NULL, &js_sys::Error::new("request dropped")).ok();
                false
            }
        });
    });
    for error in UPLOAD_ERRORS.with(|e| std::mem::take(&mut *e.borrow_mut())) {
        photocraft_ui_egui::notices::post(app, "The document wasn't saved to the server", vec![error], true, None);
    }
    ACTIVE_DOC.with(|d| *d.borrow_mut() = app.session.active().map(|st| protocol::DocInfo { id: st.doc.id.0, name: st.doc.name.clone() }));
    let dirty = app.has_unsaved_work();
    if DIRTY.with(|d| d.replace(dirty)) != dirty {
        if let Some(window) = web_sys::window() {
            let init = web_sys::CustomEventInit::new();
            init.set_detail(&JsValue::from_bool(dirty));
            if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict("photocraft-dirty", &init) {
                window.dispatch_event(&event).ok();
            }
        }
        post(&json!({ "photocraft": "dirty", "dirty": dirty }), None);
    }
}

/// Runs the one open asked of this frame and settles it once its document has appeared. One
/// open at a time: a second while the first is unsettled was refused at the queue
/// (`protocol::OPEN_BUSY`), so this holds a single entry. What happens to it is
/// [`protocol::open_step`]'s to say — a pure state machine, locked natively — and this pass
/// only reads the app into it and carries the decided step out:
///
/// 0. its import failed: File › Open's error, at once;
/// 1. it observes an import it could not cancel (below): hold the one-open window while the
///    app still holds anything of the file, then release — settling nothing either way, its
///    promise settled at its deadline;
/// 2. its bytes are still on their way (a URL fetch or a byte read): wait, or at the deadline
///    settle `protocol::OPEN_TIMEOUT` (nothing of it has run, so the release is safe — the
///    in-flight transfer is killed with it);
/// 3. it was a preset file: its bytes went to the libraries, no document will appear, at once;
/// 4. its deadline passed — checked **before** the claim, so a request past its deadline never
///    takes a document that appeared in the deadline-crossing frame (`protocol::OPEN_TIMEOUT`)
///    — and *the window only opens once nothing can still land its document*: with the
///    document already landed, release; without it, terminate the underlying import through
///    the app's own Cancel first (`open_prompt::answer(app, None)` — the public face of the
///    Cancel button: it drops every queued file and stops the reads still running), and only
///    the one cancel that API refuses — the damaged-file confirmation, whose answer is
///    crate-private — leaves the entry holding the window as an observer until the app is
///    done with the file;
/// 5. a document new since its import started and named exactly as it asked — the claim, by
///    name and nothing else (`protocol::landed_doc`);
/// 6. the app was seen holding its file behind an open prompt and now holds nothing — a prompt
///    gone *and* no job of the name still opening — with no document of the name arrived: the
///    file was disposed of in the app (its dialog cancelled), and the wait ends at once rather
///    than at the deadline.
///
/// Otherwise it waits — for its bytes, its document, its dialog's answer or its deadline. The
/// walk it has to survive:
///
/// ```text
/// A opens; its import is held behind a dialog → A waits (its file held in the app)
/// B opens on either face → refused OPEN_BUSY: one open is in flight
/// A's dialog is cancelled → the app holds nothing, no document of the name → A rejects at once
/// A's dialog is answered → the document of the name lands → A resolves with it
/// B opens again (nothing in flight now) → taken, and answers with its own document only
/// A's dialog is still up at A's deadline → A rejects OPEN_TIMEOUT and the app's Cancel stops
///   its import — the damaged-file dialog can't be cancelled, so A holds the window until the
///   app is done with its file — and B, opened meanwhile, answers BUSY, never with A's document
/// ```
fn settle_opens(app: &mut PhotocraftApp) {
    let mut queue = OPENS.with(|o| std::mem::take(&mut *o.borrow_mut()));
    if queue.is_empty() {
        return;
    }
    // An open whose bytes answered since the last pass (a URL fetch's or a byte read's): take
    // the answer in — bytes (already through every byte gate where the task read them) as the
    // entry's own, a failure as the open's — so from this pass on the entry is exactly a
    // bytes-in-hand open's.
    for p in queue.iter_mut() {
        let answered = p.source.as_ref().and_then(|fetch| fetch.borrow_mut().answer.take());
        if let Some(answer) = answered {
            p.source = None;
            match answer {
                Ok(bytes) => p.bytes = bytes,
                Err(e) => {
                    p.failed = Some(e);
                    // Nothing to import: the run step below must leave it to the settle step.
                    p.ran = true;
                }
            }
        }
    }
    for p in queue.iter_mut() {
        if p.ran || p.source.is_some() {
            continue;
        }
        p.ran = true;
        p.before = app.session.documents().iter().map(|st| st.doc.id.0).collect();
        match app.open_bytes(&p.name, &p.bytes) {
            Ok(warnings) => {
                p.bytes = Vec::new();
                p.warnings = warnings;
            }
            Err(e) => {
                p.bytes = Vec::new();
                p.failed = Some(e);
            }
        }
    }
    let docs: Vec<(u64, String)> = app.session.documents().iter().map(|st| (st.doc.id.0, st.doc.name.clone())).collect();
    // Whether the app is holding files behind open prompts (damage, profile or high-memory —
    // or queued behind another file's), read-only through `open_prompt`: this is how a
    // cancelled dialog reaches the wait. `true` covers a prompt being on screen or files
    // waiting behind one; both are the app not being done with the file.
    let prompt_up = photocraft_ui_egui::open_prompt::has_open_dialog(app) || photocraft_ui_egui::open_prompt::waiting(app);
    let now = js_sys::Date::now();
    let mut waiting = Vec::new();
    for mut p in queue {
        // Whether the app still holds anything that could turn into this entry's document: a
        // prompt up, a file queued behind one, or a job of its name still opening (the app's
        // public open-job list — an approved high-memory decode runs on after its prompt is
        // gone). The observation's release condition, and the other half of the
        // cancelled-in-the-app read.
        let app_holds = prompt_up || app.jobs.opens.iter().any(|t| t.name == p.name);
        let step = protocol::open_step(&protocol::OpenPass {
            failed: p.failed.clone(),
            fetching: p.source.is_some(),
            landed: protocol::landed_doc(&p.before, &p.name, &docs),
            app_holds,
            held: p.held,
            timed_out: now >= p.deadline,
            observing: p.observing,
            preset: !protocol::expects_document(&p.name),
        });
        match step {
            protocol::OpenStep::SettleErr(e) => settle_err(&p.settle, &e),
            protocol::OpenStep::SettleOk(doc) => {
                let doc = doc.map_or(Value::Null, |(id, name)| json!({ "id": id, "name": name }));
                settle_ok(&p.settle, &json!({ "warnings": p.warnings, "doc": doc }));
            }
            protocol::OpenStep::AbortFetch => {
                log::warn!("embed: open of {} gave up waiting for its fetch", p.name);
                // The deadline settled the promise; the abort makes sure the transfer itself
                // stops too — headers, body stream and all — instead of running on to an
                // answer nobody will read. (The task's late answer drops with the cell.)
                abort_source(&p);
                settle_err(&p.settle, protocol::OPEN_TIMEOUT);
            }
            protocol::OpenStep::TimeoutPending => {
                log::warn!("embed: open of {} gave up waiting for its document", p.name);
                // The usual import-phase entry has already detached its source. Keep the
                // terminate-or-observe boundary total even so: no source transfer may stay
                // alive underneath a timed-out entry, whichever cancellation path it takes.
                abort_source(&p);
                settle_err(&p.settle, protocol::OPEN_TIMEOUT);
                // The window may only open once nothing can still land this open's document.
                // The import is terminated through the app's own Cancel — same semantics as
                // the Cancel button, which is also its cost: anything else queued behind the
                // same prompts stops too. The one refusal (a damage front, whose answer is
                // crate-private) leaves the entry observing instead.
                p.observing = app_holds && photocraft_ui_egui::open_prompt::answer(app, None).is_err();
                if p.observing {
                    log::info!("embed: open of {} holds the busy window until the app is done with its file", p.name);
                    waiting.push(p);
                }
            }
            protocol::OpenStep::HoldWindow => waiting.push(p),
            protocol::OpenStep::Release => {
                log::info!("embed: the app is done with {}'s file; its open releases the busy window", p.name);
            }
            protocol::OpenStep::Wait => {
                p.held |= prompt_up;
                waiting.push(p);
            }
        }
    }
    let still = !waiting.is_empty();
    OPENS.with(|o| *o.borrow_mut() = waiting);
    if still {
        repaint_soon();
    }
}

/// Kills the transfer of an open whose bytes are still on their way (the deadline settled its
/// promise this pass): the abort handle lives in the entry's source cell from before the task
/// started, so it is always there to take. The answer the task would have written lands in the
/// now-detached cell and drops with it.
fn abort_source(p: &PendingOpen) {
    if let Some(source) = p.source.as_ref()
        && let Some(abort) = source.borrow_mut().abort.take()
    {
        abort.abort();
    }
}

/// Encodes the saves asked of this frame and hands each its bytes, with the export warnings
/// (`save` is where the host meets them: an adjustment layer that didn't fit the format, text
/// that was rasterized). The document is marked clean only once the bytes have been taken — the
/// promise resolved, or the reply's transfer finished — so a failure, a post the browser
/// refused or a transfer that didn't happen keeps it dirty.
fn settle_saves(app: &mut PhotocraftApp) {
    for (format, queued) in SAVES.with(|s| std::mem::take(&mut *s.borrow_mut())) {
        match (save_active(app, &format), queued) {
            (Ok(out), QueuedSave::Window { settle }) => {
                let SaveOutcome { bytes, warnings, in_place, doc, revision } = out;
                let js = js_sys::Uint8Array::from(bytes.as_slice());
                // The warnings travel on the bytes themselves, so `await save()` still gives the
                // Uint8Array it always did and carries the warnings with it.
                js_sys::Reflect::set(&js, &"warnings".into(), &to_js(&json!(warnings))).ok();
                // Resolving the promise is the delivery: the resolve call returning is the bytes
                // in the caller's hands. Anything short of that keeps the document dirty.
                if settle.0.call1(&JsValue::NULL, &js).is_ok() && in_place {
                    mark_saved(app, doc, revision);
                }
            }
            (Ok(out), QueuedSave::Post { id, .. }) => {
                let SaveOutcome { bytes, warnings, in_place, doc, revision } = out;
                // `post` replaces `result` with the transferred buffer, so the warnings ride
                // beside it, a sibling of `result` in the reply.
                let mut reply = protocol::reply(&id, Ok(Value::Null));
                reply["warnings"] = json!(warnings);
                let js = js_sys::Uint8Array::from(bytes.as_slice());
                let posted = post(&reply, Some(("result", js.buffer())));
                // Delivery over the bridge is the *transfer* finishing, not the post call
                // returning: the moment the browser takes the buffer, the view on our side is
                // detached (length 0). A post refused, or a buffer still in our hands, leaves
                // the document dirty.
                let delivered = posted && js.length() == 0;
                if delivered && in_place {
                    mark_saved(app, doc, revision);
                }
            }
            (Err(e), QueuedSave::Window { settle }) => settle_err(&settle, &e),
            (Err(e), QueuedSave::Post { id, .. }) => {
                post(&protocol::reply(&id, Err(e)), None);
            }
        }
    }
}

/// The active document in `format`, exported as File › Export writes it, with the export
/// warnings and whether taking it counts as saving (a layered save, as File › Save is).
fn save_active(app: &mut PhotocraftApp, format: &str) -> Result<SaveOutcome, String> {
    let state = app.session.active().ok_or("no document is open")?;
    let name = protocol::export_name(&state.doc.name, format)?;
    let export = app.services.export.as_ref().ok_or("no exporter")?;
    let (bytes, warnings) = export(&state.doc, &name, &ExportSettings::default())?;
    Ok(SaveOutcome {
        in_place: photocraft_engine::file_cmds::saves_in_place(&name),
        doc: state.doc.id.0,
        revision: state.revision,
        bytes,
        warnings,
    })
}

/// Records that `doc` at `revision` was delivered to whoever asked for it — the moment a save
/// makes the document clean. A document no longer active (its tab was switched away during the
/// save) stays dirty: the safe side of a miss.
fn mark_saved(app: &mut PhotocraftApp, doc: u64, revision: u64) {
    if let Some(state) = app.session.active_mut()
        && state.doc.id.0 == doc
    {
        state.saved_revision = revision;
    }
}

/// Settles a promise with a JSON value.
fn settle_ok(settle: &Settle, value: &Value) {
    settle.0.call1(&JsValue::NULL, &to_js(value)).ok();
}

/// Settles a promise with an error.
fn settle_err(settle: &Settle, msg: &str) {
    settle.1.call1(&JsValue::NULL, &js_sys::Error::new(msg)).ok();
}

fn to_js(v: &Value) -> JsValue {
    js_sys::JSON::parse(&v.to_string()).unwrap_or(JsValue::NULL)
}

fn from_js(v: &JsValue) -> Value {
    if v.is_undefined() || v.is_null() {
        return Value::Null;
    }
    js_sys::JSON::stringify(v).ok().and_then(|s| s.as_string()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}

/// A promise settled by `run` (given its resolve and reject functions).
fn promise(run: impl FnOnce(Settle)) -> js_sys::Promise {
    let mut run = Some(run);
    js_sys::Promise::new(&mut |resolve, reject| {
        if let Some(run) = run.take() {
            run((resolve, reject));
        }
    })
}

fn error(msg: &str) -> JsValue {
    js_sys::Error::new(msg).into()
}

/// Queues `bytes` as `name` to open on a coming frame, as File › Open does, and settles once a
/// document of that exact name is in the session (with its warnings and identity), or has
/// failed as File › Open reports it. The bytes are already in hand (the Photopea-style start-up
/// opens); the interactive faces take their window earlier, in [`open`], before reading
/// anything. The door check stays the one-open rule's authority either way: it runs in the
/// promise executor, synchronously.
fn open_queued(name: String, bytes: Vec<u8>) -> js_sys::Promise {
    let p = promise(|settle| {
        if OPENS.with(|o| !o.borrow().is_empty()) {
            settle_err(&settle, protocol::OPEN_BUSY);
            return;
        }
        OPENS.with(|o| o.borrow_mut().push(PendingOpen::with_bytes(name, bytes, settle)));
    });
    repaint();
    p
}

fn save(format: String) -> js_sys::Promise {
    let p = promise(|settle| SAVES.with(|s| s.borrow_mut().push((format, QueuedSave::Window { settle }))));
    repaint();
    p
}

fn request(method: String, params: Value) -> js_sys::Promise {
    // No params is an empty object, as the desktop channel gets them.
    let params = if params.is_null() { json!({}) } else { params };
    let (req, rx) = ControlRequest::new(method, params);
    let sent = CONTROL.with(|c| c.borrow().as_ref().is_some_and(|tx| tx.send(req).is_ok()));
    if !sent {
        return js_sys::Promise::reject(&error("the app isn't running"));
    }
    repaint();
    promise(|settle| WAITING.with(|w| w.borrow_mut().push((rx, settle))))
}

thread_local! {
    /// `ArrayBuffer.prototype.byteLength`'s getter of this realm, found once by walking from a
    /// buffer the engine itself made (nothing of the caller's is trusted to find it): called on
    /// a value, it applies `RequireInternalSlot(O, [[ArrayBufferData]])` — the identity probe —
    /// so it throws for anything without the internal slot (an `Object.create(ArrayBuffer
    /// .prototype)` with forged `byteLength`/`length` own properties among them: the getter
    /// reads the slot, not properties) and answers a real buffer's true length, whichever
    /// realm made the buffer (`instanceof` gets both directions wrong: forged own-realm
    /// objects pass it, real other-realm buffers fail it).
    static BYTELENGTH_GETTER: RefCell<Option<js_sys::Function>> = const { RefCell::new(None) };
}

/// `v`'s true byte length when `v` is a real ArrayBuffer from any realm; `None` when it isn't
/// one — a Proxy over a buffer included, which lacks the slot itself and reads as unreadable.
/// Never the only gate, and nothing it throws escapes: a throw here is a `None`.
fn true_buffer_len(v: &JsValue) -> Option<f64> {
    let getter = BYTELENGTH_GETTER.with(|g| {
        let cached = g.borrow().clone();
        if let Some(f) = cached {
            return Some(f);
        }
        let found = (|| {
            let probe = js_sys::ArrayBuffer::new(0);
            let proto = js_sys::Object::get_prototype_of(&JsValue::from(probe));
            let descriptor = js_sys::Object::get_own_property_descriptor(&proto, &"byteLength".into());
            let get = js_sys::Reflect::get(&descriptor, &"get".into()).ok()?;
            get.dyn_into::<js_sys::Function>().ok()
        })();
        if found.is_some() {
            *g.borrow_mut() = found.clone();
        }
        found
    })?;
    let args = js_sys::Array::new();
    js_sys::Reflect::apply(&getter, v, &args).ok().and_then(|n| n.as_f64())
}

/// The bytes `data` holds, before reading them — the byte budget is checked on this, so a
/// too-large file is refused before it is copied (a Blob's `size`, a buffer's probed true
/// length). Called only inside [`measured_len`]'s promise executor: whatever a hostile value
/// throws there (a Proxy of a Blob with a throwing getter) is that promise's rejection, never
/// an unwind through Rust frames.
fn open_byte_len(data: &JsValue) -> Option<usize> {
    if let Some(blob) = data.dyn_ref::<web_sys::Blob>() {
        return Some(blob.size() as usize);
    }
    if let Some(len) = true_buffer_len(data) {
        return Some(len as usize);
    }
    if js_sys::ArrayBuffer::is_view(data) {
        return js_sys::Reflect::get(data, &"byteLength".into()).ok().and_then(|n| n.as_f64()).map(|n| n as usize);
    }
    None
}

/// [`open_byte_len`] inside a promise: the executor's catch makes a throw read as `undefined`
/// (the bytes in hand decide), not as a hang — the old synchronous call left the caller's
/// promise forever pending and, over the bridge, its call slot taken for good.
fn measured_len(data: &JsValue) -> js_sys::Promise {
    js_sys::Promise::new(&mut |resolve, _| {
        let len = open_byte_len(data).map(|n| JsValue::from_f64(n as f64)).unwrap_or(JsValue::UNDEFINED);
        resolve.call1(&JsValue::NULL, &len).ok();
    })
}

/// Where to read an open's bytes from: a Blob's `arrayBuffer()` (resolving with that promise
/// adopts it, so its failure is this one's rejection), or the value itself. Also inside the
/// executor's catch, for the same reason.
fn bytes_source(data: &JsValue) -> js_sys::Promise {
    js_sys::Promise::new(&mut |resolve, _| {
        if let Some(blob) = data.dyn_ref::<web_sys::Blob>() {
            resolve.call1(&JsValue::NULL, &blob.array_buffer()).ok();
        } else {
            resolve.call1(&JsValue::NULL, data).ok();
        }
    })
}

/// Packs `view`, `offset` and `length` into the plain object a [`bytes_view`] answer resolves
/// with — built only from values the engine made, so reading it back can't throw.
fn pack_bytes_view(resolve: &js_sys::Function, view: JsValue, offset: f64, length: f64) {
    let out = js_sys::Object::new();
    js_sys::Reflect::set(&out, &"view".into(), &view).ok();
    js_sys::Reflect::set(&out, &"offset".into(), &JsValue::from_f64(offset)).ok();
    js_sys::Reflect::set(&out, &"length".into(), &JsValue::from_f64(length)).ok();
    resolve.call1(&JsValue::NULL, &out).ok();
}

/// A genuine `Uint8Array` over the whole of `source`'s buffer, with the range to read from it,
/// as a `{view, offset, length}` object; `undefined` when `source` is none of the byte shapes,
/// and `false` when it is one whose true size is over [`protocol::MAX_OPEN_BYTES`] — mapped by
/// [`bytes_of_view`] to the fixed [`protocol::OPEN_TOO_BIG`]. The buffer's identity is probed
/// ([`true_buffer_len`]), never `instanceof`-ed, and the budget gates on the *true* length
/// before the view — and its allocation — is made: a forged `byteLength` or `length` property
/// decides nothing (the old array-like conversion of a forged buffer allocated what the forged
/// `length` said — 67 MB of real memory — before any gate saw a byte of it). A typed array
/// answers by its buffer and byte range — its *raw bytes*: a `BigInt64Array` reads exactly what
/// it looks at (where the old element-wise conversion threw) and a `DataView` reads its true
/// range (where it read nothing). In the executor's catch too: whatever a hostile value throws
/// rejects this promise instead of unwinding.
fn bytes_view(source: &JsValue) -> js_sys::Promise {
    js_sys::Promise::new(&mut |resolve, _| {
        if let Some(len) = true_buffer_len(source) {
            if protocol::check_open_bytes(Some(len as usize)).is_err() {
                resolve.call1(&JsValue::NULL, &JsValue::from_bool(false)).ok();
            } else {
                pack_bytes_view(&resolve, js_sys::Uint8Array::new(source).into(), 0.0, len);
            }
            return;
        }
        if js_sys::ArrayBuffer::is_view(source) {
            let num = |k: &str| js_sys::Reflect::get(source, &k.into()).ok().and_then(|v| v.as_f64());
            // The view's `buffer`, probed the same way — never `instanceof`, which a forged
            // object passes: anything but a real buffer resolves `undefined`, not a throw or a
            // monster-length view, and a real one is gated on its true length before the view.
            if let Some(buffer) = js_sys::Reflect::get(source, &"buffer".into()).ok()
                && let Some(len) = true_buffer_len(&buffer)
            {
                if protocol::check_open_bytes(Some(len as usize)).is_err() {
                    resolve.call1(&JsValue::NULL, &JsValue::from_bool(false)).ok();
                } else {
                    pack_bytes_view(&resolve, js_sys::Uint8Array::new(&buffer).into(), num("byteOffset").unwrap_or_default(), num("byteLength").unwrap_or_default());
                }
                return;
            }
            resolve.call1(&JsValue::NULL, &JsValue::UNDEFINED).ok();
            return;
        }
        resolve.call1(&JsValue::NULL, &JsValue::UNDEFINED).ok();
    })
}

/// The bytes of a [`bytes_view`] answer, through [`protocol::view_refusal`] and
/// [`protocol::view_range`]: the refusals — the over-budget sentinel `false` mapped to
/// `protocol::OPEN_TOO_BIG`, a value that isn't a view to `protocol::OPEN_READ` — and the
/// range arithmetic are pure and tested natively; this side only reads off what the engine
/// itself packed ([`pack_bytes_view`]), which cannot throw. The whole buffer — what the copy
/// costs, not just the range — is gated before it is copied, then the range is taken out of
/// the copy.
fn bytes_of_view(picked: &JsValue) -> Result<Vec<u8>, String> {
    let sentinel = *picked == JsValue::from_bool(false);
    let view = if sentinel {
        None
    } else {
        js_sys::Reflect::get(picked, &"view".into()).ok().filter(|v| v.is_instance_of::<js_sys::Uint8Array>())
    };
    let view = view.map(js_sys::Uint8Array::unchecked_from_js);
    let view_len = view.as_ref().map_or(0, |v| v.length() as usize);
    if let Some(e) = protocol::view_refusal(sentinel, view.is_some(), view_len) {
        return Err(e);
    }
    // `sentinel` was false and the view is real (both were refused above), so the map never
    // falls to the empty branch.
    let whole = view.map_or_else(Vec::new, |v| v.to_vec());
    // `f64 as usize` saturates (NaN and negatives to 0, an infinity to `usize::MAX`), and the
    // range is taken saturating out of the copy — a hostile range can only fail to read.
    let num = |k: &str| js_sys::Reflect::get(picked, &k.into()).ok().and_then(|v| v.as_f64()).unwrap_or_default();
    Ok(protocol::view_range(whole, num("offset") as usize, num("length") as usize))
}

/// The byte gates a handed-over buffer passes, on either face: the measured length before the
/// copy ([`measured_len`]), the true-body probe and view read ([`bytes_view`],
/// [`bytes_of_view`]), and the measured length again with the bytes in hand. A URL open's
/// fetched bytes never reach here — they are gated where they are read ([`fetch_open`]: the
/// response face and the streaming count) against the same
/// [`protocol::MAX_OPEN_BYTES`], with the same refusal texts. One budget, whichever way the
/// bytes arrived; no face or form of open bypasses it.
async fn gated_bytes(data: &JsValue) -> Result<Vec<u8>, String> {
    // The incoming size, when it can be told without reading: refuse before the copy. A
    // value that won't even be measured reads as unknown and the bytes in hand decide.
    if let Some(n) = JsFuture::from(measured_len(data)).await.ok().and_then(|v| v.as_f64()) {
        protocol::check_open_bytes(Some(n as usize))?;
    }
    let source = JsFuture::from(bytes_source(data)).await.map_err(|_| protocol::OPEN_READ.to_string())?;
    let picked = JsFuture::from(bytes_view(&source)).await.map_err(|_| protocol::OPEN_READ.to_string())?;
    let bytes = bytes_of_view(&picked)?;
    // Measured again with the bytes in hand: a Blob can change size between the two looks.
    protocol::check_open_bytes(Some(bytes.len()))?;
    Ok(bytes)
}

/// `data` as the URL an open fetches itself, when it is a string: resolved against the app's
/// own location, its origin exactly the `?embed=` origin ([`protocol::url_on_origin`]) — a
/// cross-origin URL, another scheme, an unparseable string and a string too long to be a URL
/// (over the call budget at the worst-unit bound — never copied into Rust to find out) all
/// refuse the one fixed [`protocol::OPEN_URL`], before anything is queued or fetched. `None`:
/// `data` isn't a string, and the byte form answers it.
fn resolved_url(data: &JsValue) -> Option<Result<String, String>> {
    if !data.is_string() {
        return None;
    }
    // The length before the copy: a URL rides under the call budget, and a longer string is
    // refused without ever being spelled into Rust.
    let units = js_sys::JsString::unchecked_from_js(data.clone()).length() as usize;
    if protocol::call_string_cost(units).is_none_or(|cost| cost > protocol::MAX_CALL_BYTES) {
        return Some(Err(protocol::OPEN_URL.into()));
    }
    let resolve = || -> Result<String, String> {
        let window = web_sys::window().ok_or_else(|| protocol::OPEN_URL.to_string())?;
        let base = window.location().href().map_err(|_| protocol::OPEN_URL.to_string())?;
        let url = web_sys::Url::new_with_base(&data.as_string().unwrap_or_default(), &base)
            .map_err(|_| protocol::OPEN_URL.to_string())?;
        let href = url.href();
        protocol::url_on_origin(&href, ORIGIN.with(|o| o.borrow().clone()).as_deref())?;
        Ok(href)
    };
    Some(resolve())
}

/// Fetches `url` — absolute and on the embed origin, checked before this runs — as the iframe
/// itself would: same-origin credentials, so the page's session cookies ride the request (how
/// an embedder's authenticated asset endpoints work), and **no redirects at all**
/// (`redirect: "error"`: the embed origin's asset endpoints have no redirect need, and a
/// same-origin URL that would 302 elsewhere must not feed the pipeline — a 3xx fails the
/// fetch, the fixed [`protocol::OPEN_FETCH`], and `response.redirected` is re-checked on the
/// answer as the belt under that). The bytes it returns are already gated: the response face
/// ([`protocol::response_gate`] — 2xx, not redirected, a declared `Content-Length` inside the
/// byte budget) before the body is touched, then a **streaming read** whose running count
/// ([`protocol::stream_step`]) refuses the moment it crosses [`protocol::MAX_OPEN_BYTES`] —
/// the same budget and the same [`protocol::OPEN_TOO_BIG`] text the handed-over byte path
/// gives, enforced *while* the bytes arrive instead of after they are whole, so an unbounded
/// body or a lying length is never buffered. `abort`, when the open's deadline has its
/// controller, kills the transfer in flight.
async fn fetch_open(url: &str, abort: Option<&web_sys::AbortController>) -> Result<Vec<u8>, String> {
    let refused = || protocol::OPEN_FETCH.to_string();
    let window = web_sys::window().ok_or_else(refused)?;
    let init = web_sys::RequestInit::new();
    // The credentials a plain `fetch()` gets, spelled out: same-origin, cookies included.
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
    // Any 3xx fails the fetch outright: no follow, no cross-origin hop.
    init.set_redirect(web_sys::RequestRedirect::Error);
    if let Some(abort) = abort {
        init.set_signal(Some(&abort.signal()));
    }
    let resp: web_sys::Response = JsFuture::from(window.fetch_with_str_and_init(url, &init))
        .await
        .map_err(|_| refused())?
        .dyn_into()
        .map_err(|_| refused())?;
    let length = resp.headers().get("content-length").ok().flatten();
    if let Err(e) = protocol::response_gate(resp.ok(), resp.redirected(), length.as_deref()) {
        // Declared Content-Length over-limit or other header-level rejection: abort the
        // transfer before returning, same cleanup as the streaming over-limit path (the one
        // unified道 both converge to). Without this, a refused response could leave the fetch
        // half-open until GC, and the 30s deadline wouldn't manage it either.
        if let Some(abort) = abort {
            abort.abort();
        }
        return Err(e);
    }
    // A null body is a null-body status (204/205): no bytes to read, no stream to cancel.
    // Everything else streams, chunk by chunk, counted as it arrives.
    let Some(body) = resp.body() else { return Ok(Vec::new()) };
    let reader: web_sys::ReadableStreamDefaultReader = body.get_reader().dyn_into().map_err(|_| refused())?;
    let mut bytes = Vec::new();
    let mut total = 0usize;
    loop {
        let chunk = JsFuture::from(reader.read()).await.map_err(|_| refused())?;
        let done = js_sys::Reflect::get(&chunk, &"done".into()).ok().and_then(|v| v.as_bool());
        if done != Some(false) {
            break;
        }
        let view = js_sys::Reflect::get(&chunk, &"value".into())
            .ok()
            .and_then(|v| v.dyn_into::<js_sys::Uint8Array>().ok())
            .ok_or_else(refused)?;
        match protocol::stream_step(total, view.length() as usize) {
            Ok(next) => {
                total = next;
                bytes.append(&mut view.to_vec());
            }
            Err(e) => {
                // Over the budget mid-stream: stop the transfer and refuse — never buffer a
                // byte past the crossing.
                let _ = reader.cancel();
                if let Some(abort) = abort {
                    abort.abort();
                }
                return Err(e);
            }
        }
    }
    Ok(bytes)
}

/// Opens `data` as `name`. Both faces — `window.photocraft.open` and the `postMessage` bridge —
/// come through here, so the byte budget is one policy. A *string* `data` is the URL form
/// ([`resolved_url`] → [`fetch_open`]): the app fetches the file itself, and the bytes it gets
/// are counted against the same budget as a handed-over buffer. The one-open window is taken as the
/// open's **first step**, in the promise executor, synchronously — before the string is parsed
/// as a URL or a byte of the data is read — so `protocol::OPEN_BUSY` covers the parsing, the
/// fetching and the reading phases alike: no two opens are ever reading at once, and a refused
/// one is refused before its data is touched. A bad URL settles in the executor and never
/// holds the window (nothing was pushed — the release is as immediate as the refusal).
///
/// Everything that touches `data`'s JS side runs inside a promise executor (`measured_len`,
/// `bytes_source`, `bytes_view`) or in the spawned task's await points: whatever it throws
/// becomes the open's answer with the fixed text, so an open **always settles** — never a hang
/// past the timeout, and the bridge slot it holds always comes back with the reply. No JS
/// exception unwinds through a Rust frame on this path.
fn open(data: JsValue, name: Option<String>) -> js_sys::Promise {
    let p = promise(move |settle| {
        // The one-open rule's authority: synchronous in this executor, so neither a URL's
        // fetch nor a byte read ever starts while another open is in flight.
        if OPENS.with(|o| !o.borrow().is_empty()) {
            settle_err(&settle, protocol::OPEN_BUSY);
            return;
        }
        // The URL form, decided under the window: a refused string settles here — the window
        // is never held by it, before anything of it is queued or fetched.
        let url = match resolved_url(&data) {
            Some(Ok(url)) => Some(url),
            Some(Err(e)) => return settle_err(&settle, &e),
            None => None,
        };
        let name = name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| url.as_deref().map(protocol::file_name).unwrap_or_else(|| "document".into()));
        let source = Rc::new(RefCell::new(SourceFetch::default()));
        // A URL open's transfer is abortable from before the task starts, so a deadline that
        // fires mid-fetch can always reach it (a controller that can't be made still fetches —
        // the deadline settles the promise either way, the abort is hygiene).
        if url.is_some() {
            source.borrow_mut().abort = web_sys::AbortController::new().ok();
        }
        OPENS.with(|o| o.borrow_mut().push(PendingOpen::with_source(name, source.clone(), settle)));
        // The byte-acquisition task — a URL's fetch or a byte read — writes its one answer
        // into the shared cell; the frame pass takes it in. It never touches the promise's
        // settle: an answer that arrives after the entry's deadline settled it is read by no
        // one (the entry is gone) and drops with the cell. A URL fetch's answer is already
        // gated where it was read ([`fetch_open`]: the response face and the streaming count,
        // the same budget and refusal texts [`gated_bytes`] gives a handed-over buffer — and
        // its bytes are engine-made from typed chunks, so no caller-shaped value is read on
        // that path); a byte read's answer is gated by [`gated_bytes`] itself.
        wasm_bindgen_futures::spawn_local(async move {
            let abort = source.borrow().abort.clone();
            let answer = match url.as_deref() {
                Some(url) => fetch_open(url, abort.as_ref()).await,
                None => gated_bytes(&data).await,
            };
            source.borrow_mut().answer = Some(answer);
            repaint();
        });
    });
    repaint();
    p
}

/// `info()`: the versions and the document the session is showing.
fn info_value() -> Value {
    protocol::info_with(ACTIVE_DOC.with(|d| d.borrow().clone()).as_ref())
}

/// `window.photocraft`.
fn api_object() -> JsValue {
    let obj = js_sys::Object::new();
    let set = |name: &str, f: JsValue| {
        js_sys::Reflect::set(&obj, &name.into(), &f).ok();
    };
    set("info", Closure::<dyn Fn() -> JsValue>::new(|| to_js(&info_value())).into_js_value());
    set("isDirty", Closure::<dyn Fn() -> bool>::new(|| DIRTY.with(Cell::get)).into_js_value());
    set("open", Closure::<dyn Fn(JsValue, JsValue) -> js_sys::Promise>::new(|data: JsValue, name: JsValue| open(data, name.as_string())).into_js_value());
    set(
        "save",
        Closure::<dyn Fn(JsValue) -> js_sys::Promise>::new(|format: JsValue| save(format.as_string().unwrap_or_else(|| photocraft_format::EXTENSION.into())))
            .into_js_value(),
    );
    set(
        "request",
        Closure::<dyn Fn(String, JsValue) -> js_sys::Promise>::new(|method: String, params: JsValue| request(method, from_js(&params))).into_js_value(),
    );
    obj.into()
}

/// Posts `msg` to the embedding page (only with an `?embed=` origin, and only to that origin),
/// transferring `buffer` along with it as `msg[key]`. Whether the browser took it: a save whose
/// reply can't be posted has not reached the caller, and leaves the document dirty.
fn post(msg: &Value, buffer: Option<(&str, js_sys::ArrayBuffer)>) -> bool {
    let Some(origin) = ORIGIN.with(|o| o.borrow().clone()) else { return false };
    let Some(parent) = web_sys::window().and_then(|w| w.parent().ok().flatten()) else { return false };
    let value = to_js(msg);
    let transfer = js_sys::Array::new();
    if let Some((key, buf)) = buffer {
        js_sys::Reflect::set(&value, &key.into(), &buf).ok();
        transfer.push(&buf);
    }
    match parent.post_message_with_transfer(&value, &origin, &transfer) {
        Ok(()) => true,
        Err(e) => {
            log::error!("embed: couldn't post to the embedding page: {e:?}");
            false
        }
    }
}

/// Whether `key` (an own key of a message object) is the `data` member — asked without ever
/// copying a long key name into a Rust `String`: the name's length is read first (a JS
/// string's `length`, no allocation), and only a name of `data`'s own length — four UTF-16
/// units — is spelled out to compare. A longer key is never converted at all; the budget walk
/// has already refused the over-long ones by the time anything here runs.
fn key_is_data(key: &JsValue) -> bool {
    key.is_string() && js_sys::JsString::unchecked_from_js(key.clone()).length() == 4 && key.as_string().as_deref() == Some("data")
}

/// The message without its `data` member: an open's bytes ride the message's structured clone
/// and never go through the JSON face — stringifying a typed array would spell every byte out,
/// and an ArrayBuffer would be read as `{}` anyway. The call is parsed from what's left.
fn without_binary(msg: &JsValue) -> JsValue {
    if !msg.is_object() {
        return msg.clone();
    }
    let out = js_sys::Object::new();
    let Ok(keys) = js_sys::Reflect::own_keys(msg) else { return out.into() };
    for key in keys.iter() {
        if key_is_data(&key) {
            continue;
        }
        let Ok(value) = js_sys::Reflect::get(msg, &key) else { continue };
        if js_sys::Reflect::set(&out, &key, &value).is_err() {
            break;
        }
    }
    out.into()
}

/// The `postMessage` bridge: calls from `origin` only, replies to the parent window at that origin
/// only (the documented setup is an iframe; the reply never goes to another window or origin).
/// A message is weighed before anything decodes it (below), and at most
/// `protocol::MAX_PENDING_CALLS` calls wait at once — the next is answered `protocol::BUSY`, so
/// a page can't pile work onto the app.
fn listen(window: &web_sys::Window, origin: &str) {
    let origin = origin.to_string();
    let cb = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
        if e.origin() != origin {
            return;
        }
        let data = e.data();
        // The message is weighed on the clone before any decode: the JSON face's stringify+parse
        // is exactly the memory the gate exists to bound, so it never starts on a message over
        // the budget or carrying binary outside `data` — which answers to `MAX_OPEN_BYTES` on
        // its own. The refusal echoes the id, read alone (a number or a short string; nothing
        // else about a refused message is looked at), so the host can correlate it.
        if let Err(refusal) = call_within_budget(&data) {
            let id = js_sys::Reflect::get(&data, &"photocraft".into()).ok().and_then(|v| {
                if v.is_string() && js_sys::JsString::unchecked_from_js(v.clone()).length() <= 64 {
                    v.as_string().map(Value::String)
                } else {
                    v.as_f64().map(|n| json!(n))
                }
            });
            post(&protocol::reply(&id.unwrap_or(Value::Null), Err(refusal)), None);
            return;
        }
        let Some((id, call)) = protocol::parse_call(&from_js(&without_binary(&data))) else { return };
        let call = match call {
            Ok(call) => call,
            Err(error) => {
                post(&protocol::reply(&id, Err(error)), None);
                return;
            }
        };
        match call {
            Call::Info => {
                post(&protocol::reply(&id, Ok(info_value())), None);
            }
            Call::IsDirty => {
                post(&protocol::reply(&id, Ok(Value::Bool(DIRTY.with(Cell::get)))), None);
            }
            Call::Open { name } => {
                let Some(slot) = CallSlot::take() else {
                    post(&protocol::reply(&id, Err(protocol::BUSY.into())), None);
                    return;
                };
                // `data` rides the message's clone (bytes) or is a URL string the app fetches
                // itself; `name` absent is None, and a URL open derives its own from the URL.
                let pending = open(js_sys::Reflect::get(&data, &"data".into()).unwrap_or(JsValue::UNDEFINED), name);
                wasm_bindgen_futures::spawn_local(async move {
                    // The slot comes back once this call has been answered, on either side of
                    // the promise — the open always settles now (a conversion that fails
                    // rejects; nothing hangs), so the task always reaches its end.
                    let _answered = slot;
                    match JsFuture::from(pending).await {
                        Ok(result) => {
                            post(&protocol::reply(&id, Ok(from_js(&result))), None);
                        }
                        Err(e) => {
                            let msg = e.dyn_ref::<js_sys::Error>().map_or_else(|| format!("{e:?}"), |e| String::from(e.message()));
                            post(&protocol::reply(&id, Err(msg)), None);
                        }
                    }
                });
            }
            Call::Save { format } => {
                let Some(slot) = CallSlot::take() else {
                    post(&protocol::reply(&id, Err(protocol::BUSY.into())), None);
                    return;
                };
                // The slot rides the queue entry: whichever arm settles the save — bytes
                // posted, export failed — the guard's drop returns it.
                SAVES.with(|s| s.borrow_mut().push((format, QueuedSave::Post { id, _slot: slot })));
                repaint();
            }
        }
    });
    if window.add_event_listener_with_callback("message", cb.as_ref().unchecked_ref()).is_ok() {
        cb.forget();
    }
}

/// What one plain member of a bridge message weighs when the JSON face spells it out: 24 — the
/// longest number's spelling (`-1.7976931348623157e+308`, 24 characters) with its separator.
const CALL_NODE_BYTES: usize = 24;

/// The outcome of the weighing walk: the message fits; it doesn't (`protocol::CALL_TOO_BIG`);
/// or it carries binary on the metadata plane (`protocol::CALL_BINARY`), refused whatever it
/// weighs.
enum Weigh {
    Fits,
    TooBig,
    Binary,
}

/// Charges `cost` (an overflow reads as `usize::MAX`) onto `spent`, keeping what's left of
/// [`protocol::MAX_CALL_BYTES`]; whether it fit. The walk's only way of refusing on size.
fn charge(spent: &mut usize, cost: Option<usize>) -> bool {
    match spent.checked_add(cost.unwrap_or(usize::MAX)) {
        Some(total) if total <= protocol::MAX_CALL_BYTES => {
            *spent = total;
            true
        }
        _ => false,
    }
}

/// Whether `v` is binary-shaped: a real typed array or `DataView` (`is_view` reads the internal
/// slot, whichever realm made it — a forged object can't pass it), a real `ArrayBuffer` (the
/// identity probe [`true_buffer_len`]), or anything ducking as one — a `byteLength` reading as
/// a number, the one property only binary shapes wear (`length` alone is an array's or a
/// string's, and those are weighed as what they are). A forged duck is refused too, though it
/// would stringify cheap: on the metadata plane no binary shape is *estimated*, so there is no
/// estimate to lie to — the 16 KiB `Uint8Array` with a forged `byteLength` property is refused
/// here, whatever its forged length says. A `byteLength` getter that throws (a Proxy) reads as
/// absent: the value is then weighed as the plain value it is, which is all it can be.
fn binary_shape(v: &JsValue) -> bool {
    js_sys::ArrayBuffer::is_view(v)
        || true_buffer_len(v).is_some()
        || js_sys::Reflect::get(v, &"byteLength".into()).is_ok_and(|n| n.as_f64().is_some())
}

/// Whether `v`, a member of a bridge message, fits what is left of [`protocol::MAX_CALL_BYTES`]
/// at `depth`, and carries no binary shape — the walk stops the moment either fails. What is
/// sized is what the JSON face will spell out, upper-bounded: a string — and an object's key
/// name — by its worst unit ([`protocol::call_string_cost`]: a control character escapes to
/// `\u0000`, six characters to one UTF-16 unit, and three UTF-8 bytes of the Rust `String` is
/// the *cheapest* a non-ASCII unit becomes), an array by its length ([`protocol::call_array_floor`]:
/// stringify spells every index from 0 to `length`, hole or not, so a sparse
/// `new Array(1_000_000)` is refused on its length alone and its elements are walked only when
/// the floor fits). Binary is not sized at all: everywhere but the top-level `data` member —
/// which the walk never enters, and which answers to [`protocol::MAX_OPEN_BYTES`] where it is
/// read — it is refused whole ([`binary_shape`]). Key names are measured before they are
/// copied: a name's length is read off the JS string (`JsString::length`, no allocation) and
/// charged — an over-long name is refused right there — and only a name of `data`'s own length
/// is ever spelled into a Rust `String` ([`key_is_data`]). Nothing is read, spelled out or
/// parsed beyond the budget, so weighing stays O(budget), never O(message).
fn within_call_budget(v: &JsValue, depth: usize, spent: &mut usize) -> Weigh {
    if binary_shape(v) {
        return Weigh::Binary;
    }
    if !charge(spent, Some(CALL_NODE_BYTES)) {
        return Weigh::TooBig;
    }
    if v.is_string() {
        let units = js_sys::JsString::unchecked_from_js(v.clone()).length() as usize;
        return if charge(spent, protocol::call_string_cost(units)) { Weigh::Fits } else { Weigh::TooBig };
    }
    if js_sys::Array::is_array(v) {
        // A sparse array owns almost none of its indices as keys; stringify still spells them
        // all. The length is the floor, charged before anything is visited — over the budget on
        // it alone, the walk never starts (a million-element array is refused without touching
        // its elements; ten thousand are walked, and their floor fits).
        let length = js_sys::Reflect::get(v, &"length".into()).ok().and_then(|n| n.as_f64()).unwrap_or(f64::INFINITY);
        if !charge(spent, protocol::call_array_floor(length as usize)) {
            return Weigh::TooBig;
        }
    }
    // The depth limit before the key list: a nesting bomb pays for no level's own-keys array
    // past the refused depth.
    if depth >= protocol::MAX_CALL_DEPTH {
        return Weigh::TooBig;
    }
    // A container (own keys read fine); anything else — a number, a bool — is a leaf the node
    // price already paid. An array walks its set indices and its `length`; the holes were the
    // floor's.
    let Ok(keys) = js_sys::Reflect::own_keys(v) else { return Weigh::Fits };
    for key in keys.iter() {
        if depth == 0 && key_is_data(&key) {
            continue;
        }
        // A key's own name is spelled out — and escaped — by the JSON face too (ten members
        // behind 60 KiB key names, or one made of control characters, would otherwise pass
        // weighing and pay it all back in stringify) — its length, never its contents: the
        // length is read before anything is copied and an over-long name is refused here.
        // Symbols stringify away to nothing and aren't counted.
        if key.is_string() {
            let units = js_sys::JsString::unchecked_from_js(key.clone()).length() as usize;
            if !charge(spent, protocol::call_string_cost(units)) {
                return Weigh::TooBig;
            }
        }
        let Ok(value) = js_sys::Reflect::get(v, &key) else { return Weigh::TooBig };
        match within_call_budget(&value, depth + 1, spent) {
            Weigh::Fits => {}
            other => return other,
        }
    }
    Weigh::Fits
}

/// The whole-message gate: whether a bridge message fits [`protocol::MAX_CALL_BYTES`] and
/// [`protocol::MAX_CALL_DEPTH`] and carries no binary outside its `data` member, weighed on the
/// raw structured clone *before* any decode. A `MessageEvent` has no byte length of its own —
/// the only true byte count of a message is the serialization being gated — so the walk sizes
/// it structurally instead: every member counts except `data` at the top (the one member
/// allowed to be large, gated by [`protocol::MAX_OPEN_BYTES`]), nesting past the depth limit is
/// refused whatever it weighs, and a binary shape anywhere else refuses the call whole.
fn call_within_budget(msg: &JsValue) -> Result<(), String> {
    let mut spent = 0usize;
    match within_call_budget(msg, 0, &mut spent) {
        Weigh::Fits => Ok(()),
        Weigh::TooBig => Err(protocol::CALL_TOO_BIG.to_string()),
        Weigh::Binary => Err(protocol::CALL_BINARY.to_string()),
    }
}

/// A bridge call's place among the [`protocol::MAX_PENDING_CALLS`] calls allowed to wait,
/// returned by the guard's drop — on every path that answers the call, and on any that
/// doesn't reach one (a task that ends early still drops what it holds). No hand-written
/// release per exit: the old pairs leaked a slot whenever a path between them failed.
struct CallSlot {
    /// Only to make the guard unconstructible outside [`CallSlot::take`].
    _held: (),
}

impl CallSlot {
    /// Takes a slot; `None`, taking nothing, when too many calls already wait.
    fn take() -> Option<Self> {
        INFLIGHT.with(|c| {
            let taken = c.get() < protocol::MAX_PENDING_CALLS;
            if taken {
                c.set(c.get() + 1);
            }
            taken.then(|| CallSlot { _held: () })
        })
    }
}

impl Drop for CallSlot {
    fn drop(&mut self) {
        INFLIGHT.with(|c| c.set(c.get().saturating_sub(1)));
    }
}

// ---------- the Photopea-compatible mode (refused in this build) ----------

/// The Photopea-compatible config in the URL hash: **refused in this build** — a config is
/// rejected with one controlled log line and the app opens as usual
/// ([`PhotopeaConfig::DISABLED`] says why; `None` when the hash holds no config).
pub fn photopea_config(query: &str) -> Option<PhotopeaConfig> {
    let hash = web_sys::window().and_then(|w| w.location().hash().ok())?;
    PhotopeaConfig::trusted(&hash, query, parent_origin().as_deref()).unwrap_or_else(|e| {
        if e == PhotopeaConfig::DISABLED {
            log::warn!("embed: {e}");
        } else {
            log::error!("embed: ignoring the config in the URL: {e}");
        }
        None
    })
}

/// The origin of the page that frames the app, as the browser tells it: `None` at top level.
/// `location.ancestorOrigins` where the browser has it (Chromium, Safari), else the origin of
/// `document.referrer`, which a frame's first load gets from its parent (Firefox; a parent page
/// that sends `Referrer-Policy: no-referrer` leaves it empty, and the config unread).
fn parent_origin() -> Option<String> {
    let window = web_sys::window()?;
    let parent = window.parent().ok().flatten()?;
    if JsValue::from(parent) == JsValue::from(window.clone()) {
        return None;
    }
    let ancestors = js_sys::Reflect::get(&window.location(), &"ancestorOrigins".into()).ok().filter(|v| !v.is_undefined() && !v.is_null());
    if let Some(list) = ancestors {
        return js_sys::Reflect::get(&list, &0.into()).ok().and_then(|o| o.as_string());
    }
    protocol::url_origin(&window.document()?.referrer())
}

/// Posts a Photopea-style status string to the parent window, only ever to the `?embed=` origin
/// (Photopea posts to any origin; these carry file names).
fn post_status(message: &str) {
    let Some(parent) = web_sys::window().and_then(|w| w.parent().ok().flatten()) else { return };
    let Some(target) = ORIGIN.with(|o| o.borrow().clone()) else { return };
    parent.post_message(&JsValue::from_str(message), &target).ok();
}

fn js_err(e: JsValue) -> String {
    format!("{e:?}")
}

/// A request without cookies, as Photopea makes them (so storage that answers CORS with `*`
/// works).
fn no_cookies() -> web_sys::RequestInit {
    let init = web_sys::RequestInit::new();
    init.set_credentials(web_sys::RequestCredentials::Omit);
    init
}

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or("no window")?;
    let resp: web_sys::Response = JsFuture::from(window.fetch_with_str_and_init(url, &no_cookies())).await.map_err(js_err)?.dyn_into().map_err(js_err)?;
    if !resp.ok() {
        return Err(format!("HTTP {} for {url}", resp.status()));
    }
    let buf = JsFuture::from(resp.array_buffer().map_err(js_err)?).await.map_err(js_err)?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

/// Opens the config's files, telling the parent `"loading:<name>"` for each and
/// `"open-error:<name>"` when one can't be downloaded or opened, then `"done"` once they are open,
/// as Photopea does when it is ready.
pub fn open_config_files(config: &PhotopeaConfig) {
    let files = config.files.clone();
    wasm_bindgen_futures::spawn_local(async move {
        for url in files {
            let name = protocol::file_name(&url);
            post_status(&format!("loading:{name}"));
            let opened = match fetch_bytes(&url).await {
                Ok(bytes) => JsFuture::from(open_queued(name.clone(), bytes)).await.map(|_| ()).map_err(js_err),
                Err(e) => Err(e),
            };
            if let Err(e) = opened {
                log::error!("embed: couldn't open {url}: {e}");
                post_status(&format!("open-error:{name}"));
            }
        }
        post_status("done");
    });
}

thread_local! {
    /// Whether an upload is on its way, and the latest save waiting behind it.
    static UPLOADING: Cell<bool> = const { Cell::new(false) };
    static NEXT: RefCell<Option<(String, Vec<u8>)>> = const { RefCell::new(None) };
    /// Whether the last upload failed: until one succeeds the work isn't safe on the server.
    static FAILED: Cell<bool> = const { Cell::new(false) };
    /// Failed uploads for [`process`] to report in the app.
    static UPLOAD_ERRORS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Whether a save to the Photopea server is on its way or failed, so the page keeps asking before
/// it is closed meanwhile.
pub fn upload_unconfirmed() -> bool {
    UPLOADING.with(Cell::get) || FAILED.with(Cell::get)
}

/// Sends a save to the config's server in Photopea's body and tells the parent "saving", then the
/// server's `echoToOE` text, "saved" or "error" (a failure is also reported in the app, and leaving
/// the page asks first until a save gets through). One upload at a time: saves made meanwhile wait,
/// and only the latest is sent, so an older copy never lands after a newer one.
///
/// Unreachable while the mode is refused ([`PhotopeaConfig::DISABLED`]): this single waiting slot
/// is the whole-app queue that refusal exists for. Binding it per document is the price of
/// re-enabling.
pub fn upload(config: &PhotopeaConfig, path: &str, bytes: &[u8]) -> Result<(), String> {
    let url = config.server_url.clone().ok_or("no server url")?;
    let source = config.files.first().cloned().unwrap_or_else(|| path.to_string());
    let body = protocol::save_body(&source, &protocol::extension(path), bytes)?;
    post_status("saving");
    if UPLOADING.with(|u| u.replace(true)) {
        NEXT.with(|n| *n.borrow_mut() = Some((url, body)));
        return Ok(());
    }
    wasm_bindgen_futures::spawn_local(async move {
        let mut job = Some((url, body));
        while let Some((url, body)) = job.take() {
            let (message, failure) = match post_body(&url, &body).await {
                Ok((true, text)) => (protocol::reply_message(true, &text), None),
                Ok((false, text)) => (protocol::reply_message(false, &text), Some(format!("The server answered with an error ({url})."))),
                Err(e) => ("error".to_string(), Some(e)),
            };
            FAILED.with(|f| f.set(failure.is_some()));
            if let Some(e) = failure {
                log::error!("embed: couldn't save to the server: {e}");
                UPLOAD_ERRORS.with(|u| u.borrow_mut().push(e));
            }
            post_status(&message);
            job = NEXT.with(|n| n.borrow_mut().take());
        }
        UPLOADING.with(|u| u.set(false));
        repaint();
    });
    Ok(())
}

async fn post_body(url: &str, body: &[u8]) -> Result<(bool, String), String> {
    let window = web_sys::window().ok_or("no window")?;
    let init = no_cookies();
    init.set_method("POST");
    init.set_body(&js_sys::Uint8Array::from(body));
    let resp: web_sys::Response = JsFuture::from(window.fetch_with_str_and_init(url, &init)).await.map_err(js_err)?.dyn_into().map_err(js_err)?;
    let text = JsFuture::from(resp.text().map_err(js_err)?).await.map_err(js_err)?.as_string().unwrap_or_default();
    Ok((resp.ok(), text))
}
