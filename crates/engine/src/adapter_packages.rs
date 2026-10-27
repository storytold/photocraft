//! Explicitly trusted native worker packages, separate from the no-imports WASM loader.
//!
//! Installation snapshots a manifest and its declared files without starting anything. Only
//! explicitly enabled packages are registered on restart. Native workers have the user's OS
//! authority: digest checks detect accidental replacement, not a sandbox or publisher signature.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use photocraft_plugins::adapter::{self, Adapter, Context, Descriptor, Request, Response};
use photocraft_plugins::worker::{WorkerAdapter, WorkerStatus};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const MAX_MANIFEST: u64 = 1024 * 1024;
const MAX_FILE: u64 = 256 * 1024 * 1024;
const MAX_PACKAGE: u64 = 512 * 1024 * 1024;
// Self-contained worker runtimes can legitimately contain several hundred small files.
// Keep byte limits independent: this is not permission to bundle unbounded model weights.
const MAX_FILES: usize = 512;
const MAX_PACKAGES: usize = 64;
const WARNING: &str = "Native compatibility adapters run programs with your user account's permissions. They are NOT sandboxed WebAssembly plug-ins. Install only packages and publishers you trust. Installing does not enable or start the worker; preparing a system model may download it.";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Launch {
    Executable {
        path: String,
    },
    /// OS-registered packaged console application, resolved without PATH or a shell.
    ExecutionAlias {
        alias: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub package_version: u32,
    pub descriptor: Descriptor,
    pub launch: Launch,
    #[serde(default)]
    pub files: Vec<String>,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    manifest: Manifest,
    enabled: bool,
    hashes: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
struct Installed {
    record: Record,
    status: Value,
}

#[derive(Default)]
pub struct PackageStore {
    root: Option<PathBuf>,
    packages: BTreeMap<String, Installed>,
    warnings: Vec<String>,
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}
fn io_error(error: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("adapter package: {error}"))
}
fn adapter_error(error: adapter::Error) -> EngineError {
    crate::adapter_cmds::adapter_error(error)
}
fn valid_leaf(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.starts_with('.') && value.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}
fn relative_file(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if value.len() > 512 || value.contains('\\') || value.contains(':') || path.components().next().is_none() {
        return Err(io_error("package file names must use bounded relative paths with '/' separators"));
    }
    if path.components().any(|c| !matches!(c, Component::Normal(n) if n.to_str().is_some_and(valid_leaf))) {
        return Err(io_error("package paths may not contain roots, traversal or hidden components"));
    }
    Ok(path.to_path_buf())
}
fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path).map_err(io_error)?;
    if !meta.file_type().is_file() || meta.len() > max {
        return Err(io_error("expected a regular, bounded package file (symlinks are not accepted)"));
    }
    let mut file = File::open(path).map_err(io_error)?.take(max + 1);
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(usize::try_from(meta.len()).map_err(io_error)?).map_err(io_error)?;
    file.read_to_end(&mut bytes).map_err(io_error)?;
    if bytes.len() as u64 > max {
        return Err(io_error("package file exceeded its read limit"));
    }
    Ok(bytes)
}
fn checked_file(root: &Path, name: &str) -> Result<PathBuf> {
    let relative = relative_file(name)?;
    let root = root.canonicalize().map_err(io_error)?;
    let mut path = root.clone();
    for component in relative.components() {
        path.push(component.as_os_str());
        if fs::symlink_metadata(&path).map_err(io_error)?.file_type().is_symlink() {
            return Err(io_error("symlinks are not allowed inside a native adapter package"));
        }
    }
    let path = path.canonicalize().map_err(io_error)?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(io_error("package file escapes its package directory or is not a file"));
    }
    Ok(path)
}
fn launch_path(manifest: &Manifest, directory: &Path) -> Result<PathBuf> {
    match &manifest.launch {
        Launch::Executable { path } => checked_file(directory, path),
        Launch::ExecutionAlias { alias } => {
            if !cfg!(windows) {
                return Err(io_error("Windows execution aliases require Windows; this package cannot be enabled on this platform"));
            }
            if !valid_leaf(alias) || !alias.to_ascii_lowercase().ends_with(".exe") {
                return Err(io_error("execution_alias must be one .exe file name, not a path"));
            }
            let root =
                std::env::var_os("LOCALAPPDATA").map(PathBuf::from).filter(|p| p.is_absolute()).ok_or_else(|| io_error("LOCALAPPDATA is unavailable"))?;
            Ok(root.join("Microsoft").join("WindowsApps").join(alias))
        }
    }
}
fn validate_manifest(manifest: &Manifest) -> Result<()> {
    if manifest.package_version != 1 || !(1_000..=300_000).contains(&manifest.timeout_ms) || !valid_leaf(&manifest.descriptor.id) {
        return Err(io_error("unsupported package version, timeout (1–300 seconds), or adapter id"));
    }
    if manifest.files.len() > MAX_FILES {
        return Err(io_error("too many package files"));
    }
    let mut files = BTreeSet::new();
    for file in &manifest.files {
        relative_file(file)?;
        if !files.insert(file.to_ascii_lowercase()) || file.eq_ignore_ascii_case("record.json") {
            return Err(io_error("duplicate or reserved package file name"));
        }
    }
    match &manifest.launch {
        Launch::Executable { path } => {
            relative_file(path)?;
            if !manifest.files.contains(path) {
                return Err(io_error("the executable must appear in the package's files list"));
            }
            if matches!(Path::new(path).extension().and_then(|s| s.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("bat" | "cmd" | "ps1" | "sh")) {
                return Err(io_error("shell scripts cannot be worker executables"));
            }
        }
        Launch::ExecutionAlias { alias } => {
            if !valid_leaf(alias) || !alias.to_ascii_lowercase().ends_with(".exe") || !manifest.files.is_empty() {
                return Err(io_error("an execution-alias package must name one .exe alias and have no bundled files"));
            }
        }
    }
    // Descriptor validation is independent of starting or even locating an executable.
    struct Capabilities(Descriptor);
    impl Adapter for Capabilities {
        fn descriptor(&self) -> Descriptor {
            self.0.clone()
        }
        fn run(&self, _: &Request, _: &dyn Context) -> adapter::Result<Response> {
            Err(adapter::Error::Failed("capability validation does not execute adapters".into()))
        }
    }
    adapter::Registry::default().register(Arc::new(Capabilities(manifest.descriptor.clone()))).map_err(adapter_error)
}
fn parse_manifest(path: &Path) -> Result<(Manifest, PathBuf)> {
    let path = path.canonicalize().map_err(io_error)?;
    let bytes = read_bounded(&path, MAX_MANIFEST)?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(io_error)?;
    validate_manifest(&manifest)?;
    let root = path.parent().ok_or_else(|| io_error("manifest has no package directory"))?.to_path_buf();
    Ok((manifest, root))
}
fn not_checked() -> Value {
    json!({"state":"not_checked","message":"Availability has not been checked. No worker was started."})
}
fn verify(record: &Record, directory: &Path) -> Result<()> {
    if record.hashes.len() != record.manifest.files.len() {
        return Err(io_error("package file inventory does not match its trusted record; reinstall it"));
    }
    let mut total = 0u64;
    for name in &record.manifest.files {
        let bytes = read_bounded(&checked_file(directory, name)?, MAX_FILE)?;
        total = total.checked_add(bytes.len() as u64).filter(|n| *n <= MAX_PACKAGE).ok_or_else(|| io_error("package exceeds total file limit"))?;
        let expected = record.hashes.get(name).ok_or_else(|| io_error("package file digest is missing; reinstall it"))?;
        if blake3::hash(&bytes).to_hex().as_str() != expected {
            return Err(io_error(format!("package file {name:?} changed; disabled until explicitly reinstalled")));
        }
    }
    Ok(())
}
struct VerifiedWorker {
    inner: WorkerAdapter,
    record: Record,
    directory: PathBuf,
}
impl VerifiedWorker {
    fn new(record: &Record, directory: PathBuf) -> Result<Self> {
        verify(record, &directory)?;
        let executable = launch_path(&record.manifest, &directory)?;
        let inner =
            WorkerAdapter::new(executable, record.manifest.descriptor.clone(), Duration::from_millis(record.manifest.timeout_ms)).map_err(adapter_error)?;
        Ok(Self { inner, record: record.clone(), directory })
    }
    fn check(&self) -> adapter::Result<()> {
        verify(&self.record, &self.directory).map_err(|e| adapter::Error::Failed(e.to_string()))
    }
}
impl Adapter for VerifiedWorker {
    fn descriptor(&self) -> Descriptor {
        self.record.manifest.descriptor.clone()
    }
    fn run(&self, request: &Request, context: &dyn Context) -> adapter::Result<Response> {
        self.check()?;
        self.inner.run(request, context)
    }
}
impl PackageStore {
    /// Automatic routing consults cached state only; it never launches a probe/preparation.
    /// Packages need an explicit successful check; embedded adapters own their availability.
    /// This v1 package/default-model cache is not per-resource or hardware attestation.
    pub(crate) fn ready_for_automatic_run(&self, id: &str) -> bool {
        self.packages.get(id).is_none_or(|entry| {
            entry.record.enabled && entry.status.get("state").and_then(Value::as_str) == Some("ready")
        })
    }

    fn directory(&self, id: &str) -> Result<PathBuf> {
        if !valid_leaf(id) {
            return Err(io_error("invalid adapter id"));
        }
        Ok(self.root.as_ref().ok_or_else(|| io_error("native adapter storage is not configured by this application"))?.join(id))
    }
    fn save(&self, id: &str, record: &Record) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(record).map_err(io_error)?;
        if bytes.len() as u64 > MAX_MANIFEST * 2 {
            return Err(io_error("package record is too large"));
        }
        photocraft_format::atomic_write(&self.directory(id)?.join("record.json"), &bytes).map_err(io_error)
    }
    fn rows(&self) -> Value {
        let rows: Vec<Value> = self.packages.values().map(|entry| {
            let m = &entry.record.manifest;
            let launch = match &m.launch {
                Launch::Executable { path } => path.clone(),
                Launch::ExecutionAlias { alias } => format!("Windows packaged application alias: {alias}"),
            };
            json!({"id":m.descriptor.id,"name":m.descriptor.name,"version":m.descriptor.version,"enabled":entry.record.enabled,"descriptor":m.descriptor,"launch":launch,"status":entry.status})
        }).collect();
        json!({"available":self.root.is_some(),"packages":rows,"warnings":self.warnings})
    }
}
impl Session {
    /// Attach a frontend-chosen, native-only store. Reads explicitly trusted records only;
    /// no worker is started, no folder of arbitrary programs is discovered, no model downloaded.
    pub fn attach_adapter_directory(&mut self, root: PathBuf) -> Result<Vec<String>> {
        if !root.is_absolute() {
            return Err(io_error("adapter store must be an absolute directory"));
        }
        if self.adapter_packages.root.is_some() {
            return Err(io_error("adapter store is already attached"));
        }
        let mut store = PackageStore { root: Some(root.clone()), ..Default::default() };
        let entries = match fs::read_dir(&root) {
            Ok(entries) => Some(entries),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(io_error(e)),
        };
        if let Some(entries) = entries {
            for candidate in entries.take(MAX_PACKAGES + 1) {
                let result = (|| -> Result<()> {
                    if store.packages.len() >= MAX_PACKAGES {
                        return Err(io_error("too many installed packages; remaining packages skipped"));
                    }
                    let candidate = candidate.map_err(io_error)?;
                    let name = candidate.file_name().into_string().map_err(|_| io_error("non-Unicode package directory"))?;
                    if !valid_leaf(&name) || !candidate.file_type().map_err(io_error)?.is_dir() {
                        return Ok(());
                    }
                    let bytes = read_bounded(&candidate.path().join("record.json"), MAX_MANIFEST * 2)?;
                    let mut record: Record = serde_json::from_slice(&bytes).map_err(io_error)?;
                    validate_manifest(&record.manifest)?;
                    if record.manifest.descriptor.id != name {
                        return Err(io_error("package id does not match its directory"));
                    }
                    let mut status = not_checked();
                    if record.enabled {
                        match VerifiedWorker::new(&record, candidate.path()).and_then(|w| self.register_adapter(Arc::new(w))) {
                            Ok(()) => {}
                            Err(error) => {
                                record.enabled = false;
                                status = json!({"state":"error","message":error.to_string()});
                                store.warnings.push(format!("{name}: {error}"));
                            }
                        }
                    }
                    store.packages.insert(name, Installed { record, status });
                    Ok(())
                })();
                if let Err(error) = result {
                    store.warnings.push(error.to_string());
                }
            }
        }
        let warnings = store.warnings.clone();
        self.adapter_packages = store;
        Ok(warnings)
    }
}

fn text<'a>(p: &'a Value, key: &str, cmd: &str) -> Result<&'a str> {
    p.get(key).and_then(Value::as_str).filter(|v| !v.is_empty() && v.len() <= 4096).ok_or_else(|| bad(cmd, format!("{key} must be a nonempty bounded string")))
}
pub(crate) fn preflight(id: &str, p: &Value) -> Result<()> {
    let allowed: &[&str] = match id {
        "plugin.adapter.packages" => &[],
        "plugin.adapter.inspectPackage" => &["path"],
        "plugin.adapter.install" => &["path", "trust"],
        "plugin.adapter.enable" => &["provider", "enabled"],
        "plugin.adapter.remove" | "plugin.adapter.status" => &["provider"],
        "plugin.adapter.prepare" => &["provider", "consent"],
        _ => return Ok(()),
    };
    if p.is_null() {
        return Ok(());
    }
    adapter::Limits::default().check_params(p).map_err(adapter_error)?;
    let map = p.as_object().ok_or_else(|| bad(id, "params must be an object"))?;
    if map.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(bad(id, "unknown package-management parameter"));
    }
    for key in allowed {
        match *key {
            "path" | "provider" => {
                text(p, key, id)?;
            }
            "trust" | "enabled" | "consent" if p.get(*key).and_then(Value::as_bool).is_none() => {
                return Err(bad(id, format!("{key} must be a boolean")));
            }
            _ => {}
        }
    }
    Ok(())
}
fn inspect(_: &mut Session, p: &Value) -> Result<Value> {
    let (manifest, _) = parse_manifest(Path::new(text(p, "path", "plugin.adapter.inspectPackage")?))?;
    Ok(json!({"descriptor":manifest.descriptor,"launch":serde_json::to_value(manifest.launch).map_err(io_error)?.to_string(),"warning":WARNING}))
}
fn install(s: &mut Session, p: &Value) -> Result<Value> {
    if p.get("trust").and_then(Value::as_bool) != Some(true) {
        return Err(bad("plugin.adapter.install", WARNING));
    }
    let (manifest, source) = parse_manifest(Path::new(text(p, "path", "plugin.adapter.install")?))?;
    let id = manifest.descriptor.id.clone();
    if s.adapter_packages.packages.len() >= MAX_PACKAGES || s.adapter_packages.packages.contains_key(&id) || s.adapters.get(&id).is_some() {
        return Err(io_error("package limit or duplicate adapter id; remove the previous package before reinstalling"));
    }
    let destination = s.adapter_packages.directory(&id)?;
    let root = destination.parent().ok_or_else(|| io_error("invalid package root"))?;
    fs::create_dir_all(root).map_err(io_error)?;
    if fs::symlink_metadata(&destination).is_ok() {
        return Err(io_error("package destination already exists; it will not be overwritten"));
    }
    let staged = tempfile::Builder::new().prefix(".install-").tempdir_in(root).map_err(io_error)?;
    let mut hashes = BTreeMap::new();
    let mut total = 0u64;
    for name in &manifest.files {
        let bytes = read_bounded(&checked_file(&source, name)?, MAX_FILE)?;
        total = total.checked_add(bytes.len() as u64).filter(|n| *n <= MAX_PACKAGE).ok_or_else(|| io_error("package is too large"))?;
        let target = staged.path().join(relative_file(name)?);
        fs::create_dir_all(target.parent().ok_or_else(|| io_error("invalid package target"))?).map_err(io_error)?;
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&target).map_err(io_error)?;
        file.write_all(&bytes).and_then(|()| file.sync_all()).map_err(io_error)?;
        #[cfg(unix)]
        if matches!(&manifest.launch, Launch::Executable { path } if path == name) {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
        }
        hashes.insert(name.clone(), blake3::hash(&bytes).to_hex().to_string());
    }
    let record = Record { manifest, enabled: false, hashes };
    let bytes = serde_json::to_vec_pretty(&record).map_err(io_error)?;
    photocraft_format::atomic_write(&staged.path().join("record.json"), &bytes).map_err(io_error)?;
    fs::rename(staged.path(), &destination).map_err(io_error)?;
    s.adapter_packages.packages.insert(id, Installed { record, status: not_checked() });
    Ok(s.adapter_packages.rows())
}
fn enable(s: &mut Session, p: &Value) -> Result<Value> {
    let id = text(p, "provider", "plugin.adapter.enable")?;
    let enabled = p.get("enabled").and_then(Value::as_bool).ok_or_else(|| bad("plugin.adapter.enable", "enabled must be boolean"))?;
    let entry = s.adapter_packages.packages.get(id).ok_or_else(|| io_error("package is not installed"))?;
    if entry.record.enabled == enabled {
        return Ok(s.adapter_packages.rows());
    }
    let mut record = entry.record.clone();
    let mut registry = s.adapters.clone();
    if enabled {
        registry.register(Arc::new(VerifiedWorker::new(&record, s.adapter_packages.directory(id)?)?)).map_err(adapter_error)?;
    } else {
        registry.remove(id);
    }
    record.enabled = enabled;
    s.adapter_packages.save(id, &record)?;
    s.adapters = registry;
    if let Some(entry) = s.adapter_packages.packages.get_mut(id) {
        entry.record = record;
        entry.status = not_checked();
    }
    Ok(s.adapter_packages.rows())
}
fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let id = text(p, "provider", "plugin.adapter.remove")?;
    if !s.adapter_packages.packages.contains_key(id) {
        return Err(io_error("package is not installed"));
    }
    let directory = s.adapter_packages.directory(id)?;
    let root = s.adapter_packages.root.as_ref().ok_or_else(|| io_error("no package root"))?.canonicalize().map_err(io_error)?;
    if fs::symlink_metadata(&directory).map_err(io_error)?.file_type().is_symlink()
        || directory.canonicalize().map_err(io_error)?.parent() != Some(root.as_path())
    {
        return Err(io_error("refusing removal outside the managed adapter directory"));
    }
    fs::remove_dir_all(&directory).map_err(io_error)?;
    s.adapters.remove(id);
    s.adapter_packages.packages.remove(id);
    Ok(s.adapter_packages.rows())
}
struct JobContext(crate::jobs::JobCtx);
impl Context for JobContext {
    fn cancelled(&self) -> bool {
        self.0.cancelled()
    }
    fn progress(&self, value: f32) {
        self.0.progress(value, "Native adapter");
    }
}
fn check_status(s: &mut Session, p: &Value, prepare: bool) -> Result<Value> {
    let command = if prepare { "plugin.adapter.prepare" } else { "plugin.adapter.status" };
    if prepare && p.get("consent").and_then(Value::as_bool) != Some(true) {
        return Err(bad(command, "explicit consent is required; preparing the system AI model may download components"));
    }
    let id = text(p, "provider", command)?.to_string();
    let entry = s.adapter_packages.packages.get(&id).ok_or_else(|| io_error("package is not installed"))?;
    let record = entry.record.clone();
    let worker = VerifiedWorker::new(&record, s.adapter_packages.directory(&id)?)?;
    let label = if prepare { "Prepare Adapter Model" } else { "Check Adapter Availability" };
    crate::jobs::run(
        s,
        label,
        false,
        move |ctx| {
            let context = JobContext(ctx.clone());
            worker.check().map_err(adapter_error)?;
            let result: adapter::Result<WorkerStatus> = if prepare { worker.inner.prepare(&context) } else { worker.inner.probe(&context) };
            ctx.check()?;
            Ok(match result {
                Ok(status) => serde_json::to_value(status).map_err(io_error)?,
                Err(error) => json!({"state":"error","message":error.to_string()}),
            })
        },
        move |s, status| {
            if let Some(entry) = s.adapter_packages.packages.get_mut(&id) {
                // A remove/reinstall while a status job ran must not update a different package.
                if serde_json::to_value(&entry.record.manifest).map_err(io_error)? == serde_json::to_value(&record.manifest).map_err(io_error)? {
                    entry.status = status.clone();
                }
            }
            Ok(json!({"provider":id,"status":status}))
        },
    )
}
fn configured(s: &Session) -> std::result::Result<(), String> {
    if s.adapter_packages.root.is_some() { Ok(()) } else { Err("native adapter storage is unavailable in this application".into()) }
}
pub fn specs() -> Vec<CommandSpec> {
    [
        (
            "plugin.adapter.packages",
            "List Native Adapter Packages",
            "{}",
            crate::commands::always as fn(&Session) -> std::result::Result<(), String>,
            (|s: &mut Session, _: &Value| Ok(s.adapter_packages.rows())) as fn(&mut Session, &Value) -> Result<Value>,
        ),
        ("plugin.adapter.inspectPackage", "Inspect Native Adapter Package", r#"{"path":text}"#, configured, inspect),
        ("plugin.adapter.install", "Install Native Adapter Package", r#"{"path":text,"trust":bool}"#, configured, install),
        ("plugin.adapter.enable", "Enable Native Adapter Package", r#"{"provider":text,"enabled":bool}"#, configured, enable),
        ("plugin.adapter.remove", "Remove Native Adapter Package", r#"{"provider":text}"#, configured, remove),
        (
            "plugin.adapter.status",
            "Check Native Adapter Availability",
            r#"{"provider":text}"#,
            configured,
            (|s, p| check_status(s, p, false)) as fn(&mut Session, &Value) -> Result<Value>,
        ),
        (
            "plugin.adapter.prepare",
            "Prepare Native Adapter Model",
            r#"{"provider":text,"consent":bool}"#,
            configured,
            (|s, p| check_status(s, p, true)) as fn(&mut Session, &Value) -> Result<Value>,
        ),
    ]
    .into_iter()
    .map(|(id, label, params, enabled, run)| CommandSpec { id, label, menu: &[], shortcut: None, params, enabled, run, journal: false })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_plugins::adapter::{Device, DeviceKind, Operation, OutputKind};

    fn manifest() -> Manifest {
        Manifest {
            package_version: 1,
            descriptor: Descriptor {
                api_version: 1,
                id: "org.photocraft.test.package".into(),
                name: "Test package (not AI)".into(),
                version: "1.0".into(),
                operations: vec![Operation { id: "select-subject".into(), output_kind: OutputKind::Mask, color_modes: vec!["rgb".into()] }],
                devices: vec![Device {
                    id: "cpu.test".into(),
                    kind: DeviceKind::Cpu,
                    backend: "test".into(),
                    name: "CPU test".into(),
                    operations: vec!["select-subject".into()],
                }],
                resource_formats: vec![],
                params_schema: json!({}),
            },
            launch: Launch::Executable { path: "worker.exe".into() },
            files: vec!["worker.exe".into()],
            timeout_ms: 1000,
        }
    }
    fn package(dir: &Path) -> PathBuf {
        fs::write(dir.join("worker.exe"), b"not executable; installation must never execute this").unwrap();
        let path = dir.join("adapter.json");
        fs::write(&path, serde_json::to_vec(&manifest()).unwrap()).unwrap();
        path
    }
    #[test]
    fn installation_is_explicit_disabled_persistent_and_does_not_execute() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let path = package(source.path());
        let mut s = Session::new();
        s.attach_adapter_directory(data.path().join("Adapters")).unwrap();
        assert!(s.execute("plugin.adapter.install", json!({"path":path,"trust":false})).is_err());
        let result = s.execute("plugin.adapter.install", json!({"path":path,"trust":true})).unwrap();
        assert_eq!(result["packages"][0]["enabled"], false);
        assert!(s.adapters.list().is_empty());
        let id = manifest().descriptor.id;
        s.execute("plugin.adapter.enable", json!({"provider":id,"enabled":true})).unwrap();
        assert!(s.adapters.get(&id).is_some());
        let mut next = Session::new();
        assert!(next.attach_adapter_directory(data.path().join("Adapters")).unwrap().is_empty());
        assert!(next.adapters.get(&id).is_some());
        next.execute("plugin.adapter.enable", json!({"provider":id,"enabled":false})).unwrap();
        assert!(next.adapters.get(&id).is_none());
        next.execute("plugin.adapter.remove", json!({"provider":id})).unwrap();
        assert!(next.adapter_packages.packages.is_empty());
    }
    #[test]
    fn replaced_worker_is_not_restored_or_enabled() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let path = package(source.path());
        let mut s = Session::new();
        s.attach_adapter_directory(data.path().join("Adapters")).unwrap();
        s.execute("plugin.adapter.install", json!({"path":path,"trust":true})).unwrap();
        let id = manifest().descriptor.id;
        s.execute("plugin.adapter.enable", json!({"provider":id,"enabled":true})).unwrap();
        fs::write(data.path().join("Adapters").join(&id).join("worker.exe"), b"changed").unwrap();
        let mut next = Session::new();
        assert!(!next.attach_adapter_directory(data.path().join("Adapters")).unwrap().is_empty());
        assert!(next.adapters.get(&id).is_none());
        assert!(next.execute("plugin.adapter.enable", json!({"provider":id,"enabled":true})).is_err());
    }

    #[test]
    fn automatic_routing_requires_explicit_cached_readiness_for_packages() {
        let mut store = PackageStore::default();
        assert!(store.ready_for_automatic_run("embedded.adapter"));
        let record = Record { manifest: manifest(), enabled: true, hashes: BTreeMap::new() };
        let id = record.manifest.descriptor.id.clone();
        store.packages.insert(id.clone(), Installed { record, status: not_checked() });
        assert!(!store.ready_for_automatic_run(&id));
        for state in ["needs_model", "unavailable", "error", "not_checked", "ready"] {
            store.packages.get_mut(&id).unwrap().status = json!({ "state": state, "message": "synthetic status; no worker started" });
            assert_eq!(store.ready_for_automatic_run(&id), state == "ready");
        }
        store.packages.get_mut(&id).unwrap().record.enabled = false;
        assert!(!store.ready_for_automatic_run(&id));
    }

    #[test]
    fn runtime_file_inventory_is_bounded_without_raising_model_byte_limits() {
        let mut m = manifest();
        m.files.extend((1..MAX_FILES).map(|i| format!("runtime/component-{i}.dll")));
        assert_eq!(m.files.len(), 512);
        assert!(validate_manifest(&m).is_ok(), "complete runtimes can have more than 128 small files");
        m.files.push("runtime/one-too-many.dll".into());
        assert!(validate_manifest(&m).is_err());
        assert_eq!(MAX_FILE, 256 * 1024 * 1024);
        assert_eq!(MAX_PACKAGE, 512 * 1024 * 1024);
    }

    #[test]
    fn traversal_duplicates_invalid_descriptors_and_unconsented_preparation_fail() {
        for name in ["../worker.exe", "C:/worker.exe", "dir/../../worker.exe", "dir\\worker.exe", ".hidden", "worker.exe:stream"] {
            assert!(relative_file(name).is_err(), "{name}");
        }
        let mut m = manifest();
        m.files.push("WORKER.EXE".into());
        assert!(validate_manifest(&m).is_err());
        let mut m = manifest();
        m.descriptor.devices[0].operations.clear();
        assert!(validate_manifest(&m).is_err());
        assert!(Session::new().execute("plugin.adapter.prepare", json!({"provider":"x","consent":false})).is_err());
        assert!(preflight("plugin.adapter.install", &json!({"path":"x","trust":"yes"})).is_err());
        assert!(preflight("plugin.adapter.enable", &json!({"provider":"x","enabled":true,"command":"cmd.exe"})).is_err());
    }
}
