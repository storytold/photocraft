//! Open a .pcraft whose embedded Smart Object was saved without a render cache (#15799).

use std::sync::Arc;

use photocraft_doc::{Document, LayerContent, SmartObject};
use photocraft_engine::jobs::{OpenSource, Started};
use photocraft_engine::{Session, file_cmds};
use serde_json::json;

fn object(doc: &Document) -> &SmartObject {
    let LayerContent::Smart(smart) = &doc.layers[0].content else { panic!("expected a Smart Object") };
    smart
}

fn placed_png(directory: &std::path::Path) -> Document {
    let source = directory.join("embedded.png");
    let pixels: Vec<u8> = (0..64).flat_map(|i| [i * 3, 255 - i, i * 2, 255]).collect();
    let image = photocraft_codecs::Image::from_u8(8, 8, photocraft_codecs::ChannelLayout::Rgba, pixels).unwrap();
    let encoded = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    std::fs::write(&source, encoded).unwrap();

    let mut placed = Session::new();
    placed.execute("file.openAsSmartObject", json!({ "path": source })).unwrap();
    (*placed.active().unwrap().doc).clone()
}

fn smart_mut(doc: &mut Document) -> &mut SmartObject {
    let LayerContent::Smart(smart) = &mut doc.layers[0].content else { panic!("expected a Smart Object") };
    smart
}

fn without_cache(mut doc: Document) -> Vec<u8> {
    smart_mut(&mut doc).cache = None;
    photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap()
}

#[test]
fn a_native_open_rebuilds_the_missing_cache_of_an_embedded_png() {
    let scratch = tempfile::tempdir().unwrap();
    let mut reference = placed_png(scratch.path());
    smart_mut(&mut reference).transform.m[4] = 2.0;
    let expected_placement = photocraft_engine::smart_cmds::render(&reference, object(&reference)).unwrap();
    smart_mut(&mut reference).cache = expected_placement;
    let saved_cache = object(&reference).cache.as_ref().unwrap();
    assert!(!saved_cache.content_bounds().is_empty(), "the original Smart Object has visible pixels");

    let bundle = without_cache(reference.clone());

    let mut opened = Session::new();
    let report = file_cmds::open_bytes_as(&mut opened, "missing.pcraft", &bundle, None, None).unwrap();
    let result = &opened.active().unwrap().doc;
    assert_eq!(object(result).cache.as_ref(), Some(saved_cache), "the decoder must rebuild the same pixels as Open as Smart Object");
    assert_eq!(object(result).source, object(&reference).source, "the original remains editable");
    assert_eq!(report["warnings"], json!([]), "a recovered layer has no missing-cache warning");
}

#[test]
fn native_open_jobs_recover_embedded_pixels_using_the_actual_format() {
    let scratch = tempfile::tempdir().unwrap();
    let reference = placed_png(scratch.path());
    let bundle = without_cache(reference.clone());
    let path = scratch.path().join("renamed.png");
    std::fs::write(&path, &bundle).unwrap();
    for source in [OpenSource::Bytes(Arc::new(bundle)), OpenSource::Path(path.to_str().unwrap().to_string())] {
        let mut session = Session::new();
        let report = match session.start_open("renamed.png", source).unwrap() {
            Started::Done(report) => report,
            Started::Job(id) => session.wait_job(id).unwrap(),
        };
        assert_eq!(report["warnings"], json!([]));
        assert_eq!(object(&session.active().unwrap().doc).cache, object(&reference).cache);
    }
}

#[test]
fn a_different_embed_color_profile_keeps_the_missing_cache_warning() {
    let scratch = tempfile::tempdir().unwrap();
    let mut reference = placed_png(scratch.path());
    reference.icc_profile = Some(craft_cms::Builtin::DisplayP3.profile().to_bytes());
    let bundle = without_cache(reference);
    let mut opened = Session::new();
    let report = file_cmds::open_bytes_as(&mut opened, "different-profile.pcraft", &bundle, None, None).unwrap();
    let warnings = report["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].as_str().unwrap().contains("embedded"));
    assert!(object(&opened.active().unwrap().doc).cache.is_none());
}

#[test]
fn native_reopen_operations_keep_recoverable_pixels() {
    let scratch = tempfile::tempdir().unwrap();
    let reference = placed_png(scratch.path());
    let path = scratch.path().join("reopen.pcraft");
    let bundle = without_cache(reference.clone());
    std::fs::write(&path, &bundle).unwrap();
    let cache = object(&reference).cache.as_ref().unwrap();

    let mut session = Session::new();
    file_cmds::open_bytes_as(&mut session, "reopen.pcraft", &bundle, None, Some(path.to_str().unwrap().to_string())).unwrap();
    session.execute("file.revert", json!({})).unwrap();
    assert_eq!(object(&session.active().unwrap().doc).cache.as_ref(), Some(cache), "Revert keeps recoverable pixels");

    let mut placed = Session::new();
    placed.execute("file.openAsSmartObject", json!({ "path": path })).unwrap();
    let bounds = cache.content_bounds();
    let actual = object(&placed.active().unwrap().doc).cache.as_ref().unwrap();
    assert_eq!(actual.read_region(bounds), cache.read_region(bounds), "opening the native document as a Smart Object composites the recovered pixels");
}
