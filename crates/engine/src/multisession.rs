//! Multi-session support shared by the Craft apps: client names and per-client peer state
//! (layer selections). The event journal itself lives in `events.rs`.

use std::collections::{BTreeSet, HashMap};

use serde_json::Value;

/// Longest text (in characters) kept in an event or echoed in an error.
pub const MAX_TEXT: usize = 256;

/// Most `elements` entries listed in one `document.changed` event.
pub const MAX_ELEMENTS: usize = 256;

/// The rule a `client` name must follow.
pub const CLIENT_RULE: &str = "`client` must be lowercase letters, digits and hyphens, 1 to 40 characters";

/// Check an optional `client` name. `None` is fine (no client); a bad name gives the exact error.
pub fn validate_client(client: Option<&str>) -> Result<Option<String>, String> {
    let Some(name) = client else { return Ok(None) };
    let len = name.chars().count();
    let ok = (1..=40).contains(&len) && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok { Ok(Some(name.to_string())) } else { Err(format!("{CLIENT_RULE}, got \"{}\"", truncate(name))) }
}

/// Cut a string to at most [`MAX_TEXT`] characters (not bytes).
pub fn truncate(text: &str) -> String {
    text.chars().take(MAX_TEXT).collect()
}

/// The named clients seen on a document and the selection of each.
#[derive(Default)]
pub struct Peers {
    pub(crate) names: BTreeSet<String>,
    pub(crate) selections: HashMap<String, Value>,
    pub(crate) ever_named: bool,
    /// The pixel selection (`Document.selection`) of each named client: one slot per client,
    /// key = the client's name. `None` means that client has no pixel selection. A client that
    /// leaves frees its slot, and a new document empties them all (they are per document).
    pub(crate) pixel_selections: HashMap<String, Option<photocraft_doc::Surface>>,
    /// The slot whose selection is in the document now: a client's name, or "" for a command
    /// without a client (the common selection). Empty at start.
    pub(crate) current: String,
    /// The document the stored selections belong to. Another document takes none of them.
    pub(crate) doc: Option<photocraft_doc::DocId>,
    /// The named client that created each layer, key = (document id, layer id). A named client
    /// may only change the layers it created; a layer without an entry was made without a client.
    pub(crate) layer_owners: HashMap<(u64, u64), String>,
}

impl Peers {
    /// No peers yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Note a call from `client`; true if it was not present (it just joined).
    pub fn seen(&mut self, client: &str) -> bool {
        self.ever_named = true;
        self.names.insert(client.to_string())
    }

    /// Forget `client`; true if it was present. Its selection is dropped, and if it was the slot
    /// in the document, the document's slot is the common one ("") again.
    pub fn leave(&mut self, client: &str) -> bool {
        self.selections.remove(client);
        self.pixel_selections.remove(client);
        if self.current == client {
            self.current.clear();
        }
        self.names.remove(client)
    }

    /// Drop every stored selection (a new document: none of them belongs to it).
    pub(crate) fn forget_selections(&mut self) {
        self.selections.clear();
        self.pixel_selections.clear();
    }

    /// Whether a named client has ever been seen on this document, even if it has left since.
    pub fn any_named(&self) -> bool {
        self.ever_named
    }

    /// Store the selection of `client`; true if it differs from the previous one.
    pub fn set_selection(&mut self, client: &str, selection: Value) -> bool {
        let empty = Value::Array(Vec::new());
        let old = self.selections.get(client).unwrap_or(&empty);
        if *old == selection {
            return false;
        }
        self.selections.insert(client.to_string(), selection);
        true
    }

    /// The stored selection of `client`, or `None` when its slot was never stored.
    pub fn selection(&self, client: &str) -> Option<Value> {
        self.selections.get(client).cloned()
    }

    /// Names of the clients currently present, sorted.
    pub fn names(&self) -> Vec<String> {
        self.names.iter().cloned().collect()
    }
}
