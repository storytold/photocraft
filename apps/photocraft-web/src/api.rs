//! `window.photocraft`: the control channel for the page, as promises.
//!
//! `request`, `execute`, `commands`, `uiInspect` and `screenshot` come from `craft-web-api`
//! (storytold/craft-libs#413), the same in every Craft web build, over the JSON methods of the
//! desktop control server (`docs/control-protocol.md`). So an agent driving PhotoCraft in a browser
//! tab (or in an `<iframe>` of a page on the same origin) uses the same commands as the CLI and the
//! MCP server. `openFile` is PhotoCraft's own.
//!
//! ```js
//! await photocraft.execute("file.new", {width: 800, height: 600})  // engine command → its result
//! await photocraft.request("ui.inspect")                           // control method → its result
//! await photocraft.commands()                                      // command list
//! const {data} = await photocraft.screenshot()                     // the window as PNG (base64)
//! photocraft.openFile("photo.jpg", bytes)                          // opened as a dropped file is
//! ```
//!
//! Results resolve with the method's `result`; errors reject with an `Error`.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use craft_web_api::methods::Methods;
use craft_web_api::page::{PageApi, Request};
use photocraft_ui_egui::ControlRequest;
use photocraft_ui_egui::control::Dispatch;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::web::Inbox;

thread_local! {
    static TX: RefCell<Option<Sender<ControlRequest>>> = const { RefCell::new(None) };
    static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
    static INBOX: RefCell<Option<Inbox>> = const { RefCell::new(None) };
}

/// How often a waiting call looks for the app's reply, in milliseconds (about a frame).
const POLL_MS: i32 = 16;

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

/// A queued control request whose reply has not arrived. Dropping it before the reply (the page's
/// timeout fired) gives the request up, so the app never runs it late if it had not started it
/// (#3662).
struct Waiting {
    reply: Receiver<Value>,
    dispatch: Dispatch,
    answered: bool,
}

impl Waiting {
    /// The app's reply once it has answered or gone away; `None` while the request is waiting.
    fn poll(&mut self) -> Option<Value> {
        match self.reply.try_recv() {
            Ok(reply) => {
                self.answered = true;
                Some(reply)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.answered = true;
                Some(json!({"ok": false, "error": "request dropped"}))
            }
        }
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        if !self.answered {
            self.dispatch.give_up();
        }
    }
}

/// Queue a control request for the app and wait for its reply, as the protocol's `{ok, result}` /
/// `{ok: false, error}` object.
fn send(method: &str, mut params: Value) -> Pin<Box<dyn Future<Output = Value>>> {
    if method == "ui.screenshot"
        && let Some(o) = params.as_object_mut()
    {
        // `screenshot()` asks for the PNG inline (`data`); PhotoCraft always answers with it when
        // there is no `path`, and its `ui.screenshot` refuses fields it doesn't know.
        o.remove("data");
    }
    let (req, rx) = ControlRequest::new(method, params);
    let dispatch = req.dispatch.clone();
    let sent = TX.with(|t| t.borrow().as_ref().is_some_and(|tx| tx.send(req).is_ok()));
    repaint();
    if !sent {
        return Box::pin(async { json!({"ok": false, "error": "PhotoCraft is not running yet"}) });
    }
    let mut waiting = Waiting { reply: rx, dispatch, answered: false };
    Box::pin(async move {
        loop {
            if let Some(reply) = waiting.poll() {
                return reply;
            }
            sleep(POLL_MS).await;
        }
    })
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = JsFuture::from(promise).await;
}

/// Install `window.photocraft`: the crate's functions, then `openFile`. PhotoCraft has no
/// `document.inspect`, so there is no `docInspect`.
pub fn install() -> Result<(), JsValue> {
    let request: Request = Rc::new(|method, params| send(&method, params));
    let methods = Methods { doc_inspect: None, ..Methods::default() };
    Rc::new(PageApi::new("photocraft", methods, request)).install()?;
    let w = web_sys::window().ok_or("no window")?;
    let obj = js_sys::Reflect::get(&w, &"photocraft".into())?;
    // Not our object when the page made `window.photocraft` read-only (or froze `window`) first.
    let ours = obj.is_object() && js_sys::Reflect::get(&obj, &"request".into())?.is_function();
    if !ours {
        return Err("window.photocraft is read-only".into());
    }
    let open_file = Closure::<dyn Fn(String, js_sys::Uint8Array) -> JsValue>::new(|name: String, bytes: js_sys::Uint8Array| {
        let queued = INBOX.with(|i| {
            i.borrow().as_ref().is_some_and(|inbox| {
                inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes.to_vec()));
                true
            })
        });
        repaint();
        JsValue::from_bool(queued)
    });
    if !js_sys::Reflect::set(&obj, &"openFile".into(), &open_file.into_js_value())? {
        return Err("window.photocraft is read-only".into());
    }
    Ok(())
}
