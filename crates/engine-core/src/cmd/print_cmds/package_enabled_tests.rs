//! File › Package… is enabled only for a document saved to disk that has a linked smart object,
//! at any depth of embedded smart objects (#14073). The helpers are shared with
//! `package_missing_link_tests`.

use std::sync::Arc;

use photocraft_doc::{Affine, Layer, SampleType, Size};

use super::*;

/// An empty folder of its own, spelt with `/` like command results.
pub(super) fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-pkgenabled-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().replace('\\', "/")
}

/// Writes a 4 x 4 solid PNG at `path` and returns `path`.
pub(super) fn png(path: &str, colour: &str) -> String {
    let mut src = crate::session();
    src.execute("file.new", json!({"width": 4, "height": 4, "background": colour})).unwrap();
    crate::file_cmds::save_doc(&src.active().unwrap().doc, path, None).unwrap();
    path.to_string()
}

/// A 40 x 30 document named Pack, never saved.
pub(super) fn unsaved() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 40, "height": 30, "name": "Pack", "background": "#3366cc"})).unwrap();
    s
}

/// Saves the active document as `path` and makes that its file, with no unsaved changes.
pub(super) fn save_at(s: &mut Session, path: &str) {
    crate::file_cmds::save_doc(&s.active().unwrap().doc, path, None).unwrap();
    let d = s.active_mut().unwrap();
    d.path = Some(path.to_string());
    d.saved_revision = d.revision;
}

/// [`save_at`] `<dir>/Pack.pcraft`.
pub(super) fn save(s: &mut Session, dir: &str) {
    save_at(s, &format!("{dir}/Pack.pcraft"));
}

/// [`unsaved`], saved in `dir` with `layers` added first.
pub(super) fn saved_with(dir: &str, layers: Vec<Layer>) -> Session {
    let mut s = unsaved();
    s.edit("Add layers", |d, _| {
        d.layers.extend(layers);
        Ok(())
    })
    .unwrap();
    save(&mut s, dir);
    s
}

/// An empty RGB 8-bit pixel layer.
pub(super) fn plain(name: &str) -> Layer {
    Layer::raster(name, Document::pixel_format_for(ColorMode::Rgb, SampleType::U8))
}

/// A smart object layer named `name` with `source`.
pub(super) fn smart(name: &str, source: SmartSource) -> Layer {
    Layer::new(name, LayerContent::Smart(SmartObject::new(source, Affine::IDENTITY, None)))
}

/// A smart object layer linked to `path`.
pub(super) fn linked(name: &str, path: &str) -> Layer {
    smart(name, SmartSource::Linked { path: path.into() })
}

/// An embedded smart object layer whose `.pcraft` contents hold `layers`.
pub(super) fn embedded(name: &str, layers: Vec<Layer>) -> Layer {
    let mut inner = Document::new(name, Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
    inner.layers.extend(layers);
    let bytes = crate::smart_cmds::encode_source(&inner).unwrap();
    smart(name, SmartSource::Embedded { file_name: format!("{name}.pcraft"), bytes: Arc::new(bytes) })
}

/// `layer` inside `levels` embedded smart objects, one in the other.
pub(super) fn nested(layer: Layer, levels: usize) -> Layer {
    (0..levels).fold(layer, |l, i| embedded(&format!("Level {i}"), vec![l]))
}

/// Adds a smart object `name` the way a PSD keeps a placed file: the source is a uuid and the
/// file's bytes (`file`) sit in the document's linked-layer block.
pub(super) fn add_psd_embedded(s: &mut Session, name: &str, file: &str) {
    let f = photocraft_io::linked::LinkedFile { uuid: format!("{name}-uuid"), file_name: file_name(file), bytes: std::fs::read(file).unwrap() };
    s.edit("PSD placement", |d, _| {
        d.metadata.psd_global_blocks.push((*b"8BIM", *b"lnk2", Arc::new(photocraft_io::linked::encode_linked_file(&f))));
        d.layers.push(smart(name, SmartSource::Linked { path: f.uuid.clone() }));
        Ok(())
    })
    .unwrap();
}

/// The reason File › Package is greyed, `None` when it is enabled.
pub(super) fn greyed(s: &Session) -> Option<String> {
    s.disabled_reason("file.package")
}

#[test]
fn greyed_without_a_document() {
    let mut s = crate::session();
    assert_eq!(greyed(&s).as_deref(), Some("no document open"));
    let dir = tmp("no-doc");
    let e = s.execute("file.package", json!({"dir": format!("{dir}/pkg")})).unwrap_err();
    assert!(e.to_string().contains("no document open"), "{e}");
    assert!(!std::path::Path::new(&format!("{dir}/pkg")).exists());
}

/// The measured case: a new 50 x 50 document was packaged as `<dir>/Untitled/Untitled.pcraft`.
#[test]
fn greyed_for_a_document_never_saved() {
    let dir = tmp("never-saved");
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 50, "height": 50})).unwrap();
    assert_eq!(greyed(&s).as_deref(), Some("the document is not saved to disk"));
    // A linked smart object doesn't make up for the missing file.
    s.edit("Link", |d, _| {
        d.layers.push(linked("Red", &png(&format!("{dir}/red.png"), "#ff0000")));
        Ok(())
    })
    .unwrap();
    assert_eq!(greyed(&s).as_deref(), Some("the document is not saved to disk"));
    let e = s.execute("file.package", json!({"dir": format!("{dir}/pkg")})).unwrap_err();
    assert!(e.to_string().contains("the document is not saved to disk"), "{e}");
    assert!(!std::path::Path::new(&format!("{dir}/pkg")).exists());
    // A path whose file is gone is no file on disk either.
    s.active_mut().unwrap().path = Some(format!("{dir}/gone.pcraft"));
    assert_eq!(greyed(&s).as_deref(), Some("the document is not saved to disk"));
    save(&mut s, &dir);
    assert_eq!(greyed(&s), None);
}

#[test]
fn greyed_for_a_saved_document_without_a_link() {
    let dir = tmp("no-link");
    // Layers, an embedded smart object and contents without a link inside are no link.
    let s = saved_with(&dir, vec![plain("Plain"), embedded("Outer", vec![plain("Inner")])]);
    assert_eq!(greyed(&s).as_deref(), Some("the document has no linked smart object"));
}

/// The reference application measured: enabled with a linked smart object, also with unsaved
/// changes, and also when its file is missing (Package then reports it, #14076).
#[test]
fn enabled_for_a_saved_document_with_a_link() {
    let dir = tmp("link");
    let red = png(&format!("{dir}/red.png"), "#ff0000");
    let mut s = saved_with(&dir, vec![linked("Red", &red)]);
    assert_eq!(greyed(&s), None);
    assert!(s.is_enabled("file.package"));
    assert!(!s.active().unwrap().is_dirty());
    s.execute("layer.new.layer", json!({})).unwrap();
    assert!(s.active().unwrap().is_dirty(), "the document has unsaved changes");
    assert_eq!(greyed(&s), None);
    std::fs::remove_file(&red).unwrap();
    assert_eq!(greyed(&s), None, "a missing linked file still counts");
}

/// A link inside a group counts like one at the top.
#[test]
fn enabled_for_a_link_inside_a_group() {
    let dir = tmp("group");
    let red = png(&format!("{dir}/red.png"), "#ff0000");
    let s = saved_with(&dir, vec![Layer::group("Group", vec![linked("Red", &red)])]);
    assert_eq!(greyed(&s), None);
}

/// The measured nested case: the only link sits inside an embedded smart object.
#[test]
fn enabled_for_a_link_inside_an_embedded_smart_object() {
    let dir = tmp("nested");
    let red = png(&format!("{dir}/red.png"), "#ff0000");
    let s = saved_with(&dir, vec![embedded("Outer", vec![linked("Red", &red)])]);
    assert_eq!(greyed(&s), None);
    let s = saved_with(&dir, vec![nested(linked("Red", &red), 3)]);
    assert_eq!(greyed(&s), None, "three levels down");
}

/// Embedded contents are opened [`PACKAGE_NESTING`] levels down, not further.
#[test]
fn nested_links_are_looked_for_down_to_the_nesting_limit() {
    let dir = tmp("deep");
    let red = png(&format!("{dir}/red.png"), "#ff0000");
    let s = saved_with(&dir, vec![nested(linked("Red", &red), PACKAGE_NESTING)]);
    assert_eq!(greyed(&s), None, "{PACKAGE_NESTING} levels down");
    let s = saved_with(&dir, vec![nested(linked("Red", &red), PACKAGE_NESTING + 1)]);
    assert_eq!(greyed(&s).as_deref(), Some("the document has no linked smart object"));
}

/// Smart objects a PSD keeps embedded have a uuid for a path: they are not links.
#[test]
fn greyed_when_the_only_smart_objects_are_embedded_in_a_psd() {
    let dir = tmp("psd-embedded");
    let red = png(&format!("{dir}/red.png"), "#ff0000");
    let mut s = saved_with(&dir, vec![]);
    add_psd_embedded(&mut s, "Placed", &red);
    assert_eq!(greyed(&s).as_deref(), Some("the document has no linked smart object"));
    // A real link next to it enables Package.
    s.edit("Link", |d, _| {
        d.layers.push(linked("Red", &red));
        Ok(())
    })
    .unwrap();
    assert_eq!(greyed(&s), None);
}

/// The answer for embedded contents is remembered, but follows contents that are replaced.
#[test]
fn replaced_contents_are_looked_at_again() {
    let dir = tmp("replaced");
    let red = png(&format!("{dir}/red.png"), "#ff0000");
    let mut s = saved_with(&dir, vec![embedded("Outer", vec![plain("Inner")])]);
    let outer = s.active().unwrap().doc.layers.last().unwrap().id;
    assert!(greyed(&s).is_some());
    let replace = |s: &mut Session, contents: Layer| {
        s.edit("Replace Contents", |d, _| {
            d.layer_mut(outer).unwrap().content = contents.content;
            Ok(())
        })
        .unwrap();
    };
    replace(&mut s, embedded("Outer", vec![linked("Red", &red)]));
    assert_eq!(greyed(&s), None);
    replace(&mut s, embedded("Outer", vec![plain("Inner")]));
    assert_eq!(greyed(&s).as_deref(), Some("the document has no linked smart object"));
}

/// Contents repeated by every smart object of every level are opened once per level, not once
/// per use: six copies [`PACKAGE_NESTING`] levels deep would otherwise be 6^8 decodes (minutes,
/// against milliseconds).
#[test]
fn repeated_contents_are_opened_once() {
    let dir = tmp("repeated");
    let mut layers = vec![plain("Inner")];
    for i in 0..PACKAGE_NESTING {
        let name = format!("Level {i}");
        let LayerContent::Smart(so) = embedded(&name, layers).content else { panic!("an embedded smart object") };
        layers = (0..6).map(|_| smart(&name, so.source.clone())).collect();
    }
    let s = saved_with(&dir, layers);
    let start = std::time::Instant::now();
    assert_eq!(greyed(&s).as_deref(), Some("the document has no linked smart object"));
    assert!(!package_link_missing(&s));
    assert!(start.elapsed() < std::time::Duration::from_secs(20), "took {:?}", start.elapsed());
}
