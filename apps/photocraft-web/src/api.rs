//! `window.photocraft`: the control channel for the page, as promises.
//!
//! The same JSON methods as the desktop control server (`docs/control-protocol.md`), so an agent
//! driving PhotoCraft in a browser tab (or an `<iframe>` in another app) uses the same commands as
//! the CLI and the MCP server:
//!
//! ```js
//! await photocraft.execute("file.new", {width: 800, height: 600})  // engine command → its result
//! await photocraft.request("ui.inspect")                           // control method → its result
//! await photocraft.commands()                                      // command list
//! const {base64} = await photocraft.screenshot()                   // the window as PNG (base64)
//! photocraft.openFile("photo.jpg", bytes)                          // opened as a dropped file is
//! ```
//!
//! Results resolve with the method's `result`; errors reject with an `Error`.

use std::cell::RefCell;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use photocraft_ui_egui::ControlRequest;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::web::Inbox;

type Waiting = (Receiver<Value>, js_sys::Function, js_sys::Function);

thread_local! {
    static TX: RefCell<Option<Sender<ControlRequest>>> = const { RefCell::new(None) };
    static WAITING: RefCell<Vec<Waiting>> = const { RefCell::new(Vec::new()) };
    static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
    static INBOX: RefCell<Option<Inbox>> = const { RefCell::new(None) };
}

/// Wire the page's requests to the app: control requests go to `tx`, opened files to `inbox`.
pub fn connect(tx: Sender<ControlRequest>, inbox: Inbox, ctx: egui::Context) {
    TX.with(|t| *t.borrow_mut() = Some(tx));
    INBOX.with(|i| *i.borrow_mut() = Some(inbox));
    CTX.with(|c| *c.borrow_mut() = Some(ctx));
}

fn repaint() {
    CTX.with(|c| {
        if let Some(ctx) = c.borrow().as_ref() {
            ctx.request_repaint();
        }
    });
}

fn to_js(v: &Value) -> JsValue {
    js_sys::JSON::parse(&v.to_string()).unwrap_or(JsValue::NULL)
}

/// The params a page passed: `{}` for none (`undefined` or `null`), else their JSON. Params that
/// have no JSON (a cyclic object, a function, a throwing `toJSON`) are an error, never `{}`: a
/// command run without its params would act with its defaults.
fn from_js(v: &JsValue) -> Result<Value, String> {
    if v.is_undefined() || v.is_null() {
        return Ok(json!({}));
    }
    let text = js_sys::JSON::stringify(v).ok().and_then(|s| s.as_string()).ok_or("params are not JSON (a cyclic object or a function?)")?;
    serde_json::from_str(&text).map_err(|e| format!("params are not JSON: {e}"))
}

/// Settle the promises of answered requests (called every frame).
pub fn poll_replies() {
    WAITING.with(|w| {
        w.borrow_mut().retain(|(rx, resolve, reject)| match rx.try_recv() {
            Ok(v) => {
                if v.get("ok").and_then(Value::as_bool) == Some(true) {
                    let _ = resolve.call1(&JsValue::NULL, &to_js(v.get("result").unwrap_or(&Value::Null)));
                } else {
                    let msg = v.get("error").and_then(Value::as_str).unwrap_or("error").to_string();
                    let _ = reject.call1(&JsValue::NULL, &js_sys::Error::new(&msg));
                }
                false
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => {
                let _ = reject.call1(&JsValue::NULL, &js_sys::Error::new("request dropped"));
                false
            }
        });
    });
}

fn request(method: &str, params: Result<Value, String>) -> js_sys::Promise {
    let params = match params {
        Ok(params) => params,
        Err(e) => return js_sys::Promise::reject(&js_sys::Error::new(&format!("{method}: {e}"))),
    };
    let method = method.to_string();
    let p = js_sys::Promise::new(&mut |resolve, reject| {
        let (req, rx) = ControlRequest::new(method.clone(), params.clone());
        let sent = TX.with(|t| t.borrow().as_ref().is_some_and(|tx| tx.send(req).is_ok()));
        if sent {
            WAITING.with(|w| w.borrow_mut().push((rx, resolve, reject)));
        } else {
            let _ = reject.call1(&JsValue::NULL, &js_sys::Error::new("PhotoCraft is not running yet"));
        }
    });
    repaint();
    p
}

fn set_fn(obj: &js_sys::Object, name: &str, f: JsValue) -> Result<(), JsValue> {
    js_sys::Reflect::set(obj, &name.into(), &f).map(|_| ())
}

/// Install `window.photocraft`.
pub fn install() -> Result<(), JsValue> {
    let obj = js_sys::Object::new();
    set_fn(
        &obj,
        "request",
        Closure::<dyn Fn(String, JsValue) -> js_sys::Promise>::new(|method: String, params: JsValue| request(&method, from_js(&params))).into_js_value(),
    )?;
    set_fn(
        &obj,
        "execute",
        Closure::<dyn Fn(String, JsValue) -> js_sys::Promise>::new(|command: String, params: JsValue| {
            request("engine.execute", from_js(&params).map(|params| json!({"command": command, "params": params})))
        })
        .into_js_value(),
    )?;
    set_fn(&obj, "commands", Closure::<dyn Fn() -> js_sys::Promise>::new(|| request("engine.commands", Ok(json!({})))).into_js_value())?;
    set_fn(&obj, "inspect", Closure::<dyn Fn() -> js_sys::Promise>::new(|| request("ui.inspect", Ok(json!({})))).into_js_value())?;
    set_fn(
        &obj,
        "screenshot",
        Closure::<dyn Fn(JsValue) -> js_sys::Promise>::new(|params: JsValue| request("ui.screenshot", from_js(&params))).into_js_value(),
    )?;
    set_fn(
        &obj,
        "openFile",
        Closure::<dyn Fn(String, js_sys::Uint8Array) -> JsValue>::new(|name: String, bytes: js_sys::Uint8Array| {
            let queued = INBOX.with(|i| {
                i.borrow().as_ref().is_some_and(|inbox| {
                    inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes.to_vec()));
                    true
                })
            });
            repaint();
            JsValue::from_bool(queued)
        })
        .into_js_value(),
    )?;
    let w = web_sys::window().ok_or("no window")?;
    js_sys::Reflect::set(&w, &"photocraft".into(), &obj)?;
    Ok(())
}
