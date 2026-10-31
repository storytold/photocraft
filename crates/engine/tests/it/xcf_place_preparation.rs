//! Placement and replacement prepare imported native content before composition.

use std::sync::Arc;

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, StackMode};
use photocraft_engine::{EngineError, Session, file_cmds, smart_cmds};
use photocraft_geom::{Rect, Size};
use serde_json::{Value, json};

const PIXELS: [u8; 8] = [40, 100, 180, 255, 180, 60, 30, 255];

struct Wire(Vec<u8>);
impl Wire {
    /// Encode one independently authored big-endian XCF field.
    fn word(&mut self, value: u32) {
        self.0.extend(value.to_be_bytes());
    }
    /// Reserve a current-version absolute section pointer.
    fn slot(&mut self) -> usize {
        let at = self.0.len();
        self.0.extend(0u64.to_be_bytes());
        at
    }
    /// Resolve a reserved pointer using the actual section position.
    fn patch(&mut self, at: usize) {
        let value = self.0.len() as u64;
        self.0[at..at + 8].copy_from_slice(&value.to_be_bytes());
    }
    /// Store a NUL-terminated source string with its wire byte count.
    fn text(&mut self, value: &str) {
        self.word(value.len() as u32 + 1);
        self.0.extend(value.as_bytes());
        self.0.push(0);
    }
    /// End a property list without optional source state.
    fn end(&mut self) {
        self.word(0);
        self.word(0);
    }
}

/// Original source pixels plus one stored invert operation; no native cache is supplied by the fixture.
fn xcf() -> Vec<u8> {
    let mut w = Wire(b"gimp xcf v026\0".to_vec());
    for value in [2, 1, 0, 150] {
        w.word(value);
    }
    w.word(17);
    w.word(1);
    w.0.push(0);
    w.end();
    let layer = w.slot();
    w.slot();
    w.slot();
    w.slot();
    w.patch(layer);
    for value in [2, 1, 1] {
        w.word(value);
    }
    w.text("Original asymmetric pixels");
    w.end();
    let hierarchy = w.slot();
    w.slot();
    let effect = w.slot();
    w.slot();
    w.patch(hierarchy);
    for value in [2, 1, 4] {
        w.word(value);
    }
    let level = w.slot();
    w.slot();
    w.patch(level);
    w.word(2);
    w.word(1);
    let tile = w.slot();
    w.slot();
    w.patch(tile);
    w.0.extend(PIXELS);
    w.patch(effect);
    w.text("Stored source invert");
    w.text("");
    w.text("gegl:invert-gamma");
    w.text("0:0");
    w.end();
    w.slot();
    w.0
}

/// Construct the expected appearance using the ordinary native adjustment command, independently of XCF import.
fn counterpart() -> Document {
    let mut doc = Document::new("Native oracle", Size::new(2, 1), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("Original", doc.pixel_format());
    layer.surface_mut().unwrap().write_region(Rect::new(0, 0, 2, 1), &PIXELS.map(|v| f32::from(v) / 255.0));
    doc.layers.push(layer);
    let mut session = Session::new();
    session.add_document(doc, None);
    session.execute("image.adjustments.invert", json!({})).unwrap();
    (*session.active().unwrap().doc).clone()
}

/// Start with a transparent destination so only the placed source defines the first rendered output.
fn destination() -> Session {
    let mut session = Session::new();
    session.add_document(Document::new("Destination", Size::new(2, 1), ColorMode::Rgb, SampleType::U8), None);
    session
}

/// Compare native composition at storage precision, with short diagnostics rather than entire document dumps.
fn assert_pixels(actual: &Document, expected: &Document) {
    let a = photocraft_compose::flatten(actual).px;
    let b = photocraft_compose::flatten(expected).px;
    assert_eq!(a.len(), b.len());
    for (index, (a, b)) in a.iter().zip(b).enumerate() {
        assert!(a.iter().zip(b).all(|(a, b)| (*a - b).abs() <= 1.0 / 255.0), "native pixel {index}: {a:?} != {b:?}");
    }
}

#[test]
/// Both pixel and Smart placement show the prepared native effect immediately and preserve the original source bytes.
fn xcf_raster_and_smart_placement_prepare_before_first_flatten() {
    let bytes = xcf();
    let expected = counterpart();
    for smart in [false, true] {
        let mut session = destination();
        file_cmds::place_bytes(&mut session, "original.xcf", bytes.clone(), None, &json!({"smartObject":smart,"scale":100})).unwrap();
        assert_pixels(&session.active().unwrap().doc, &expected);
        if smart {
            let id = session.active().unwrap().active_layer.unwrap();
            let (name, retained) = smart_cmds::contents_of(&session.active().unwrap().doc, id).unwrap();
            assert_eq!(name, "original.xcf");
            assert_eq!(retained.as_slice(), bytes);
            smart_cmds::purge_stack_cache();
            let doc = Arc::make_mut(&mut session.active_mut().unwrap().doc);
            let LayerContent::Smart(object) = &mut doc.layer_mut(id).unwrap().content else { panic!("placed Smart") };
            object.cache = None;
            assert!(smart_cmds::refresh(doc, id).unwrap());
            assert_pixels(doc, &expected);
            assert_eq!(smart_cmds::contents_of(doc, id).unwrap().1.as_slice(), bytes);
        }
    }
}

#[test]
/// Public decoding and actual replacement use prepared native pixels, while replacement remains one undoable edit.
fn xcf_decode_and_replace_prepare_native_source_and_undo() {
    let bytes = xcf();
    let expected = counterpart();
    assert_pixels(&smart_cmds::decode_source("original.xcf", &bytes).unwrap(), &expected);
    for image in [
        smart_cmds::source_image("original.xcf", &bytes, expected.pixel_format()).unwrap(),
        smart_cmds::stack_image("original.xcf", &bytes, expected.pixel_format(), StackMode::Mean).unwrap(),
    ] {
        let mut actual = Document::new("Source image", expected.size, expected.mode, expected.depth);
        actual.layers.push(Layer::new("Prepared pixels", LayerContent::Raster((*image.surface).clone())));
        assert_pixels(&actual, &expected);
    }
    let mut session = destination();
    let original = photocraft_format::save_to_bytes(&counterpart(), &Default::default()).unwrap();
    file_cmds::place_bytes(&mut session, "previous.pcraft", original, None, &json!({"smartObject":true,"scale":100})).unwrap();
    let before = session.active().unwrap().doc.clone();
    let id = session.active().unwrap().active_layer.unwrap();
    smart_cmds::replace_contents_bytes(&mut session, id, "original.xcf", bytes.clone()).unwrap();
    assert_pixels(&session.active().unwrap().doc, &expected);
    assert_eq!(smart_cmds::contents_of(&session.active().unwrap().doc, id).unwrap().1.as_slice(), bytes);
    session.execute("edit.undo", json!({})).unwrap();
    assert!(session.active().unwrap().doc.as_ref() == before.as_ref(), "undo must restore the original Smart source and cache");
}

/// Reject replay of stored native Invert while permitting the outer placement/replacement request.
fn deny_invert(command: &str, _: &Value) -> photocraft_engine::Result<()> {
    if command == "image.adjustments.invert" { Err(EngineError::Other("stored invert denied by test authority".into())) } else { Ok(()) }
}

#[test]
/// A denied stored effect cannot publish a placed layer or replace existing contents, even after a trusted cache warmup.
fn denied_xcf_placement_and_replacement_leave_document_and_history_unchanged() {
    let bytes = xcf();
    let format = counterpart().pixel_format();
    smart_cmds::source_image("original.xcf", &bytes, format).unwrap();
    smart_cmds::stack_image("original.xcf", &bytes, format, StackMode::Mean).unwrap();
    let mut session = destination();
    let source = photocraft_format::save_to_bytes(&counterpart(), &Default::default()).unwrap();
    file_cmds::place_bytes(&mut session, "previous.pcraft", source, None, &json!({"smartObject":true,"scale":100})).unwrap();
    session.authorize = Some(deny_invert);
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let history = session.active().unwrap().history.entries();
    let id = session.active().unwrap().active_layer.unwrap();
    assert!(file_cmds::place_bytes(&mut session, "denied.xcf", bytes.clone(), None, &json!({"smartObject":false})).is_err());
    assert!(smart_cmds::replace_contents_bytes(&mut session, id, "denied.xcf", bytes).is_err());
    let state = session.active().unwrap();
    assert!(Arc::ptr_eq(&before, &state.doc), "failure must not publish a replacement document");
    assert_eq!(state.revision, revision);
    assert_eq!(state.history.entries(), history);
}

#[test]
/// A caller's already-cancelled inline job must refuse placement and replacement without creating history.
fn cancelled_xcf_placement_and_replacement_do_not_publish() {
    let mut session = destination();
    let source = photocraft_format::save_to_bytes(&counterpart(), &Default::default()).unwrap();
    file_cmds::place_bytes(&mut session, "previous.pcraft", source, None, &json!({"smartObject":true})).unwrap();
    let before = session.active().unwrap().doc.clone();
    let history = session.active().unwrap().history.entries();
    let id = session.active().unwrap().active_layer.unwrap();
    let ctx = photocraft_engine::jobs::JobCtx::new();
    ctx.cancel();
    session.set_inline_job_ctx(Some(ctx));
    assert!(matches!(file_cmds::place_bytes(&mut session, "cancelled.xcf", xcf(), None, &json!({})), Err(EngineError::Cancelled)));
    assert!(matches!(smart_cmds::replace_contents_bytes(&mut session, id, "cancelled.xcf", xcf()), Err(EngineError::Cancelled)));
    assert!(Arc::ptr_eq(&before, &session.active().unwrap().doc));
    assert_eq!(session.active().unwrap().history.entries(), history);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
/// A real background Place worker installs prepared pixels only on success; cancellation and denied replay discard its result.
fn background_xcf_place_installs_only_authorized_uncancelled_worker_results() {
    use photocraft_engine::jobs::Started;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("original.xcf");
    let original = xcf();
    std::fs::write(&path, &original).unwrap();
    let params = json!({"path":path.to_string_lossy(),"smartObject":false,"scale":100});
    let mut session = destination();
    let before = session.active().unwrap().doc.clone();
    let history = session.active().unwrap().history.entries();
    let Started::Job(cancelled) = session.start("file.placeEmbedded", params.clone()).unwrap() else { panic!("Place must start a native worker") };
    // Even a worker that finished before this cancellation has not published its result yet.
    assert!(session.cancel_job(cancelled));
    assert!(session.wait_job(cancelled).is_err());
    assert!(Arc::ptr_eq(&before, &session.active().unwrap().doc));
    assert_eq!(session.active().unwrap().history.entries(), history);
    session.authorize = Some(deny_invert);
    let Started::Job(denied) = session.start("file.placeEmbedded", params.clone()).unwrap() else { panic!("Denied source is checked on the worker") };
    assert!(session.wait_job(denied).is_err());
    assert!(Arc::ptr_eq(&before, &session.active().unwrap().doc));
    assert_eq!(session.active().unwrap().history.entries(), history);
    session.authorize = None;
    let Started::Job(accepted) = session.start("file.placeEmbedded", params).unwrap() else { panic!("Place must start a native worker") };
    session.wait_job(accepted).unwrap();
    assert_pixels(&session.active().unwrap().doc, &counterpart());
    assert_eq!(std::fs::read(path).unwrap(), original);
}

const HISTORICAL_SAMPLES: [u16; 8] = [0x1234, 0x4567, 0x89ab, 0xffff, 0xabcd, 0x2345, 0x6789, 0x8001];

/// Author original version-11 wire with declared component order; no modern file header is rewritten.
fn historical_xcf(little_endian: bool) -> Vec<u8> {
    let mut wire = Wire(b"gimp xcf v011\0".to_vec());
    for value in [2, 1, 0, 250] {
        wire.word(value);
    }
    wire.word(17);
    wire.word(1);
    wire.0.push(0);
    wire.end();
    let layer = wire.slot();
    wire.slot();
    wire.slot();
    wire.patch(layer);
    for value in [2, 1, 1] {
        wire.word(value);
    }
    wire.text("Declared historical U16 samples");
    wire.end();
    let hierarchy = wire.slot();
    wire.slot();
    wire.patch(hierarchy);
    for value in [2, 1, 8] {
        wire.word(value);
    }
    let level = wire.slot();
    wire.slot();
    wire.patch(level);
    wire.word(2);
    wire.word(1);
    let tile = wire.slot();
    wire.slot();
    wire.patch(tile);
    for sample in HISTORICAL_SAMPLES {
        wire.0.extend(if little_endian { sample.to_le_bytes() } else { sample.to_be_bytes() });
    }
    wire.0
}

/// Check exact stored U16 components and coverage independently of the historical importer or compositor.
fn assert_historical_samples(document: &Document) {
    assert_eq!(document.depth, SampleType::U16);
    let actual = document.layers[0].surface().unwrap().read_region(Rect::new(0, 0, 2, 1));
    let expected = HISTORICAL_SAMPLES.map(|sample| f32::from(sample) / 65535.0);
    assert_eq!(actual.as_slice(), expected.as_slice());
    let Some(photocraft_doc::import_conversion::ImportConversion::Report(report)) = document.metadata.import_conversion.as_deref() else {
        panic!("explicit input policy must survive native normalization");
    };
    assert_eq!(report.source_version, Some(11));
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.historical_sample_order"));
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
/// An explicit historical byte order becomes native embedded data, remaining replayable without the ambiguous source or a new choice.
fn historical_smart_place_normalizes_explicit_sample_order_for_refresh_edit_and_native_reopen() {
    for little_endian in [true, false] {
        let original = historical_xcf(little_endian);
        let unchanged = original.clone();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("historical.xcf");
        std::fs::write(&path, &original).unwrap();
        let mut session = Session::new();
        session.add_document(Document::new("Native destination", Size::new(2, 1), ColorMode::Rgb, SampleType::U16), None);
        file_cmds::place_bytes(
            &mut session,
            &path.to_string_lossy(),
            original,
            None,
            &json!({
                "smartObject":true,"scale":100,"historicalXcfSampleOrder":if little_endian { "little-endian" } else { "big-endian" },
            }),
        )
        .unwrap();
        let id = session.active().unwrap().active_layer.unwrap();
        let (name, native) = smart_cmds::contents_of(&session.active().unwrap().doc, id).unwrap();
        assert_eq!(name, "historical.pcraft");
        assert!(photocraft_format::is_pcraft(&native));
        assert_historical_samples(&smart_cmds::decode_source(&name, &native).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), unchanged, "disk source must remain unchanged");
        std::fs::remove_file(path).unwrap();
        smart_cmds::purge_stack_cache();
        let parent = Arc::make_mut(&mut session.active_mut().unwrap().doc);
        let LayerContent::Smart(object) = &mut parent.layer_mut(id).unwrap().content else { panic!("placed Smart") };
        object.cache = None;
        assert!(smart_cmds::refresh(parent, id).unwrap());
        let saved = photocraft_format::save_to_bytes(parent, &Default::default()).unwrap();
        session.execute("layer.smartObjects.editContents", json!({"layer":id.0})).unwrap();
        assert_historical_samples(&session.active().unwrap().doc);
        session.execute("image.adjustments.invert", json!({})).unwrap();
        session.execute("edit.undo", json!({})).unwrap();
        assert_historical_samples(&session.active().unwrap().doc);
        let mut reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        let LayerContent::Smart(object) = &mut reopened.layer_mut(id).unwrap().content else { panic!("reopened Smart") };
        object.cache = None;
        assert!(smart_cmds::refresh(&mut reopened, id).unwrap());
        let (name, contents) = smart_cmds::contents_of(&reopened, id).unwrap();
        assert_historical_samples(&smart_cmds::decode_source(&name, &contents).unwrap());
    }
}

#[test]
/// Linked historical data cannot persist its required choice, so admission fails before a source or layer is published.
fn historical_linked_place_requires_native_source_and_is_atomic() {
    let mut session = destination();
    let before = session.active().unwrap().doc.clone();
    let history = session.active().unwrap().history.entries();
    let error = file_cmds::place_bytes(
        &mut session,
        "historical.xcf",
        historical_xcf(true),
        Some("historical.xcf".into()),
        &json!({
            "historicalXcfSampleOrder":"little-endian",
        }),
    )
    .unwrap_err();
    assert!(error.to_string().contains(".pcraft"));
    assert!(error.to_string().contains("Place Linked"));
    assert!(Arc::ptr_eq(&before, &session.active().unwrap().doc));
    assert_eq!(session.active().unwrap().history.entries(), history);
}
