//! `window.photocraft`: the page command API (`craft-web-api`, storytold/craft-libs#413) over the
//! app's own control channel, the one the desktop control server drives (`docs/control-protocol.md`).
//!
//! The crate owns the JavaScript side (`request`, `execute`, `commands`, `uiInspect`, `screenshot`, as
//! promises with a timeout; PhotoCraft has no `document.inspect`, so no `docInspect`). This module is
//! PhotoCraft's part: a [`Request`] that queues a [`ControlRequest`] for the app, which answers it on
//! its next frame, and waits for the reply. A call the page gave up on is released ([`Pending`]
//! gives up the request's dispatch), so the app skips it if it has not started it instead of
//! applying a command the page was told had failed.

use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use craft_web_api::methods::Methods;
use craft_web_api::page::{PageApi, Request};
use photocraft_ui_egui::ControlRequest;
use serde_json::{Value, json};
use wasm_bindgen_futures::JsFuture;

/// The app id in `window.<app>`.
const APP: &str = "photocraft";

/// How often a waiting call looks for the app's reply, in milliseconds (about a frame).
const POLL_MS: i32 = 16;

/// A queued control request whose reply has not arrived. Dropping it before the reply (the page's
/// timeout fired) gives the request up: the app skips it if it has not started it.
struct Pending {
    reply: Receiver<Value>,
    give_up: Box<dyn Fn()>,
    answered: bool,
}

impl Pending {
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
                Some(json!({"ok": false, "error": "the app dropped the request"}))
            }
        }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if !self.answered {
            (self.give_up)();
        }
    }
}

/// Put `window.photocraft` on the page and return the queue the app drains each frame (hand it to
/// `PhotocraftApp::with_control`). An existing `window.photocraft` keeps the functions it already has.
pub fn install(ctx: egui::Context) -> Receiver<ControlRequest> {
    let (queue, requests) = channel::<ControlRequest>();
    let request: Request = Rc::new(move |method, mut params| {
        // `ui.screenshot` rejects fields it does not know (`path`, `focus`), and the crate adds `data: true`
        // to ask for an inline PNG. Without a `path` the reply is inline already, so drop it.
        if method == "ui.screenshot"
            && let Some(fields) = params.as_object_mut()
        {
            fields.remove("data");
        }
        let (control, reply) = ControlRequest::new(method, params);
        let dispatch = control.dispatch.clone();
        if queue.send(control).is_err() {
            return Box::pin(async { json!({"ok": false, "error": "the app is not running"}) });
        }
        ctx.request_repaint();
        let mut pending = Pending {
            reply,
            give_up: Box::new(move || {
                let _ = dispatch.give_up();
            }),
            answered: false,
        };
        Box::pin(async move {
            loop {
                if let Some(reply) = pending.poll() {
                    return reply;
                }
                sleep(POLL_MS).await;
            }
        })
    });
    let methods = Methods { doc_inspect: None, ..Methods::default() };
    if let Err(e) = Rc::new(PageApi::new(APP, methods, request)).install() {
        log::warn!("couldn't install window.{APP}: {e:?}");
    }
    requests
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = JsFuture::from(promise).await;
}
