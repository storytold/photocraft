//! A live panel instance: one persistent wasmi instance, its blobs, timers and event queue.
//!
//! A filter gets a fresh instance per band ([`crate::Plugin::apply`]); a panel keeps one for as
//! long as the panel is open, because it holds the plug-in's whole state machine. The host owns
//! everything around it — the queue, the timers, the blobs — so the plug-in still has no imports.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use serde_json::Value;

use crate::blobs::Blobs;
use crate::ops::{self, Operation};
use crate::runtime::{Deadline, Live, Plugin};
use crate::{Error, Kind, Result};

/// Most events queued for one instance before the oldest UI events are dropped.
pub const MAX_QUEUE: usize = 1024;
/// Most timers one instance may have pending.
pub const MAX_TIMERS: usize = 32;
/// Failures inside [`FAILURE_WINDOW_MS`] that stop automatic recovery.
pub const MAX_FAILURES: usize = 3;
/// The window those failures are counted in.
pub const FAILURE_WINDOW_MS: u64 = 60_000;

/// The event queue. `doc.changed` and `panel.visible` describe a state, so a newer one replaces the
/// pending one; when the queue is full the oldest `ui.*` event is dropped, and only then the front.
#[derive(Debug, Default, Clone)]
pub struct Queue {
    events: VecDeque<Value>,
}

impl Queue {
    pub fn push(&mut self, event: Value) {
        let kind = event.get("type").and_then(Value::as_str).unwrap_or_default().to_string();
        if matches!(kind.as_str(), "doc.changed" | "panel.visible")
            && let Some(slot) = self.events.iter_mut().find(|e| e.get("type").and_then(Value::as_str) == Some(kind.as_str()))
        {
            *slot = event;
            return;
        }
        if self.events.len() >= MAX_QUEUE {
            match self.events.iter().position(|e| e.get("type").and_then(Value::as_str).is_some_and(|t| t.starts_with("ui."))) {
                Some(i) => {
                    self.events.remove(i);
                }
                None => {
                    self.events.pop_front();
                }
            }
        }
        self.events.push_back(event);
    }

    pub fn pop(&mut self) -> Option<Value> {
        self.events.pop_front()
    }
    pub fn len(&self) -> usize {
        self.events.len()
    }
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
    pub fn clear(&mut self) {
        self.events.clear();
    }
    /// The queued event types, oldest first (for `plugin.panel.inspect`).
    pub fn kinds(&self) -> Vec<String> {
        self.events.iter().map(|e| e.get("type").and_then(Value::as_str).unwrap_or_default().to_string()).collect()
    }
}

/// Pending timers, keyed by the id the plug-in chose.
#[derive(Debug, Default, Clone)]
pub struct Timers {
    due: BTreeMap<String, u64>,
}

impl Timers {
    /// Arms `id` for `now + ms`, or moves it if it was already armed.
    pub fn set(&mut self, id: &str, ms: u64, now: u64) -> Result<()> {
        if !self.due.contains_key(id) && self.due.len() >= MAX_TIMERS {
            return Err(Error::Limit(format!("{MAX_TIMERS} timers are already pending")));
        }
        self.due.insert(id.to_string(), now.saturating_add(ms));
        Ok(())
    }
    pub fn clear(&mut self, id: &str) -> bool {
        self.due.remove(id).is_some()
    }
    /// Ids that came due, in id order, removed from the set.
    pub fn take_due(&mut self, now: u64) -> Vec<String> {
        let due: Vec<String> = self.due.iter().filter(|(_, at)| **at <= now).map(|(id, _)| id.clone()).collect();
        for id in &due {
            self.due.remove(id);
        }
        due
    }
    pub fn len(&self) -> usize {
        self.due.len()
    }
    pub fn is_empty(&self) -> bool {
        self.due.is_empty()
    }
    pub fn clear_all(&mut self) {
        self.due.clear();
    }
    /// Milliseconds until the next timer is due, for a host that repaints when one is; `None` when
    /// none is armed. A timer already due gives `0`.
    pub fn next_due_in(&self, now: u64) -> Option<u64> {
        self.due.values().map(|at| at.saturating_sub(now)).min()
    }
}

/// The failure window: three failures inside a minute stop automatic recovery.
///
/// The host owns one of these per panel plug-in, not per instance: it has to survive a restart for
/// a crash-looping plug-in to be recognisable as one.
#[derive(Debug, Default, Clone)]
pub struct Failures {
    at: VecDeque<u64>,
}

impl Failures {
    /// Records a failure and says whether the window is now tripped.
    pub fn record(&mut self, now: u64) -> bool {
        self.at.push_back(now);
        while self.at.front().is_some_and(|t| now.saturating_sub(*t) > FAILURE_WINDOW_MS) {
            self.at.pop_front();
        }
        self.tripped()
    }
    pub fn tripped(&self) -> bool {
        self.at.len() >= MAX_FAILURES
    }
    pub fn len(&self) -> usize {
        self.at.len()
    }
    pub fn is_empty(&self) -> bool {
        self.at.is_empty()
    }
    pub fn clear(&mut self) {
        self.at.clear();
    }
}

/// A running panel plug-in. Dropping it ends the instance and releases its blobs.
pub struct Instance {
    plugin: Arc<Plugin>,
    live: Live,
    blobs: Blobs,
    queue: Queue,
    timers: Timers,
    stopped: Option<Error>,
}

impl std::fmt::Debug for Instance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Instance")
            .field("plugin", &self.plugin.id())
            .field("stopped", &self.stopped)
            .field("queued", &self.queue.len())
            .field("timers", &self.timers.len())
            .field("blobs", &self.blobs.len())
            .finish()
    }
}

impl Instance {
    /// Instantiates a panel plug-in. Only `kind: "panel"` plug-ins have an instance: a filter is run
    /// per band, never kept.
    pub fn start(plugin: Arc<Plugin>) -> Result<Instance> {
        if plugin.manifest().kind != Kind::Panel {
            return Err(Error::Abi(format!("plug-in {:?} is a filter, not a panel", plugin.id())));
        }
        let deadline = Deadline::after_ms(plugin.limits().panel_wall_time_ms);
        let live = plugin.instantiate(&deadline)?;
        let (max_blob_bytes, max_blob_total_bytes) = (plugin.limits().max_blob_bytes, plugin.limits().max_blob_total_bytes);
        Ok(Instance {
            plugin,
            live,
            blobs: Blobs::with_caps(max_blob_bytes, max_blob_total_bytes),
            queue: Queue::default(),
            timers: Timers::default(),
            stopped: None,
        })
    }

    pub fn plugin(&self) -> &Plugin {
        &self.plugin
    }

    /// Sends one event and returns the operations the plug-in replied with.
    ///
    /// Any failure — a trap, an exhausted budget, unparsable or invalid operations — stops the
    /// instance. The caller decides what to do next: nothing here touches the document, and the
    /// failure window ([`Failures`]) belongs to the caller, since it has to outlive a restart.
    pub fn send(&mut self, event: &Value) -> Result<Vec<Operation>> {
        if let Some(error) = &self.stopped {
            return Err(error.clone());
        }
        match self.send_inner(event) {
            Ok(ops) => Ok(ops),
            Err(error) => {
                self.stop(error.clone());
                Err(error)
            }
        }
    }

    fn send_inner(&mut self, event: &Value) -> Result<Vec<Operation>> {
        let bytes = serde_json::to_vec(event).map_err(|e| Error::Ops(format!("could not serialize an event: {e}")))?;
        let plugin = Arc::clone(&self.plugin);
        let deadline = Deadline::after_ms(plugin.limits().panel_wall_time_ms);
        let reply = plugin.send_event(&mut self.live, &bytes, &deadline)?;
        ops::parse(&reply)
    }

    /// Stops the instance and drops everything it held.
    pub fn stop(&mut self, error: Error) {
        self.stopped = Some(error);
        self.queue.clear();
        self.timers.clear_all();
        self.blobs.clear();
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.is_some()
    }
    pub fn stopped_reason(&self) -> Option<&Error> {
        self.stopped.as_ref()
    }

    pub fn blobs(&self) -> &Blobs {
        &self.blobs
    }
    pub fn blobs_mut(&mut self) -> &mut Blobs {
        &mut self.blobs
    }
    pub fn queue(&self) -> &Queue {
        &self.queue
    }
    pub fn queue_mut(&mut self) -> &mut Queue {
        &mut self.queue
    }
    pub fn timers(&self) -> &Timers {
        &self.timers
    }
    pub fn timers_mut(&mut self) -> &mut Timers {
        &mut self.timers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn state_events_coalesce_and_ui_events_do_not() {
        let mut q = Queue::default();
        q.push(json!({"type": "doc.changed", "revision": 1}));
        q.push(json!({"type": "doc.changed", "revision": 2}));
        q.push(json!({"type": "panel.visible", "visible": true}));
        q.push(json!({"type": "panel.visible", "visible": false}));
        q.push(json!({"type": "ui.click", "id": "a"}));
        q.push(json!({"type": "ui.click", "id": "b"}));
        assert_eq!(q.len(), 4);
        assert_eq!(q.kinds(), ["doc.changed", "panel.visible", "ui.click", "ui.click"]);
        assert_eq!(q.pop().unwrap()["revision"], 2, "the newest doc.changed replaced the pending one");
        assert_eq!(q.pop().unwrap()["visible"], false);
    }

    #[test]
    fn a_full_queue_drops_the_oldest_ui_event_first() {
        let mut q = Queue::default();
        q.push(json!({"type": "doc.changed", "revision": 1}));
        for i in 0..MAX_QUEUE - 1 {
            q.push(json!({"type": "ui.change", "id": i}));
        }
        assert_eq!(q.len(), MAX_QUEUE);
        // One more: the oldest ui.change goes, not the doc.changed behind it.
        q.push(json!({"type": "ui.change", "id": "last"}));
        assert_eq!(q.len(), MAX_QUEUE);
        assert_eq!(q.kinds()[0], "doc.changed");
        assert_eq!(q.pop().unwrap()["revision"], 1);
        assert_eq!(q.pop().unwrap()["id"], 1);
    }

    #[test]
    fn timers_are_capped_rearmed_and_taken_in_order() {
        let mut t = Timers::default();
        t.set("a", 100, 1_000).unwrap();
        t.set("b", 50, 1_000).unwrap();
        assert_eq!(t.take_due(1_040), Vec::<String>::new());
        assert_eq!(t.take_due(1_050), ["b"]);
        assert_eq!(t.take_due(2_000), ["a"]);
        assert!(t.is_empty(), "a fired timer is gone");

        // Re-arming is not a new timer, so the cap is not reached.
        t.set("a", 10, 0).unwrap();
        t.set("a", 20, 0).unwrap();
        assert_eq!(t.len(), 1);
        for i in 0..MAX_TIMERS - 1 {
            t.set(&format!("t{i}"), 10, 0).unwrap();
        }
        assert_eq!(t.len(), MAX_TIMERS);
        assert!(matches!(t.set("one-too-many", 10, 0), Err(Error::Limit(_))));
        assert!(t.clear("t0"));
        assert!(!t.clear("t0"));

        // What a host that repaints when a timer is due asks: the nearest one, and none when idle.
        let mut t = Timers::default();
        assert_eq!(t.next_due_in(0), None);
        t.set("soon", 50, 1_000).unwrap();
        t.set("late", 500, 1_000).unwrap();
        assert_eq!(t.next_due_in(1_000), Some(50));
        assert_eq!(t.next_due_in(1_040), Some(10));
        assert_eq!(t.next_due_in(1_050), Some(0), "a due timer is not a negative wait");
        assert_eq!(t.next_due_in(9_999), Some(0));
    }

    #[test]
    fn three_failures_in_a_minute_trip_the_window() {
        let mut f = Failures::default();
        assert!(!f.record(0));
        assert!(!f.record(1_000));
        assert!(f.record(2_000));
        assert!(f.tripped());
        // Failures older than the window fall out of it, so an old crash spree does not count
        // against a plug-in that has since been fine.
        let mut g = Failures::default();
        g.record(0);
        g.record(1);
        assert!(!g.record(FAILURE_WINDOW_MS + 5), "the first two are outside the window");
        assert_eq!(g.len(), 1, "both old failures aged out");
    }
}
