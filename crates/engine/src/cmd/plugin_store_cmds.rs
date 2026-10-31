//! `plugin.store.*` commands: Plugins › Browse Plugins…, a curated catalog of WebAssembly filter
//! plug-ins published on GitHub.
//!
//! The catalog (`plugin_store_cmds/catalog.json`) is bundled and pins every module by URL, size and
//! SHA-256, so a changed or substituted download is refused rather than installed. Downloads go
//! through a transport the frontend injects ([`FetchFn`]; the engine does no network I/O). A
//! verified module is installed in the process-wide registry and, when the frontend gave a folder
//! ([`PluginStore::set_dir`]), saved there as `<id>.wasm` so it loads again at the next start.
//! Installed modules still run in the plug-in sandbox (`photocraft-plugins`): no imports, capped
//! memory, fuel and time.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::sync::mpsc::{Receiver, TryRecvError};

use photocraft_plugins::{Limits, Plugin, registry};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::commands::{CommandSpec, always};
use crate::{EngineError, Result, Session};

pub type FetchResult = std::result::Result<Vec<u8>, String>;
/// Starts a download and returns at once; the answer arrives on the receiver.
pub type FetchFn = Box<dyn FnMut(Request) -> std::result::Result<Receiver<FetchResult>, String> + Send>;

/// What the transport is asked to fetch.
pub struct Request {
    pub url: String,
    /// The catalog's size: the transport refuses more.
    pub size: usize,
    /// The catalog's SHA-256 (the engine checks it again on arrival).
    pub sha256: String,
    pub progress: Arc<Progress>,
}

/// A download's progress, shared with the transport: it records the bytes it has, and stops (with
/// any answer) once [`cancelled`](Progress::cancelled).
#[derive(Debug, Default)]
pub struct Progress {
    received: AtomicU64,
    cancel: AtomicBool,
}

impl Progress {
    pub fn set_received(&self, bytes: u64) {
        self.received.store(bytes, Ordering::Relaxed);
    }

    pub fn received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Hosts a catalog URL may point at.
const HOSTS: &[&str] = &["github.com", "raw.githubusercontent.com", "akkk09.github.io"];
/// Downloads running at once; more wait their turn.
const MAX_RUNNING: usize = 4;

/// One catalog listing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    /// The GitHub project it comes from (one collection per repository).
    pub collection: String,
    pub repo: String,
    pub license: String,
    /// Why the listing gets no more updates, when its project was given up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<String>,
    pub url: String,
    pub sha256: String,
    pub size: usize,
}

#[derive(Deserialize)]
struct Catalog {
    plugins: Vec<Entry>,
}

/// Why an entry can't be offered, if it can't: a bad URL, checksum or size.
fn check_entry(e: &Entry) -> std::result::Result<(), String> {
    let rest = e.url.strip_prefix("https://").ok_or("not an https URL")?;
    let host = rest.split('/').next().unwrap_or_default();
    if !HOSTS.contains(&host) {
        return Err(format!("{host} is not an allowed host"));
    }
    if e.sha256.len() != 64 || !e.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("bad sha256".into());
    }
    if e.size == 0 || e.size > Limits::default().max_module_bytes {
        return Err("bad size".into());
    }
    Ok(())
}

/// The bundled catalog, listings that fail [`check_entry`] left out.
pub fn bundled() -> &'static [Entry] {
    static CATALOG: OnceLock<Vec<Entry>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str::<Catalog>(include_str!("plugin_store_cmds/catalog.json"))
            .map(|c| c.plugins.into_iter().filter(|e| check_entry(e).is_ok()).collect())
            .unwrap_or_default()
    })
}

/// Where a store download stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Job {
    Queued,
    Downloading,
    Failed { message: String },
}

/// Store state on the session: the catalog, the transport and the downloads in flight.
pub struct PluginStore {
    pub fetch: Option<FetchFn>,
    pub(crate) catalog: Vec<Entry>,
    #[cfg(not(target_arch = "wasm32"))]
    dir: Option<std::path::PathBuf>,
    jobs: BTreeMap<String, Job>,
    running: Vec<Running>,
}

/// A download in flight. One cancelled (or uninstalled) while it runs keeps its slot until the
/// transport answers, which it does at its next read, so cancelling and installing again can't
/// run more than [`MAX_RUNNING`] at once; its result is dropped.
struct Running {
    id: String,
    rx: Receiver<FetchResult>,
    progress: Arc<Progress>,
}

impl Default for PluginStore {
    fn default() -> Self {
        Self {
            fetch: None,
            catalog: bundled().to_vec(),
            #[cfg(not(target_arch = "wasm32"))]
            dir: None,
            jobs: BTreeMap::new(),
            running: Vec::new(),
        }
    }
}

impl PluginStore {
    /// Saves installs to `dir` from now on and installs the `*.wasm` already there. Native only;
    /// the web keeps installs for the session.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_dir(&mut self, dir: std::path::PathBuf) -> Value {
        let report = if dir.is_dir() {
            match registry::load_folder(&dir) {
                Ok(r) => json!({"loaded": r.loaded, "failed": r.failed.iter().map(|(f, e)| json!({"file": f, "error": e})).collect::<Vec<_>>()}),
                Err(e) => json!({"error": e.to_string()}),
            }
        } else {
            json!({"loaded": []})
        };
        self.dir = Some(dir);
        report
    }

    pub fn catalog(&self) -> &[Entry] {
        &self.catalog
    }

    /// Replaces the catalog (tests, mirrors); listings that fail validation are left out.
    pub fn set_catalog(&mut self, entries: Vec<Entry>) {
        self.catalog = entries.into_iter().filter(|e| check_entry(e).is_ok()).collect();
    }

    /// Whether this frontend can download plug-ins.
    pub fn available(&self) -> bool {
        self.fetch.is_some()
    }

    pub fn job(&self, id: &str) -> Option<&Job> {
        self.jobs.get(id)
    }

    /// Bytes received and expected, while `id` downloads.
    pub fn progress(&self, id: &str) -> Option<(u64, u64)> {
        let r = self.running.iter().find(|r| r.id == id && !r.progress.cancelled())?;
        let total = self.entry(id).map_or(0, |e| e.size as u64);
        Some((r.progress.received().min(total), total))
    }

    /// Stops `id`'s queued or running download. True when there was one.
    pub fn cancel(&mut self, id: &str) -> bool {
        let busy = matches!(self.jobs.get(id), Some(Job::Queued | Job::Downloading));
        if busy {
            self.jobs.remove(id);
        }
        for r in self.running.iter_mut().filter(|r| r.id == id) {
            r.progress.cancel();
        }
        busy
    }

    /// Whether a download is queued or running.
    pub fn busy(&self) -> bool {
        self.jobs.values().any(|j| matches!(j, Job::Queued | Job::Downloading))
    }

    fn entry(&self, id: &str) -> Option<&Entry> {
        self.catalog.iter().find(|e| e.id == id)
    }

    /// Finishes completed downloads and starts queued ones. True when something changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let mut k = 0;
        while let Some(Running { id, rx, progress }) = self.running.get(k) {
            let result = match rx.try_recv() {
                Ok(r) => r,
                Err(TryRecvError::Empty) => {
                    k += 1;
                    continue;
                }
                Err(TryRecvError::Disconnected) => Err("the download stopped before it finished".into()),
            };
            let (id, cancelled) = (id.clone(), progress.cancelled());
            self.running.remove(k);
            if cancelled {
                continue;
            }
            let done = match self.entry(&id).cloned() {
                Some(entry) => result.and_then(|bytes| self.finish(&entry, &bytes)),
                None => Err("no longer in the catalog".into()),
            };
            match done {
                Ok(()) => self.jobs.remove(&id),
                Err(message) => self.jobs.insert(id, Job::Failed { message }),
            };
            changed = true;
        }
        changed | self.start_queued()
    }

    fn start_queued(&mut self) -> bool {
        let mut changed = false;
        let queued: Vec<String> = self.jobs.iter().filter(|(_, j)| **j == Job::Queued).map(|(id, _)| id.clone()).collect();
        for id in queued {
            if self.running.len() >= MAX_RUNNING {
                break;
            }
            // A cancelled download of the same module is still answering: two would share its file.
            if self.running.iter().any(|r| r.id == id) {
                continue;
            }
            let Some(entry) = self.entry(&id).cloned() else {
                self.jobs.insert(id, Job::Failed { message: "no longer in the catalog".into() });
                continue;
            };
            let progress = Arc::new(Progress::default());
            let request = Request { url: entry.url.clone(), size: entry.size, sha256: entry.sha256.clone(), progress: progress.clone() };
            let started = match self.fetch.as_mut() {
                Some(fetch) => fetch(request),
                None => Err(UNAVAILABLE.into()),
            };
            match started {
                Ok(rx) => {
                    self.running.push(Running { id: id.clone(), rx, progress });
                    self.jobs.insert(id, Job::Downloading);
                }
                Err(message) => {
                    self.jobs.insert(id, Job::Failed { message });
                }
            }
            changed = true;
        }
        changed
    }

    /// Verifies downloaded bytes against the catalog, installs the module and saves it.
    fn finish(&self, entry: &Entry, bytes: &[u8]) -> std::result::Result<(), String> {
        let plugin = verify(entry, bytes)?;
        let plugin = match self.save(entry, bytes)? {
            Some(path) => plugin.with_source(path),
            None => plugin,
        };
        registry::install(plugin);
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save(&self, entry: &Entry, bytes: &[u8]) -> std::result::Result<Option<String>, String> {
        let Some(dir) = &self.dir else { return Ok(None) };
        std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {} ({e})", dir.display()))?;
        let path = file_for(dir, &entry.id);
        let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| format!("couldn't write in {} ({e})", dir.display()))?;
        std::io::Write::write_all(&mut tmp, bytes).map_err(|e| format!("couldn't write {} ({e})", path.display()))?;
        tmp.persist(&path).map_err(|e| format!("couldn't save {} ({e})", path.display()))?;
        Ok(Some(path.display().to_string()))
    }

    #[cfg(target_arch = "wasm32")]
    fn save(&self, _: &Entry, _: &[u8]) -> std::result::Result<Option<String>, String> {
        Ok(None)
    }

    /// Deletes the saved copy of `id`, if this store saved one.
    #[cfg(not(target_arch = "wasm32"))]
    fn delete(&self, id: &str) -> std::result::Result<bool, String> {
        let Some(dir) = &self.dir else { return Ok(false) };
        let path = file_for(dir, id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("couldn't delete {} ({e})", path.display())),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn delete(&self, _: &str) -> std::result::Result<bool, String> {
        Ok(false)
    }
}

/// `<dir>/<id>.wasm`. Manifest ids are `[A-Za-z0-9._-]`, so the name stays inside `dir`.
#[cfg(not(target_arch = "wasm32"))]
fn file_for(dir: &std::path::Path, id: &str) -> std::path::PathBuf {
    dir.join(format!("{id}.wasm"))
}

const UNAVAILABLE: &str = "Downloading plug-ins is not available in this frontend";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The module in `bytes`, if it is exactly the one the catalog lists.
pub fn verify(entry: &Entry, bytes: &[u8]) -> std::result::Result<Plugin, String> {
    if bytes.len() != entry.size {
        return Err(format!("the download is {} bytes, the catalog says {}", bytes.len(), entry.size));
    }
    let sum = hex(&Sha256::digest(bytes));
    if !sum.eq_ignore_ascii_case(&entry.sha256) {
        return Err("the download doesn't match the catalog's checksum (it changed since the catalog was made)".into());
    }
    let plugin = Plugin::load(bytes, Limits::default()).map_err(|e| e.to_string())?;
    if plugin.id() != entry.id {
        return Err(format!("the module calls itself {:?}, not {:?}", plugin.id(), entry.id));
    }
    // Otherwise it would install and then be offered as an update to itself, again and again.
    if plugin.manifest().version != entry.version {
        return Err(format!("the module is version {:?}, the catalog says {:?}", plugin.manifest().version, entry.version));
    }
    Ok(plugin)
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// A listing with its install state.
fn describe(store: &PluginStore, e: &Entry) -> Value {
    let installed = registry::get(&e.id).map(|p| p.manifest().version.clone());
    let mut v = json!({
        "id": e.id, "name": e.name, "version": e.version, "author": e.author, "description": e.description,
        "collection": e.collection, "repo": e.repo, "license": e.license, "deprecated": e.deprecated, "size": e.size,
        "installed": installed.is_some(), "installedVersion": installed,
        "updateAvailable": installed.as_ref().is_some_and(|v| *v != e.version),
    });
    if let (Some(job), Some(o)) = (store.jobs.get(&e.id), v.as_object_mut()) {
        let mut job = json!(job);
        if let (Some((received, total)), Some(j)) = (store.progress(&e.id), job.as_object_mut()) {
            j.insert("received".into(), json!(received));
            j.insert("total".into(), json!(total));
        }
        o.insert("job".into(), job);
    }
    v
}

fn matches(e: &Entry, query: &str) -> bool {
    let q = query.to_lowercase();
    [&e.name, &e.description, &e.author, &e.collection, &e.id].iter().any(|f| f.to_lowercase().contains(&q))
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let query = match p.get("query") {
        None | Some(Value::Null) => "",
        Some(v) => v.as_str().ok_or_else(|| bad("plugin.store.list", "`query` must be text"))?,
    };
    let installed_only = p.get("installed").and_then(Value::as_bool).unwrap_or(false);
    s.plugin_store.poll();
    let store = &s.plugin_store;
    let plugins: Vec<Value> = store
        .catalog
        .iter()
        .filter(|e| matches(e, query.trim()))
        .filter(|e| !installed_only || registry::get(&e.id).is_some())
        .map(|e| describe(store, e))
        .collect();
    Ok(json!({"available": store.fetch.is_some(), "busy": store.busy(), "plugins": plugins}))
}

/// The ids in `id` or `ids`.
fn ids(cmd: &str, p: &Value) -> Result<Vec<String>> {
    let ids: Vec<String> = match (p.get("id"), p.get("ids")) {
        (Some(id), None) => vec![id.as_str().ok_or_else(|| bad(cmd, "`id` must be text"))?.to_string()],
        (None, Some(Value::Array(a))) => a.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| bad(cmd, "`ids` must be a list of text"))).collect::<Result<_>>()?,
        (None, Some(_)) => return Err(bad(cmd, "`ids` must be a list of text")),
        _ => return Err(bad(cmd, "give `id` or `ids`")),
    };
    if ids.is_empty() {
        return Err(bad(cmd, "`ids` is empty"));
    }
    Ok(ids)
}

fn install(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "plugin.store.install";
    let ids = ids(CMD, p)?;
    let reinstall = p.get("reinstall").and_then(Value::as_bool).unwrap_or(false);
    let store = &mut s.plugin_store;
    if let Some(unknown) = ids.iter().find(|id| store.entry(id).is_none()) {
        return Err(bad(CMD, format!("{unknown:?} is not in the plug-in catalog (see plugin.store.list)")));
    }
    if store.fetch.is_none() {
        return Err(EngineError::Other(UNAVAILABLE.into()));
    }
    for id in &ids {
        let current = store.entry(id).map(|e| e.version.clone());
        let up_to_date = registry::get(id).is_some_and(|p| Some(&p.manifest().version) == current.as_ref());
        let busy = matches!(store.jobs.get(id), Some(Job::Queued | Job::Downloading));
        if !busy && (reinstall || !up_to_date) {
            store.jobs.insert(id.clone(), Job::Queued);
        }
    }
    store.poll();
    let store = &s.plugin_store;
    Ok(json!({"plugins": ids.iter().filter_map(|id| store.entry(id)).map(|e| describe(store, e)).collect::<Vec<_>>()}))
}

fn uninstall(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "plugin.store.uninstall";
    let ids = ids(CMD, p)?;
    let store = &mut s.plugin_store;
    if let Some(unknown) = ids.iter().find(|id| store.entry(id).is_none()) {
        return Err(bad(CMD, format!("{unknown:?} is not in the plug-in catalog")));
    }
    let mut removed = Vec::new();
    for id in &ids {
        let deleted = store.delete(id).map_err(EngineError::Other)?;
        // A queued or running download would otherwise install it again when it finishes.
        store.cancel(id);
        if registry::remove(id) || deleted {
            removed.push(id.clone());
        }
    }
    Ok(json!({"removed": removed}))
}

fn cancel(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "plugin.store.cancel";
    let ids = ids(CMD, p)?;
    let store = &mut s.plugin_store;
    if let Some(unknown) = ids.iter().find(|id| store.entry(id).is_none()) {
        return Err(bad(CMD, format!("{unknown:?} is not in the plug-in catalog")));
    }
    let cancelled: Vec<&String> = ids.iter().filter(|id| store.cancel(id)).collect();
    Ok(json!({"cancelled": cancelled}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "plugin.store.list",
            label: "Plug-in Catalog",
            menu: &[],
            shortcut: None,
            params: r##"{"query":text,"installed":bool=false} (listings from the curated GitHub catalog, with install state)"##,
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "plugin.store.install",
            label: "Install Plug-in from Catalog",
            menu: &[],
            shortcut: None,
            params: r##"{"id":text,"ids":json,"reinstall":bool=false} (downloads in the background; poll plugin.store.list)"##,
            enabled: always,
            run: install,
            journal: false,
        },
        CommandSpec {
            id: "plugin.store.uninstall",
            label: "Uninstall Catalog Plug-in",
            menu: &[],
            shortcut: None,
            params: r##"{"id":text,"ids":json}"##,
            enabled: always,
            run: uninstall,
            journal: false,
        },
        CommandSpec {
            id: "plugin.store.cancel",
            label: "Cancel Plug-in Download",
            menu: &[],
            shortcut: None,
            params: r##"{"id":text,"ids":json} (stops queued or running catalog downloads)"##,
            enabled: always,
            run: cancel,
            journal: false,
        },
    ]
}

#[cfg(test)]
#[path = "plugin_store_cmds/tests.rs"]
mod tests;
