//! App-private storage for the Android prototype. No document overwrite without a picker.

use std::io::Write;
use std::path::{Path, PathBuf};

pub fn save_path(dir: &Path, suggested: &str) -> PathBuf {
    let name = Path::new(suggested).file_name().filter(|n| !n.is_empty()).unwrap_or_else(|| std::ffi::OsStr::new("Untitled.pcraft"));
    dir.join(name)
}

/// Publish a complete document without replacing an existing file. The prototype has no
/// overwrite confirmation dialog, so even a second Save must fail rather than discard data.
pub fn save_document(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let file = stage(path, bytes)?;
    file.persist_noclobber(path).map(|_| ()).map_err(|e| format!("couldn't save {} without overwriting: {e}", path.display()))
}

/// Preferences can replace the previous version, but only after the new bytes reach disk.
pub fn save_preferences(path: &Path, text: &str) -> Result<(), String> {
    let file = stage(path, text.as_bytes())?;
    file.persist(path).map(|_| ()).map_err(|e| format!("couldn't save {}: {e}", path.display()))
}

fn stage(path: &Path, bytes: &[u8]) -> Result<tempfile::NamedTempFile, String> {
    let parent = path.parent().ok_or("missing parent folder")?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| format!("couldn't create a temporary file in {}: {e}", parent.display()))?;
    file.write_all(bytes).and_then(|()| file.as_file().sync_all()).map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
    Ok(file)
}

/// Matches the desktop shell's last-resort guard: engine commands already catch panics, but
/// import and export are platform callbacks and need their own guard.
pub fn guard<T>(what: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err(format!("{what} failed with an internal error (logged); your open documents are unchanged")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_cannot_escape_the_documents_folder() {
        let root = Path::new("/documents");
        assert_eq!(save_path(root, "/elsewhere/file.png"), root.join("file.png"));
        assert_eq!(save_path(root, "../../file.png"), root.join("file.png"));
        assert_eq!(save_path(root, ".."), root.join("Untitled.pcraft"));
        assert_eq!(save_path(root, ""), root.join("Untitled.pcraft"));
    }

    #[test]
    fn a_second_save_preserves_the_existing_document() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Untitled.pcraft");
        save_document(&path, b"original").unwrap();
        assert!(save_document(&path, b"replacement").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn preferences_can_be_replaced_without_leaving_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        save_preferences(&path, "old").unwrap();
        save_preferences(&path, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn missing_folder_reports_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(save_document(&dir.path().join("missing/file.png"), b"data").is_err());
    }

    #[test]
    fn import_export_guard_preserves_errors_and_catches_panics() {
        assert_eq!(guard("Open", || Ok(3)), Ok(3));
        assert_eq!(guard::<()>("Open", || Err("bad file".into())), Err("bad file".into()));
        let result: Result<(), String> = guard("Open", || panic!("test panic"));
        assert!(result.unwrap_err().contains("Open failed"));
    }
}
