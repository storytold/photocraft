//! The page's command entry point: `await photocraft.command("image.canvasSize", '{"width":1080}')`
//! runs a command by id and resolves to its result as JSON text, or rejects with the error.
//!
//! Browsers can't listen on sockets, so this stands in for the desktop control server's
//! `engine.execute`: each call becomes a [`ControlRequest`] on the app's control channel and runs
//! between frames exactly as the desktop's does (engine commands with their params and no dialog,
//! long commands waited for as background jobs). Kept free of browser types so it is tested
//! natively; `web.rs` wires it to JavaScript promises and the network.
//!
//! Two ids are the page's own, for files that live outside the browser's file system
//! (`crate::transfer`): [`OPEN_URL`] fetches a file and opens it as a document, and [`EXPORT`]
//! encodes the active document and hands the bytes back. All calls run in submission order.

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use photocraft_ui_egui::{ControlRequest, ControlResponse};
use serde_json::Value;

/// Delivers a command's outcome: its result as JSON text, or the error.
pub type Settle = Box<dyn FnOnce(Result<String, String>)>;

/// `web.openUrl {url, name?}`: fetch `url` (the page's own origin, or one that allows it) and
/// open the bytes as a new document named `name` (default: the URL's last path segment).
pub const OPEN_URL: &str = "web.openUrl";
/// `web.exportBytes {format, quality?, name?}`: the active document encoded as `format`, returned
/// as `{name, mediaType, size, base64}`. The document itself is untouched (a copy, not a save).
pub const EXPORT: &str = "web.exportBytes";

const STOPPED: &str = "PhotoCraft stopped before the command finished";

/// What the page's own commands do to the app ([`crate::transfer`] for `PhotocraftApp`).
pub trait Host {
    /// Open `bytes` as a new document named `name`; the result describes it.
    fn open(&mut self, name: &str, bytes: &[u8]) -> Result<Value, String>;
    /// [`EXPORT`] with its params.
    fn export(&mut self, params: &Value) -> Result<Value, String>;
}

/// A URL for the shell to fetch: settle it with [`Commands::fetched`] under `id`.
pub struct Fetch {
    pub id: u64,
    pub url: String,
}

enum Job {
    /// An engine/UI command. `req` is `None` once sent; a request queued behind unfinished work
    /// is sent when that work is done, so commands never overtake earlier ones.
    Control {
        req: Option<ControlRequest>,
        reply: Receiver<ControlResponse>,
    },
    /// [`OPEN_URL`] waiting for its bytes.
    Fetching {
        name: String,
    },
    Open {
        name: String,
        bytes: Vec<u8>,
    },
    Export(Value),
}

struct Entry {
    id: u64,
    job: Job,
    settle: Settle,
}

/// Commands submitted from the page and waiting for their turn or the app's reply.
pub struct Commands {
    tx: Sender<ControlRequest>,
    pending: Vec<Entry>,
    fetches: Vec<Fetch>,
    next_id: u64,
}

impl Commands {
    /// The queue, and the receiving end for [`photocraft_ui_egui::PhotocraftApp::with_control`].
    pub fn new() -> (Self, Receiver<ControlRequest>) {
        let (tx, rx) = channel();
        (Self { tx, pending: Vec::new(), fetches: Vec::new(), next_id: 0 }, rx)
    }

    /// Queue command `id` with `params` (JSON text; empty means `{}`). Params that aren't a JSON
    /// object settle at once with an error rather than running the command with its defaults.
    pub fn submit(&mut self, id: &str, params: &str, settle: Settle) {
        let params = match parse_params(params) {
            Ok(p) => p,
            Err(e) => return settle(Err(e)),
        };
        let entry_id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let job = match id {
            OPEN_URL => match crate::transfer::open_url_params(&params) {
                Ok((url, name)) => {
                    self.fetches.push(Fetch { id: entry_id, url });
                    Job::Fetching { name }
                }
                Err(e) => return settle(Err(e)),
            },
            EXPORT => Job::Export(params),
            _ => {
                let (req, reply) = ControlRequest::new("engine.execute", serde_json::json!({"command": id, "params": params}));
                // Only commands already sent ahead (the channel keeps their order): send now, so
                // it runs on the very next frame.
                let req = if self.pending.iter().all(|e| matches!(e.job, Job::Control { req: None, .. })) {
                    if self.tx.send(req).is_err() {
                        return settle(Err(STOPPED.into()));
                    }
                    None
                } else {
                    Some(req)
                };
                Job::Control { req, reply }
            }
        };
        self.pending.push(Entry { id: entry_id, job, settle });
    }

    /// URLs the shell should start fetching now (each once).
    pub fn take_fetches(&mut self) -> Vec<Fetch> {
        std::mem::take(&mut self.fetches)
    }

    /// The bytes (or the failure) for [`Fetch`] `id`. The document opens in its turn.
    pub fn fetched(&mut self, id: u64, bytes: Result<Vec<u8>, String>) {
        let Some(at) = self.pending.iter().position(|e| e.id == id) else { return };
        let limit = crate::transfer::MAX_OPEN_BYTES;
        let bytes =
            bytes.and_then(|b| if b.len() > limit { Err(format!("the file is larger than PhotoCraft opens from a URL ({} MB)", limit >> 20)) } else { Ok(b) });
        match bytes {
            Ok(bytes) => {
                if let Some(entry) = self.pending.get_mut(at)
                    && let Job::Fetching { name } = &mut entry.job
                {
                    let name = std::mem::take(name);
                    entry.job = Job::Open { name, bytes };
                }
            }
            Err(e) => {
                let entry = self.pending.remove(at);
                (entry.settle)(Err(e));
            }
        }
    }

    /// Whether any command is still waiting for its turn or reply.
    pub fn waiting(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Run what may run, in submission order, and settle what has finished (call after the app's
    /// frame logic, which drains the control channel). An open or export waits for everything
    /// ahead of it; a command waits only for opens and exports ahead (commands ahead of it are
    /// already in the channel, in order).
    pub fn poll(&mut self, host: &mut impl Host) {
        let mut still = Vec::with_capacity(self.pending.len());
        // Something ahead is unfinished: an open or export must wait.
        let mut blocked = false;
        // An open, export or unsent command is ahead: a command must not be sent yet.
        let mut hold = false;
        for Entry { id, job, settle } in std::mem::take(&mut self.pending) {
            match job {
                Job::Control { mut req, reply } => {
                    if let Some(r) = req.take_if(|_| !hold)
                        && self.tx.send(r).is_err()
                    {
                        settle(Err(STOPPED.into()));
                        continue;
                    }
                    if req.is_some() {
                        (blocked, hold) = (true, true);
                        still.push(Entry { id, job: Job::Control { req, reply }, settle });
                        continue;
                    }
                    match reply.try_recv() {
                        Ok(r) => settle(outcome(&r)),
                        Err(TryRecvError::Empty) => {
                            blocked = true;
                            still.push(Entry { id, job: Job::Control { req, reply }, settle });
                        }
                        Err(TryRecvError::Disconnected) => settle(Err(STOPPED.into())),
                    }
                }
                Job::Open { name, bytes } if !blocked => settle(host.open(&name, &bytes).map(|v| v.to_string())),
                Job::Export(params) if !blocked => settle(host.export(&params).map(|v| v.to_string())),
                job => {
                    (blocked, hold) = (true, true);
                    still.push(Entry { id, job, settle });
                }
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
            let services = photocraft_ui_egui::Services {
                import: Some(Box::new(|name: &str, bytes: &[u8], opts: &photocraft_io::ImportOptions| {
                    photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
                })),
                ..Default::default()
            };
            let app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
            match rx.borrow_mut().take() {
                Some(rx) => app.with_control(rx),
                None => app,
            }
        })
    }

    /// For tests where no page command reaches the app.
    struct NoHost;
    impl Host for NoHost {
        fn open(&mut self, _: &str, _: &[u8]) -> Result<Value, String> {
            Err("no host".into())
        }
        fn export(&mut self, _: &Value) -> Result<Value, String> {
            Err("no host".into())
        }
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let img = photocraft_codecs::Image::from_u8(width, height, photocraft_codecs::ChannelLayout::Rgba, vec![200; (width * height * 4) as usize]).unwrap();
        photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap()
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
        commands.poll(h.state_mut());
        assert!(outcomes.borrow().is_empty(), "nothing runs until the app's frame drains the channel");
        h.run_steps(2);
        commands.poll(h.state_mut());
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
        commands.poll(h.state_mut());
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
        commands.poll(&mut NoHost);
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

    /// The voice flow's ends: a fetched file opens as a document, a command edits it, and the
    /// result comes back as bytes — each in turn, though all three were submitted at once.
    #[test]
    fn open_url_then_edit_then_export_run_in_order() {
        let (mut commands, rx) = Commands::new();
        let mut h = app(rx);
        let outcomes = Outcomes::default();
        commands.submit(OPEN_URL, r#"{"url": "https://files.example/api/storage/9f1c?sig=x", "name": "generated.png"}"#, record(&outcomes, "open"));
        commands.submit("image.canvasSize", r#"{"width": 108, "height": 192, "anchor": "center"}"#, record(&outcomes, "canvas"));
        commands.submit(EXPORT, r#"{"format": "png", "name": "tiktok-ad"}"#, record(&outcomes, "export"));
        let fetches = commands.take_fetches();
        assert_eq!(fetches.iter().map(|f| f.url.as_str()).collect::<Vec<_>>(), ["https://files.example/api/storage/9f1c?sig=x"]);
        assert!(commands.take_fetches().is_empty(), "each URL is handed out once");
        // Frames pass while the file downloads: nothing behind the open runs.
        h.run_steps(2);
        commands.poll(h.state_mut());
        assert!(outcomes.borrow().is_empty(), "{:?}", outcomes.borrow());
        assert_eq!(h.state().session.documents().len(), 0);

        commands.fetched(fetches[0].id, Ok(png(40, 30)));
        for _ in 0..4 {
            commands.poll(h.state_mut());
            h.run_steps(1);
        }
        commands.poll(h.state_mut());
        assert!(!commands.waiting());
        let outcomes = outcomes.borrow();
        assert_eq!(outcomes.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["open", "canvas", "export"]);
        let opened = parse(outcomes[0].1.as_deref().unwrap());
        assert_eq!((opened["name"].as_str(), opened["width"].as_u64(), opened["height"].as_u64()), (Some("generated.png"), Some(40), Some(30)), "{opened}");
        assert!(outcomes[1].1.is_ok(), "{:?}", outcomes[1].1);
        let exported = parse(outcomes[2].1.as_deref().unwrap());
        assert_eq!((exported["name"].as_str(), exported["mediaType"].as_str()), (Some("tiktok-ad.png"), Some("image/png")), "{exported}");
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, exported["base64"].as_str().unwrap()).unwrap();
        assert_eq!(exported["size"].as_u64(), Some(bytes.len() as u64));
        let back = photocraft_codecs::decode(&bytes).unwrap();
        assert_eq!((back.width(), back.height()), (108, 192), "the export is the edited document");
    }

    #[test]
    fn a_failed_download_rejects_and_lets_later_commands_run() {
        let (mut commands, rx) = Commands::new();
        let mut h = app(rx);
        let outcomes = Outcomes::default();
        commands.submit(OPEN_URL, r#"{"url": "https://files.example/gone.png"}"#, record(&outcomes, "open"));
        commands.submit("file.new", r#"{"width": 10, "height": 10}"#, record(&outcomes, "new"));
        let fetch = commands.take_fetches().remove(0);
        commands.fetched(fetch.id, Err("HTTP 403".into()));
        for _ in 0..3 {
            commands.poll(h.state_mut());
            h.run_steps(1);
        }
        let outcomes = outcomes.borrow();
        assert_eq!(outcomes[0], ("open".to_string(), Err("HTTP 403".to_string())));
        assert_eq!(outcomes[1].0, "new");
        assert!(outcomes[1].1.is_ok(), "{:?}", outcomes[1].1);
    }

    #[test]
    fn open_and_export_report_what_went_wrong() {
        let (mut commands, rx) = Commands::new();
        let mut h = app(rx);
        let outcomes = Outcomes::default();
        commands.submit(EXPORT, r#"{"format": "png"}"#, record(&outcomes, "nothing open"));
        commands.submit(OPEN_URL, r#"{"url": "https://files.example/x.png"}"#, record(&outcomes, "not an image"));
        commands.submit(OPEN_URL, r#"{"url": "data:image/png;base64,AAAA"}"#, record(&outcomes, "bad scheme"));
        let fetch = commands.take_fetches().remove(0);
        commands.fetched(fetch.id, Ok(b"not a png".to_vec()));
        commands.poll(h.state_mut());
        h.run_steps(1);
        let outcomes = outcomes.borrow();
        let by = |l: &str| outcomes.iter().find(|(x, _)| x == l).map(|(_, r)| r.clone()).unwrap();
        assert_eq!(by("bad scheme"), Err("web.openUrl reads http(s) and blob URLs, not `data:`".into()));
        assert_eq!(by("nothing open"), Err("no document is open to export".into()));
        assert!(matches!(by("not an image"), Err(e) if !e.is_empty()));
        assert_eq!(h.state().session.documents().len(), 0);
    }
}
