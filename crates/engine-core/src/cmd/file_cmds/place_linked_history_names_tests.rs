//! History names of File › Place Linked (#14117), as measured in the reference app: a 400 × 300
//! RGB 8-bit 72 ppi document and a 100 × 60 PNG. A linked place is one "Place Linked Smart
//! Object" state, also with Always Create Smart Objects when Placing off. When the linked file is
//! saved again while the document is open, the layers update by themselves in one "Update Smart
//! Objects" state, also when two layers show the file; Layer › Smart Objects › Update Modified
//! Content keeps its own name.

use super::place_history_names_tests::{added_by, entries, placed_kind, png_100x60, reference_document, reference_png};
use super::*;

/// Write `bytes` to `path` as another app saving it again would, and move its modification time
/// two seconds past the old one, so the change is seen whatever the file system's time resolution.
fn save_again(path: &std::path::Path, bytes: &[u8]) {
    let before = std::fs::metadata(path).unwrap().modified().unwrap();
    std::fs::write(path, bytes).unwrap();
    let later = std::fs::FileTimes::new().set_modified(before + std::time::Duration::from_secs(2));
    std::fs::File::options().write(true).open(path).unwrap().set_times(later).unwrap();
}

/// The reference PNG, red, in a temporary folder of its own: (the folder's guard, its path, the
/// same as text).
fn linked_png() -> (tempfile::TempDir, std::path::PathBuf, String) {
    let (dir, path) = reference_png();
    let text = path.to_string_lossy().into_owned();
    (dir, path, text)
}

/// Whether the canvas centre, inside the placed image, shows `rgb`.
fn centre_is(s: &Session, rgb: [f32; 3]) -> bool {
    let p = photocraft_compose::render(&s.active().unwrap().doc, Rect::new(200, 150, 201, 151)).px[0];
    p.iter().zip(rgb).all(|(a, b)| (a - b).abs() < 0.01)
}

#[test]
fn a_linked_place_is_one_place_linked_smart_object_state() {
    let (_dir, _, path) = linked_png();
    let mut s = reference_document();
    assert_eq!(added_by(&mut s, "file.placeLinked", json!({"path": path})), ["Place Linked Smart Object"]);
    assert_eq!(placed_kind(&s), "linked");
    // Bytes placed linked to their path (the desktop picker) are named the same.
    let before = entries(&s).len();
    place_bytes(&mut s, "art.png", png_100x60([255, 0, 0]), Some(path.clone()), &json!({})).unwrap();
    assert_eq!(entries(&s)[before..], ["Place Linked Smart Object"]);
    assert_eq!(placed_kind(&s), "linked");
}

#[test]
fn a_linked_place_keeps_its_name_without_always_create_smart_objects() {
    let (_dir, _, path) = linked_png();
    let mut s = reference_document();
    s.execute("prefs.set", json!({"path": "general.alwaysCreateSmartObjectsWhenPlacing", "value": false})).unwrap();
    assert_eq!(added_by(&mut s, "file.placeLinked", json!({"path": path})), ["Place Linked Smart Object"]);
    assert_eq!(placed_kind(&s), "linked", "still a linked smart object");
    // An embedded place in the same document is a pixel layer named Place Image.
    assert_eq!(added_by(&mut s, "file.placeEmbedded", json!({"path": path})), ["Place Image"]);
}

#[test]
fn a_linked_file_saved_again_is_one_update_smart_objects_state_for_all_its_layers() {
    let (_dir, file, path) = linked_png();
    let mut s = reference_document();
    s.execute("file.placeLinked", json!({"path": path})).unwrap();
    let first = s.active().unwrap().active_layer.unwrap().0;
    s.execute("file.placeLinked", json!({"path": path})).unwrap();
    let second = s.active().unwrap().active_layer.unwrap().0;
    // The first look only remembers the file.
    assert_eq!(added_by(&mut s, "layer.smartObjects.updateChanged", json!({})), Vec::<String>::new());
    assert!(centre_is(&s, [1.0, 0.0, 0.0]));
    save_again(&file, &png_100x60([0, 0, 255]));
    let before = entries(&s).len();
    let updated = s.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
    let mut ids: Vec<u64> = updated["updated"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    ids.sort_unstable();
    assert_eq!(ids, [first.min(second), first.max(second)], "both layers that show the file: {updated}");
    assert_eq!(entries(&s)[before..], ["Update Smart Objects"], "one state for the file, not one per layer");
    assert!(centre_is(&s, [0.0, 0.0, 1.0]));
    // Undone, the redo state carries the same name.
    assert!(s.undo());
    assert_eq!(s.active().unwrap().history.redo_label(), Some(crate::link_watch::UPDATE_SMART_OBJECTS));
    assert!(centre_is(&s, [1.0, 0.0, 0.0]));
}

#[test]
fn update_modified_content_from_the_menu_keeps_its_own_name() {
    let (_dir, file, path) = linked_png();
    let mut s = reference_document();
    s.execute("file.placeLinked", json!({"path": path})).unwrap();
    save_again(&file, &png_100x60([0, 0, 255]));
    assert_eq!(added_by(&mut s, "layer.smartObjects.updateModifiedContent", json!({})), ["Update Modified Content"]);
    assert!(centre_is(&s, [0.0, 0.0, 1.0]));
}
