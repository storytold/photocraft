//! #13794: Quick Export's second location writes into an assets folder next to the document:
//! `dir/photo-assets/photo.<ext>` for a saved `dir/photo.psd`, `<Documents>-assets/<name>.<ext>`
//! for an unsaved one. The folder is made on demand, no dialog, a repeat replaces the file.

use super::*;
use std::path::Path;

#[test]
fn the_target_is_an_assets_folder_next_to_the_document_or_documents() {
    assert_eq!(assets_target(Some("/work/photo.psd"), "photo.psd", "png", None).as_deref(), Some("/work/photo-assets/photo.png"));
    assert_eq!(assets_target(Some("/work/photo.psd"), "photo.psd", "jpg", None).as_deref(), Some("/work/photo-assets/photo.jpg"));
    let docs = Path::new("/home/test/Documents");
    assert_eq!(assets_target(None, "exportprefs-test", "png", Some(docs)).as_deref(), Some("/home/test/Documents-assets/exportprefs-test.png"));
    assert_eq!(assets_target(None, "exportprefs-test", "gif", None), None, "no home folder");
    let stored: crate::prefs::Export = serde_json::from_value(json!({"quickExportLocation": "sameFolder"})).unwrap();
    assert_eq!(stored.quick_export_location, crate::prefs::ExportLocation::SameFolder, "a stored sameFolder still loads");
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
