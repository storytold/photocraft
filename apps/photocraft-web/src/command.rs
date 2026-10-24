//! The page's command entry point: `await photocraft.command("image.canvasSize", '{"width":1080}')`
//! runs a command by id and resolves to its result as JSON text, or rejects with the error.
//!
//! Browsers can't listen on sockets, so this stands in for the desktop control server's
//! `engine.execute`: each call becomes a [`ControlRequest`] on the app's control channel and runs
//! between frames exactly as the desktop's does (engine commands with their params and no dialog,
//! long commands waited for as background jobs). Kept free of browser types so it is tested
//! natively; `web.rs` wires it to JavaScript promises.

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use photocraft_ui_egui::{ControlRequest, ControlResponse};
use serde_json::Value;

/// Delivers a command's outcome: its result as JSON text, or the error.
pub type Settle = Box<dyn FnOnce(Result<String, String>)>;

const STOPPED: &str = "PhotoCraft stopped before the command finished";

/// Commands submitted from the page and waiting for the app's reply.
pub struct Commands {
    tx: Sender<ControlRequest>,
    pending: Vec<(Receiver<ControlResponse>, Settle)>,
}

impl Commands {
    /// The queue, and the receiving end for [`photocraft_ui_egui::PhotocraftApp::with_control`].
    pub fn new() -> (Self, Receiver<ControlRequest>) {
        let (tx, rx) = channel();
        (Self { tx, pending: Vec::new() }, rx)
    }

    /// Queue command `id` with `params` (JSON text; empty means `{}`). Params that aren't a JSON
    /// object settle at once with an error rather than running the command with its defaults.
    pub fn submit(&mut self, id: &str, params: &str, settle: Settle) {
        let params = match parse_params(params) {
            Ok(p) => p,
            Err(e) => return settle(Err(e)),
        };
        let (req, reply) = ControlRequest::new("engine.execute", serde_json::json!({"command": id, "params": params}));
        match self.tx.send(req) {
            Ok(()) => self.pending.push((reply, settle)),
            Err(_) => settle(Err(STOPPED.into())),
        }
    }

    /// Whether any command is still waiting for its reply.
    pub fn waiting(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Settle every command the app has replied to, in submission order (call after the app's
    /// frame logic, which drains the control channel).
    pub fn poll(&mut self) {
        let mut still = Vec::with_capacity(self.pending.len());
        for (rx, settle) in std::mem::take(&mut self.pending) {
            match rx.try_recv() {
                Ok(reply) => settle(outcome(&reply)),
                Err(TryRecvError::Empty) => still.push((rx, settle)),
                Err(TryRecvError::Disconnected) => settle(Err(STOPPED.into())),
            }
        }
        self.pending = still;
    }
}

fn parse_params(text: &str) -> Result<Value, String> {
    if text.trim().is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    match serde_json::from_str::<Value>(text) {
        Ok(v @ Value::Object(_)) => Ok(v),
        Ok(other) => Err(format!("params must be a JSON object, not {other}")),
        Err(e) => Err(format!("params are not valid JSON: {e}")),
    }
}

/// The control channel's reply (`{ok: true, result}` / `{ok: false, error}`) as the command's
/// outcome: the result as JSON text, or the error message.
pub fn outcome(reply: &ControlResponse) -> Result<String, String> {
    match reply.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(reply.get("result").cloned().unwrap_or(Value::Null).to_string()),
        _ => Err(reply.get("error").and_then(Value::as_str).map_or_else(|| format!("unexpected reply: {reply}"), str::to_string)),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use egui_kittest::Harness;
    use photocraft_ui_egui::PhotocraftApp;
    use serde_json::{Value, json};

    use super::*;

    type Outcomes = Rc<RefCell<Vec<(String, Result<String, String>)>>>;

    fn record(outcomes: &Outcomes, label: &str) -> Settle {
        let (outcomes, label) = (outcomes.clone(), label.to_string());
        Box::new(move |r| outcomes.borrow_mut().push((label, r)))
    }

    fn app(rx: Receiver<ControlRequest>) -> Harness<'static, PhotocraftApp> {
        let rx = RefCell::new(Some(rx));
        Harness::builder().with_size(egui::vec2(1000.0, 700.0)).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            match rx.borrow_mut().take() {
                Some(rx) => app.with_control(rx),
                None => app,
            }
        })
    }

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    /// The suite's TikTok flow in miniature: new document, Canvas Size, read it back. Each call
    /// resolves to its own result in submission order, after the app's next frame.
    #[test]
    fn commands_run_on_the_next_frame_and_resolve_to_json_text() {
        let (mut commands, rx) = Commands::new();
        let mut h = app(rx);
        let outcomes = Outcomes::default();
        commands.submit("file.new", r#"{"width": 40, "height": 30, "background": "white"}"#, record(&outcomes, "new"));
        commands.submit("image.canvasSize", r#"{"width": 108, "height": 192, "anchor": "center"}"#, record(&outcomes, "canvas"));
        commands.submit("document.inspect", "", record(&outcomes, "inspect"));
        assert!(commands.waiting());
        commands.poll();
        assert!(outcomes.borrow().is_empty(), "nothing runs until the app's frame drains the channel");
        h.run_steps(2);
        commands.poll();
        assert!(!commands.waiting());
        let outcomes = outcomes.borrow();
        let labels: Vec<&str> = outcomes.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["new", "canvas", "inspect"]);
        assert!(outcomes.iter().all(|(_, r)| r.is_ok()), "{outcomes:?}");
        let inspect = parse(outcomes[2].1.as_deref().unwrap());
        assert_eq!((inspect["width"].as_u64(), inspect["height"].as_u64()), (Some(108), Some(192)), "{inspect}");
        assert_eq!(h.state().session.active().map(|d| (d.doc.size.width, d.doc.size.height)), Some((108, 192)));
    }

    /// A command the engine rejects rejects the promise with the engine's own message.
    #[test]
    fn engine_errors_reject_with_the_message() {
        let (mut commands, rx) = Commands::new();
        let mut h = app(rx);
        let outcomes = Outcomes::default();
        commands.submit("document.inspect", "{}", record(&outcomes, "no document"));
        commands.submit("no.such.command", "{}", record(&outcomes, "unknown"));
        h.run_steps(2);
        commands.poll();
        let outcomes = outcomes.borrow();
        assert_eq!(outcomes.len(), 2);
        let direct = h.state_mut().session.execute("document.inspect", json!({})).map_err(|e| e.to_string());
        assert_eq!(outcomes[0].1, direct.map(|v| v.to_string()), "same error as the engine reports");
        assert!(matches!(&outcomes[1].1, Err(e) if e.contains("no.such.command")), "{:?}", outcomes[1].1);
    }

    #[test]
    fn bad_params_settle_at_once_without_running() {
        let (mut commands, _rx) = Commands::new();
        let outcomes = Outcomes::default();
        commands.submit("file.new", "{width: 40}", record(&outcomes, "not json"));
        commands.submit("file.new", "[40, 30]", record(&outcomes, "array"));
        assert!(!commands.waiting());
        let outcomes = outcomes.borrow();
        assert!(matches!(&outcomes[0].1, Err(e) if e.starts_with("params are not valid JSON: ")), "{:?}", outcomes[0].1);
        assert_eq!(outcomes[1].1, Err("params must be a JSON object, not [40,30]".to_string()));
    }

    /// The app is gone (or never attached the channel): calls reject rather than hang.
    #[test]
    fn a_stopped_app_rejects_instead_of_hanging() {
        let (mut commands, rx) = Commands::new();
        let outcomes = Outcomes::default();
        drop(rx);
        commands.submit("file.new", "{}", record(&outcomes, "after stop"));
        assert_eq!(outcomes.borrow()[0].1, Err(STOPPED.to_string()));

        let (mut commands, rx) = Commands::new();
        commands.submit("file.new", "{}", record(&outcomes, "dropped while queued"));
        // The request (and its reply sender) dies with the receiver, unanswered.
        drop(rx);
        commands.poll();
        assert_eq!(outcomes.borrow()[1].1, Err(STOPPED.to_string()));
        assert!(!commands.waiting());
    }

    #[test]
    fn outcome_unwraps_the_control_reply() {
        assert_eq!(outcome(&json!({"ok": true, "result": {"width": 1080}})), Ok(r#"{"width":1080}"#.to_string()));
        assert_eq!(outcome(&json!({"ok": true})), Ok("null".to_string()));
        assert_eq!(outcome(&json!({"ok": false, "error": "no document"})), Err("no document".to_string()));
        assert_eq!(outcome(&json!({"weird": 1})), Err(r#"unexpected reply: {"weird":1}"#.to_string()));
    }
}
