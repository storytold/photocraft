//! History names of File › Place Embedded (#14106), as measured in the reference app: a 400 × 300
//! RGB 8-bit 72 ppi document and a 100 × 60 PNG. An embedded smart object is one "Place Embedded
//! Smart Object" state; with Always Create Smart Objects when Placing off the place makes a pixel
//! layer and is one "Place Image" state, for a one-page PDF too. The helpers here are shared with
//! the Place Linked names (`place_linked_history_names_tests.rs`, #14117).

use super::*;

/// The reference app's test document: 400 × 300, RGB, 8 bit, 72 ppi.
pub(super) fn reference_document() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "depth": 8, "resolution": 72, "background": "white"})).unwrap();
    s
}

/// The bytes of the reference app's placed file: a 100 × 60 PNG of solid `rgb`.
pub(super) fn png_100x60(rgb: [u8; 3]) -> Vec<u8> {
    let px: Vec<u8> = std::iter::repeat_n(rgb, 100 * 60).flatten().collect();
    let img = photocraft_codecs::Image::from_u8(100, 60, photocraft_codecs::ChannelLayout::Rgb, px).unwrap();
    photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap()
}

/// The History panel's entries of the active document, oldest first.
pub(super) fn entries(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

/// Run `cmd` with `params` and return the History entries it added.
pub(super) fn added_by(s: &mut Session, cmd: &str, params: Value) -> Vec<String> {
    let before = entries(s).len();
    s.execute(cmd, params).unwrap_or_else(|e| panic!("{cmd}: {e}"));
    entries(s).split_off(before)
}

/// The content of the active layer: "linked", "embedded" (smart objects) or "pixels".
pub(super) fn placed_kind(s: &Session) -> &'static str {
    let d = s.active().unwrap();
    match &d.doc.layer(d.active_layer.unwrap()).unwrap().content {
        LayerContent::Smart(so) if matches!(so.source, SmartSource::Linked { .. }) => "linked",
        LayerContent::Smart(_) => "embedded",
        LayerContent::Raster(_) => "pixels",
        other => panic!("not a placed layer: {other:?}"),
    }
}

/// Write the reference PNG, red, to a temporary folder of its own (removed when the guard drops):
/// (the guard, the file's path).
pub(super) fn reference_png() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("art.png");
    std::fs::write(&path, png_100x60([255, 0, 0])).unwrap();
    (dir, path)
}

/// [`reference_png`] with the path as text, as the place commands take it.
fn reference_png_path() -> (tempfile::TempDir, String) {
    let (dir, path) = reference_png();
    (dir, path.to_string_lossy().into_owned())
}

#[test]
fn an_embedded_place_is_one_place_embedded_smart_object_state() {
    let (_dir, path) = reference_png_path();
    let mut s = reference_document();
    assert_eq!(added_by(&mut s, "file.placeEmbedded", json!({"path": path})), ["Place Embedded Smart Object"]);
    assert_eq!(placed_kind(&s), "embedded");
    // Bytes without a path (a dropped or web file) go through the same place.
    let before = entries(&s).len();
    place_bytes(&mut s, "art.png", png_100x60([0, 0, 255]), None, &json!({})).unwrap();
    assert_eq!(entries(&s)[before..], ["Place Embedded Smart Object"]);
    assert_eq!(placed_kind(&s), "embedded");
}

#[test]
fn a_place_that_makes_a_pixel_layer_is_one_place_image_state() {
    let (_dir, path) = reference_png_path();
    // The preference off, with nothing in the params.
    let mut s = reference_document();
    s.execute("prefs.set", json!({"path": "general.alwaysCreateSmartObjectsWhenPlacing", "value": false})).unwrap();
    assert_eq!(added_by(&mut s, "file.placeEmbedded", json!({"path": path})), ["Place Image"]);
    assert_eq!(placed_kind(&s), "pixels");
    // An explicit `"smartObject": false` with the preference on.
    let mut s = reference_document();
    assert_eq!(added_by(&mut s, "file.placeEmbedded", json!({"path": path, "smartObject": false})), ["Place Image"]);
    assert_eq!(placed_kind(&s), "pixels");
    // And `"smartObject": true` with the preference off makes a smart object again.
    s.execute("prefs.set", json!({"path": "general.alwaysCreateSmartObjectsWhenPlacing", "value": false})).unwrap();
    assert_eq!(added_by(&mut s, "file.placeEmbedded", json!({"path": path, "smartObject": true})), ["Place Embedded Smart Object"]);
    assert_eq!(placed_kind(&s), "embedded");
}

#[test]
fn a_one_page_pdf_is_named_after_what_its_place_made() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("page.pdf").to_string_lossy().into_owned();
    let mut src = Session::new();
    src.execute("file.new", json!({"width": 100, "height": 60, "depth": 8, "resolution": 72, "background": "#ff0000"})).unwrap();
    src.execute("file.saveACopy", json!({"path": pdf})).unwrap();
    for (smart, name, kind) in [(true, "Place Embedded Smart Object", "embedded"), (false, "Place Image", "pixels")] {
        let mut s = reference_document();
        s.execute("prefs.set", json!({"path": "general.alwaysCreateSmartObjectsWhenPlacing", "value": smart})).unwrap();
        assert_eq!(added_by(&mut s, "file.placeEmbedded", json!({"path": pdf})), [name], "smart objects {smart}");
        assert_eq!(placed_kind(&s), kind, "smart objects {smart}");
    }
}

#[test]
fn undo_and_redo_keep_the_place_name() {
    let (_dir, path) = reference_png_path();
    for (smart, name) in [(true, "Place Embedded Smart Object"), (false, "Place Image")] {
        let mut s = reference_document();
        let before = entries(&s);
        s.execute("file.placeEmbedded", json!({"path": path, "smartObject": smart})).unwrap();
        assert!(s.undo());
        assert_eq!(entries(&s), before, "smart {smart}: undo takes the place back");
        assert_eq!(s.active().unwrap().history.redo_label(), Some(name), "smart {smart}");
        assert!(s.redo());
        assert_eq!(s.active().unwrap().history.undo_label(), Some(name), "smart {smart}");
    }
}

#[test]
fn the_name_follows_the_kind_of_layer_a_place_makes() {
    assert_eq!(place_label(false, true), PLACE_EMBEDDED);
    assert_eq!(place_label(false, false), PLACE_IMAGE);
    // A linked place is always a smart object.
    assert_eq!(place_label(true, true), PLACE_LINKED);
    assert_eq!((PLACE_EMBEDDED, PLACE_IMAGE, PLACE_LINKED), ("Place Embedded Smart Object", "Place Image", "Place Linked Smart Object"));
}
