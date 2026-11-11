//! Multi-agent helpers on `Session`: the `client` param, peers, and the layer selection kept per
//! client (slots). Events go to the session journal (`events.rs`); the command entry point is
//! `session/execute.rs`.

use photocraft_doc::LayerId;
use serde_json::{Value, json};

use crate::multisession;
use crate::{EngineError, Result, Session};

/// Remove the optional `client` param and check it (before any command lookup).
/// Shared by `execute`, `start` and `precheck`.
pub(crate) fn take_client(params: &mut Value) -> Result<Option<String>> {
    let raw = match params.as_object_mut() {
        Some(o) => o.remove("client"),
        None => None,
    };
    let name = match &raw {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.as_str()),
        Some(_) => Some(""),
    };
    multisession::validate_client(name).map_err(EngineError::Other)
}

/// Whether two versions of a layer are the same, leaving a group's children aside (each child
/// is compared on its own).
fn same_layer(a: &photocraft_doc::Layer, b: &photocraft_doc::Layer) -> bool {
    if a.children().is_none() && b.children().is_none() {
        return a == b;
    }
    let (mut a, mut b) = (a.clone(), b.clone());
    if let Some(children) = a.children_mut() {
        children.clear();
    }
    if let Some(children) = b.children_mut() {
        children.clear();
    }
    a == b
}

/// `(parent, layer)` of every layer `mine` does not accept, in stacking order.
fn placement(doc: &photocraft_doc::Document, mine: &dyn Fn(u64) -> bool) -> Vec<(Option<u64>, u64)> {
    let mut parents: Vec<u64> = Vec::new();
    let mut out = Vec::new();
    for (_, depth, layer) in doc.walk() {
        parents.truncate(depth);
        if !mine(layer.id.0) {
            out.push((parents.last().copied(), layer.id.0));
        }
        parents.push(layer.id.0);
    }
    out
}

/// The first layer of `before` that `mine` does not accept and that `after` changes, removes,
/// moves or reorders: its id and name.
fn foreign_change(before: &photocraft_doc::Document, after: &photocraft_doc::Document, mine: &dyn Fn(u64) -> bool) -> Option<(u64, String)> {
    for (_, _, layer) in before.walk() {
        if !mine(layer.id.0) && !after.layer(layer.id).is_some_and(|now| same_layer(layer, now)) {
            return Some((layer.id.0, multisession::truncate(&layer.name)));
        }
    }
    let was = placement(before, mine);
    let now = placement(after, mine);
    let moved = was.iter().zip(&now).find(|(a, b)| a != b).map(|(a, _)| a.1)?;
    let name = before.layer(LayerId(moved)).map(|l| multisession::truncate(&l.name)).unwrap_or_default();
    Some((moved, name))
}

impl Session {
    /// Before a command of a named client runs: the state of the active document, put back by
    /// [`Self::keep_to_own_layers`] if the command changes a layer that client did not create.
    /// `None` without a named client or without a document.
    pub(crate) fn layer_guard(&self, client: Option<&str>) -> Option<crate::DocState> {
        client.filter(|c| !c.is_empty())?;
        self.active().cloned()
    }

    /// After a command of the named `client` succeeded: the layers it created are now its own;
    /// if it changed, removed or moved any other layer, the document goes back to `guard` (the
    /// state from [`Self::layer_guard`]) and the command fails, naming that layer. Nothing
    /// without a client, without a guard, or when the document is gone.
    pub(crate) fn keep_to_own_layers(&mut self, command: &str, client: Option<&str>, guard: Option<crate::DocState>) -> Result<()> {
        let (Some(client), Some(guard)) = (client.filter(|c| !c.is_empty()), guard) else {
            return Ok(());
        };
        let doc_id = guard.doc.id;
        let key = doc_id.0;
        let (foreign, created) = {
            let Some(st) = self.docs.iter().find(|st| st.doc.id == doc_id) else {
                return Ok(());
            };
            if std::sync::Arc::ptr_eq(&st.doc, &guard.doc) {
                return Ok(());
            }
            let before: std::collections::HashSet<u64> = guard.doc.walk().iter().map(|(_, _, l)| l.id.0).collect();
            let owners = &self.peers.layer_owners;
            let mine = |id: u64| !before.contains(&id) || owners.get(&(key, id)).is_some_and(|owner| owner == client);
            let foreign = foreign_change(&guard.doc, &st.doc, &mine);
            let created: Vec<u64> = st.doc.walk().iter().map(|(_, _, l)| l.id.0).filter(|id| !before.contains(id)).collect();
            (foreign, created)
        };
        let Some((layer, name)) = foreign else {
            for id in created {
                self.peers.layer_owners.insert((key, id), client.to_string());
            }
            return Ok(());
        };
        let owner = match self.peers.layer_owners.get(&(key, layer)) {
            Some(owner) => format!("created by `{owner}`"),
            None => "not created by a client".to_string(),
        };
        if let Some(st) = self.docs.iter_mut().find(|st| st.doc.id == doc_id) {
            *st = guard;
        }
        Err(EngineError::Other(format!(
            "`{command}` refused: client `{client}` may only change the layers it created, and this would change layer {layer} \"{name}\" ({owner}). Create a layer, or duplicate this one, and work on that; the document is unchanged"
        )))
    }

    /// `client.leave`: `client` is what [`take_client`] returned. Forgets the client and answers
    /// `{"left": name}`; without a client it is an error.
    pub(crate) fn leave_client(&mut self, client: Option<&str>) -> Result<Value> {
        match client {
            None => Err(EngineError::Other("client required".to_string())),
            Some(name) => {
                self.disconnect_client(name);
                Ok(json!({"left": name}))
            }
        }
    }

    /// `hub.probe`: notes the client (if any) and answers `{"ok": true}`.
    pub(crate) fn probe_client(&mut self, client: Option<&str>) -> Value {
        if let Some(name) = client {
            self.note_peer(name);
        }
        json!({"ok": true})
    }

    /// Note a call from `client`; the first time it is seen, publish `peer.joined`.
    pub(crate) fn note_peer(&mut self, client: &str) {
        if self.peers.seen(client) {
            let doc = self.active().map(|st| st.doc.id.0);
            self.events.push("peer.joined", doc, Some(client), json!({"agent": client}));
        }
    }

    /// The layer selection of the active document as a value (active, selected and anchor layer
    /// ids), or `None` when no document is open.
    pub(crate) fn selection_value(&self) -> Option<Value> {
        let st = self.active()?;
        Some(json!({
            "active": st.active_layer.map(|id| id.0),
            "selected": st.selected_layers.iter().map(|id| id.0).collect::<Vec<u64>>(),
            "anchor": st.layer_anchor.map(|id| id.0),
        }))
    }

    /// Put a selection value (see [`Self::selection_value`]) on the active document. Missing or
    /// invalid fields mean an empty selection; if the restore fails (an active layer that is
    /// gone), the empty selection is put back instead.
    fn pose_selection(&mut self, value: &Value) {
        let active = value.get("active").and_then(Value::as_u64).map(LayerId);
        let anchor = value.get("anchor").and_then(Value::as_u64).map(LayerId);
        let selected: Vec<LayerId> =
            value.get("selected").and_then(Value::as_array).map(|ids| ids.iter().filter_map(Value::as_u64).map(LayerId).collect()).unwrap_or_default();
        // Restoring bumps the revision and redraws: only do it when the selection really differs.
        if self.selection_is(active, &selected, anchor) {
            return;
        }
        if self.restore_layer_selection(active, &selected, anchor).is_err() {
            let _ = self.restore_layer_selection(None, &[], None);
        }
    }

    /// Whether the active document already has this selection (the ids that exist, as
    /// `restore_layer_selection` keeps them). False without a document, or when the active layer is gone.
    fn selection_is(&self, active: Option<LayerId>, selected: &[LayerId], anchor: Option<LayerId>) -> bool {
        let Some(st) = self.active() else {
            return false;
        };
        if active.is_some_and(|id| st.doc.layer(id).is_none()) {
            return false;
        }
        let kept: Vec<LayerId> = selected.iter().copied().filter(|id| st.doc.layer(*id).is_some()).collect();
        st.active_layer == active && st.selected_layers == kept && st.layer_anchor == anchor.filter(|_| active.is_some())
    }

    /// The selection a slot starts with when it was never stored: no layer selected, but the active
    /// layer and the anchor the document has now. Each slot then keeps its own active layer.
    fn fresh_selection(&self) -> Value {
        let (active, anchor) = self.active().map(|st| (st.active_layer, st.layer_anchor)).unwrap_or((None, None));
        json!({
            "active": active.map(|id| id.0),
            "selected": Vec::<u64>::new(),
            "anchor": anchor.map(|id| id.0),
        })
    }

    /// Before a command runs: the document takes the selection of the command's slot (its client,
    /// or "" without a client). The selection currently in the document is first kept under the
    /// slot it belongs to. A document other than the one the slots were kept for resets them: the
    /// selection it has is the common slot's (a call without a client leaves it as it is), and
    /// every named slot starts with `fresh_selection`. Nothing without a document.
    pub(crate) fn enter_selection_slot(&mut self, slot: &str) {
        let Some(doc) = self.active().map(|st| st.doc.id) else {
            return;
        };
        if self.peers.doc != Some(doc) {
            self.peers.doc = Some(doc);
            self.peers.forget_selections();
            self.peers.current = String::new();
        }
        if self.peers.current == slot {
            return;
        }
        let Some(current_value) = self.selection_value() else {
            return;
        };
        let previous = self.peers.current.clone();
        self.peers.set_selection(&previous, current_value);
        let next = self.peers.selection(slot).unwrap_or_else(|| self.fresh_selection());
        self.pose_selection(&next);
        self.peers.current = slot.to_string();
    }

    /// After a command from `client` ("" for none): keep the document's selection under that
    /// slot, if the document is still the one the slots belong to. When a named client's
    /// selection changed, publish `peer.selection` (source: the client).
    pub(crate) fn store_selection_slot(&mut self, client: &str) {
        let Some(doc) = self.active().map(|st| st.doc.id) else {
            return;
        };
        if self.peers.doc != Some(doc) {
            return;
        }
        let Some(value) = self.selection_value() else {
            return;
        };
        if !self.peers.set_selection(client, value) || client.is_empty() {
            return;
        }
        let selection: Vec<u64> = self.active().map(|st| st.selected_layers.iter().map(|id| id.0).collect()).unwrap_or_default();
        self.events.push("peer.selection", Some(doc.0), Some(client), json!({"agent": client, "page": Value::Null, "selection": selection}));
    }

    /// A client leaves: forget it, publish `peer.left` (only if it was present), and give the
    /// document the common selection back if it held the leaving client's.
    pub(crate) fn disconnect_client(&mut self, client: &str) {
        let doc = self.active().map(|st| st.doc.id);
        let was_current = self.peers.current == client && self.peers.doc == doc;
        if self.peers.leave(client) {
            self.events.push("peer.left", doc.map(|d| d.0), Some(client), json!({"agent": client}));
        }
        if was_current {
            let common = self.peers.selection("").unwrap_or_else(|| self.fresh_selection());
            self.pose_selection(&common);
        }
    }

    /// Before a command from the named `client`: the document takes that client's pixel selection
    /// (`Document.selection`; none for a new client) and the interface's selection is kept under
    /// the slot "". Call after `enter_selection_slot`, and only for the outermost call. Nothing
    /// for an empty client or without a document.
    pub(crate) fn enter_pixel_slot(&mut self, client: &str) {
        if client.is_empty() {
            return;
        }
        if self.active().is_none() {
            return;
        }
        let incoming = self.peers.pixel_selections.remove(client).flatten();
        let Some(st) = self.active_mut() else {
            return;
        };
        if st.doc.selection.is_none() && incoming.is_none() {
            // Nothing to swap: avoid copying a shared document.
            self.peers.pixel_selections.insert(String::new(), None);
            return;
        }
        let old = std::sync::Arc::downgrade(&st.doc);
        let doc = std::sync::Arc::make_mut(&mut st.doc);
        let interface = std::mem::replace(&mut doc.selection, incoming);
        let new = std::sync::Arc::downgrade(&st.doc);
        self.jobs.follow_selection_swap(&old, &new);
        self.peers.pixel_selections.insert(String::new(), interface);
    }

    /// After a command from the named `client` (Ok or Err): keep the document's pixel selection in
    /// the client's slot and give the interface's selection back. Call before
    /// `store_selection_slot`. If the active document is no longer the one of `peers.doc`, nothing
    /// is written to the client's slot and the interface's selection goes back to `peers.doc`
    /// when it still exists, else it is dropped.
    pub(crate) fn exit_pixel_slot(&mut self, client: &str) {
        if client.is_empty() {
            return;
        }
        let Some(doc_id) = self.active().map(|st| st.doc.id) else {
            return;
        };
        let interface = self.peers.pixel_selections.remove("").flatten();
        if self.peers.doc == Some(doc_id) {
            let Some(st) = self.active_mut() else {
                return;
            };
            if st.doc.selection.is_none() && interface.is_none() {
                self.peers.pixel_selections.insert(client.to_string(), None);
                return;
            }
            let old = std::sync::Arc::downgrade(&st.doc);
            let doc = std::sync::Arc::make_mut(&mut st.doc);
            let mine = std::mem::replace(&mut doc.selection, interface);
            let new = std::sync::Arc::downgrade(&st.doc);
            self.jobs.follow_selection_swap(&old, &new);
            self.peers.pixel_selections.insert(client.to_string(), mine);
            return;
        }
        let Some(home) = self.peers.doc else {
            return;
        };
        if let Some(st) = self.docs.iter_mut().find(|st| st.doc.id == home) {
            let old = std::sync::Arc::downgrade(&st.doc);
            std::sync::Arc::make_mut(&mut st.doc).selection = interface;
            let new = std::sync::Arc::downgrade(&st.doc);
            self.jobs.follow_selection_swap(&old, &new);
        }
    }
}
