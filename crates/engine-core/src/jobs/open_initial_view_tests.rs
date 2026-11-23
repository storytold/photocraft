use super::*;
use photocraft_doc::{ColorMode, Layer, SampleType};
use photocraft_geom::{Rect, Size};
use photocraft_io::NativeInitialView;

fn session(cooperative: bool) -> Session {
    let mut s = crate::session();
    s.set_cooperative_jobs(cooperative);
    s.color_mut().settings.ask_on_missing = false;
    s.color_mut().settings.ask_on_mismatch = false;
    s
}

#[allow(clippy::unwrap_used, reason = "The fixture constructs the raster layer immediately before accessing it")]
fn imported(read_only: bool, invalid: bool) -> photocraft_io::ImportResult {
    let mut document = Document::new("Decoded", Size::new(8, 6), ColorMode::Rgb, SampleType::U8);
    let mut first = Layer::raster("First", photocraft_color::PixelFormat::RGBA8);
    first.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 8, 6), &[1., 0., 0., 1.]);
    let selected = first.id;
    document.layers.extend([first, Layer::raster("Last", photocraft_color::PixelFormat::RGBA8)]);
    photocraft_io::ImportResult {
        document,
        source_read_only: read_only,
        preview_only: false,
        historical_xcf_sample_order: None,
        warnings: vec!["Decode note".into()],
        initial_view: Some(NativeInitialView {
            selected_layers: vec![selected],
            active_layer: Some(selected),
            samplers: vec![[2., 3.]],
            selected_path: invalid.then_some(0),
            ..Default::default()
        }),
    }
}

type ExecutionThread = Arc<Mutex<Option<std::thread::ThreadId>>>;

struct PreparedOpen {
    imported: Option<photocraft_io::ImportResult>,
    executed_on: ExecutionThread,
}

impl StepWork<(photocraft_io::ImportResult, bool)> for PreparedOpen {
    fn step(&mut self, ctx: &JobCtx, _: web_time::Instant) -> Result<Option<(photocraft_io::ImportResult, bool)>> {
        ctx.check()?;
        *self.executed_on.lock().map_err(|_| EngineError::Other("prepared Open thread record was poisoned".into()))? = Some(std::thread::current().id());
        let imported = self.imported.take().ok_or_else(|| EngineError::Other("prepared Open ran twice".into()))?;
        Ok(Some((imported, false)))
    }
}

// Ordinary non-stepped work runs inline in cooperative mode. Use the actual
// stepped scheduler to test deferred publication/cancellation of the same Open
// finish stage on both a native worker and the cooperative polling thread.
#[allow(clippy::unwrap_used, clippy::panic, reason = "This test helper requires a successfully queued fixture")]
fn start_prepared(s: &mut Session, imported: photocraft_io::ImportResult, source: Option<String>) -> (JobId, ExecutionThread) {
    let executed_on = Arc::new(Mutex::new(None));
    let work = PreparedOpen { imported: Some(imported), executed_on: Arc::clone(&executed_on) };
    let started = s
        .start_job_stepped(OPEN_JOB, json!({"name":"display.png"}), "Opening prepared fixture", false, work, move |s, imported| {
            s.finish_open_import("display.png", source, imported, false)
        })
        .unwrap();
    let Started::Job(id) = started else { panic!("stepped Open must await publication") };
    (id, executed_on)
}

#[test]
fn decoded_open_jobs_apply_view_in_threaded_and_cooperative_modes_without_changing_path_semantics() {
    for cooperative in [false, true] {
        for read_only in [false, true] {
            let mut s = session(cooperative);
            let source = imported(read_only, false);
            let selected = source.initial_view.as_ref().unwrap().active_layer;
            let caller = std::thread::current().id();
            let (id, executed_on) = start_prepared(&mut s, source, Some("source.xcf".into()));
            assert!(s.documents().is_empty(), "decoding cannot publish view state before polling");
            let result = s.wait_job(id).unwrap();
            let execution = executed_on.lock().unwrap().expect("prepared work executed");
            assert_eq!(execution == caller, cooperative, "exercise the requested worker mode");
            assert_eq!(result["name"], "display.png");
            assert_eq!(result["warnings"], json!(["Decode note"]));
            let state = s.active().unwrap();
            assert_eq!(state.active_layer(), selected);
            assert_eq!(state.color_samplers, [[2., 3.]]);
            assert_eq!(state.snapshots()[0].layers.active, selected);
            assert_eq!((state.revision, state.history.past_len()), (1, 0));
            assert!(state.path.is_none(), "the host retains responsibility for the writable Open path");
            assert_eq!(state.protected_import_source.as_deref(), read_only.then_some("source.xcf"));
        }
    }
}

#[test]
fn cancelled_open_jobs_publish_neither_view_origin_nor_a_held_payload() {
    for cooperative in [false, true] {
        let mut s = session(cooperative);
        s.execute("file.new", json!({"width":8,"height":6})).unwrap();
        s.active_mut().unwrap().color_samplers = vec![[1., 2.]];
        let before = s.active().unwrap().clone();
        s.color_mut().settings.ask_on_missing = true;
        let (id, _) = start_prepared(&mut s, imported(true, false), Some("source.xcf".into()));
        assert!(s.cancel_job(id));
        assert!(matches!(s.wait_job(id), Err(EngineError::Cancelled)));
        assert_eq!(s.documents().len(), 1);
        assert!(Arc::ptr_eq(&before.doc, &s.active().unwrap().doc));
        assert_eq!(s.active().unwrap().color_samplers, before.color_samplers);
        assert_eq!(s.active().unwrap().protected_import_source, before.protected_import_source);
        assert!(s.held_opens.is_empty());
        assert!(s.pending_preflights.is_empty());
    }
}

#[test]
fn color_and_damage_questions_keep_optional_view_and_origin_until_the_exact_payload_is_taken() {
    for damage in [false, true] {
        let mut s = session(false);
        s.color_mut().settings.ask_on_missing = !damage;
        let source = imported(true, false);
        let expected_view = source.initial_view.clone();
        let result = s.finish_open_import("display.png", Some("source.xcf".into()), (source, damage), damage).unwrap();
        assert!(s.documents().is_empty());
        assert_eq!(result["name"], "display.png");
        assert_eq!(result["damaged"], damage);
        assert_eq!(result["color"].is_object(), !damage);
        let id = result["held"].as_u64().unwrap();
        assert!(s.held_open_named("Decoded"));
        let mut held = s.take_held_open_with_view(id).unwrap();
        assert!(s.take_held_open_with_view(id).is_none());
        assert_eq!(held.initial_view, expected_view);
        assert!(held.source_read_only);
        assert_eq!(held.protected_import_source.as_deref(), Some("source.xcf"));
        // The future host consumes this payload only after answering the same question.
        s.color().open_policy(&mut held.document, s.prefs().file_handling.ignore_exif_profile_tag);
        let (_, warning) = s.admit_import_with_initial_view(held.document, held.protected_import_source, held.source_read_only, held.initial_view);
        assert!(warning.is_none());
        assert_eq!(s.active().unwrap().color_samplers, [[2., 3.]]);
        assert_eq!(s.active().unwrap().protected_import_source.as_deref(), Some("source.xcf"));
        assert!(s.active().unwrap().path.is_none());
    }
}

#[test]
fn legacy_held_take_preserves_its_contract_and_does_not_consume_a_different_prompt() {
    let mut s = session(false);
    let first = imported(true, false);
    let first_id = s.hold_open_with_view(first.document, first.source_read_only, first.initial_view, Some("first.xcf".into()));
    let mut second = imported(true, false);
    second.document.name = "Second".into();
    let second_view = second.initial_view.clone();
    let second_id = s.hold_open_with_view(second.document, second.source_read_only, second.initial_view, Some("second.xcf".into()));
    let (document, read_only) = s.take_held_open(first_id).unwrap();
    assert_eq!(document.name, "Decoded");
    assert!(read_only);
    assert!(s.take_held_open(first_id).is_none());
    assert!(s.held_open_named("Second"));
    let second = s.take_held_open_with_view(second_id).unwrap();
    assert_eq!(second.document.name, "Second");
    assert_eq!(second.initial_view, second_view);
    assert_eq!(second.protected_import_source.as_deref(), Some("second.xcf"));
    assert!(s.held_opens.is_empty());
}

#[test]
fn real_open_origin_uses_the_path_source_and_never_the_bytes_display_name() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("actual.pcraft");
    let bytes = Arc::new(crate::file_cmds::import_only_source_bytes_for_test());
    std::fs::write(&path, bytes.as_slice()).unwrap();
    let path = path.to_str().unwrap();
    for cooperative in [false, true] {
        for from_bytes in [false, true] {
            let mut s = session(cooperative);
            let source = if from_bytes { OpenSource::Bytes(Arc::clone(&bytes)) } else { OpenSource::Path(path.into()) };
            let name = if from_bytes { path } else { "unrelated-display.png" };
            let result = match s.start_open(name, source).unwrap() {
                Started::Done(value) => value,
                Started::Job(id) => s.wait_job(id).unwrap(),
            };
            assert!(result.get("document").is_some(), "{result}");
            let state = s.active().unwrap();
            assert!(state.source_read_only);
            assert!(state.path.is_none());
            assert_eq!(state.protected_import_source.as_deref(), (!from_bytes).then_some(path));
            assert_eq!(photocraft_compose::flatten(&state.doc).get(0, 0), [0., 1., 0., 1.]);
        }
    }
}

#[test]
fn invalid_worker_hint_keeps_decoded_pixels_and_the_previously_open_document() {
    for cooperative in [false, true] {
        let mut s = session(cooperative);
        s.execute("file.new", json!({"width":8,"height":6})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let source = imported(true, true);
        let default_layer = source.document.top_layer();
        let (id, _) = start_prepared(&mut s, source, Some("source.xcf".into()));
        let result = s.wait_job(id).unwrap();
        assert_eq!(result["warnings"].as_array().unwrap().len(), 2);
        assert!(result["warnings"][1].as_str().unwrap().contains("selected path is absent"));
        assert_eq!(s.documents().len(), 2);
        assert!(Arc::ptr_eq(&s.documents()[0].doc, &before));
        let state = s.active().unwrap();
        assert_eq!(state.active_layer(), default_layer);
        assert!(state.color_samplers.is_empty());
        assert_eq!(photocraft_compose::flatten(&state.doc).get(0, 0), [1., 0., 0., 1.]);
        assert_eq!(state.protected_import_source.as_deref(), Some("source.xcf"));
    }
}
