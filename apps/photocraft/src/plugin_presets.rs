//! The plug-in presets store: the only code that reads or writes a plug-in's presets file.
//!
//! Each plug-in has one file, `<config dir>/plugins/<plug-in id>.json` (format and validation in
//! [`photocraft_plugins::presets`]). Plug-ins never get a path or file access; the dialog calls
//! [`load`] and [`save`] through `Services`, with nothing but the plug-in's id. The file name comes
//! only from that id, after [`photocraft_plugins::presets::file_name`] has checked it.
//!
//! Failures never stop the dialog: a missing, unreadable or damaged file loads as no presets
//! (with a logged warning). Only a missing or readable file is ever replaced: a damaged or
//! unreadable file, or one written by a newer PhotoCraft (another `version`), is never
//! overwritten, so presets that couldn't be loaded aren't lost by the next save.

use std::path::{Path, PathBuf};

use photocraft_plugins::presets::{self, MAX_FILE_BYTES, Parsed, Presets};

/// The folder under the settings directory that holds one presets file per plug-in.
pub const DIR: &str = "plugins";

/// The saved presets of `plugin_id` (empty when there are none or the file can't be used).
pub fn load(plugin_id: &str) -> Presets {
    match crate::services::config_dir() {
        Some(dir) => load_in(&dir, plugin_id),
        None => Presets::new(),
    }
}

/// Replaces the saved presets of `plugin_id` (atomically: temp file + rename in the same folder).
pub fn save(plugin_id: &str, presets: &Presets) -> Result<(), String> {
    save_in(&crate::services::config_dir().ok_or("no settings folder")?, plugin_id, presets)
}

fn path_in(config_dir: &Path, plugin_id: &str) -> Option<PathBuf> {
    presets::file_name(plugin_id).map(|f| config_dir.join(DIR).join(f))
}

fn load_in(config_dir: &Path, plugin_id: &str) -> Presets {
    let Some(path) = path_in(config_dir, plugin_id) else {
        log::warn!("plug-in presets: refusing unsafe plug-in id {plugin_id:?}");
        return Presets::new();
    };
    match read(&path) {
        Ok(None) => Presets::new(),
        Ok(Some(Parsed::Presets(p))) => p,
        Ok(Some(Parsed::UnknownVersion(v))) => {
            log::warn!("plug-in presets: {} has version {v}, which this PhotoCraft doesn't know; ignoring it", path.display());
            Presets::new()
        }
        Ok(Some(Parsed::Corrupt(e))) => {
            log::warn!("plug-in presets: {} is damaged ({e}); ignoring it", path.display());
            Presets::new()
        }
        Err(e) => {
            log::warn!("plug-in presets: can't read {}: {e}", path.display());
            Presets::new()
        }
    }
}

fn save_in(config_dir: &Path, plugin_id: &str, presets: &Presets) -> Result<(), String> {
    let path = path_in(config_dir, plugin_id).ok_or_else(|| format!("unsafe plug-in id {plugin_id:?}"))?;
    // Only a missing or readable file is replaced: anything else would lose presets never loaded.
    match read(&path) {
        Ok(None | Some(Parsed::Presets(_))) => {}
        Ok(Some(Parsed::UnknownVersion(v))) => {
            return Err(format!("{} has version {v}, which this PhotoCraft doesn't know; not overwriting it", path.display()));
        }
        Ok(Some(Parsed::Corrupt(e))) => return Err(format!("{} is damaged ({e}); not overwriting it", path.display())),
        Err(e) => return Err(format!("can't read {}: {e}; not overwriting it", path.display())),
    }
    // A file over the read limit would never load again, and then could never be replaced.
    let bytes = presets::to_json(presets);
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(format!("these presets need {} bytes (limit {MAX_FILE_BYTES}); not saving them", bytes.len()));
    }
    // `atomic_write` creates the `plugins` folder, writes a temp file beside the target and renames it.
    photocraft_format::atomic_write(&path, bytes.as_bytes()).map_err(|e| e.to_string())
}

/// The parsed file, `None` when it doesn't exist. Refuses non-files and oversized files.
fn read(path: &Path) -> Result<Option<Parsed>, String> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !meta.is_file() {
        return Err("not a regular file".into());
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("{} bytes (limit {MAX_FILE_BYTES})", meta.len()));
    }
    std::fs::read(path).map(|b| Some(presets::parse(&b))).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const ID: &str = "com.stack-tech.freecandy7";

    fn temp(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("photocraft-plugin-presets-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample() -> Presets {
        let mut p = Presets::new();
        p.insert("Soft".into(), json!({"amount": 25.0, "mode": "screen", "inner": "#cfeaff"}).as_object().unwrap().clone());
        p
    }

    #[test]
    fn round_trip_creates_the_plugins_folder() {
        let root = temp("roundtrip");
        assert!(load_in(&root, ID).is_empty(), "missing file: no presets");
        save_in(&root, ID, &sample()).unwrap();
        let file = root.join("plugins").join(format!("{ID}.json"));
        assert!(file.is_file());
        let v: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(v, json!({"version": 1, "presets": {"Soft": {"amount": 25.0, "mode": "screen", "inner": "#cfeaff"}}}));
        assert_eq!(load_in(&root, ID), sample());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn save_is_atomic_and_leaves_no_temp_files() {
        let root = temp("atomic");
        save_in(&root, ID, &sample()).unwrap();
        let mut more = sample();
        more.insert("Hard".into(), json!({"amount": 90.0}).as_object().unwrap().clone());
        save_in(&root, ID, &more).unwrap();
        let names: Vec<String> = std::fs::read_dir(root.join("plugins")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, [format!("{ID}.json")], "only the target remains: the temp file was renamed over it");
        assert_eq!(load_in(&root, ID), more);
        // A failed write (the target is a folder) keeps what was there and leaves no temp file.
        let blocked = temp("blocked");
        std::fs::create_dir_all(blocked.join("plugins").join(format!("{ID}.json"))).unwrap();
        assert!(save_in(&blocked, ID, &sample()).is_err());
        assert_eq!(std::fs::read_dir(blocked.join("plugins")).unwrap().count(), 1);
        assert!(load_in(&blocked, ID).is_empty(), "a folder in the way is not fatal");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&blocked);
    }

    #[test]
    fn presets_too_large_to_load_again_are_not_saved() {
        let root = temp("oversized");
        let mut big = Presets::new();
        let filler = "x".repeat(4096);
        while presets::to_json(&big).len() as u64 <= MAX_FILE_BYTES {
            let name = format!("p{}", big.len());
            big.insert(name, json!({"note": filler}).as_object().unwrap().clone());
        }
        assert!(save_in(&root, ID, &big).unwrap_err().contains("limit"));
        assert!(!root.join("plugins").join(format!("{ID}.json")).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_corrupt_file_loads_nothing() {
        let root = temp("corrupt");
        std::fs::create_dir_all(root.join("plugins")).unwrap();
        let file = root.join("plugins").join(format!("{ID}.json"));
        std::fs::write(&file, b"{not json").unwrap();
        assert!(load_in(&root, ID).is_empty());
        // Saving must not replace the damaged file: its presets were never loaded.
        assert!(save_in(&root, ID, &sample()).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"{not json");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unknown_version_is_left_alone() {
        let root = temp("version");
        let file = root.join("plugins").join(format!("{ID}.json"));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let newer = br#"{"version":2,"presets":{"Soft":{"amount":1}}}"#;
        std::fs::write(&file, newer).unwrap();
        assert!(load_in(&root, ID).is_empty());
        assert!(save_in(&root, ID, &sample()).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), newer, "never overwritten");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unsafe_ids_never_reach_the_file_system() {
        let root = temp("ids");
        for bad in ["..", "../x", "a/b", "a\\b", ".hidden", "a..b", ""] {
            assert!(save_in(&root, bad, &sample()).is_err(), "{bad:?}");
            assert!(load_in(&root, bad).is_empty());
        }
        assert!(!root.join("plugins").exists(), "nothing written");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `load`/`save` themselves, with `PHOTOCRAFT_CONFIG_DIR` pointing at a temp folder. The
    /// settings folder is resolved once per process, so this re-runs the test binary with the
    /// variable set (setting it in-process would need `unsafe`, and other tests share the process).
    #[test]
    fn load_and_save_use_the_config_dir_override() {
        const CHILD: &str = "PHOTOCRAFT_PLUGIN_PRESETS_CHILD";
        if let Some(expected) = std::env::var_os(CHILD) {
            // Child: refuse to run against anything but the temp folder the parent chose.
            assert_eq!(std::env::var_os("PHOTOCRAFT_CONFIG_DIR"), Some(expected.clone()));
            assert_eq!(crate::services::config_dir(), Some(PathBuf::from(&expected)));
            assert!(load(ID).is_empty());
            save(ID, &sample()).unwrap();
            assert_eq!(load(ID), sample());
            return;
        }
        let root = temp("env");
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "plugin_presets::tests::load_and_save_use_the_config_dir_override", "--test-threads=1", "--nocapture"])
            .env("PHOTOCRAFT_CONFIG_DIR", &root)
            .env(CHILD, &root)
            .output()
            .unwrap();
        assert!(out.status.success(), "child failed:\n{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(String::from_utf8_lossy(&out.stdout).contains("1 passed"), "the child really ran the test");
        assert!(root.join("plugins").join(format!("{ID}.json")).is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}
