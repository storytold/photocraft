//! #14499: Apply Data Set refuses with the reference application's alert when a Text value would
//! replace the text of a type layer under Lock All. Nothing of the set is applied, but one history
//! state that changes nothing is still added. Visibility and Pixel Replacement values keep
//! applying to locked layers; partial locks and Data Sets as Files keep today's behaviour.

use super::*;
use photocraft_doc::{Layer, Locks, TextLayer};
use photocraft_geom::Rect;

const APPLY: &str = "image.applyDataSet";

/// A document with a raster layer "photo", a red raster "badge" and a text layer "title", bound
/// to the variables `photo` (pixels), `showBadge`, `showTitle` (visibility) and `headline` (text).
fn session() -> (Session, LayerId, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let (photo, badge, title) = s
        .edit("setup", |doc, _| {
            let fmt = doc.pixel_format();
            let mut photo = Layer::raster("photo", fmt);
            photo.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 64, 48), &photocraft_raster::from_rgba(&fmt, [0.5, 0.5, 0.5, 1.0]));
            let mut badge = Layer::raster("badge", fmt);
            badge.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 10, 10), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
            let title = Layer::new("title", LayerContent::Text(TextLayer { text: "Old".into(), ..Default::default() }));
            let ids = (photo.id, badge.id, title.id);
            doc.layers.extend([photo, badge, title]);
            Ok(ids)
        })
        .unwrap();
    s.execute(
        "image.variables.define",
        json!({"defs": [
            {"name": "photo", "layer": photo.0, "type": "pixelReplacement", "method": "conform"},
            {"name": "showBadge", "layer": badge.0, "type": "visibility"},
            {"name": "showTitle", "layer": title.0, "type": "visibility"},
            {"name": "headline", "layer": title.0, "type": "textReplacement"},
        ]}),
    )
    .unwrap();
    (s, photo, badge, title)
}

/// A blue 8x8 PNG for the `photo` variable.
fn blue_png(name: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("pc-vars-{}-locked-{name}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let png = dir.join("blue.png");
    let mut src = Session::new();
    src.execute("file.new", json!({"width": 8, "height": 8, "background": "#0000ff"})).unwrap();
    let bytes = photocraft_io::export(&src.active().unwrap().doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes;
    std::fs::write(&png, bytes).unwrap();
    (dir.to_string_lossy().into_owned(), png.to_string_lossy().into_owned())
}

fn data_sets(s: &mut Session, sets: Value) {
    s.execute("image.variables.dataSets", json!({ "dataSets": sets })).unwrap();
}

fn lock(s: &mut Session, id: LayerId, locks: Locks) {
    s.edit("lock", |doc, _| {
        doc.layer_mut(id).unwrap().locks = locks;
        Ok(())
    })
    .unwrap();
}

fn all() -> Locks {
    Locks { all: true, ..Locks::default() }
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn text_of(s: &Session, id: LayerId) -> String {
    match &doc(s).layer(id).unwrap().content {
        LayerContent::Text(t) => t.text.clone(),
        _ => panic!("not a text layer"),
    }
}

fn visible(s: &Session, id: LayerId) -> bool {
    doc(s).layer(id).unwrap().visible
}

fn pixel(s: &Session, id: LayerId) -> Vec<f32> {
    doc(s).layer(id).unwrap().surface().unwrap().pixel(32, 24)
}

fn history(s: &Session) -> (usize, Option<String>) {
    let h = &s.active().unwrap().history;
    (h.past_len(), h.entries().last().map(|e| e.to_string()))
}

#[test]
fn a_text_value_for_a_locked_type_layer_applies_nothing_and_alerts() {
    let (mut s, photo, badge, title) = session();
    let (dir, png) = blue_png("refuse");
    data_sets(
        &mut s,
        json!([{"name": "Zed", "values": [
            {"variable": "photo", "kind": "pixels", "value": png},
            {"variable": "showBadge", "kind": "visibility", "value": false},
            {"variable": "showTitle", "kind": "visibility", "value": false},
            {"variable": "headline", "kind": "text", "value": "Alpha\rLine2"},
        ]}]),
    );
    lock(&mut s, title, all());
    let photo_before = pixel(&s, photo);
    let (n, _) = history(&s);

    let r = s.execute(APPLY, json!({"name": "Zed"}));
    assert!(matches!(&r, Err(EngineError::Other(m)) if m == DATA_SET_LAYER_LOCKED), "{r:?}");
    assert_eq!(DATA_SET_LAYER_LOCKED, "Could not apply data set because the layer is locked.");

    // Nothing of the set: the text, both visibilities and the photo's pixels stay.
    assert_eq!(text_of(&s, title), "Old");
    assert!(visible(&s, title) && visible(&s, badge));
    assert_eq!(pixel(&s, photo), photo_before);
    assert_eq!(doc(&s).variables.active, None, "the set did not become the applied one");

    // One history state for the attempt, which changes nothing.
    let (after, label) = history(&s);
    assert_eq!(after, n + 1);
    assert!(label.as_deref().is_some_and(|l| l.starts_with("Apply Data Set")), "{label:?}");
    let applied = doc(&s).clone();
    assert!(s.undo());
    assert_eq!(history(&s).0, n);
    assert_eq!(text_of(&s, title), "Old");
    assert!(s.redo());
    assert_eq!(doc(&s).layers, applied.layers, "the state changes nothing");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_same_set_applies_once_the_layer_is_unlocked() {
    let (mut s, _photo, badge, title) = session();
    data_sets(
        &mut s,
        json!([{"name": "Zed", "values": [
            {"variable": "showBadge", "kind": "visibility", "value": false},
            {"variable": "headline", "kind": "text", "value": "Alpha"},
        ]}]),
    );
    lock(&mut s, title, all());
    assert!(s.execute(APPLY, json!({"name": "Zed"})).is_err());
    lock(&mut s, title, Locks::default());
    s.execute(APPLY, json!({"name": "Zed"})).unwrap();
    assert_eq!(text_of(&s, title), "Alpha");
    assert!(!visible(&s, badge));
    assert_eq!(doc(&s).variables.active, Some(0));
}

#[test]
fn visibility_values_still_apply_to_locked_layers() {
    let (mut s, _photo, badge, title) = session();
    data_sets(
        &mut s,
        json!([{"name": "Hide", "values": [
            {"variable": "showBadge", "kind": "visibility", "value": false},
            {"variable": "showTitle", "kind": "visibility", "value": false},
        ]}]),
    );
    lock(&mut s, badge, all());
    lock(&mut s, title, all());
    s.execute(APPLY, json!({"name": "Hide"})).unwrap();
    assert!(!visible(&s, badge) && !visible(&s, title));
}

#[test]
fn pixel_replacement_still_applies_to_a_locked_pixel_layer() {
    let (mut s, photo, _badge, _title) = session();
    let (dir, png) = blue_png("pixels");
    data_sets(&mut s, json!([{"name": "Blue", "values": [{"variable": "photo", "kind": "pixels", "value": png}]}]));
    lock(&mut s, photo, all());
    s.execute(APPLY, json!({"name": "Blue"})).unwrap();
    assert_eq!(pixel(&s, photo), vec![0.0, 0.0, 1.0, 1.0]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn partial_locks_keep_applying_text() {
    let partial = [
        Locks { transparency: true, ..Locks::default() },
        Locks { pixels: true, ..Locks::default() },
        Locks { position: true, ..Locks::default() },
        Locks { transparency: true, pixels: true, position: true, artboard: true, all: false },
    ];
    for locks in partial {
        let (mut s, _photo, _badge, title) = session();
        data_sets(&mut s, json!([{"name": "T", "values": [{"variable": "headline", "kind": "text", "value": "New"}]}]));
        lock(&mut s, title, locks);
        s.execute(APPLY, json!({"name": "T"})).unwrap_or_else(|e| panic!("{locks:?}: {e}"));
        assert_eq!(text_of(&s, title), "New", "{locks:?}");
    }
}

#[test]
fn data_sets_as_files_keeps_exporting_a_locked_type_layer() {
    let (mut s, _photo, _badge, title) = session();
    data_sets(&mut s, json!([{"name": "T", "values": [{"variable": "headline", "kind": "text", "value": "New"}]}]));
    lock(&mut s, title, all());
    let dir = std::env::temp_dir().join(format!("pc-vars-{}-locked-export", std::process::id()));
    let out = s.execute("file.export.dataSetsAsFiles", json!({"dir": dir.to_string_lossy(), "format": "png"})).unwrap();
    assert_eq!(out["files"].as_array().map(Vec::len), Some(1), "{out}");
    assert_eq!(text_of(&s, title), "Old", "the export leaves the document alone");
    let _ = std::fs::remove_dir_all(dir);
}
