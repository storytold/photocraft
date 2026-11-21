//! #14500: Apply Data Set with a Pixel Replacement value naming a file that does not exist fails
//! with the reference application's alert, the error naming the file. Nothing of the set is
//! applied, but one history state that changes nothing is still added. A file that exists but
//! cannot be decoded keeps an error naming the file (not measured).

use super::*;
use photocraft_doc::{Layer, TextLayer};
use photocraft_geom::Rect;

const APPLY: &str = "image.applyDataSet";

/// A document with a grey raster "photo", a red raster "badge" and a text layer "title", bound to
/// the variables `photo` (pixels), `showBadge` (visibility) and `headline` (text).
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
            {"name": "headline", "layer": title.0, "type": "textReplacement"},
        ]}),
    )
    .unwrap();
    (s, photo, badge, title)
}

/// A fresh scratch folder for one test.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pc-vars-{}-missing-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The set "Two": hides the badge, replaces the headline and the photo with `png`.
fn set_two(s: &mut Session, png: &str) {
    s.execute(
        "image.variables.dataSets",
        json!({"dataSets": [{"name": "Two", "values": [
            {"variable": "showBadge", "kind": "visibility", "value": false},
            {"variable": "headline", "kind": "text", "value": "New"},
            {"variable": "photo", "kind": "pixels", "value": png},
        ]}]}),
    )
    .unwrap();
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

fn history(s: &Session) -> (usize, Option<String>) {
    let h = &s.active().unwrap().history;
    (h.past_len(), h.entries().last().map(|e| e.to_string()))
}

#[test]
fn a_missing_replacement_file_fails_with_the_alert_text_and_names_the_file() {
    let (mut s, ..) = session();
    let dir = scratch("text");
    let png = dir.join("renamed.png").to_string_lossy().into_owned();
    set_two(&mut s, &png);

    let r = s.execute(APPLY, json!({"name": "Two"}));
    let Err(EngineError::Other(m)) = &r else { panic!("{r:?}") };
    assert_eq!(DATA_SET_FILE_MISSING, "Could not apply data set because the replacement file was not found.");
    assert!(m.starts_with(DATA_SET_FILE_MISSING), "{m}");
    assert!(m.contains(&png), "the error names the file: {m}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_missing_replacement_file_applies_nothing_but_adds_one_history_state() {
    let (mut s, photo, badge, title) = session();
    let dir = scratch("nothing");
    set_two(&mut s, &dir.join("renamed.png").to_string_lossy());
    let before = doc(&s).clone();
    let (n, _) = history(&s);

    assert!(s.execute(APPLY, json!({"name": "Two"})).is_err());

    // Nothing of the set: also not its valid Visibility and Text values.
    assert!(doc(&s).layer(badge).unwrap().visible);
    assert_eq!(text_of(&s, title), "Old");
    assert_eq!(doc(&s).layer(photo).unwrap().surface().unwrap().pixel(32, 24), before.layer(photo).unwrap().surface().unwrap().pixel(32, 24));
    assert_eq!(doc(&s).layers, before.layers);
    assert_eq!(doc(&s).variables.active, None, "the set did not become the applied one");

    // One history state for the attempt, which changes nothing.
    let (after, label) = history(&s);
    assert_eq!(after, n + 1);
    assert!(label.as_deref().is_some_and(|l| l.starts_with("Apply Data Set")), "{label:?}");
    assert!(s.undo());
    assert_eq!(history(&s).0, n);
    assert_eq!(doc(&s).layers, before.layers);
    assert!(s.redo());
    assert_eq!(doc(&s).layers, before.layers, "the state changes nothing");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_file_that_cannot_be_decoded_keeps_an_error_naming_it_and_no_history_state() {
    let (mut s, _photo, badge, _title) = session();
    let dir = scratch("garbage");
    let png = dir.join("broken.png");
    std::fs::write(&png, b"not an image at all").unwrap();
    let png = png.to_string_lossy().into_owned();
    set_two(&mut s, &png);
    let (n, _) = history(&s);

    let r = s.execute(APPLY, json!({"name": "Two"}));
    let Err(e) = &r else { panic!("{r:?}") };
    let m = e.to_string();
    assert!(!m.contains(DATA_SET_FILE_MISSING), "not the missing-file alert: {m}");
    assert!(m.contains("broken.png"), "the error names the file: {m}");
    assert!(doc(&s).layer(badge).unwrap().visible, "nothing applied");
    assert_eq!(history(&s).0, n, "today's behaviour: no history state");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_same_set_applies_once_the_file_is_back() {
    let (mut s, photo, badge, title) = session();
    let dir = scratch("back");
    let png = dir.join("blue.png");
    set_two(&mut s, &png.to_string_lossy());
    assert!(s.execute(APPLY, json!({"name": "Two"})).is_err());

    let mut src = Session::new();
    src.execute("file.new", json!({"width": 8, "height": 8, "background": "#0000ff"})).unwrap();
    let bytes = photocraft_io::export(&src.active().unwrap().doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes;
    std::fs::write(&png, bytes).unwrap();
    s.execute(APPLY, json!({"name": "Two"})).unwrap();
    assert!(!doc(&s).layer(badge).unwrap().visible);
    assert_eq!(text_of(&s, title), "New");
    assert_eq!(doc(&s).layer(photo).unwrap().surface().unwrap().pixel(32, 24), vec![0.0, 0.0, 1.0, 1.0]);
    assert_eq!(doc(&s).variables.active, Some(0));
    let _ = std::fs::remove_dir_all(dir);
}
