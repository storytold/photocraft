//! The embedding API (#1614): `window.photocraft` for the page the app runs in, the same document
//! calls over `postMessage` for a page that embeds it in a cross-origin iframe (`?embed=<origin>`),
//! and the Photopea-compatible mode (`#{"files":…,"server":…}` in the URL hash).
//!
//! ```js
//! photocraft.info()                       // {app: "0.6.0", format: 1}
//! photocraft.isDirty()                    // unsaved changes in an open document?
//! await photocraft.open(bytes, "a.psd")   // ArrayBuffer, typed array or Blob → {warnings} once open
//! await photocraft.save("psd")            // the active document as a Uint8Array (pcraft, psd, png…)
//! await photocraft.request("ui.inspect")  // a desktop control-channel method → its result
//! window.addEventListener("photocraft-dirty", e => e.detail)   // true / false as it changes
//! ```
//!
//! Over `postMessage` (only from the `?embed=` origin, replies only to it): send
//! `{photocraft: id, method: "open" | "save" | "isDirty" | "info", …}` (`open`: `data` an
//! ArrayBuffer and `name`; `save`: `format`), get `{photocraft: id, ok, result | error}` back
//! (`save`'s result is an ArrayBuffer, transferred). The app posts `{photocraft: "ready", info}`
//! once it is up and `{photocraft: "dirty", dirty}` when that changes. `request` stays with the
//! app's own page: the embedding page names itself in `?embed=`, so that origin check keeps
//! strangers from reading or replying to the app's messages but can't tell a trusted embedder
//! from any other site (`frame-ancestors` on the app's host does that).
//!
//! A save the page takes counts as a save: a PSD, PSB or `.pcraft` leaves the document clean, as
//! File › Save does.

use std::cell::{Cell, RefCell};
use std::sync::mpsc::{Receiver, Sender};

use photocraft_ui_egui::{ControlRequest, ControlResponse, ExportSettings, PhotocraftApp};
use serde_json::{Value, json};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::embed_protocol::{self as protocol, Call, PhotopeaConfig};

type Settle = (js_sys::Function, js_sys::Function);

thread_local! {
    static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
    static CONTROL: RefCell<Option<Sender<ControlRequest>>> = const { RefCell::new(None) };
    /// Opens and saves waiting for the next frame (they need the app), with how to settle each.
    static OPENS: RefCell<Vec<(String, Vec<u8>, Settle)>> = const { RefCell::new(Vec::new()) };
    static SAVES: RefCell<Vec<(String, Settle)>> = const { RefCell::new(Vec::new()) };
    /// Control requests sent, waiting for the app's reply.
    static WAITING: RefCell<Vec<(Receiver<ControlResponse>, Settle)>> = const { RefCell::new(Vec::new()) };
    static DIRTY: Cell<bool> = const { Cell::new(false) };
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
            post(&json!({ "photocraft": "ready", "info": protocol::info() }), None);
        }
    }
    rx
}

/// Every frame, with the app: opens and saves what was asked for, settles answered control
/// requests, reports Photopea uploads that failed, and tells the page when the unsaved state
/// changes (after the saves, so `await save(); isDirty()` sees the save).
pub fn process(app: &mut PhotocraftApp) {
    for (name, bytes, (resolve, reject)) in OPENS.with(|o| std::mem::take(&mut *o.borrow_mut())) {
        match app.open_bytes(&name, &bytes) {
            Ok(warnings) => resolve.call1(&JsValue::NULL, &to_js(&json!({ "warnings": warnings }))).ok(),
            Err(e) => {
                app.open_failed(&name, &e);
                reject.call1(&JsValue::NULL, &js_sys::Error::new(&e)).ok()
            }
        };
    }
    for (format, (resolve, reject)) in SAVES.with(|s| std::mem::take(&mut *s.borrow_mut())) {
        match save_active(app, &format) {
            Ok(bytes) => resolve.call1(&JsValue::NULL, &js_sys::Uint8Array::from(bytes.as_slice())).ok(),
            Err(e) => reject.call1(&JsValue::NULL, &js_sys::Error::new(&e)).ok(),
        };
    }
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

/// The active document in `format`, exported as File › Export writes it. A layered save (PSD,
/// PSB, `.pcraft`) marks it saved.
fn save_active(app: &mut PhotocraftApp, format: &str) -> Result<Vec<u8>, String> {
    let state = app.session.active().ok_or("no document is open")?;
    let name = protocol::export_name(&state.doc.name, format)?;
    let export = app.services.export.as_ref().ok_or("no exporter")?;
    let (bytes, _warnings) = export(&state.doc, &name, &ExportSettings::default())?;
    if photocraft_engine::file_cmds::saves_in_place(&name)
        && let Some(state) = app.session.active_mut()
    {
        state.saved_revision = state.revision;
    }
    Ok(bytes)
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

/// Opens `bytes` as `name` on the next frame, as File › Open does; settles once it is open (with
/// its warnings) or has failed (as File › Open reports it, too).
fn open_queued(name: String, bytes: Vec<u8>) -> js_sys::Promise {
    let p = promise(|settle| OPENS.with(|o| o.borrow_mut().push((name, bytes, settle))));
    repaint();
    p
}

fn save(format: String) -> js_sys::Promise {
    let p = promise(|settle| SAVES.with(|s| s.borrow_mut().push((format, settle))));
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

/// The bytes of an ArrayBuffer, a typed array or a Blob.
async fn bytes_of(data: JsValue) -> Result<Vec<u8>, String> {
    if let Some(blob) = data.dyn_ref::<web_sys::Blob>() {
        let buf = JsFuture::from(blob.array_buffer()).await.map_err(|e| format!("{e:?}"))?;
        return Ok(js_sys::Uint8Array::new(&buf).to_vec());
    }
    if data.is_instance_of::<js_sys::ArrayBuffer>() || js_sys::ArrayBuffer::is_view(&data) {
        return Ok(js_sys::Uint8Array::new(&data).to_vec());
    }
    Err("open takes an ArrayBuffer, a typed array or a Blob".into())
}

fn open(data: JsValue, name: Option<String>) -> js_sys::Promise {
    let name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "document".into());
    wasm_bindgen_futures::future_to_promise(async move {
        let bytes = bytes_of(data).await.map_err(|e| error(&e))?;
        JsFuture::from(open_queued(name, bytes)).await
    })
}

/// `window.photocraft`.
fn api_object() -> JsValue {
    let obj = js_sys::Object::new();
    let set = |name: &str, f: JsValue| {
        js_sys::Reflect::set(&obj, &name.into(), &f).ok();
    };
    set("info", Closure::<dyn Fn() -> JsValue>::new(|| to_js(&protocol::info())).into_js_value());
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
/// transferring `buffer` along with it as `msg[key]`.
fn post(msg: &Value, buffer: Option<(&str, js_sys::ArrayBuffer)>) {
    let Some(origin) = ORIGIN.with(|o| o.borrow().clone()) else { return };
    let Some(parent) = web_sys::window().and_then(|w| w.parent().ok().flatten()) else { return };
    let value = to_js(msg);
    let transfer = js_sys::Array::new();
    if let Some((key, buf)) = buffer {
        js_sys::Reflect::set(&value, &key.into(), &buf).ok();
        transfer.push(&buf);
    }
    if let Err(e) = parent.post_message_with_transfer(&value, &origin, &transfer) {
        log::error!("embed: couldn't post to the embedding page: {e:?}");
    }
}

/// The `postMessage` bridge: calls from `origin` only, replies to the parent window at that origin
/// only (the documented setup is an iframe; the reply never goes to another window or origin).
fn listen(window: &web_sys::Window, origin: &str) {
    let origin = origin.to_string();
    let cb = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
        if e.origin() != origin {
            return;
        }
        let data = e.data();
        let Some((id, call)) = protocol::parse_call(&from_js(&data)) else { return };
        let call = match call {
            Ok(call) => call,
            Err(error) => return post(&protocol::reply(&id, Err(error)), None),
        };
        let pending = match call {
            Call::Info => return post(&protocol::reply(&id, Ok(protocol::info())), None),
            Call::IsDirty => return post(&protocol::reply(&id, Ok(Value::Bool(DIRTY.with(Cell::get)))), None),
            Call::Open { name } => open(js_sys::Reflect::get(&data, &"data".into()).unwrap_or(JsValue::UNDEFINED), Some(name)),
            Call::Save { format } => save(format),
        };
        wasm_bindgen_futures::spawn_local(async move {
            match JsFuture::from(pending).await {
                Ok(result) => match result.dyn_into::<js_sys::Uint8Array>() {
                    // A save: the bytes go as a transferred ArrayBuffer (a fresh one, their size).
                    Ok(bytes) => post(&protocol::reply(&id, Ok(Value::Null)), Some(("result", bytes.buffer()))),
                    Err(other) => post(&protocol::reply(&id, Ok(from_js(&other))), None),
                },
                Err(e) => {
                    let msg = e.dyn_ref::<js_sys::Error>().map_or_else(|| format!("{e:?}"), |e| String::from(e.message()));
                    post(&protocol::reply(&id, Err(msg)), None);
                }
            }
        });
    });
    if window.add_event_listener_with_callback("message", cb.as_ref().unchecked_ref()).is_ok() {
        cb.forget();
    }
}

// ---------- the Photopea-compatible mode ----------

/// The Photopea-compatible config in the URL hash, if any.
pub fn photopea_config() -> Option<PhotopeaConfig> {
    let hash = web_sys::window().and_then(|w| w.location().hash().ok())?;
    PhotopeaConfig::from_hash(&hash)
}

/// Posts a Photopea-style status string to the parent window. Photopea posts to any origin; with
/// an `?embed=` origin only that one gets it.
fn post_status(message: &str) {
    let Some(parent) = web_sys::window().and_then(|w| w.parent().ok().flatten()) else { return };
    let target = ORIGIN.with(|o| o.borrow().clone()).unwrap_or_else(|| "*".into());
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

/// Whether a save to the Photopea server is on its way or failed: the app counts the document
/// saved once File › Save hands it over, so the page keeps asking before it is closed meanwhile.
pub fn upload_unconfirmed() -> bool {
    UPLOADING.with(Cell::get) || FAILED.with(Cell::get)
}

/// Sends a save to the config's server in Photopea's body and tells the parent "saving", then the
/// server's `echoToOE` text, "saved" or "error" (a failure is also reported in the app, and leaving
/// the page asks first until a save gets through). One upload at a time: saves made meanwhile wait,
/// and only the latest is sent, so an older copy never lands after a newer one.
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
