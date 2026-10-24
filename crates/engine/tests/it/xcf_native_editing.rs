//! Exercise the ordinary command/history path with published XCF pixel and selection facts.
#![cfg(feature = "corpus")]

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Document, Layer};
use photocraft_engine::{Session, file_cmds::open_bytes_as};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;
use serde_json::json;

fn source() -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/gimp/channel.xcf");
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}; run cargo xtask corpus --gimp", path.display()))
}

fn counterpart() -> Document {
    let mut doc = Document::new("channel.xcf", Size::new(40, 30), ColorMode::Rgb, SampleType::U8);
    doc.resolution_dpi = 300.0; // Resolution stored by the published GIMP-authored fixture.
    let mut layer = Layer::raster("Background", doc.pixel_format());
    let mut channel = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
    for y in 0..30u32 {
        for x in 0..40u32 {
            let mut px: Vec<_> = (0..3).map(|c| ((x * 31 + y * 17 + c * 101 + 7) * 13 % 256) as f32 / 255.0).collect();
            px.push(((x + y) * 255 / 66).min(255) as f32 / 255.0);
            layer.surface_mut().unwrap().write_pixel(x as i32, y as i32, &px);
            channel.write_pixel(x as i32, y as i32, &[(y * 255 / 29) as f32 / 255.0]);
        }
    }
    doc.layers.push(layer);
    doc.channels.push(AlphaChannel::new("Alpha 1", channel));
    let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
    selection.write_region(Rect::new(4, 6, 24, 18), &vec![1.0; 20 * 12]);
    doc.selection = Some(selection);
    doc
}

fn same_surface(a: &Surface, b: &Surface, label: &str, bounds: Rect) {
    if a == b {
        return;
    }
    let region = bounds.union(&a.content_bounds()).union(&b.content_bounds());
    let mut first = None;
    'rows: for y in region.y0..region.y1 {
        for x in region.x0..region.x1 {
            let left = a.pixel(x, y);
            let right = b.pixel(x, y);
            if left != right {
                first = Some((x, y, left, right));
                break 'rows;
            }
        }
    }
    // Keep complete Surface equality, including sparse backing/default pixels. A
    // bounded diagnostic avoids dumping an entire padded256x256 tile on failure.
    assert!(
        a == b,
        "{label}: full Surface mismatch; first pixel {first:?}; format {:?}/{:?}, default {:?}/{:?}, content {:?}/{:?}, tiles {}/{}",
        a.format(),
        b.format(),
        a.default_pixel(),
        b.default_pixel(),
        a.content_bounds(),
        b.content_bounds(),
        a.tile_count(),
        b.tile_count()
    );
}
fn compare(a: &Session, b: &Session, stage: &str) {
    let a = &a.active().unwrap().doc;
    let b = &b.active().unwrap().doc;
    let document_bounds = a.bounds();
    assert_eq!((a.size, a.mode, a.depth, a.resolution_dpi), (b.size, b.mode, b.depth, b.resolution_dpi));
    match (&a.selection, &b.selection) {
        (Some(a), Some(b)) => same_surface(a, b, &format!("{stage}: selection"), document_bounds),
        (None, None) => {}
        _ => panic!("{stage}: selection presence differs"),
    }
    assert_eq!(a.channels.len(), b.channels.len(), "{stage}: channel count");
    for (index, (a, b)) in a.channels.iter().zip(&b.channels).enumerate() {
        same_surface(&a.surface, &b.surface, &format!("{stage}: alpha channel{index}"), document_bounds);
        let mut metadata = a.clone();
        metadata.surface = b.surface.clone();
        assert_eq!(&metadata, b, "{stage}: channel metadata");
    }
    assert_eq!(a.layers.len(), b.layers.len());
    for (a, b) in a.layers.iter().zip(&b.layers) {
        if let (Some(left), Some(right)) = (a.surface(), b.surface()) {
            same_surface(left, right, &format!("{stage}: layer {}", a.name), document_bounds);
        }
        let mut a = a.clone();
        a.id = b.id; // Fresh IDs and inert provenance do not affect native operations.
        assert_eq!(&a, b);
    }
    assert_eq!(photocraft_compose::flatten(a).px, photocraft_compose::flatten(b).px);
}

#[test]
fn imported_layers_use_ordinary_editing_locks_and_history_then_native_reopen() {
    let mut imported = Session::new();
    imported.color.settings.ask_on_missing = false;
    open_bytes_as(&mut imported, "channel.xcf", &source(), None, None).unwrap();
    let mut native = Session::new();
    native.add_document(counterpart(), None);
    compare(&imported, &native, "initial import");
    // Independently reopened stock GIMP fixture selects Alpha1, with no selected
    // layers. The importer correctly restores that native editing target. This
    // test deliberately exercises layer edits, so choose the composite through
    // the ordinary public command on both sessions before painting.
    assert_eq!(imported.active().unwrap().channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::Alpha(0));
    for session in [&mut imported, &mut native] {
        session.execute("channel.target.composite", json!({})).unwrap();
        assert_eq!(session.active().unwrap().channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::Composite);
    }
    for (command, params) in [
        ("select.rect", json!({"x": 7, "y": 8, "width": 10, "height": 9})),
        ("paint.stroke", json!({"points": [[5, 10], [25, 15]], "size": 5, "hardness": 1.0, "color": "#ff0000"})),
        ("layer.setProps", json!({"opacity": 0.65, "blend": "Multiply"})),
        ("layer.duplicate", json!({})),
        ("layer.translate", json!({"dx": -3, "dy": 2})),
        ("edit.undo", json!({})),
        ("edit.redo", json!({})),
        ("layer.setProps", json!({"locks": {"pixels": true, "position": true}})),
    ] {
        imported.execute(command, params.clone()).unwrap();
        native.execute(command, params).unwrap();
        compare(&imported, &native, command);
    }
    for session in [&mut imported, &mut native] {
        let before = session.active().unwrap().doc.clone();
        assert!(session.execute("layer.translate", json!({"dx": 1})).is_err());
        assert!(session.execute("paint.stroke", json!({"points": [[10, 10]], "size": 5, "color": "#00ff00"})).is_err());
        assert_eq!(session.active().unwrap().doc, before);
        let bytes = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(&reopened, before.as_ref(), "native save/reopen needs neither XCF nor GIMP");
        session.execute("edit.undo", json!({})).unwrap();
        session.execute("edit.redo", json!({})).unwrap();
    }
    compare(&imported, &native, "after native reopen and history");
}

#[test]
fn damaged_xcf_open_preserves_the_current_document_and_history() {
    let mut session = Session::new();
    session.add_document(counterpart(), None);
    session.execute("layer.setProps", json!({"opacity": 0.4})).unwrap();
    let before = session.active().unwrap().doc.clone();
    let undo = session.active().unwrap().history.undo_label().map(str::to_owned);
    let bytes = source();
    assert!(open_bytes_as(&mut session, "broken.xcf", &bytes[..bytes.len() / 2], None, None).is_err());
    assert_eq!(session.documents().len(), 1);
    assert_eq!(session.active().unwrap().doc, before);
    assert_eq!(session.active().unwrap().history.undo_label(), undo.as_deref());
}

#[test]
fn background_xcf_open_is_atomic_cancellable_and_preserves_source_policy() {
    use photocraft_engine::jobs::{OpenSource, Started};
    use std::sync::Arc;
    let bytes = Arc::new(source());
    let mut session = Session::new();
    session.color.settings.ask_on_missing = false;
    session.add_document(counterpart(), None);
    let before = session.active().unwrap().doc.clone();
    let Started::Job(id) = session.start_open("channel.xcf", OpenSource::Bytes(bytes.clone())).unwrap() else { panic!("native background job expected") };
    assert!(session.cancel_job(id)); // Even a decoded but unpublished result must be discarded.
    assert!(session.wait_job(id).is_err());
    assert_eq!(session.documents().len(), 1);
    assert_eq!(session.active().unwrap().doc, before);
    let opened = match session.start_open("channel.xcf", OpenSource::Bytes(bytes)).unwrap() {
        Started::Done(value) => value,
        Started::Job(id) => session.wait_job(id).unwrap(),
    };
    assert!(opened.get("document").is_some());
    let state = session.active().unwrap();
    assert!(state.source_read_only);
    assert!(state.path.is_none());
    assert!(state.doc.metadata.import_conversion.is_some());
    assert!(!state.is_dirty());
    session.execute("layer.setProps", json!({"opacity": 0.75})).unwrap();
    assert!(session.active().unwrap().is_dirty());
}
