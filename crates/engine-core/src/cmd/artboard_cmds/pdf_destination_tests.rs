//! #13657: Artboards to PDF refuses a destination folder that doesn't exist with an error naming
//! it, before anything is written, instead of creating the folder.

use serde_json::json;

fn session() -> crate::Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
    s.execute("layer.new.artboard", json!({"rect": [0, 0, 16, 16]})).unwrap();
    s
}

#[test]
fn a_missing_destination_folder_is_refused_and_nothing_is_written() {
    let root = std::env::temp_dir().join(format!("photocraft-pdf-dest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let missing = root.join("nodir");
    let path = missing.join("x.pdf").to_string_lossy().into_owned();
    let mut s = session();
    let err = s.execute("file.export.artboardsToPdf", json!({"path": path})).unwrap_err().to_string();
    assert!(err.contains("Destination does not exist") && err.contains("nodir"), "names the missing destination: {err}");
    assert!(!missing.exists() && !root.exists(), "no folder is created");
    // The same check the dialog runs before it closes.
    assert!(super::pdf_destination(&path, "doc.psd").is_err());
}

#[test]
fn an_existing_folder_still_exports() {
    let dir = std::env::temp_dir().join(format!("photocraft-pdf-dest-ok-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("boards.pdf").to_string_lossy().into_owned();
    let mut s = session();
    let r = s.execute("file.export.artboardsToPdf", json!({"path": path})).unwrap();
    assert_eq!(r["pages"], 1);
    assert!(std::fs::read(&path).unwrap().starts_with(b"%PDF"));
    let _ = std::fs::remove_dir_all(&dir);
}
