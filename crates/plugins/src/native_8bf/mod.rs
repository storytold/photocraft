//! Desktop-only native 8BF support through a separate Windows worker process.
pub mod client;
mod launch;
pub mod pe;
pub mod surface;
pub mod wire;

use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};
#[derive(Clone, Debug, serde::Serialize)]
pub struct Descriptor {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub architecture: pe::Architecture,
    pub entry: String,
}
#[derive(Clone, Debug)]
pub struct Plugin {
    pub descriptor: Descriptor,
    pub launch: client::Launch,
}
static PLUGINS: RwLock<BTreeMap<String, Arc<Plugin>>> = RwLock::new(BTreeMap::new());
fn prepare(path: &Path, launch: Option<client::Launch>) -> io::Result<Arc<Plugin>> {
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("8bf")) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "choose a .8bf Windows filter library"));
    }
    let path = std::fs::canonicalize(path)?;
    let metadata = pe::inspect_file(&path)?;
    let launch = match launch {
        Some(l) => l,
        None => client::Launch::from_environment(metadata.architecture)?,
    };
    // Canonical path is the per-process identity. Reinstall replaces its metadata/configuration.
    let id = format!("8bf:{}", path.display());
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("8BF filter").to_string();
    let plugin = Arc::new(Plugin { descriptor: Descriptor { id: id.clone(), name, path, architecture: metadata.architecture, entry: metadata.entry }, launch });
    PLUGINS.write().unwrap_or_else(|e| e.into_inner()).insert(id, plugin.clone());
    Ok(plugin)
}
pub fn install(path: &Path) -> io::Result<Arc<Plugin>> {
    prepare(path, None)
}
/// Install using explicit worker process configuration (for embedding and controlled environments).
pub fn install_with_launch(path: &Path, launch: client::Launch) -> io::Result<Arc<Plugin>> {
    prepare(path, Some(launch))
}
pub fn list() -> Vec<Arc<Plugin>> {
    PLUGINS.read().unwrap_or_else(|e| e.into_inner()).values().cloned().collect()
}
pub fn get(id: &str) -> Option<Arc<Plugin>> {
    PLUGINS.read().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
}
pub fn remove(id: &str) -> bool {
    PLUGINS.write().unwrap_or_else(|e| e.into_inner()).remove(id).is_some()
}
#[derive(Debug, Default, serde::Serialize)]
pub struct FolderReport {
    pub loaded: Vec<Descriptor>,
    pub failed: Vec<(PathBuf, String)>,
}
/// Metadata-only discovery; no worker or native library is started.
pub fn load_folder(folder: &Path) -> io::Result<FolderReport> {
    drop(std::fs::read_dir(folder)?);
    load_folders(&[folder.to_path_buf()])
}
/// Recursively scans all roots, deduplicating aliases/overlaps and reporting unreadable paths.
pub fn load_folders(folders: &[PathBuf]) -> io::Result<FolderReport> {
    let scan = crate::discovery::scan(folders, "8bf");
    let mut report = FolderReport { loaded: Vec::new(), failed: scan.failed };
    for path in scan.files {
        match install(&path) {
            Ok(p) => report.loaded.push(p.descriptor.clone()),
            Err(e) => report.failed.push((path, e.to_string())),
        }
    }
    Ok(report)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn folder_and_direct_install_accept_matching_aliases_once() {
        let folder = std::env::temp_dir().join(format!("pc-8bf-alias-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let target = folder.join("filter.dll");
        let alias = folder.join("filter.8bf");
        std::fs::write(&target, pe::fixture(pe::Architecture::X86)).unwrap();
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        std::os::unix::fs::symlink(&target, folder.join("second.8bf")).unwrap();
        let direct = install(&alias).unwrap();
        let report = load_folder(&folder).unwrap();
        assert_eq!(report.loaded.len(), 1);
        assert!(report.failed.is_empty());
        assert_eq!(report.loaded[0].id, direct.descriptor.id);
        remove(&direct.descriptor.id);
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn configured_folder_recurses_into_nested_plugin_directories() {
        let folder = std::env::temp_dir().join(format!("pc-8bf-nested-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("vendor/deeper")).unwrap();
        std::fs::write(folder.join("root.8bf"), pe::fixture(pe::Architecture::X86)).unwrap();
        std::fs::write(folder.join("vendor/deeper/nested.8BF"), pe::fixture(pe::Architecture::X64)).unwrap();
        let report = load_folder(&folder).unwrap();
        for p in &report.loaded {
            remove(&p.id);
        }
        std::fs::remove_dir_all(folder).unwrap();
        assert_eq!(report.loaded.len(), 2, "both root and nested plugins must be discovered");
        assert!(report.failed.is_empty());
    }
    #[test]
    fn configured_folder_discovers_native_metadata_and_reports_invalid_files() {
        let folder = std::env::temp_dir().join(format!("pc-8bf-folder-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("good.8bf"), pe::fixture(pe::Architecture::X64)).unwrap();
        std::fs::write(folder.join("broken.8bf"), b"no DLL").unwrap();
        std::fs::write(folder.join("ignored.txt"), b"ignored").unwrap();
        let report = load_folder(&folder).unwrap();
        assert_eq!(report.loaded.len(), 1);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.loaded[0].architecture, pe::Architecture::X64);
        assert!(remove(&report.loaded[0].id));
        assert!(load_folder(&folder.join("missing")).is_err());
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn installs_metadata_and_removes_without_starting_worker() {
        let path = std::env::temp_dir().join(format!("pc-8bf-registry-{}.8bf", std::process::id()));
        std::fs::write(&path, pe::fixture(pe::Architecture::X86)).unwrap();
        let launch =
            client::Launch { program: "does-not-exist".into(), args: vec![], environment: Default::default(), timeout: std::time::Duration::from_secs(1) };
        let plugin = install_with_launch(&path, launch).unwrap();
        assert_eq!(plugin.descriptor.architecture, pe::Architecture::X86);
        assert!(get(&plugin.descriptor.id).is_some());
        assert!(remove(&plugin.descriptor.id));
        assert!(get(&plugin.descriptor.id).is_none());
        std::fs::remove_file(path).unwrap();
    }
}
