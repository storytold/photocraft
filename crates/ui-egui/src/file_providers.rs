//! Optional external document storage, supplied by the page hosting the web editor.
//!
//! A provider is a trusted same-origin capability. It receives immutable captures and may
//! acknowledge only requests the editor created. Resource IDs are never filesystem paths.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use photocraft_doc::DocId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{FileDialogAnswer, FileDialogReply, PhotocraftApp};

const MAX_BYTES: usize = 80 * 1024 * 1024;
const MAX_PENDING: usize = 16;
const MAX_EVENTS: usize = 64;
const MAX_RESOURCES: usize = 100;

/// One provider-owned entry. Its identity is independent of its display name.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resource {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub updated_at: String,
}

/// An owned event for the web adapter. Taking it transfers its captured bytes, not a live view.
pub struct Event {
    pub metadata: Value,
    pub bytes: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, Eq)]
struct Identity {
    provider_id: String,
    label: String,
    namespace: String,
}

struct Save {
    doc: DocId,
    revision: u64,
    path: String,
    warnings: Vec<String>,
    copy: bool,
    destination: String,
    /// This capture predates an overwrite of its document's former provider checkpoint.
    /// Keep the operation busy until it settles, but it cannot restore that stale binding.
    invalidated_resource: Option<String>,
}

#[derive(Clone)]
struct Opening {
    request: String,
    resource: String,
    name: String,
}

enum Command {
    Result(String, String, String),
    Open(Opening, Vec<u8>),
}

#[derive(Default)]
struct State {
    provider: Option<Identity>,
    connection_id: String,
    enabled: bool,
    nonce: String,
    serial: u64,
    pending: HashMap<String, Save>,
    events: Vec<Event>,
    commands: Vec<Command>,
    resources: Vec<Resource>,
    bindings: HashMap<DocId, String>,
    provider_names: HashMap<DocId, String>,
    provider_revisions: HashMap<DocId, u64>,
    requested_opens: HashMap<String, String>,
    opening: Option<Opening>,
    opening_docs: HashMap<DocId, Opening>,
    intents: HashMap<DocId, String>,
    destinations: HashMap<DocId, String>,
    resolved_requests: HashMap<DocId, String>,
    dialog_doc: Option<DocId>,
    dialogs: HashMap<String, (Option<DocId>, FileDialogReply)>,
    snapshot: Value,
}

impl State {
    fn saving(&self) -> bool {
        !self.pending.is_empty()
            || !self.dialogs.is_empty()
            || !self.destinations.is_empty()
            || self.commands.iter().any(|command| matches!(command, Command::Result(..)))
    }

    fn busy(&self) -> bool {
        !self.pending.is_empty()
            || !self.dialogs.is_empty()
            || !self.destinations.is_empty()
            || !self.commands.is_empty()
            || !self.requested_opens.is_empty()
            || self.opening.is_some()
            || !self.opening_docs.is_empty()
    }

    fn next_request(&mut self, kind: &str) -> Result<String, String> {
        if self.nonce.is_empty() {
            return Err("file provider is not connected".into());
        }
        self.serial = self.serial.checked_add(1).ok_or("file provider request counter exhausted")?;
        Ok(format!("{}-{kind}-{}", self.nonce, self.serial))
    }

    fn event(&mut self, mut metadata: Value, bytes: Option<Vec<u8>>) {
        if let (Some(identity), Some(fields)) = (&self.provider, metadata.as_object_mut()) {
            fields.insert("providerId".into(), identity.provider_id.clone().into());
            fields.insert("namespace".into(), identity.namespace.clone().into());
            fields.insert("connectionId".into(), self.connection_id.clone().into());
        }
        self.events.push(Event { metadata, bytes });
    }

    fn event_bytes(&self) -> usize {
        self.events.iter().fold(0usize, |sum, e| sum.saturating_add(e.bytes.as_ref().map_or(0, Vec::len)))
    }
}

/// A single explicitly registered provider. The default has no storage authority or UI.
#[derive(Clone, Default)]
pub struct Host(Arc<Mutex<State>>);

impl Host {
    fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A per-WASM-instance cryptographic nonce, supplied by the platform shell once at startup.
    pub fn set_session(&self, nonce: String) -> Result<(), String> {
        let mut s = self.state();
        if s.provider.is_some() || !s.nonce.is_empty() || !valid_id(&nonce) || nonce.len() < 16 {
            return Err("invalid or already initialized file provider session".into());
        }
        s.nonce = nonce;
        Ok(())
    }

    /// Register one provider/account namespace. Re-registering that exact identity is idempotent.
    pub fn connect(&self, provider_id: String, label: String, namespace: String) -> Result<String, String> {
        if !valid_id(&provider_id) || !valid_label(&label) || !valid_id(&namespace) {
            return Err("invalid file provider identity".into());
        }
        let mut s = self.state();
        if s.nonce.is_empty() {
            return Err("file provider session is not initialized".into());
        }
        let next = Identity { provider_id, label, namespace };
        if let Some(current) = &s.provider {
            return if *current == next { Ok(s.connection_id.clone()) } else { Err("unregister the current file provider before changing identity".into()) };
        }
        let connection_id = s.next_request("connection")?;
        s.connection_id = connection_id.clone();
        s.provider = Some(next);
        s.enabled = true;
        s.snapshot = Value::Null;
        Ok(connection_id)
    }

    /// Unregister without losing an unresolved capture, dialog or open operation.
    pub fn disconnect(&self) -> Result<(), String> {
        let mut s = self.state();
        if s.busy() {
            return Err("finish or cancel pending file provider operations before unregistering".into());
        }
        s.provider = None;
        s.connection_id.clear();
        s.enabled = false;
        s.resources.clear();
        s.bindings.clear();
        s.provider_names.clear();
        s.provider_revisions.clear();
        s.events.clear();
        s.intents.clear();
        s.resolved_requests.clear();
        s.dialog_doc = None;
        s.snapshot = Value::Null;
        // Keep the nonce and monotonic counter, so delayed replies cannot match later requests.
        Ok(())
    }

    pub fn connected(&self) -> bool {
        self.state().provider.is_some()
    }

    pub fn enabled(&self) -> bool {
        let s = self.state();
        s.provider.is_some() && s.enabled
    }

    /// Change availability without discarding an in-flight operation or its document binding.
    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        let mut s = self.state();
        if s.provider.is_none() {
            return Err("file provider is not connected".into());
        }
        s.enabled = enabled;
        Ok(())
    }

    pub(crate) fn connection_id(&self) -> Option<String> {
        let s = self.state();
        s.provider.as_ref().map(|_| s.connection_id.clone())
    }

    pub(crate) fn acknowledged_at(&self, doc: DocId, revision: u64) -> bool {
        let s = self.state();
        s.bindings.contains_key(&doc) && s.provider_revisions.get(&doc) == Some(&revision)
    }

    pub fn busy(&self) -> bool {
        self.state().busy()
    }

    /// Save side effects must settle before closing. Reads do not lock unrelated discard UI.
    pub fn saving(&self) -> bool {
        self.state().saving()
    }

    pub fn pending_for(&self, doc: DocId) -> bool {
        let s = self.state();
        s.pending.values().any(|p| p.doc == doc) || s.dialogs.values().any(|(d, _)| *d == Some(doc))
    }

    pub fn bound(&self, doc: DocId) -> bool {
        self.state().bindings.contains_key(&doc)
    }

    pub(crate) fn bound_filename(&self, doc: DocId) -> Option<String> {
        let s = self.state();
        s.bindings.get(&doc).and_then(|_| s.provider_names.get(&doc).cloned())
    }

    pub fn prepared_for(&self, doc: DocId) -> bool {
        self.state().destinations.contains_key(&doc)
    }

    pub(crate) fn prepared_device(&self, doc: DocId) -> bool {
        self.state().destinations.get(&doc).is_some_and(|destination| destination == "device")
    }

    pub fn unchanged_provider_save(&self, doc: DocId, revision: u64, path: &str) -> bool {
        let s = self.state();
        s.destinations.get(&doc).is_some_and(|value| value == "provider")
            && s.intents.get(&doc).is_some_and(|value| value == "save")
            && s.bindings.contains_key(&doc)
            && s.provider_names.get(&doc).is_some_and(|name| name == path)
            && s.provider_revisions.get(&doc) == Some(&revision)
    }

    pub fn finish_unchanged(&self, doc: DocId, revision: u64, path: &str) {
        let mut s = self.state();
        if let (Some(request), Some(resource)) = (s.resolved_requests.remove(&doc), s.bindings.get(&doc).cloned()) {
            s.event(json!({"type":"saveUnchanged","requestId":request,"documentId":doc.0.to_string(),"revision":revision.to_string(),"resourceId":resource,"filename":path}), None);
        }
        s.intents.remove(&doc);
        s.destinations.remove(&doc);
    }

    pub fn ensure_intent(&self, doc: DocId, intent: &str) {
        if matches!(intent, "save" | "saveAs") {
            self.state().intents.entry(doc).or_insert_with(|| intent.into());
        }
    }

    pub fn intent(&self, doc: DocId, intent: &str) {
        if matches!(intent, "save" | "saveAs" | "copy") {
            self.state().intents.insert(doc, intent.into());
        }
    }

    pub fn clear_intent(&self, doc: DocId) {
        let mut s = self.state();
        s.intents.remove(&doc);
        s.destinations.remove(&doc);
        s.resolved_requests.remove(&doc);
    }

    pub fn prepare_dialog(&self, doc: Option<DocId>) {
        self.state().dialog_doc = doc;
    }

    pub fn wants_dialog(&self) -> bool {
        let s = self.state();
        s.provider.is_some() && s.enabled && s.dialog_doc.and_then(|doc| s.intents.get(&doc)).is_some_and(|intent| matches!(intent.as_str(), "save" | "saveAs"))
    }

    pub fn dialog(&self, suggested: String, reply: FileDialogReply) {
        let mut s = self.state();
        let doc = s.dialog_doc.take();
        let request = if s.provider.is_none() || !valid_name(&suggested) || s.dialogs.len() >= MAX_PENDING || s.events.len() >= MAX_EVENTS {
            Err("file provider save dialog is unavailable or busy".to_string())
        } else {
            s.next_request("dialog")
        };
        let Ok(request) = request else {
            reply.send(Some(FileDialogAnswer::Unavailable("file provider save dialog is unavailable or busy".into())));
            return;
        };
        let intent = doc.and_then(|d| s.intents.get(&d).cloned()).unwrap_or_else(|| "saveAs".into());
        s.event(json!({"type":"saveDialog","requestId":request,"documentId":doc.map(|d|d.0.to_string()),"intent":intent,"filename":suggested}), None);
        s.dialogs.insert(request, (doc, reply));
    }

    pub fn resolve_dialog(&self, request: &str, destination: &str, name: &str) -> Result<(), String> {
        if !matches!(destination, "provider" | "device" | "cancel") || (destination != "cancel" && !valid_name(name)) {
            return Err("invalid file provider save dialog answer".into());
        }
        let mut s = self.state();
        let (doc, reply) = s.dialogs.remove(request).ok_or("unknown file provider save dialog")?;
        if destination == "cancel" {
            if let Some(doc) = doc {
                s.intents.remove(&doc);
                s.destinations.remove(&doc);
                s.resolved_requests.remove(&doc);
            }
            reply.send(None);
        } else {
            if let Some(doc) = doc {
                s.destinations.insert(doc, destination.into());
                s.resolved_requests.insert(doc, request.into());
            }
            reply.send(Some(FileDialogAnswer::SaveTo(name.into())));
        }
        Ok(())
    }

    pub fn set_resources(&self, connection_id: &str, resources: Vec<Resource>) -> Result<(), String> {
        let mut seen = HashSet::new();
        if resources.len() > MAX_RESOURCES
            || resources.iter().any(|r| !valid_id(&r.id) || !valid_name(&r.name) || r.updated_at.len() > 100 || !seen.insert(r.id.clone()))
        {
            return Err("invalid file provider resource list".into());
        }
        let mut s = self.state();
        if s.provider.is_none() || s.connection_id != connection_id {
            return Err("the file provider resource list belongs to another connection".into());
        }
        s.resources = resources;
        Ok(())
    }

    pub fn open(&self, request: String, name: String, bytes: Vec<u8>) -> Result<(), String> {
        if !valid_name(&name) || bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err("invalid file provider resource".into());
        }
        let mut s = self.state();
        let queued = s.commands.iter().fold(0usize, |sum, command| match command {
            Command::Open(_, bytes) => sum.saturating_add(bytes.len()),
            Command::Result(..) => sum,
        });
        if s.provider.is_none() || s.commands.len() >= MAX_PENDING || s.opening_docs.len() >= MAX_PENDING || queued.saturating_add(bytes.len()) > MAX_BYTES {
            return Err("file provider is unavailable or busy".into());
        }
        let resource = s.requested_opens.remove(&request).ok_or("unknown or already completed file provider open request")?;
        s.commands.push(Command::Open(Opening { request, resource, name }, bytes));
        Ok(())
    }

    /// Retire a requested fetch that did not produce bytes. Empty `error` means user cancellation.
    pub fn cancel_open(&self, request: String, error: String) -> Result<(), String> {
        if error.len() > 1024 {
            return Err("invalid file provider open result".into());
        }
        let mut s = self.state();
        if s.events.len() >= MAX_EVENTS {
            return Err("file provider event queue is full".into());
        }
        let resource = s.requested_opens.remove(&request).ok_or("unknown or already completed file provider open request")?;
        let event = if error.is_empty() {
            json!({"type":"openCancelled","requestId":request,"resourceId":resource})
        } else {
            json!({"type":"openFailed","requestId":request,"resourceId":resource,"error":error})
        };
        s.event(event, None);
        Ok(())
    }

    /// The request owns its document and revision. A reply cannot select a different checkpoint.
    pub fn result(&self, request: String, status: String, resource: String) -> Result<(), String> {
        if !matches!(status.as_str(), "saved" | "failed" | "download" | "cancel")
            || (status == "saved" && !valid_id(&resource))
            || (!resource.is_empty() && !valid_id(&resource))
        {
            return Err("invalid file provider save result".into());
        }
        let mut s = self.state();
        if s.provider.is_none() || !s.pending.contains_key(&request) || s.commands.len() >= MAX_PENDING {
            return Err("unknown or already completed file provider save request".into());
        }
        if s.commands.iter().any(|command| matches!(command, Command::Result(queued, _, _) if *queued == request)) {
            return Err("the file provider save result is already queued".into());
        }
        if status == "saved" && !s.pending.get(&request).is_some_and(|p| p.destination == "provider") {
            return Err("a device download cannot acknowledge provider persistence".into());
        }
        if status == "download" && !s.pending.get(&request).is_some_and(|p| p.destination == "device") {
            return Err("a provider save cannot be reported as a device download".into());
        }
        s.commands.push(Command::Result(request, status, resource));
        Ok(())
    }

    pub(crate) fn begin(&self, doc: DocId, revision: u64, path: &str, bytes: Vec<u8>, warnings: Vec<String>, copy: bool) -> Result<(), String> {
        let mut s = self.state();
        if s.provider.is_none()
            || (!s.enabled && !s.destinations.contains_key(&doc))
            || !valid_name(path)
            || bytes.is_empty()
            || bytes.len() > MAX_BYTES
            || s.pending.len() >= MAX_PENDING
            || s.pending.values().any(|p| p.doc == doc)
            || s.events.len() >= MAX_EVENTS
            || s.event_bytes().saturating_add(bytes.len()) > MAX_BYTES
        {
            return Err("a save is pending, the provider is unavailable, or the capture exceeds 80 MiB".into());
        }
        let request = s.next_request("save")?;
        let requested_intent = s.intents.remove(&doc).unwrap_or_else(|| "save".into());
        let intent = if copy { "copy".into() } else { requested_intent };
        let destination = s.destinations.remove(&doc).unwrap_or_else(|| "provider".into());
        s.resolved_requests.remove(&doc);
        let resource = s.bindings.get(&doc).cloned();
        s.event(json!({"type":"save","requestId":request,"documentId":doc.0.to_string(),"revision":revision.to_string(),"intent":intent,"destination":destination,"resourceId":resource,"filename":path,"copy":copy}), Some(bytes));
        s.pending.insert(request, Save { doc, revision, path: path.into(), warnings, copy, destination, invalidated_resource: None });
        Ok(())
    }

    pub(crate) fn opening_document(&self, doc: DocId) {
        let mut s = self.state();
        if let Some(resource) = s.opening.take() {
            s.opening_docs.insert(doc, resource);
        }
    }

    /// Admission may remap a decoded file's ID to avoid another open document's identity.
    pub(crate) fn remap_opening_document(&self, from: DocId, to: DocId) {
        if from == to {
            return;
        }
        let mut s = self.state();
        if let Some(opening) = s.opening_docs.remove(&from) {
            let rejected = match s.opening_docs.entry(to) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(opening);
                    None
                }
                std::collections::hash_map::Entry::Occupied(_) => Some(opening),
            };
            if let Some(opening) = rejected {
                s.event(json!({"type":"openFailed","requestId":opening.request,"resourceId":opening.resource,"error":"the admitted document identity is already opening"}), None);
            }
        }
    }

    /// A profile/damage prompt cancelled or rejected a particular decoded document.
    pub(crate) fn abandon_opening_document(&self, doc: DocId, error: Option<&str>) {
        let mut s = self.state();
        let Some(opening) = s.opening_docs.remove(&doc) else { return };
        let event = match error {
            Some(error) => json!({"type":"openFailed","requestId":opening.request,"resourceId":opening.resource,"error":bounded_error(error)}),
            None => json!({"type":"openCancelled","requestId":opening.request,"resourceId":opening.resource}),
        };
        s.event(event, None);
    }

    pub(crate) fn resources(&self) -> Vec<Resource> {
        self.state().resources.clone()
    }

    pub fn provider_label(&self) -> Option<String> {
        self.state().provider.as_ref().map(|identity| identity.label.clone())
    }

    pub fn request_open(&self, id: &str) -> Result<String, String> {
        if !valid_id(id) {
            return Err("invalid file provider resource ID".into());
        }
        let mut s = self.state();
        if s.provider.is_none() || !s.enabled {
            return Err("file provider is disabled".into());
        }
        if s.requested_opens.len() >= MAX_PENDING || s.events.len() >= MAX_EVENTS || s.requested_opens.values().any(|resource| resource == id) {
            return Err("the file provider resource is already opening or the provider is busy".into());
        }
        let request = s.next_request("open")?;
        let name = s.resources.iter().find(|r| r.id == id).map(|r| r.name.clone());
        s.requested_opens.insert(request.clone(), id.into());
        s.event(json!({"type":"openRequest","requestId":request,"resourceId":id,"filename":name}), None);
        Ok(request)
    }

    pub(crate) fn request_recent(&self, id: &str) -> Result<(), String> {
        if !self.state().resources.iter().any(|resource| resource.id == id) {
            return Err("the file provider resource is no longer in Recents".into());
        }
        self.request_open(id).map(|_| ())
    }

    pub fn snapshot(&self) -> Value {
        let s = self.state();
        let mut snapshot = if s.snapshot.is_object() { s.snapshot.clone() } else { json!({"activeDocumentId":null,"documents":[]}) };
        if let Some(fields) = snapshot.as_object_mut() {
            fields.insert("protocolVersion".into(), 1.into());
            fields.insert("providerEnabled".into(), (s.provider.is_some() && s.enabled).into());
            fields.insert("providerId".into(), s.provider.as_ref().map(|i| i.provider_id.clone()).into());
            fields.insert("providerLabel".into(), s.provider.as_ref().map(|i| i.label.clone()).into());
            fields.insert("namespace".into(), s.provider.as_ref().map(|i| i.namespace.clone()).into());
            fields.insert("connectionId".into(), s.provider.as_ref().map(|_| s.connection_id.clone()).into());
        }
        snapshot
    }

    pub fn take_events(&self) -> Vec<Event> {
        std::mem::take(&mut self.state().events)
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 160 && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn valid_name(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 255 && !value.chars().any(|c| c.is_control() || matches!(c, '/' | '\\')) && !matches!(value, "." | "..")
}

fn valid_label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 100 && !value.chars().any(char::is_control)
}

fn bounded_error(value: &str) -> String {
    value
        .chars()
        .scan(0usize, |bytes, ch| {
            *bytes = bytes.saturating_add(ch.len_utf8());
            (*bytes <= 1024).then_some(ch)
        })
        .collect()
}

/// Apply queued provider replies on the application thread, then publish a fresh snapshot.
pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(host) = app.services.file_provider.clone().filter(Host::connected) else { return };
    let commands = std::mem::take(&mut host.state().commands);
    for command in commands {
        match command {
            Command::Result(request, status, resource) => apply_result(app, ctx, &host, &request, &status, resource),
            Command::Open(opening, bytes) => apply_open(app, ctx, &host, opening, bytes),
        }
    }
    finish_opens(app, &host);
    capture(app);
}

fn apply_result(app: &mut PhotocraftApp, ctx: &egui::Context, host: &Host, request: &str, status: &str, resource: String) {
    if status == "failed" {
        let device = host.state().pending.get(request).is_some_and(|save| save.destination == "device");
        app.ui.status =
            crate::i18n::t(if device { "The device download failed; you can retry." } else { "The file provider save failed; your changes remain unsaved." })
                .into();
        app.ui.status_error = true;
        return;
    }
    let Some(save) = host.state().pending.remove(request) else { return };
    if save.invalidated_resource.as_ref().is_some_and(|old| (status == "saved" && old == &resource) || status == "download") {
        // A newer write already retired this capture's checkpoint authority. A completed
        // provider write may still have replaced the resource, so invalidate its current
        // aliases too, while never cleaning/rebinding this stale sender. A device download
        // has no provider side effect; a Save As to another resource remains independent.
        if status == "saved" && save.destination == "provider" {
            invalidate_resource_aliases(app, host, &resource, None);
        }
        app.ui.status = crate::i18n::t("The provider resource changed while this save was pending; save the document again to keep its changes.").into();
        app.ui.status_error = true;
        return;
    }
    if status == "saved" && save.destination == "provider" {
        // A stable resource can be open in several tabs. Replacing it invalidates the other
        // tabs' old checkpoints, even when this write was only a copy of the current tab.
        invalidate_resource_aliases(app, host, &resource, (!save.copy).then_some(save.doc));
        if app.session.documents().iter().any(|d| d.doc.id == save.doc) {
            // The captured revision, never the current one, is the only checkpoint this reply owns.
            app.saved_provider(save.doc, save.revision, &save.path, &save.warnings, save.copy);
            if !save.copy {
                {
                    let mut s = host.state();
                    s.provider_names.insert(save.doc, save.path);
                    s.provider_revisions.insert(save.doc, save.revision);
                    s.bindings.insert(save.doc, resource);
                }
                crate::discard_ui::provider_saved_document(app, ctx, host, save.doc, save.revision, true);
            }
        }
    } else if status == "download" && save.destination == "device" {
        app.ui.status_error = false;
        app.ui.status = crate::i18n::t("Download requested; the browser does not confirm that it was saved.").into();
        if !save.copy && app.session.documents().iter().any(|doc| doc.doc.id == save.doc && doc.revision == save.revision) {
            crate::discard_ui::provider_saved_document(app, ctx, host, save.doc, save.revision, false);
        }
    } else if status == "cancel" {
        app.ui.status_error = false;
        app.ui.status = crate::i18n::t("Save cancelled; the document keeps its unsaved state.").into();
    }
}

fn invalidate_resource_aliases(app: &mut PhotocraftApp, host: &Host, resource: &str, saver: Option<DocId>) {
    let aliases: Vec<_> = {
        let mut s = host.state();
        let aliases: Vec<_> = s.bindings.iter().filter(|(doc, bound)| Some(**doc) != saver && bound.as_str() == resource).map(|(doc, _)| *doc).collect();
        for save in s.pending.values_mut().filter(|save| aliases.contains(&save.doc)) {
            save.invalidated_resource = Some(resource.to_string());
        }
        for doc in &aliases {
            s.bindings.remove(doc);
            s.provider_names.remove(doc);
            s.provider_revisions.remove(doc);
        }
        aliases
    };
    for doc in aliases {
        let _ = app.with_document(doc, |app| {
            if let Some(state) = app.session.active_mut()
                && state.path.is_none()
            {
                // A separately saved physical file remains durable. Only a document whose
                // sole saved copy was this replaced provider resource becomes unsaved.
                state.saved_revision = u64::MAX;
            }
            Ok(())
        });
    }
}

fn open_would_retarget_ui(app: &PhotocraftApp, ctx: &egui::Context) -> bool {
    // Delivery runs in logic, before this frame's widgets can commit their input. Reuse the
    // close-continuation guard for document-owned drafts instead of switching their target.
    app.file_dialog_open()
        || app.save_options_open()
        || app.discard.is_some()
        || !app.ui.dialogs.is_empty()
        || app.filter_preview.is_some()
        || !app.remove_pending.is_empty()
        || ctx.text_edit_focused()
        || ctx.input(|input| {
            // Motion/leave alone only changes hover. A held button or document-owned gesture
            // is still protected here and by provider_ui_work, even without another press.
            input.events.iter().any(|event| !matches!(event, egui::Event::PointerMoved(_) | egui::Event::MouseMoved(_) | egui::Event::PointerGone))
                || !input.raw.dropped_files.is_empty()
                || input.pointer.any_down()
                || !input.keys_down.is_empty()
        })
        || app.synthetic_batch.is_some()
        || !app.synthetic.is_empty()
        || app.session.documents().iter().any(|state| crate::discard_ui::provider_ui_work(app, ctx, state.doc.id))
}

fn apply_open(app: &mut PhotocraftApp, ctx: &egui::Context, host: &Host, opening: Opening, bytes: Vec<u8>) {
    if open_would_retarget_ui(app, ctx) {
        let error = crate::i18n::t("Finish or cancel the current edit before opening a provider file.");
        app.ui.status = error.into();
        app.ui.status_error = true;
        host.state().event(json!({"type":"openFailed","requestId":opening.request,"resourceId":opening.resource,"error":error}), None);
        return;
    }
    host.state().opening = Some(opening.clone());
    // The web decoder is synchronous. Keep this identity-correlated path synchronous in native
    // fixtures too; ordinary desktop opens retain their background-job behaviour.
    let background_jobs = app.background_jobs;
    app.background_jobs = false;
    let result = app.open_bytes(&opening.name, &bytes);
    app.background_jobs = background_jobs;
    if let Err(error) = result {
        app.open_failed(&opening.name, &error);
        host.state().event(json!({"type":"openFailed","requestId":opening.request,"resourceId":opening.resource,"error":bounded_error(&error)}), None);
    } else if host.state().opening.is_some() {
        host.state().event(
            json!({"type":"openFailed","requestId":opening.request,"resourceId":opening.resource,"error":"the resource did not decode as a document"}),
            None,
        );
    }
    host.state().opening = None;
}

fn finish_opens(app: &PhotocraftApp, host: &Host) {
    let mut s = host.state();
    let opening: Vec<_> = s.opening_docs.iter().map(|(doc, opening)| (*doc, opening.clone())).collect();
    for (doc, opening) in opening {
        if app.held_opens.iter().any(|held| held.doc.id == doc) {
            continue;
        }
        if let Some(state) = app.session.documents().iter().find(|state| state.doc.id == doc) {
            s.bindings.insert(doc, opening.resource.clone());
            s.provider_names.insert(doc, opening.name.clone());
            // Admission can be followed by an edit before this frame. Only the decoded file's
            // saved checkpoint came from the provider; the current revision may include new work.
            s.provider_revisions.insert(doc, state.saved_revision);
            s.event(
                json!({"type":"opened","requestId":opening.request,"resourceId":opening.resource,"documentId":doc.0.to_string(),"filename":opening.name}),
                None,
            );
            s.opening_docs.remove(&doc);
        } else if !app.held_opens.iter().any(|held| held.doc.id == doc) {
            s.event(json!({"type":"openCancelled","requestId":opening.request,"resourceId":opening.resource}), None);
            s.opening_docs.remove(&doc);
        }
    }
}

/// Refresh observable state after application/UI work without consuming replies a second time.
pub fn capture(app: &mut PhotocraftApp) {
    let Some(host) = app.services.file_provider.clone().filter(Host::connected) else { return };
    // A cancelled format prompt has no capture to finish. Retaining its earlier destination
    // would make the provider permanently busy and could reuse that choice on a later Save.
    let keep_prepared = app.file_dialog_open() || app.save_options_open();
    let mut s = host.state();
    let ids: Vec<_> = app.session.documents().iter().map(|doc| doc.doc.id).collect();
    s.bindings.retain(|doc, _| ids.contains(doc));
    s.provider_names.retain(|doc, _| ids.contains(doc));
    s.provider_revisions.retain(|doc, _| ids.contains(doc));
    s.intents.retain(|doc, _| ids.contains(doc));
    s.destinations.retain(|doc, _| ids.contains(doc));
    s.resolved_requests.retain(|doc, _| ids.contains(doc));
    s.pending.retain(|_, save| ids.contains(&save.doc));
    if !keep_prepared {
        let abandoned: Vec<_> = s.destinations.keys().copied().filter(|doc| !s.pending.values().any(|save| save.doc == *doc)).collect();
        for doc in abandoned {
            s.destinations.remove(&doc);
            s.resolved_requests.remove(&doc);
            s.intents.remove(&doc);
        }
    }
    s.snapshot = json!({"activeDocumentId":app.session.active().map(|doc|doc.doc.id.0.to_string()),"documents":app.session.documents().iter().map(|doc| {
        let saved = if s.enabled { s.provider_revisions.get(&doc.doc.id).copied().unwrap_or(doc.saved_revision) } else { doc.saved_revision };
        json!({"documentId":doc.doc.id.0.to_string(),"revision":doc.revision.to_string(),"savedRevision":saved.to_string(),"name":doc.doc.name,"dirty":doc.is_dirty() || (s.enabled && s.provider_revisions.get(&doc.doc.id).is_some_and(|saved|*saved!=doc.revision)),"resourceId":s.bindings.get(&doc.doc.id),"pendingRequestId":s.pending.iter().find(|(_,p)|p.doc==doc.doc.id).map(|(id,_)|id)})
    }).collect::<Vec<_>>()});
}

pub(crate) fn home_height(app: &PhotocraftApp) -> f32 {
    app.services.file_provider.as_ref().filter(|host| host.enabled()).map_or(0.0, |host| 45.0 + host.resources().len().min(6) as f32 * 30.0)
}

pub(crate) fn home_recent(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(host) = app.services.file_provider.clone().filter(Host::enabled) else { return };
    let label = host.provider_label().unwrap_or_default();
    ui.add_space(16.0);
    ui.label(egui::RichText::new(crate::i18n::fmt(crate::i18n::t("Recent files from {provider}"), &[("provider", &label)])).strong());
    let resources = host.resources();
    if resources.is_empty() {
        ui.label(crate::i18n::t("Files saved with this provider will appear here."));
    }
    egui::ScrollArea::vertical().id_salt("file_provider_recent").max_height(180.0).show(ui, |ui| {
        for resource in resources {
            if ui.add_sized([ui.available_width(), 28.0], egui::Button::new(&resource.name)).on_hover_text(&resource.updated_at).clicked()
                && let Err(error) = host.request_recent(&resource.id)
            {
                app.ui.status = error;
                app.ui.status_error = true;
            }
        }
    });
}

#[cfg(test)]
mod tests;
