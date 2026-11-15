//! #13794: Quick Export's second location writes into an assets folder next to the document:
//! `dir/photo-assets/photo.<ext>` for a saved `dir/photo.psd`, `<Documents>-assets/<name>.<ext>`
//! for an unsaved one. The folder is made on demand, no dialog, a repeat replaces the file.

use super::*;
use std::path::Path;

/// A target as a `Path`, so the comparison ignores which separator the platform's `join` used.
fn target(doc_path: Option<&str>, name: &str, ext: &str, documents: Option<&Path>) -> Option<std::path::PathBuf> {
    assets_target(doc_path, name, ext, documents).map(std::path::PathBuf::from)
}

#[test]
fn the_target_is_an_assets_folder_next_to_the_document_or_documents() {
    assert_eq!(target(Some("/work/photo.psd"), "photo.psd", "png", None), Some(Path::new("/work/photo-assets/photo.png").to_path_buf()));
    assert_eq!(target(Some("/work/photo.psd"), "photo.psd", "jpg", None), Some(Path::new("/work/photo-assets/photo.jpg").to_path_buf()));
    let docs = Path::new("/home/test/Documents");
    assert_eq!(target(None, "exportprefs-test", "png", Some(docs)), Some(Path::new("/home/test/Documents-assets/exportprefs-test.png").to_path_buf()));
    // A redirected Documents folder (a Windows known folder moved off the profile) names the folder.
    let moved = Path::new("/mnt/data/Docs");
    assert_eq!(target(None, "exportprefs-test", "png", Some(moved)), Some(Path::new("/mnt/data/Docs-assets/exportprefs-test.png").to_path_buf()));
    assert_eq!(target(None, "exportprefs-test", "gif", None), None, "no Documents folder");
    let stored: crate::prefs::Export = serde_json::from_value(json!({"quickExportLocation": "sameFolder"})).unwrap();
    assert_eq!(stored.quick_export_location, crate::prefs::ExportLocation::SameFolder, "a stored sameFolder still loads");
}

/// CodeRabbit on #15082: the Documents folder is what the platform says (a redirected one included),
/// and `<home>/Documents` only when it says nothing.
#[test]
fn the_documents_folder_is_the_platforms_before_the_home_default() {
    let (home, moved) = (Some(Path::new("/home/ada").to_path_buf()), Some(Path::new("/mnt/data/Docs").to_path_buf()));
    assert_eq!(documents_from(moved.clone(), home.clone()), moved, "a redirected folder wins");
    assert_eq!(documents_from(None, home), Some(Path::new("/home/ada/Documents").to_path_buf()));
    assert_eq!(documents_from(None, None), None);
}

/// CodeRabbit on #15082: the file name follows the host's path rules. On Unix a backslash belongs to a
/// name, so `weird\name.psd` is its own document and does not collide with `name.psd`.
#[cfg(unix)]
#[test]
fn a_backslash_in_a_unix_file_name_is_part_of_the_name() {
    assert_eq!(
        target(Some("/work/weird\\name.psd"), "weird\\name.psd", "png", None),
        Some(Path::new("/work/weird\\name-assets/weird\\name.png").to_path_buf())
    );
    assert_ne!(target(Some("/work/weird\\name.psd"), "x", "png", None), target(Some("/work/name.psd"), "x", "png", None));
}

fn session(doc_path: &Path, location: &str, format: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 6, "name": "photo.psd"})).unwrap();
    s.active_mut().unwrap().path = Some(doc_path.to_string_lossy().into_owned());
    s.execute("file.export.exportPreferences", json!({"quickExportLocation": location, "quickExportFormat": format})).unwrap();
    s
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pc-assets-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_saved_document_exports_into_its_assets_folder_and_a_repeat_replaces_the_file() {
    let dir = scratch("saved");
    let mut s = session(&dir.join("photo.psd"), "sameFolder", "png");
    let r = s.execute("file.export.quickExport", json!({})).unwrap();
    let want = dir.join("photo-assets").join("photo.png");
    assert_eq!(r["path"].as_str().map(Path::new), Some(want.as_path()));
    let first = std::fs::metadata(&want).unwrap().len();
    assert!(first > 0, "the folder was made and the file written");
    s.execute("file.export.quickExport", json!({})).unwrap();
    assert_eq!(std::fs::read_dir(dir.join("photo-assets")).unwrap().count(), 1, "replaced, not a second file");
    s.execute("file.export.exportPreferences", json!({"quickExportFormat": "jpg"})).unwrap();
    s.execute("file.export.quickExport", json!({})).unwrap();
    assert!(dir.join("photo-assets").join("photo.jpg").exists(), "each format keeps its extension");
    assert!(!quick_export_target_is_own_file(&s), "never the document's own file");
}

#[test]
fn ask_still_needs_a_path_and_a_folder_that_cannot_be_made_is_an_error() {
    let dir = scratch("errors");
    let mut s = session(&dir.join("photo.psd"), "ask", "png");
    assert!(s.execute("file.export.quickExport", json!({})).unwrap_err().to_string().contains("missing `path`"));
    // A file where the folder's parent should be: the folder can't be made.
    std::fs::write(dir.join("blocker"), b"x").unwrap();
    let mut s = session(&dir.join("blocker").join("photo.psd"), "sameFolder", "png");
    let err = s.execute("file.export.quickExport", json!({})).unwrap_err().to_string();
    assert!(err.contains("cannot create the assets folder"), "{err}");
}
