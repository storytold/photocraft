//! File › Open as Smart Object… (#11678): a file opens as a new untitled document holding one
//! embedded smart object layer.

use super::*;

fn tmp(name: &str) -> String {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("photocraft-open-as-so-{}-{name}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

/// A small PNG with a gradient, so the pixels differ from place to place.
fn write_png(dir: &str, name: &str, w: u32, h: u32) -> String {
    let px: Vec<u8> = (0..h).flat_map(|y| (0..w).flat_map(move |x| [(x * 255 / w) as u8, (y * 255 / h) as u8, ((x + y) % 256) as u8])).collect();
    let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgb, px).unwrap();
    let bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let path = join(dir, name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// A two-layer 16-bit PSD at 300 ppi tagged with Display P3, made with the engine's own save.
fn write_psd16(dir: &str, name: &str, w: u32, h: u32) -> String {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "depth": 16})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("image.imageSize", json!({"resolution": 300, "resample": "none"})).unwrap();
    s.execute("edit.assignProfile", json!({"profile": "display-p3"})).unwrap();
    let path = join(dir, name);
    save_doc(&s.active().unwrap().doc, &path, None).unwrap();
    path
}

fn imported(path: &str) -> Document {
    photocraft_io::import(&file_name(path), &std::fs::read(path).unwrap()).unwrap().document
}

fn active(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn names(s: &Session) -> Vec<String> {
    s.documents().iter().map(|d| d.doc.name.clone()).collect()
}

/// The one layer of the active document, which must be an embedded smart object.
fn embedded(s: &Session) -> (String, String, Arc<Vec<u8>>) {
    let d = active(s);
    assert_eq!(d.layers.len(), 1, "one layer, not a Background plus a layer");
    let layer = &d.layers[0];
    match &layer.content {
        LayerContent::Smart(so) => match &so.source {
            SmartSource::Embedded { file_name, bytes } => (layer.name.clone(), file_name.clone(), Arc::clone(bytes)),
            other => panic!("expected an embedded smart object, got {other:?}"),
        },
        _ => panic!("the layer is not a smart object"),
    }
}

#[test]
fn a_png_opens_as_an_untitled_document_with_one_embedded_smart_object() {
    let dir = tmp("png");
    let path = write_png(&dir, "a_test.png", 64, 48);
    let mut s = Session::new();
    let r = s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    assert_eq!(r["name"], "a_test-1", "{r}");
    assert_eq!(s.documents().len(), 1);
    let st = s.active().unwrap();
    assert_eq!(st.doc.name, "a_test-1");
    assert_eq!(st.path, None, "untitled: Save asks where to save");

    let file = imported(&path);
    let d = &st.doc;
    assert_eq!((d.size.width, d.size.height), (64, 48));
    assert_eq!(d.resolution_dpi, file.resolution_dpi);
    assert_eq!((d.mode, d.depth), (file.mode, file.depth));
    assert_eq!(d.icc_profile, file.icc_profile);

    let (layer, original, bytes) = embedded(&s);
    assert_eq!(layer, "a_test", "named after the file without its extension");
    assert_eq!(original, "a_test.png", "Edit Contents opens the original file name");
    assert_eq!(*bytes, std::fs::read(&path).unwrap(), "the file itself is embedded");
}

#[test]
fn the_smart_object_shows_the_file() {
    let dir = tmp("pixels");
    let path = write_png(&dir, "grad.png", 32, 16);
    let mut s = Session::new();
    s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    let want = photocraft_compose::render(&imported(&path), Rect::new(0, 0, 32, 16)).px.clone();
    let got = photocraft_compose::render(active(&s), Rect::new(0, 0, 32, 16)).px.clone();
    assert_eq!(got, want);
}

#[test]
fn a_16_bit_psd_keeps_size_resolution_mode_depth_and_profile() {
    let dir = tmp("psd");
    let path = write_psd16(&dir, "c.layered.psd", 40, 30);
    let mut s = Session::new();
    s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    let file = imported(&path);
    assert_eq!(file.resolution_dpi, 300.0);
    assert!(file.icc_profile.is_some());
    let d = active(&s);
    assert_eq!(d.name, "c.layered-1", "only the last extension is dropped");
    assert_eq!((d.size.width, d.size.height), (40, 30));
    assert_eq!(d.resolution_dpi, 300.0);
    assert_eq!((d.mode, d.depth), (file.mode, file.depth));
    assert_eq!(d.depth, photocraft_doc::SampleType::U16);
    assert_eq!(d.icc_profile, file.icc_profile);
    let (layer, original, _) = embedded(&s);
    assert_eq!((layer.as_str(), original.as_str()), ("c.layered", "c.layered.psd"));
}

#[test]
fn history_is_the_opening_snapshot_then_open_as_smart_object() {
    let dir = tmp("history");
    let path = write_png(&dir, "a_test.png", 16, 16);
    let mut s = Session::new();
    s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.history.entries().last().map(String::as_str), Some("Open As Smart Object"));
    assert_eq!(st.history.entries().len(), 2, "the opening state, then one step");
    assert_eq!(st.snapshots.first().map(|snap| snap.name.as_str()), Some("a_test"), "the opening snapshot is named after the layer");
}

#[test]
fn the_number_is_the_next_free_one_and_starts_again_after_they_close() {
    let dir = tmp("numbers");
    let path = write_png(&dir, "a_test.png", 8, 8);
    let mut s = Session::new();
    for _ in 0..3 {
        s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    }
    assert_eq!(names(&s), ["a_test-1", "a_test-2", "a_test-3"]);
    while !s.documents().is_empty() {
        s.close(0);
    }
    s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    assert_eq!(names(&s), ["a_test-1"]);
}

#[test]
fn a_plain_open_of_the_same_file_is_unchanged() {
    let dir = tmp("plain");
    let path = write_png(&dir, "a_test.png", 8, 8);
    let mut s = Session::new();
    s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    open_bytes_as(&mut s, &path, &std::fs::read(&path).unwrap(), None, Some(path.clone())).unwrap();
    assert_eq!(names(&s), ["a_test-1", "a_test.png"]);
    assert!(matches!(active(&s).layers[0].content, LayerContent::Raster(_)));
}

#[test]
fn several_files_open_one_document_each() {
    let dir = tmp("several");
    let a = write_png(&dir, "a_test.png", 8, 8);
    let b = write_psd16(&dir, "d_other.psd", 12, 10);
    let mut s = Session::new();
    let r = s.execute("file.openAsSmartObject", json!({"paths": [a, b]})).unwrap();
    assert_eq!(r["documents"].as_array().map(Vec::len), Some(2), "{r}");
    assert_eq!(names(&s), ["a_test-1", "d_other-1"]);
    for st in s.documents() {
        assert_eq!(st.doc.layers.len(), 1);
        assert!(matches!(st.doc.layers[0].content, LayerContent::Smart(_)));
    }
}

#[test]
fn a_missing_or_unreadable_file_is_an_error_not_a_crash() {
    let dir = tmp("bad");
    let junk = join(&dir, "notes.png");
    std::fs::write(&junk, b"this is not an image").unwrap();
    let text = join(&dir, "readme.txt");
    std::fs::write(&text, b"hello").unwrap();
    let mut s = Session::new();
    for path in [join(&dir, "missing.png"), junk, text] {
        let r = s.execute("file.openAsSmartObject", json!({"path": path}));
        assert!(r.is_err(), "{path}: {r:?}");
    }
    assert!(s.documents().is_empty(), "a failed open leaves no document behind");
}

#[test]
fn bad_params_are_errors() {
    let mut s = Session::new();
    for params in [json!({}), json!({"path": 7}), json!({"path": null}), json!({"paths": "a.png"}), json!({"paths": []}), json!({"paths": [1]})] {
        let r = s.execute("file.openAsSmartObject", params.clone());
        assert!(r.is_err(), "{params}: {r:?}");
    }
    assert!(s.documents().is_empty());
}
