//! Event polling and bounded push subscriptions for control clients (#4939).

use serde_json::{Value, json};

use super::params::{err, ok};
use super::{ControlRequest, Outcome};
use crate::PhotocraftApp;

/// Events a subscriber's transport may have waiting before it loses some (`gap`).
pub const PUSH_QUEUE: usize = 1024;

/// A transport subscribed to events (#4939): its queue, what it wants, and where it has read to.
pub struct Subscriber {
    tx: std::sync::mpsc::SyncSender<String>,
    filter: photocraft_engine::events::Filter,
    cursor: u64,
    /// Events its queue had no room for since the last one it took.
    dropped: u64,
}

/// `events.poll`: the events since a cursor.
pub(super) fn events_poll(app: &PhotocraftApp, p: &Value) -> Outcome {
    match photocraft_engine::events::poll(&app.session.events, p) {
        Ok(batch) => ok(serde_json::to_value(batch).unwrap_or(Value::Null)),
        Err(e) => err(e),
    }
}

/// `events.subscribe {events?, documents?, since?}`: from now on (or from `since`), matching
/// events are pushed to this transport as they happen.
pub(super) fn events_subscribe(app: &mut PhotocraftApp, req: &ControlRequest) -> Outcome {
    let Some(tx) = req.push.clone() else { return err("events.subscribe needs a transport that can receive pushed events (the TCP control channel)") };
    let filter = match photocraft_engine::events::Filter::parse(&req.params) {
        Ok(f) => f,
        Err(e) => return err(e),
    };
    let last = app.session.events.last();
    let cursor = match req.params.get("since") {
        None | Some(Value::Null) => last,
        Some(v) => match v.as_u64() {
            Some(n) => n.min(last),
            None => return err("`since` must be a sequence number (an integer from `next`)"),
        },
    };
    let kinds = filter.kinds.clone().unwrap_or_else(|| photocraft_engine::events::KINDS.iter().map(|k| k.to_string()).collect());
    app.event_subscribers.push(Subscriber { tx, filter, cursor, dropped: 0 });
    ok(json!({"subscribed": true, "next": cursor, "events": kinds, "subscribers": app.event_subscribers.len()}))
}

/// Push new events to every subscriber (once a frame, after the control requests ran), and log
/// a tool change the shell made. A subscriber whose transport went away is dropped; one whose
/// queue is full keeps its place and is told how many it missed once there is room again.
pub fn flush_events(app: &mut PhotocraftApp) {
    if app.last_tool_event != Some(app.ui.tool) {
        let tool = serde_json::to_value(app.ui.tool).unwrap_or(Value::Null);
        app.session.events.push("tool.changed", None, Some("ui"), json!({"tool": tool}));
        app.last_tool_event = Some(app.ui.tool);
    }
    let log = &app.session.events;
    app.event_subscribers.retain_mut(|sub| {
        use std::sync::mpsc::TrySendError;
        loop {
            let batch = log.since(sub.cursor, &sub.filter, 64);
            if batch.gap {
                let oldest = log.oldest();
                sub.dropped = sub.dropped.saturating_add(oldest.saturating_sub(sub.cursor.saturating_add(1)));
                // The evicted range has now been counted, even if the queue cannot accept the
                // notice until a later frame. Keep retained events for after that notice.
                sub.cursor = oldest.saturating_sub(1);
            }
            if sub.dropped > 0 {
                match sub.tx.try_send(json!({"event": "gap", "dropped": sub.dropped}).to_string()) {
                    Ok(()) => sub.dropped = 0,
                    Err(TrySendError::Full(_)) => return true,
                    Err(TrySendError::Disconnected(_)) => return false,
                }
            }
            for e in &batch.events {
                let line = serde_json::to_string(e).unwrap_or_default();
                match sub.tx.try_send(line) {
                    Ok(()) => sub.cursor = e.seq,
                    Err(TrySendError::Full(_)) => {
                        // Lost: it will be counted into the next gap notice.
                        sub.dropped = sub.dropped.saturating_add(1);
                        sub.cursor = e.seq;
                    }
                    Err(TrySendError::Disconnected(_)) => return false,
                }
            }
            if batch.next <= sub.cursor || batch.events.is_empty() {
                sub.cursor = batch.next.max(sub.cursor);
                return true;
            }
            sub.cursor = batch.next;
        }
    });
}
