//! What just changed, for subscribers (#4939): a bounded log of committed changes that the
//! control channel pushes to subscribers, `events.poll` returns since a sequence number, and the
//! MCP server exposes. Photoshop Generator's `imageChanged` / `toolChanged` were the model; the
//! names and fields are PhotoCraft's own.
//!
//! Only committed changes are logged (a command that ran, a job that landed, a document that
//! opened, saved or closed, undo and redo), never previews. Every event carries the document it
//! concerns (`document`, the document id) where there is one and its `source`: the command id
//! for a command, `"ui"` for the shell. The log keeps the last [`CAPACITY`] events; a reader
//! that falls further behind is told so (`gap`), as the proposal asks, rather than silently
//! missing some.

use std::collections::VecDeque;

use serde::Serialize;
use serde_json::{Value, json};

/// Events kept for late readers.
pub const CAPACITY: usize = 4096;

/// Every event kind, in the order a reader is likely to want them explained.
pub const KINDS: [&str; 9] = [
    "document.opened",
    "document.closed",
    "document.activated",
    "document.saved",
    "document.changed",
    "selection.changed",
    "layers.selected",
    "history.changed",
    "tool.changed",
];

/// One committed change.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// Position in the log, from 1; a reader polls with the last one it saw.
    pub seq: u64,
    /// One of [`KINDS`].
    pub event: &'static str,
    /// The document concerned (its id), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<u64>,
    /// What caused it: a command id, or `ui`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Milliseconds since the Unix epoch (0 where the platform has no clock).
    pub time: f64,
    /// Kind-specific fields (see `docs/control-protocol.md` › Events).
    pub data: Value,
}

/// What a reader wants: some kinds, some documents, or everything.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub kinds: Option<Vec<String>>,
    pub documents: Option<Vec<u64>>,
}

impl Filter {
    /// `events` (kind names) and `documents` (ids) from a request's params; an unknown kind or
    /// a malformed list is an error naming it.
    pub fn parse(p: &Value) -> Result<Filter, String> {
        let kinds = match p.get("events") {
            None | Some(Value::Null) => None,
            Some(Value::Array(a)) => {
                let mut out = Vec::with_capacity(a.len());
                for v in a {
                    let k = v.as_str().ok_or("`events` must be an array of event names")?;
                    if !KINDS.contains(&k) {
                        return Err(format!("unknown event `{k}` (events: {})", KINDS.join(", ")));
                    }
                    out.push(k.to_string());
                }
                Some(out)
            }
            Some(_) => return Err("`events` must be an array of event names".into()),
        };
        let documents = match p.get("documents") {
            None | Some(Value::Null) => None,
            Some(Value::Array(a)) => Some(a.iter().map(|v| v.as_u64().ok_or("`documents` must be an array of document ids")).collect::<Result<Vec<_>, _>>()?),
            Some(_) => return Err("`documents` must be an array of document ids".into()),
        };
        Ok(Filter { kinds, documents })
    }

    pub fn matches(&self, e: &Event) -> bool {
        self.kinds.as_ref().is_none_or(|k| k.iter().any(|k| k == e.event))
            && self.documents.as_ref().is_none_or(|d| e.document.is_some_and(|id| d.contains(&id)))
    }
}

/// The log: the last [`CAPACITY`] events and where the next one goes.
#[derive(Clone, Debug, Default)]
pub struct EventLog {
    events: VecDeque<Event>,
    /// Sequence number of the last event logged (0 before any).
    last: u64,
}

/// What [`EventLog::since`] returns.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Batch {
    pub events: Vec<Event>,
    /// Poll with this next time.
    pub next: u64,
    /// Events between the reader's cursor and the oldest kept one were dropped (the reader fell
    /// more than [`CAPACITY`] events behind).
    pub gap: bool,
}

fn now_ms() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0.0
    }
}

impl EventLog {
    /// Log an event; returns its sequence number.
    pub fn push(&mut self, event: &'static str, document: Option<u64>, source: Option<&str>, data: Value) -> u64 {
        self.last = self.last.saturating_add(1);
        if self.events.len() >= CAPACITY {
            self.events.pop_front();
        }
        self.events.push_back(Event { seq: self.last, event, document, source: source.map(str::to_string), time: now_ms(), data });
        self.last
    }

    /// The sequence number of the latest event (0 before any): a reader starting now polls from
    /// here.
    pub fn last(&self) -> u64 {
        self.last
    }

    /// The events after `since` that `filter` wants, at most `limit` of them (the rest wait for
    /// the next poll: `next` stops at the last one returned).
    pub fn since(&self, since: u64, filter: &Filter, limit: usize) -> Batch {
        let oldest = self.events.front().map_or(self.last.saturating_add(1), |e| e.seq);
        // A cursor greater than this run's sequence cannot make progress. Restart at the
        // oldest retained event and tell the reader that its earlier run was lost.
        let restarted = since > self.last;
        let since = if restarted { oldest.saturating_sub(1) } else { since };
        // A cursor before the oldest kept event means events were dropped, unless nothing was
        // ever logged past it.
        let gap = restarted || (since.saturating_add(1) < oldest && self.last > since);
        let mut events = Vec::new();
        let mut next = since.max(if gap { oldest.saturating_sub(1) } else { since });
        for e in self.events.iter().filter(|e| e.seq > since) {
            if events.len() >= limit.max(1) {
                break;
            }
            next = e.seq;
            if filter.matches(e) {
                events.push(e.clone());
            }
        }
        if events.len() < limit.max(1) {
            next = self.last.max(next);
        }
        Batch { events, next, gap }
    }
}

/// Most events one poll returns.
pub const MAX_POLL: usize = 1024;

/// `events.poll {since?, limit?, events?, documents?}`: the events after `since` (omitted: none,
/// just the cursor to poll from), at most `limit` (default 256, at most [`MAX_POLL`]).
pub fn poll(log: &EventLog, p: &Value) -> Result<Batch, String> {
    let filter = Filter::parse(p)?;
    let limit = match p.get("limit") {
        None | Some(Value::Null) => 256,
        Some(v) => v.as_u64().filter(|n| *n >= 1).ok_or("`limit` must be a positive integer")? as usize,
    }
    .min(MAX_POLL);
    match p.get("since") {
        None | Some(Value::Null) => Ok(Batch { events: Vec::new(), next: log.last(), gap: false }),
        Some(v) => {
            let since = v.as_u64().ok_or("`since` must be a sequence number (an integer from `next`)")?;
            Ok(log.since(since, &filter, limit))
        }
    }
}

/// What a command's document looked like before it ran, so [`crate::Session`] can log what the
/// command changed.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub document: Option<u64>,
    pub revision: u64,
    pub saved_revision: u64,
    pub history: usize,
    pub history_id: Option<u64>,
    pub active_layer: Option<u64>,
    pub selected: Vec<u64>,
    pub layers: Vec<u64>,
    /// A copy-on-write selection clone; comparing tile identity costs one pass over tile keys,
    /// while `content_bounds` can scan pixels around a large selection on every command.
    pub selection: Option<photocraft_raster::Surface>,
}

/// Bounds of the pixel selection as `[x0, y0, x1, y1]`, if any.
pub fn selection_bounds(doc: &photocraft_doc::Document) -> Option<[i32; 4]> {
    doc.selection.as_ref().map(|s| {
        let r = s.content_bounds();
        [r.x0, r.y0, r.x1, r.y1]
    })
}

impl Snapshot {
    pub fn of(st: Option<&crate::DocState>) -> Snapshot {
        let Some(st) = st else { return Snapshot::default() };
        Snapshot {
            document: Some(st.doc.id.0),
            revision: st.revision,
            saved_revision: st.saved_revision,
            history: st.history.past_len(),
            history_id: st.history.row_id(st.history.past_len()),
            active_layer: st.active_layer.map(|l| l.0),
            selected: st.selected_layers().iter().map(|l| l.0).collect(),
            layers: st.doc.walk().iter().map(|(_, _, l)| l.id.0).collect(),
            selection: st.doc.selection.clone(),
        }
    }
}

/// The events a successful command `id` produced, from the document before it ([`Snapshot`])
/// and after (`st`, the document the command ran on, when still open).
pub fn after_command(log: &mut EventLog, id: &str, before: &Snapshot, st: Option<&crate::DocState>) {
    let Some(st) = st.filter(|st| before.document == Some(st.doc.id.0)) else { return };
    let doc = st.doc.id.0;
    let src = Some(id);
    if st.revision != before.revision {
        let now: Vec<u64> = st.doc.walk().iter().map(|(_, _, l)| l.id.0).collect();
        let added: Vec<u64> = now.iter().copied().filter(|l| !before.layers.contains(l)).collect();
        let removed: Vec<u64> = before.layers.iter().copied().filter(|l| !now.contains(l)).collect();
        let mut data = json!({"command": id, "revision": st.revision});
        if let Some(r) = st.last_damage {
            data["damage"] = json!([r.x0, r.y0, r.x1, r.y1]);
        }
        if !added.is_empty() {
            data["layersAdded"] = json!(added);
        }
        if !removed.is_empty() {
            data["layersRemoved"] = json!(removed);
        }
        log.push("document.changed", Some(doc), src, data);
    }
    let selection_changed = match (&before.selection, &st.doc.selection) {
        (None, None) => false,
        (Some(before), Some(now)) => !before.shares_storage_with(now),
        _ => true,
    };
    if selection_changed || (id.starts_with("select.") && st.revision != before.revision) {
        log.push("selection.changed", Some(doc), src, json!({"bounds": selection_bounds(&st.doc)}));
    }
    let active = st.active_layer.map(|l| l.0);
    let selected: Vec<u64> = st.selected_layers().iter().map(|l| l.0).collect();
    if active != before.active_layer || selected != before.selected {
        log.push("layers.selected", Some(doc), src, json!({"active": active, "selected": selected}));
    }
    let history = st.history.past_len();
    let history_id = st.history.row_id(history);
    if history != before.history || history_id != before.history_id || matches!(id, "edit.undo" | "edit.redo") {
        let action = match id {
            "edit.undo" => "undo",
            "edit.redo" => "redo",
            _ if history == 0 && before.history > 0 => "clear",
            _ if history > before.history || (history_id != before.history_id && st.revision != before.revision) => "step",
            _ => "trim",
        };
        log.push(
            "history.changed",
            Some(doc),
            src,
            json!({"action": action, "states": history, "canUndo": st.history.can_undo(), "canRedo": st.history.can_redo()}),
        );
    }
    if st.saved_revision != before.saved_revision && st.saved_revision == st.revision {
        log.push("document.saved", Some(doc), src, json!({"path": st.path, "revision": st.revision}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;

    #[test]
    fn selection_pixels_changing_with_the_same_bounds_emit_an_event() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        session.execute("select.rect", json!({"x": 2, "y": 2, "width": 8, "height": 8})).unwrap();
        let before = Snapshot::of(session.active());
        let st = session.active_mut().unwrap();
        let doc = std::sync::Arc::make_mut(&mut st.doc);
        doc.selection.as_mut().unwrap().write_pixel(4, 4, &[0.5]);
        let mut log = EventLog::default();
        after_command(&mut log, "select.refineEdge", &before, session.active());
        let batch = log.since(0, &Filter::default(), 10);
        assert!(batch.events.iter().any(|e| e.event == "selection.changed" && e.data["bounds"] == json!([2, 2, 10, 10])), "{batch:?}");
    }

    fn kinds(s: &Session, since: u64) -> Vec<(&'static str, Value)> {
        s.events.since(since, &Filter::default(), 1000).events.into_iter().map(|e| (e.event, e.data)).collect()
    }

    #[test]
    fn history_change_is_logged_when_the_oldest_state_is_replaced() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
        s.active_mut().unwrap().history.max_states = 1;
        s.execute("image.adjustments.invert", json!({})).unwrap();
        let cursor = s.events.last();
        s.execute("image.adjustments.invert", json!({})).unwrap();
        assert!(kinds(&s, cursor).iter().any(|(kind, _)| *kind == "history.changed"), "{:?}", kinds(&s, cursor));
    }

    #[test]
    fn closing_the_active_document_activates_the_next_one() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "name": "First"})).unwrap();
        s.execute("file.new", json!({"width": 8, "height": 8, "name": "Second"})).unwrap();
        let cursor = s.events.last();
        s.close(1).unwrap();
        let events = kinds(&s, cursor);
        assert_eq!(events.iter().map(|(event, _)| *event).collect::<Vec<_>>(), ["document.closed", "document.activated"]);
        assert_eq!(events[1].1["name"], "First");
    }

    #[test]
    fn a_session_logs_what_commands_change() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16, "background": "white", "name": "A"})).unwrap();
        let opened = kinds(&s, 0);
        assert_eq!(opened[0].0, "document.opened");
        assert_eq!(opened[0].1["name"], "A");
        let doc = s.active().unwrap().doc.id.0;
        let mut cursor = s.events.last();
        s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
        let got = kinds(&s, cursor);
        let names: Vec<&str> = got.iter().map(|(k, _)| *k).collect();
        assert_eq!(names, ["document.changed", "layers.selected", "history.changed"], "{got:?}");
        assert_eq!(got[0].1["command"], "layer.new.layer");
        assert_eq!(got[0].1["layersAdded"].as_array().unwrap().len(), 1);
        assert_eq!(got[2].1["action"], "step");
        assert!(
            s.events.since(cursor, &Filter::default(), 10).events.iter().all(|e| e.document == Some(doc) && e.source.as_deref() == Some("layer.new.layer"))
        );
        cursor = s.events.last();
        s.execute("select.rect", json!({"x": 2, "y": 2, "width": 4, "height": 4})).unwrap();
        let got = kinds(&s, cursor);
        assert!(got.iter().any(|(k, d)| *k == "selection.changed" && d["bounds"] == json!([2, 2, 6, 6])), "{got:?}");
        cursor = s.events.last();
        s.execute("edit.undo", json!({})).unwrap();
        let got = kinds(&s, cursor);
        assert!(got.iter().any(|(k, d)| *k == "history.changed" && d["action"] == "undo" && d["canRedo"] == true), "{got:?}");
        assert!(got.iter().any(|(k, _)| *k == "selection.changed"), "undoing the selection reports it: {got:?}");
        // A query logs nothing.
        cursor = s.events.last();
        s.execute("document.inspect", json!({})).unwrap();
        assert!(kinds(&s, cursor).is_empty());
        // Saving, activating and closing.
        cursor = s.events.last();
        assert!(s.document_saved(0, "/out/a.psd".into(), None));
        let got = kinds(&s, cursor);
        assert!(got.iter().any(|(k, d)| *k == "document.saved" && d["path"] == "/out/a.psd"), "{got:?}");
        assert_eq!(s.active().unwrap().doc.name, "a.psd");
        assert!(!s.document_saved(9, "/nope".into(), None));
        s.execute("file.new", json!({"width": 8, "height": 8, "name": "B"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        cursor = s.events.last();
        s.set_active(0);
        let got = kinds(&s, cursor);
        assert_eq!(got.last().map(|(k, _)| *k), Some("document.activated"));
        cursor = s.events.last();
        s.set_active(0);
        assert!(kinds(&s, cursor).is_empty(), "activating the active document is not a change");
        s.close(1);
        let got = kinds(&s, cursor);
        assert!(got.iter().any(|(k, d)| *k == "document.closed" && d["name"] == "B" && d["unsaved"] == true), "{got:?}");
    }

    #[test]
    fn a_background_job_logs_its_change_when_it_lands() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
        let cursor = s.events.last();
        let crate::jobs::Started::Job(id) = s.start("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap() else { panic!("expected a job") };
        assert!(kinds(&s, cursor).is_empty(), "nothing until it lands");
        s.wait_job(id).unwrap();
        let got = kinds(&s, cursor);
        assert!(got.iter().any(|(k, d)| *k == "document.changed" && d["command"] == "filter.blur.gaussianBlur"), "{got:?}");
        assert!(got.iter().any(|(k, d)| *k == "history.changed" && d["action"] == "step"), "{got:?}");
    }

    #[test]
    fn poll_parses_its_params() {
        let mut log = EventLog::default();
        log.push("tool.changed", None, Some("ui"), json!({"tool": "brush"}));
        let b = poll(&log, &json!({})).unwrap();
        assert!(b.events.is_empty() && b.next == 1, "no `since`: just the cursor");
        assert_eq!(poll(&log, &json!({"since": 0})).unwrap().events.len(), 1);
        assert!(poll(&log, &json!({"since": "0"})).is_err());
        assert!(poll(&log, &json!({"since": 0, "limit": 0})).is_err());
        assert!(poll(&log, &json!({"since": 0, "events": ["nope"]})).is_err());
    }

    #[test]
    fn the_log_is_bounded_and_reports_a_gap() {
        let mut log = EventLog::default();
        assert_eq!(log.last(), 0);
        let b = log.since(0, &Filter::default(), 100);
        assert_eq!(b, Batch { events: vec![], next: 0, gap: false }, "nothing yet, no gap");
        for i in 0..(CAPACITY as u64 + 10) {
            log.push("document.changed", Some(1), Some("x"), json!({"i": i}));
        }
        assert_eq!(log.last(), CAPACITY as u64 + 10);
        let b = log.since(0, &Filter::default(), 100_000);
        assert!(b.gap, "the first ten were dropped");
        assert_eq!(b.events.len(), CAPACITY);
        assert_eq!(b.events[0].seq, 11);
        assert_eq!(b.next, log.last());
        let b = log.since(log.last(), &Filter::default(), 10);
        assert!(b.events.is_empty() && !b.gap && b.next == log.last(), "{b:?}");
        // A limit leaves the rest for the next poll.
        let b = log.since(100, &Filter::default(), 5);
        assert_eq!(b.events.len(), 5);
        assert_eq!(b.next, 105);
        let b2 = log.since(b.next, &Filter::default(), 5);
        assert_eq!(b2.events[0].seq, 106);
    }

    #[test]
    fn a_cursor_from_an_earlier_run_resynchronizes_to_this_log() {
        let mut log = EventLog::default();
        log.push("document.opened", Some(7), Some("ui"), json!({}));
        let batch = log.since(9000, &Filter::default(), 10);
        assert!(batch.gap, "a future cursor belongs to a different log: {batch:?}");
        assert_eq!(batch.next, 1, "the client must be able to resume from this run");
        assert_eq!(batch.events.len(), 1, "the first retained event is available after resynchronizing");
    }

    #[test]
    fn filters_select_kinds_and_documents() {
        let mut log = EventLog::default();
        log.push("document.opened", Some(1), None, json!({}));
        log.push("document.changed", Some(1), Some("edit.fill"), json!({}));
        log.push("document.changed", Some(2), Some("edit.fill"), json!({}));
        log.push("tool.changed", None, Some("ui"), json!({"tool": "brush"}));
        let f = Filter::parse(&json!({"events": ["document.changed"], "documents": [2]})).unwrap();
        let b = log.since(0, &f, 100);
        assert_eq!(b.events.len(), 1);
        assert_eq!(b.events[0].document, Some(2));
        assert_eq!(b.next, 4, "the cursor moves past filtered-out events too");
        let f = Filter::parse(&json!({"documents": [1]})).unwrap();
        assert_eq!(log.since(0, &f, 100).events.len(), 2, "a tool event has no document and is left out by a document filter");
        assert!(Filter::parse(&json!({"events": ["nope"]})).unwrap_err().contains("unknown event"));
        assert!(Filter::parse(&json!({"events": "document.changed"})).is_err());
        assert!(Filter::parse(&json!({"documents": ["1"]})).is_err());
        let e = serde_json::to_value(&log.since(3, &Filter::default(), 1).events[0]).unwrap();
        assert_eq!(e["event"], "tool.changed");
        assert!(e.get("document").is_none(), "absent, not null: {e}");
        assert_eq!(e["seq"], 4);
    }
}
