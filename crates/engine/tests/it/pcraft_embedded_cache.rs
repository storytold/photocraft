//! Open a .pcraft whose embedded Smart Object was saved without a render cache (#15799).

use std::sync::Arc;

use photocraft_doc::{Document, LayerContent, SmartObject, SmartSource, StackMode};
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
    let outer = &placed.active().unwrap().doc;
    let actual = object(outer).cache.as_ref().unwrap();
    assert_eq!(actual.read_region(bounds), cache.read_region(bounds), "opening the native document as a Smart Object composites the recovered pixels");

    for stack in [None, Some(StackMode::Maximum)] {
        let mut layer = outer.layers[0].clone();
        let LayerContent::Smart(smart) = &mut layer.content else { panic!("expected a Smart Object") };
        smart.stack_mode = stack;
        assert!(photocraft_engine::smart_cmds::refresh_layer(outer, &mut layer).unwrap());
        let LayerContent::Smart(refreshed) = &layer.content else { panic!("the refreshed layer is a Smart Object") };
        assert_eq!(refreshed.cache.as_ref().unwrap().read_region(bounds), actual.read_region(bounds), "refresh keeps the pixels recovered at open");
    }

    // A later source can't recover the embedded PNG with this document profile.
    let mut different_profile = reference;
    different_profile.icc_profile = Some(craft_cms::Builtin::DisplayP3.profile().to_bytes());
    let mut layer = outer.layers[0].clone();
    let LayerContent::Smart(smart) = &mut layer.content else { panic!("expected a Smart Object") };
    smart.source = SmartSource::Embedded { file_name: "different-profile.pcraft".into(), bytes: Arc::new(without_cache(different_profile)) };
    let failed = photocraft_engine::smart_cmds::refresh_layer(outer, &mut layer).unwrap_err();
    assert!(failed.to_string().contains("couldn't rebuild the cache for"));
    let LayerContent::Smart(smart) = &layer.content else { panic!("expected a Smart Object") };
    assert_eq!(smart.cache.as_ref(), Some(actual), "an incomplete refresh must retain the current image");
}

#[test]
fn native_revert_returns_the_named_warning_for_uncached_linked_art() {
    let scratch = tempfile::tempdir().unwrap();
    let mut reference = placed_png(scratch.path());
    reference.layers[0].name = "Linked art".to_string();
    smart_mut(&mut reference).source = SmartSource::Linked { path: "not-present.png".to_string() };
    let bundle = without_cache(reference);
    let path = scratch.path().join("linked-cache.pcraft");
    std::fs::write(&path, &bundle).unwrap();

    let mut opened = Session::new();
    let before = file_cmds::open_bytes_as(&mut opened, "linked-cache.pcraft", &bundle, None, Some(path.to_str().unwrap().to_string())).unwrap();
    assert!(before["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("Linked art")));
    let reverted = opened.execute("file.revert", json!({})).unwrap();
    assert_eq!(reverted["warnings"], before["warnings"], "Revert must report the same warning with the affected layer name");
    assert!(object(&opened.active().unwrap().doc).cache.is_none());
}

#[test]
fn native_place_and_open_as_smart_object_recover_or_reject_embedded_caches() {
    let scratch = tempfile::tempdir().unwrap();
    let reference = placed_png(scratch.path());
    let cache = object(&reference).cache.as_ref().unwrap();
    let bounds = cache.content_bounds();

    let recoverable = scratch.path().join("recoverable.pcraft");
    std::fs::write(&recoverable, without_cache(reference.clone())).unwrap();

    let mut host = Session::new();
    host.execute("file.new", json!({ "width": 8, "height": 8 })).unwrap();
    host.execute("file.placeEmbedded", json!({ "path": recoverable })).unwrap();
    let placed_layer = host.active().unwrap().doc.layers.last().unwrap();
    let LayerContent::Smart(placed) = &placed_layer.content else { panic!("expected placed Smart Object") };
    assert_eq!(placed.cache.as_ref().unwrap().read_region(bounds), cache.read_region(bounds));

    let mut unrecoverable = reference;
    unrecoverable.icc_profile = Some(craft_cms::Builtin::DisplayP3.profile().to_bytes());
    let unrecoverable_path = scratch.path().join("unrecoverable.pcraft");
    std::fs::write(&unrecoverable_path, without_cache(unrecoverable)).unwrap();

    let place_err = host.execute("file.placeEmbedded", json!({ "path": unrecoverable_path })).unwrap_err();
    assert!(place_err.to_string().contains("couldn't rebuild the cache for \"embedded\""), "{place_err}");

    let mut opener = Session::new();
    let open_err = opener.execute("file.openAsSmartObject", json!({ "path": unrecoverable_path })).unwrap_err();
    assert!(open_err.to_string().contains("couldn't rebuild the cache for \"embedded\""), "{open_err}");
}

#[test]
fn native_open_does_not_treat_developed_note_in_layer_name_as_raw() {
    let scratch = tempfile::tempdir().unwrap();
    let mut reference = placed_png(scratch.path());
    reference.resolution_dpi = 72.0;
    reference.icc_profile = None;
    reference.layers[0].name = format!("Photo {}", photocraft_io::raw::DEVELOPED_NOTE);
    smart_mut(&mut reference).source = SmartSource::Linked { path: "not-present.png".to_string() };
    let bundle = without_cache(reference);
    let path = scratch.path().join("native-note.pcraft");
    std::fs::write(&path, &bundle).unwrap();

    let mut session = Session::new();
    session.prefs.edit(|p| p.raw_defaults.resolution = 300.0);
    session.execute("edit.colorSettings", json!({ "askOnMissing": true })).unwrap();

    let report = match session.start_open("native-note.pcraft", OpenSource::Bytes(Arc::new(bundle.clone()))).unwrap() {
        Started::Done(report) => report,
        Started::Job(id) => session.wait_job(id).unwrap(),
    };
    assert_eq!(report["color"]["missing"], json!(true), "native open must keep the missing-profile question");
    let held = report["held"].as_u64().expect("held for profile question");
    let (doc, _) = session.take_held_open(held).expect("held document");
    assert_eq!(doc.resolution_dpi, 72.0, "native open must keep saved resolution");

    let smart_report = session.execute("file.openAsSmartObject", json!({ "path": path })).unwrap();
    assert_eq!(smart_report["color"]["missing"], json!(true), "Open as Smart Object must keep the missing-profile question");
    let (smart_doc, _) = session.take_held_open(smart_report["held"].as_u64().expect("held smart open")).expect("held smart doc");
    assert_eq!(smart_doc.resolution_dpi, 72.0, "Open as Smart Object must keep saved resolution");

    let mut direct = Session::new();
    direct.prefs.edit(|p| p.raw_defaults.resolution = 300.0);
    file_cmds::open_bytes_as(&mut direct, "native-note.pcraft", &bundle, None, None).unwrap();
    assert_eq!(direct.active().unwrap().doc.resolution_dpi, 72.0);
}
