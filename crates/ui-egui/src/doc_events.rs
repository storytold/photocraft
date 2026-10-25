//! Document lifecycle events for external tools: `document.opened`, `document.saved` and
//! `document.closed`, pushed to control-channel connections that sent `events.subscribe`.
//!
//! A photo manager's "Edit in", a script or an editor integration (as with `code --wait` or git's
//! `core.editor`) hands PhotoCraft a file and needs to know when it was saved (to re-read it) and
//! when the user is done with it (closed). Opened and closed are found by comparing the open
//! documents with the previous snapshot (every frame, and before each saved event so the order is
//! always opened → saved → closed); saved is reported by the save itself, so a save with no
//! changes still counts.
//!
//! Every event is one JSON line with no `id`, so clients tell it from a reply:
//! `{"event": "document.saved", "params": {"path": "/x/a.tif", "format": "tif", "docId": 3}}`.

use photocraft_doc::DocId;
use serde_json::{Value, json};
use std::sync::mpsc::Sender;

/// Every event name, as `events.subscribe` accepts and `app.capabilities` lists them.
pub const EVENTS: &[&str] = &["document.opened", "document.saved", "document.closed"];

/// At most this many subscriptions at once (each control connection holds at most one).
const MAX_SUBSCRIBERS: usize = 64;

struct Subscriber {
    tx: Sender<Value>,
    /// Event names this subscriber wants.
    events: Vec<String>,
}

/// The open documents seen last, and who wants to hear about changes.
#[derive(Default)]
pub struct DocEvents {
    known: Vec<(DocId, Option<String>)>,
    subscribers: Vec<Subscriber>,
    /// Internal listeners (`--wait`, forwarded opens): every event, in order.
    taps: Vec<Sender<Value>>,
}

/// The `format` of an event: the path's lower-case extension, or `null`.
fn format_of(path: Option<&str>) -> Value {
    path.and_then(photocraft_engine::file_cmds::extension).map_or(Value::Null, Value::String)
}

/// One event line.
pub fn event(name: &str, doc: DocId, path: Option<&str>) -> Value {
    json!({"event": name, "params": {"path": path, "format": format_of(path), "docId": doc.0}})
}

impl DocEvents {
    /// Registers a subscriber for `events` (all when empty). Unknown names are an error.
    pub fn subscribe(&mut self, tx: Sender<Value>, events: Vec<String>) -> Result<Vec<String>, String> {
        if let Some(bad) = events.iter().find(|e| !EVENTS.contains(&e.as_str())) {
            return Err(format!("unknown event `{bad}` (known: {})", EVENTS.join(", ")));
        }
        let events = if events.is_empty() { EVENTS.iter().map(|e| e.to_string()).collect() } else { events };
        if self.subscribers.len() >= MAX_SUBSCRIBERS {
            return Err(format!("too many event subscriptions (at most {MAX_SUBSCRIBERS})"));
        }
        self.subscribers.push(Subscriber { tx, events: events.clone() });
        Ok(events)
    }

    /// Adds an internal listener that receives every event.
    pub fn tap(&mut self, tx: Sender<Value>) {
        self.taps.push(tx);
    }

    /// Whether anyone listens (the per-frame comparison is skipped otherwise).
    pub fn active(&self) -> bool {
        !self.subscribers.is_empty() || !self.taps.is_empty()
    }

    fn emit(&mut self, value: Value) {
        let name = value.get("event").and_then(Value::as_str).unwrap_or("").to_string();
        // A closed connection drops its receiver: forget it on the next send.
        self.subscribers.retain(|s| !s.events.contains(&name) || s.tx.send(value.clone()).is_ok());
        self.taps.retain(|t| t.send(value.clone()).is_ok());
    }

    /// Compares the open documents with the last snapshot: emits `document.opened` for new ones
    /// and `document.closed` for gone ones.
    pub fn sync(&mut self, docs: impl Iterator<Item = (DocId, Option<String>)>) {
        let now: Vec<(DocId, Option<String>)> = docs.collect();
        if !self.active() {
            self.known = now;
            return;
        }
        let gone: Vec<_> = self.known.iter().filter(|(id, _)| !now.iter().any(|(n, _)| n == id)).cloned().collect();
        let new: Vec<_> = now.iter().filter(|(id, _)| !self.known.iter().any(|(k, _)| k == id)).cloned().collect();
        self.known = now;
        for (id, path) in new {
            self.emit(event("document.opened", id, path.as_deref()));
        }
        for (id, path) in gone {
            self.emit(event("document.closed", id, path.as_deref()));
        }
    }

    /// A save of `doc` to `path` finished (not a copy). Call [`Self::sync`] first.
    pub fn saved(&mut self, doc: DocId, path: &str) {
        // A Save As changes the path a later `closed` reports.
        if let Some(k) = self.known.iter_mut().find(|(id, _)| *id == doc) {
            k.1 = Some(path.to_string());
        }
        if self.active() {
            self.emit(event("document.saved", doc, Some(path)));
        }
    }
}

impl crate::PhotocraftApp {
    /// Compare the open documents with the last snapshot and emit opened/closed events.
    pub(crate) fn sync_doc_events(&mut self) {
        let docs: Vec<_> = self.session.documents().iter().map(|d| (d.doc.id, d.path.clone())).collect();
        self.doc_events.sync(docs.into_iter());
    }

    /// Emit `document.saved` for `doc` written to `path`.
    pub(crate) fn doc_saved_event(&mut self, doc: DocId, path: &str) {
        self.sync_doc_events();
        self.doc_events.saved(doc, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn names(rx: &std::sync::mpsc::Receiver<Value>) -> Vec<String> {
        rx.try_iter().filter_map(|v| v["event"].as_str().map(str::to_string)).collect()
    }

    #[test]
    fn opened_saved_closed_in_order_and_filtered() {
        let mut ev = DocEvents::default();
        let (tx, rx) = channel();
        let (only_saved, saved_rx) = channel();
        ev.subscribe(tx, vec![]).unwrap();
        ev.subscribe(only_saved, vec!["document.saved".into()]).unwrap();
        ev.sync([(DocId(1), Some("/a.png".into()))].into_iter());
        ev.saved(DocId(1), "/a.tif");
        ev.sync(std::iter::empty());
        assert_eq!(names(&rx), ["document.opened", "document.saved", "document.closed"]);
        assert_eq!(names(&saved_rx), ["document.saved"]);
        let (tx, rx) = channel();
        ev.tap(tx);
        ev.sync([(DocId(2), None)].into_iter());
        let opened = rx.try_recv().unwrap();
        assert_eq!(opened, json!({"event": "document.opened", "params": {"path": null, "format": null, "docId": 2}}));
    }

    #[test]
    fn closed_reports_the_path_of_the_last_save_and_dead_subscribers_drop() {
        let mut ev = DocEvents::default();
        let (tx, rx) = channel();
        ev.subscribe(tx, vec!["document.closed".into()]).unwrap();
        ev.sync([(DocId(1), Some("/a.png".into()))].into_iter());
        ev.saved(DocId(1), "/b.psd");
        ev.sync(std::iter::empty());
        let closed = rx.try_recv().unwrap();
        assert_eq!(closed["params"], json!({"path": "/b.psd", "format": "psd", "docId": 1}));
        drop(rx);
        ev.sync([(DocId(3), None)].into_iter());
        ev.sync(std::iter::empty());
        assert!(!ev.active(), "a dropped connection unsubscribes");
    }

    #[test]
    fn the_app_reports_open_save_and_close_to_a_control_subscriber() {
        use crate::control::{ControlRequest, Outcome, handle};
        let services =
            crate::Services { export: Some(Box::new(|_, _, _| Ok((b"x".to_vec(), Vec::new())))), write: Some(Box::new(|_, _| Ok(()))), ..Default::default() };
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let ctx = egui::Context::default();
        let call = |app: &mut crate::PhotocraftApp, method: &str, params: Value, events: Option<Sender<Value>>| {
            let (mut req, _rx) = ControlRequest::new(method, params);
            req.events = events;
            match handle(app, &ctx, &req) {
                Outcome::Done(v) => v,
                _ => panic!("{method}: expected a reply"),
            }
        };
        // Without a transport that can push, subscribing is refused; bad names too.
        assert_eq!(call(&mut app, "events.subscribe", json!({}), None)["ok"], false);
        let (tx, rx) = channel();
        assert_eq!(call(&mut app, "events.subscribe", json!({"events": "all"}), Some(tx.clone()))["ok"], false);
        assert_eq!(call(&mut app, "events.subscribe", json!({"events": [1]}), Some(tx.clone()))["ok"], false);
        let r = call(&mut app, "events.subscribe", json!({}), Some(tx));
        assert_eq!(r["result"]["events"], json!(EVENTS), "{r}");
        app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
        app.sync_doc_events();
        app.save_as(Some("/work/out.psd".into())).unwrap();
        app.run("file.close", json!({})).unwrap();
        app.sync_doc_events();
        let got: Vec<Value> = rx.try_iter().collect();
        let names: Vec<_> = got.iter().map(|v| v["event"].as_str().unwrap()).collect();
        assert_eq!(names, ["document.opened", "document.saved", "document.closed"]);
        assert_eq!(got[1]["params"]["path"], "/work/out.psd");
        assert_eq!(got[1]["params"]["format"], "psd");
        assert_eq!(got[2]["params"]["path"], "/work/out.psd");
        let caps = call(&mut app, "app.capabilities", json!({}), None);
        assert!(caps["result"]["features"].as_array().unwrap().contains(&json!("events")), "{caps}");
    }

    #[test]
    fn bad_subscriptions_are_errors() {
        let mut ev = DocEvents::default();
        let (tx, _rx) = channel();
        assert!(ev.subscribe(tx.clone(), vec!["document.exploded".into()]).is_err());
        for _ in 0..MAX_SUBSCRIBERS {
            ev.subscribe(tx.clone(), vec![]).unwrap();
        }
        assert!(ev.subscribe(tx, vec![]).is_err());
    }
}
