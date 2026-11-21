use super::*;
use crate::dialogs::historical_sample_order::tests_support::{cancel, confirm, dialogs, set_order};
use craft::host::{Picked, ScriptedFiles};
use photocraft_doc::{ColorMode, Document, HistoricalXcfSampleOrder, Layer, SampleType, SmartObject, SmartSource};
use photocraft_geom::{Affine, Size};
use std::sync::Arc;

use crate::historical_xcf_fixture as fixture;

fn app(answers: Vec<Option<Picked>>, old_order: HistoricalXcfSampleOrder) -> Box<PhotocraftApp> {
    let mut session = photocraft_engine::Session::new();
    session.color.settings.ask_on_missing = false;
    session.color.settings.ask_on_mismatch = false;
    let mut document = Document::new("Parent", Size::new(1, 1), ColorMode::Rgb, SampleType::U16);
    let mut smart =
        SmartObject::new(SmartSource::Embedded { file_name: "old.xcf".into(), bytes: Arc::new(fixture::historical_pixel(false)) }, Affine::IDENTITY, None);
    smart.historical_xcf_sample_order = Some(old_order);
    document.layers.push(Layer::new("Target", LayerContent::Smart(smart)));
    session.add_document(document, None);
    let mut app = PhotocraftApp::new(
        session,
        crate::Services {
            files: Some(Arc::new(ScriptedFiles::answering(answers.into_iter().map(|answer| answer.unwrap_or(Picked::Cancelled))))),
            import: Some(Box::new(|name, bytes, opts| photocraft_io::import_with_options(name, bytes, opts).map_err(|error| error.to_string()))),
            ..Default::default()
        },
    );
    app.background_jobs = false;
    app.sync_views();
    app
}

fn pick_from_menu(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    crate::menus::invoke(app, &ctx, COMMAND, json!({})).unwrap();
    app.poll_file_dialog();
    app.poll_file_dialog();
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

fn choose(app: &mut PhotocraftApp, choice: &str) {
    let id = dialogs(app).into_iter().find(|dialog| owns(&dialog.fields)).unwrap().id;
    set_order(app, id, json!(choice));
    confirm(app, id).unwrap();
}

fn object(app: &PhotocraftApp) -> &SmartObject {
    let state = app.session.documents().iter().find(|state| state.doc.name == "Parent").unwrap();
    let LayerContent::Smart(smart) = &state.doc.layers[0].content else { panic!("smart object") };
    smart
}

fn contents(name: &str, bytes: Vec<u8>) -> Picked {
    Picked::Contents(vec![(name.to_owned(), bytes)])
}

fn modern_pick(wrapped: bool) -> Picked {
    fn word(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_be_bytes());
    }
    fn slot(bytes: &mut Vec<u8>) -> usize {
        let at = bytes.len();
        bytes.extend([0; 8]);
        at
    }
    fn point(bytes: &mut [u8], slot: usize, offset: usize) {
        bytes[slot..slot + 8].copy_from_slice(&(offset as u64).to_be_bytes());
    }
    let mut bytes = b"gimp xcf v026\0".to_vec();
    for value in [1, 1, 0, 150, 17, 1] {
        word(&mut bytes, value);
    }
    bytes.push(0);
    word(&mut bytes, 0);
    word(&mut bytes, 0);
    let layer = slot(&mut bytes);
    for _ in 0..3 {
        slot(&mut bytes); // End layers, channels, and paths.
    }
    let offset = bytes.len();
    point(&mut bytes, layer, offset);
    for value in [1, 1, 1, 2] {
        word(&mut bytes, value);
    }
    bytes.extend(b"m\0");
    word(&mut bytes, 0);
    word(&mut bytes, 0);
    let hierarchy = slot(&mut bytes);
    slot(&mut bytes); // No mask.
    slot(&mut bytes); // No effects.
    let offset = bytes.len();
    point(&mut bytes, hierarchy, offset);
    for value in [1, 1, 4] {
        word(&mut bytes, value);
    }
    let level = slot(&mut bytes);
    slot(&mut bytes);
    let offset = bytes.len();
    point(&mut bytes, level, offset);
    word(&mut bytes, 1);
    word(&mut bytes, 1);
    let tile = slot(&mut bytes);
    slot(&mut bytes);
    let offset = bytes.len();
    point(&mut bytes, tile, offset);
    bytes.extend([17, 31, 57, 255]);
    if wrapped {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&bytes).unwrap();
        contents("modern.xcf.gz", encoder.finish().unwrap())
    } else {
        contents("modern.xcf", bytes)
    }
}

fn tick_probe(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |ui| crate::jobs_ui::tick(app, ui.ctx()));
    output.textures_delta.clear();
}

fn cancel_choice(app: &mut PhotocraftApp) {
    let id = dialogs(app).into_iter().find(|dialog| owns(&dialog.fields)).unwrap().id;
    cancel(app, id).unwrap();
}

#[test]
fn plain_and_wrapped_modern_picks_wait_then_reject_a_changed_same_target() {
    for wrapped in [false, true] {
        let first = fixture::historical_pixel(true);
        let mut app = app(vec![Some(contents("first.xcf", first.clone())), Some(modern_pick(wrapped))], HistoricalXcfSampleOrder::BigEndian);
        app.session.set_cooperative_jobs(true);
        app.session.active_mut().unwrap().source_read_only = true;
        let before = app.session.active().unwrap().clone();
        pick_from_menu(&mut app);
        let dialog = dialogs(&app)[0].id;
        pick_from_menu(&mut app);
        assert!(Arc::ptr_eq(&before.doc, &app.session.active().unwrap().doc), "a later plain or wrapped pick cannot bypass the earlier reservation");
        assert_eq!(app.session.active().unwrap().revision, before.revision);
        assert_eq!(app.session.active().unwrap().history.past_len(), before.history.past_len());
        tick_probe(&mut app);
        assert_eq!(app.historical_replacements.len(), 2);
        assert_eq!(dialogs(&app).len(), 1);
        assert_eq!(dialogs(&app)[0].id, dialog);
        choose(&mut app, "little-endian");
        let state = app.session.active().unwrap();
        assert_eq!(state.revision, before.revision + 1);
        assert_eq!(state.saved_revision, before.saved_revision);
        assert_eq!(state.history.past_len(), before.history.past_len() + 1);
        assert_eq!(state.path, before.path);
        assert_eq!(state.source_read_only, before.source_read_only);
        let SmartSource::Embedded { bytes, .. } = &object(&app).source else { panic!("first replacement") };
        assert_eq!(bytes.as_slice(), first);
        assert_eq!(object(&app).historical_xcf_sample_order, Some(HistoricalXcfSampleOrder::LittleEndian));
        assert!(app.ui.status_error);
        assert!(app.ui.status.contains("changed"), "{}", app.ui.status);
        assert!(app.prompt_order.is_empty());
        assert!(app.historical_replacements.is_empty());
        assert!(dialogs(&app).is_empty());
    }
}

#[test]
fn cancelling_earlier_choice_allows_unchanged_plain_and_wrapped_modern_targets() {
    for wrapped in [false, true] {
        let modern = modern_pick(wrapped);
        let Picked::Contents(files) = &modern else { panic!("modern fixture") };
        let expected = &files[0].1;
        let expected = expected.clone();
        let mut app = app(vec![Some(contents("earlier.xcf", fixture::historical_pixel(true))), Some(modern)], HistoricalXcfSampleOrder::BigEndian);
        app.session.set_cooperative_jobs(true);
        let before = app.session.active().unwrap().clone();
        pick_from_menu(&mut app);
        pick_from_menu(&mut app);
        tick_probe(&mut app);
        assert!(Arc::ptr_eq(&before.doc, &app.session.active().unwrap().doc));
        assert_eq!(app.historical_replacements.len(), 2);
        cancel_choice(&mut app);
        let state = app.session.active().unwrap();
        assert_eq!(state.revision, before.revision + 1);
        assert_eq!(state.saved_revision, before.saved_revision);
        assert_eq!(state.history.past_len(), before.history.past_len() + 1);
        assert_eq!(state.path, before.path);
        assert_eq!(state.source_read_only, before.source_read_only);
        let modern = object(&app);
        assert_eq!(modern.historical_xcf_sample_order, None);
        let SmartSource::Embedded { bytes, .. } = &modern.source else { panic!("modern source") };
        assert_eq!(bytes.as_slice(), expected);
        let mut pixel = [0.; 4];
        modern.cache.as_ref().unwrap().read_pixel(0, 0, &mut pixel);
        for (actual, expected) in pixel.into_iter().zip([17. / 255., 31. / 255., 57. / 255., 1.]) {
            assert!((actual - expected).abs() <= 1. / 65535.);
        }
        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert!(app.prompt_order.is_empty());
        assert!(app.historical_replacements.is_empty());
    }
}

#[test]
fn queued_modern_replacement_of_another_document_applies_in_order_without_stealing_the_tab() {
    for wrapped in [false, true] {
        let first = fixture::historical_pixel(true);
        let modern = modern_pick(wrapped);
        let Picked::Contents(files) = &modern else { panic!("modern fixture") };
        let expected = &files[0].1;
        let expected = expected.clone();
        let mut app = app(vec![Some(contents("first.xcf", first.clone())), Some(modern)], HistoricalXcfSampleOrder::BigEndian);
        app.session.set_cooperative_jobs(true);
        pick_from_menu(&mut app);
        let parent = app.session.active().unwrap().clone();
        let mut other = Document::new("Other target", Size::new(1, 1), ColorMode::Rgb, SampleType::U16);
        other.layers.push(Layer::new("Other smart", LayerContent::Smart(object(&app).clone())));
        app.session.add_document(other, None);
        app.session.active_mut().unwrap().source_read_only = true;
        app.sync_views();
        let other = app.session.active().unwrap().clone();
        pick_from_menu(&mut app);
        tick_probe(&mut app);
        assert!(Arc::ptr_eq(&app.session.documents()[0].doc, &parent.doc));
        assert!(Arc::ptr_eq(&app.session.documents()[1].doc, &other.doc));
        assert_eq!(app.historical_replacements.len(), 2);
        choose(&mut app, "little-endian");
        assert_eq!(app.session.active().unwrap().doc.id, other.doc.id);
        let changed = app.session.active().unwrap();
        assert_eq!(changed.revision, other.revision + 1);
        assert_eq!(changed.history.past_len(), other.history.past_len() + 1);
        assert_eq!(changed.path, other.path);
        assert_eq!(changed.source_read_only, other.source_read_only);
        let LayerContent::Smart(smart) = &changed.doc.layers[0].content else { panic!("other smart") };
        assert_eq!(smart.historical_xcf_sample_order, None);
        let SmartSource::Embedded { bytes, .. } = &smart.source else { panic!("modern source") };
        assert_eq!(bytes.as_slice(), expected);
        let changed_parent = &app.session.documents()[0];
        assert_eq!(changed_parent.revision, parent.revision + 1);
        assert_eq!(changed_parent.history.past_len(), parent.history.past_len() + 1);
        let SmartSource::Embedded { bytes, .. } = &object(&app).source else { panic!("first source") };
        assert_eq!(bytes.as_slice(), first);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert!(app.prompt_order.is_empty());
        assert!(app.historical_replacements.is_empty());
    }
}

#[test]
fn queued_modern_failures_preserve_earlier_choice_and_unchanged_target() {
    for wrapped in [false, true] {
        let invalid = if wrapped { contents("broken.xcf.gz", vec![0x1f, 0x8b, 0]) } else { contents("broken.pcraft", b"not a native document".to_vec()) };
        let mut app = app(vec![Some(contents("first.xcf", fixture::historical_pixel(true))), Some(invalid)], HistoricalXcfSampleOrder::BigEndian);
        app.session.set_cooperative_jobs(true);
        let before = app.session.active().unwrap().clone();
        pick_from_menu(&mut app);
        let earlier = dialogs(&app)[0].id;
        pick_from_menu(&mut app);
        assert_eq!(app.historical_replacements.len(), 2);
        assert!(Arc::ptr_eq(&before.doc, &app.session.active().unwrap().doc));
        assert_eq!(dialogs(&app)[0].id, earlier);
        if wrapped {
            tick_probe(&mut app);
            assert!(app.ui.status_error, "the failed header job reports only its own source");
            assert_eq!(app.historical_replacements.len(), 1);
            assert_eq!(dialogs(&app)[0].id, earlier);
        }
        cancel_choice(&mut app);
        let after = app.session.active().unwrap();
        assert!(Arc::ptr_eq(&before.doc, &after.doc));
        assert_eq!((after.revision, after.saved_revision, after.history.past_len()), (before.revision, before.saved_revision, before.history.past_len()));
        assert_eq!(after.path, before.path);
        assert_eq!(after.source_read_only, before.source_read_only);
        assert_eq!(object(&app).historical_xcf_sample_order, Some(HistoricalXcfSampleOrder::BigEndian));
        assert!(app.prompt_order.is_empty());
        assert!(app.historical_replacements.is_empty());
        assert!(dialogs(&app).is_empty());
    }
}

#[test]
fn desktop_replacement_picker_asks_fresh_and_uses_exact_picked_bytes_for_both_orders() {
    for (little, order, previous) in
        [(true, "little-endian", HistoricalXcfSampleOrder::BigEndian), (false, "big-endian", HistoricalXcfSampleOrder::LittleEndian)]
    {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("picked.xcf");
        let original = fixture::historical_pixel(little);
        std::fs::write(&path, &original).unwrap();
        let mut app = app(vec![Some(Picked::Paths(vec![path.clone()]))], previous);
        let before = app.session.active().unwrap().doc.clone();
        pick_from_menu(&mut app);
        assert_eq!(app.historical_replacements.len(), 1);
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        let fields = &dialogs(&app).into_iter().find(|dialog| owns(&dialog.fields)).unwrap().fields;
        assert_eq!(fields.get("historicalXcfSampleOrder"), Some(&Value::Null));
        assert!(!can_confirm(fields));
        // A later disk change cannot change the source being interpreted by this answer.
        let later = fixture::historical_pixel(!little);
        std::fs::write(&path, &later).unwrap();
        choose(&mut app, order);
        let smart = object(&app);
        assert_eq!(smart.historical_xcf_sample_order.map(|value| value.to_string()).as_deref(), Some(order));
        let SmartSource::Embedded { bytes, .. } = &smart.source else { panic!("embedded original") };
        assert_eq!(bytes.as_slice(), original);
        let mut pixel = [0.; 4];
        smart.cache.as_ref().unwrap().read_pixel(0, 0, &mut pixel);
        assert_eq!(pixel, [0x1234 as f32 / 65535., 0x4567 as f32 / 65535., 0x89ab as f32 / 65535., 1.]);
        assert_eq!(std::fs::read(&path).unwrap(), later);
        assert_eq!(app.historical_replacements.len(), 0);
        assert!(app.prompt_order.is_empty());
    }
}

#[test]
fn replacement_missing_invalid_and_cancel_answers_never_guess_or_mutate() {
    let mut app = app(vec![Some(contents("picked.xcf", fixture::historical_pixel(true)))], HistoricalXcfSampleOrder::BigEndian);
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    pick_from_menu(&mut app);
    for choice in [Value::Null, json!("native"), json!(false), json!(1)] {
        let id = dialogs(&app).into_iter().find(|dialog| owns(&dialog.fields)).unwrap().id;
        set_order(&mut app, id, choice);
        assert!(confirm(&mut app, id).is_err());
        assert!(dialogs(&app).into_iter().any(|dialog| dialog.id == id));
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    }
    let id = dialogs(&app).into_iter().find(|dialog| owns(&dialog.fields)).unwrap().id;
    cancel(&mut app, id).unwrap();
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    assert_eq!(app.historical_replacements.len(), 0);
    assert!(app.prompt_order.is_empty());
}

#[test]
fn replacement_queue_keeps_other_open_prompts_and_rejects_stale_second_pick() {
    let bytes = fixture::historical_pixel(true);
    let mut app = app(vec![Some(contents("first.xcf", bytes.clone())), Some(contents("second.xcf", bytes))], HistoricalXcfSampleOrder::BigEndian);
    pick_from_menu(&mut app);
    let first = dialogs(&app).into_iter().find(|dialog| owns(&dialog.fields)).unwrap().id;
    pick_from_menu(&mut app);
    assert_eq!(dialogs(&app).into_iter().filter(|dialog| owns(&dialog.fields)).count(), 1);
    assert_eq!(app.historical_replacements.len(), 2);
    let later = Document::new("Later", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let native = photocraft_format::save_to_bytes(&later, &Default::default()).unwrap();
    app.open_bytes("later.pcraft", &native).unwrap();
    assert_eq!(app.session.documents().len(), 1, "the unrelated open waits behind replacement choices");
    choose(&mut app, "little-endian");
    assert!(!dialogs(&app).into_iter().any(|dialog| dialog.id == first));
    let after_first = app.session.active().unwrap().doc.clone();
    let second = dialogs(&app).into_iter().find(|dialog| owns(&dialog.fields)).unwrap().id;
    set_order(&mut app, second, json!("little-endian"));
    assert!(confirm(&mut app, second).unwrap_err().contains("changed"));
    assert!(Arc::ptr_eq(&after_first, &app.session.active().unwrap().doc));
    cancel(&mut app, second).unwrap();
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(app.session.active().unwrap().doc.name, "Later");
    assert!(app.prompt_order.is_empty());
    assert_eq!(app.historical_replacements.len(), 0);
}

#[test]
fn changed_or_closed_replacement_target_can_only_be_cancelled_without_affecting_another_document() {
    for close in [false, true] {
        let mut app = app(vec![Some(contents("picked.xcf", fixture::historical_pixel(true)))], HistoricalXcfSampleOrder::BigEndian);
        pick_from_menu(&mut app);
        if close {
            app.session.close(0);
        } else {
            app.session.execute("layer.new.layer", json!({})).unwrap();
        }
        app.session.add_document(Document::new("Other", Size::new(1, 1), ColorMode::Rgb, SampleType::U8), None);
        app.sync_views();
        let other = app.session.active().unwrap().doc.clone();
        let id = dialogs(&app).into_iter().find(|dialog| owns(&dialog.fields)).unwrap().id;
        set_order(&mut app, id, json!("little-endian"));
        assert!(confirm(&mut app, id).is_err());
        assert!(Arc::ptr_eq(&other, &app.session.active().unwrap().doc));
        cancel(&mut app, id).unwrap();
        assert!(Arc::ptr_eq(&other, &app.session.active().unwrap().doc));
        assert_eq!(app.historical_replacements.len(), 0);
    }
}

#[test]
fn replacement_choices_wait_behind_an_existing_historical_open_reservation() {
    let mut app = app(vec![Some(contents("replacement.xcf", fixture::historical_pixel(true)))], HistoricalXcfSampleOrder::BigEndian);
    app.open_bytes("earlier.xcf", &fixture::historical_pixel(false)).unwrap();
    let earlier = dialogs(&app).into_iter().find(|dialog| dialog.fields.contains_key("__historicalXcfOpen")).unwrap().id;
    pick_from_menu(&mut app);
    assert_eq!(dialogs(&app).len(), 1);
    assert_eq!(dialogs(&app)[0].id, earlier);
    assert_eq!(app.historical_opens.len(), 1);
    set_order(&mut app, earlier, json!("big-endian"));
    confirm(&mut app, earlier).unwrap();
    assert_eq!(app.session.documents().len(), 2);
    let active = app.session.active().unwrap().doc.id;
    choose(&mut app, "little-endian");
    assert_eq!(app.session.active().unwrap().doc.id, active, "confirming a parent replacement preserves the user's active tab");
    assert_eq!(object(&app).historical_xcf_sample_order, Some(HistoricalXcfSampleOrder::LittleEndian));
    assert!(app.prompt_order.is_empty());
    assert!(app.historical_opens.is_empty());
}

#[test]
fn replacing_with_a_modern_source_clears_old_order_without_a_prompt() {
    let modern = Document::new("Modern", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let bytes = photocraft_format::save_to_bytes(&modern, &Default::default()).unwrap();
    let mut app = app(vec![Some(contents("modern.pcraft", bytes.clone()))], HistoricalXcfSampleOrder::LittleEndian);
    pick_from_menu(&mut app);
    assert_eq!(object(&app).historical_xcf_sample_order, None);
    let SmartSource::Embedded { bytes: retained, .. } = &object(&app).source else { panic!("modern embedded source") };
    assert_eq!(retained.as_slice(), bytes);
    assert!(app.historical_replacements.is_empty());
    assert!(dialogs(&app).is_empty());
}

#[test]
fn wrapped_picker_defers_codec_work_and_can_cancel_before_the_cooperative_probe() {
    // This invalid wrapper would synchronously fail a header probe. A cooperative
    // job must be registered without entering the codec in the picker callback.
    for cancel in [false, true] {
        let mut app = app(vec![Some(contents("broken.xcf.gz", vec![0x1f, 0x8b, 0]))], HistoricalXcfSampleOrder::BigEndian);
        app.session.set_cooperative_jobs(true);
        let before = app.session.active().unwrap().doc.clone();
        pick_from_menu(&mut app);
        assert_eq!(app.historical_replacements.len(), 1);
        let held = &app.historical_replacements.held[0];
        assert!(held.question.is_none());
        let job = held.job.expect("deferred header job");
        assert!(app.session.has_jobs());
        assert!(dialogs(&app).is_empty(), "no choice is guessed before probing");
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        if cancel {
            assert!(app.session.cancel_job(job));
        }
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| crate::jobs_ui::tick(&mut app, ui.ctx()));
        output.textures_delta.clear();
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        assert_eq!(app.historical_replacements.len(), 0);
        assert!(app.prompt_order.is_empty());
        assert!(dialogs(&app).is_empty());
        assert_eq!(app.ui.status_error, !cancel);
    }
}

#[test]
fn wrapped_replacement_probe_keeps_exact_bytes_and_queue_identity_until_fresh_choice() {
    use std::io::{Read, Write};
    let original = fixture::historical_pixel(true);
    let mut encoder = flate2::GzBuilder::new().comment("x".repeat(65_535)).write(Vec::new(), flate2::Compression::default());
    encoder.write_all(&original).unwrap();
    let wrapped = encoder.finish().unwrap();
    // flate2 permits 65,535 non-NUL header bytes followed by the terminator.
    // Verify the valid maximum-length envelope independently of the UI worker.
    let mut verifier = flate2::read::GzDecoder::new(wrapped.as_slice());
    let mut decoded = Vec::new();
    verifier.read_to_end(&mut decoded).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(verifier.header().and_then(|header| header.comment()).map(|comment| comment.len()), Some(65_535));
    let mut app = app(vec![Some(contents("picked.xcf.gz", wrapped.clone()))], HistoricalXcfSampleOrder::BigEndian);
    app.session.set_cooperative_jobs(true);
    let before = app.session.active().unwrap().doc.clone();
    pick_from_menu(&mut app);
    assert_eq!(app.historical_replacements.len(), 1);
    assert!(dialogs(&app).is_empty());
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |ui| crate::jobs_ui::tick(&mut app, ui.ctx()));
    output.textures_delta.clear();
    assert_eq!(dialogs(&app).into_iter().filter(|dialog| owns(&dialog.fields)).count(), 1);
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    choose(&mut app, "little-endian");
    let smart = object(&app);
    assert_eq!(smart.historical_xcf_sample_order, Some(HistoricalXcfSampleOrder::LittleEndian));
    let SmartSource::Embedded { bytes, .. } = &smart.source else { panic!("original wrapped source") };
    assert_eq!(bytes.as_slice(), wrapped);
    let mut pixel = [0.; 4];
    smart.cache.as_ref().unwrap().read_pixel(0, 0, &mut pixel);
    assert_eq!(pixel, [0x1234 as f32 / 65535., 0x4567 as f32 / 65535., 0x89ab as f32 / 65535., 1.]);
    assert!(app.prompt_order.is_empty());
    assert!(app.historical_replacements.is_empty());
}

fn owns(fields: &serde_json::Map<String, Value>) -> bool {
    fields.contains_key(FIELD)
}

fn can_confirm(fields: &serde_json::Map<String, Value>) -> bool {
    matches!(fields.get("historicalXcfSampleOrder").and_then(Value::as_str), Some("little-endian" | "big-endian"))
}

#[test]
fn relink_picker_reuses_identical_bytes_and_asks_for_different_source_bytes() {
    let folder = tempfile::tempdir().unwrap();
    let same = folder.path().join("same.xcf");
    let different = folder.path().join("different.xcf");
    let original = fixture::historical_pixel(false);
    let changed = fixture::historical_pixel(true);
    std::fs::write(&same, &original).unwrap();
    std::fs::write(&different, &changed).unwrap();
    let mut app =
        app(vec![Some(contents(same.to_str().unwrap(), original)), Some(contents(different.to_str().unwrap(), changed))], HistoricalXcfSampleOrder::BigEndian);
    let pick = |app: &mut PhotocraftApp| {
        let ctx = egui::Context::default();
        crate::menus::invoke(app, &ctx, "layer.smartObjects.relinkToFile", json!({})).unwrap();
        app.poll_file_dialog();
        app.poll_file_dialog();
        assert!(!app.ui.status_error, "{}", app.ui.status);
    };
    pick(&mut app);
    assert!(dialogs(&app).is_empty());
    assert!(matches!(&object(&app).source, SmartSource::Linked { path } if path == same.to_str().unwrap()));
    pick(&mut app);
    assert_eq!(dialogs(&app).iter().filter(|dialog| owns(&dialog.fields)).count(), 1);
    assert!(matches!(&object(&app).source, SmartSource::Linked { path } if path == same.to_str().unwrap()));
    choose(&mut app, "little-endian");
    assert_eq!(object(&app).historical_xcf_sample_order, Some(HistoricalXcfSampleOrder::LittleEndian));
    assert!(matches!(&object(&app).source, SmartSource::Linked { path } if path == different.to_str().unwrap()));
}

#[test]
fn relink_picker_refuses_a_changed_modern_source_before_rendering() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("modern.xcf");
    let picked = fixture::modern_pixel(0x1234);
    std::fs::write(&path, fixture::modern_pixel(0x4321)).unwrap();
    let mut app = app(vec![Some(contents(path.to_str().unwrap(), picked))], HistoricalXcfSampleOrder::BigEndian);
    let before = app.session.active().unwrap().doc.clone();
    crate::menus::invoke(&mut app, &egui::Context::default(), "layer.smartObjects.relinkToFile", json!({})).unwrap();
    app.poll_file_dialog();
    app.poll_file_dialog();
    assert!(app.ui.status_error && app.ui.status.contains("changed after it was picked"), "{}", app.ui.status);
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
}

#[test]
fn desktop_place_and_open_as_smart_object_resume_the_picked_historical_source() {
    for command in ["file.placeEmbedded", "file.placeLinked", "file.openAsSmartObject"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picked.xcf");
        let original = fixture::historical_pixel(true);
        std::fs::write(&path, &original).unwrap();
        let mut app = app(vec![Some(Picked::Paths(vec![path.clone()]))], HistoricalXcfSampleOrder::BigEndian);
        let before = app.session.active().unwrap().doc.clone();
        crate::menus::invoke(&mut app, &egui::Context::default(), command, json!({})).unwrap();
        app.poll_file_dialog();
        app.poll_file_dialog();
        assert!(!app.ui.status_error, "{command}: {}", app.ui.status);
        assert_eq!(app.session.documents().len(), 1, "{command}");
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "{command}");
        let dialog = dialogs(&app)
            .into_iter()
            .find(|dialog| {
                let field = if command == "file.openAsSmartObject" { "__historicalXcfOpen" } else { FIELD };
                dialog.fields.contains_key(field)
            })
            .expect("order prompt");
        assert!(dialog.fields["historicalXcfSampleOrder"].is_null());
        if command != "file.placeLinked" {
            std::fs::write(&path, fixture::historical_pixel(false)).unwrap();
        }
        set_order(&mut app, dialog.id, json!("little-endian"));
        confirm(&mut app, dialog.id).unwrap();
        let active = app.session.active().unwrap();
        let LayerContent::Smart(placed) = &active.doc.layer(active.active_layer.unwrap()).unwrap().content else { panic!("placed source") };
        match &placed.source {
            SmartSource::Embedded { bytes, .. } => assert_eq!(bytes.as_slice(), original, "{command}"),
            SmartSource::Linked { path: linked } => {
                assert_eq!(std::path::Path::new(linked), path);
                assert_eq!(placed.historical_xcf_link_digest, Some(photocraft_io::linked::historical_xcf_digest(&original)));
            }
        }
        assert_eq!(placed.historical_xcf_sample_order, Some(HistoricalXcfSampleOrder::LittleEndian), "{command}");
        assert_eq!(placed.cache.as_ref().unwrap().pixel(0, 0), [0x1234 as f32 / 65535., 0x4567 as f32 / 65535., 0x89ab as f32 / 65535., 1.], "{command}");
        assert_eq!(app.session.documents().len(), if command == "file.openAsSmartObject" { 2 } else { 1 });
        assert!(app.prompt_order.is_empty());
    }
}

#[test]
fn desktop_place_linked_refuses_a_file_changed_during_the_historical_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("picked.xcf");
    std::fs::write(&path, fixture::historical_pixel(true)).unwrap();
    let mut app = app(vec![Some(Picked::Paths(vec![path.clone()]))], HistoricalXcfSampleOrder::BigEndian);
    let before = app.session.active().unwrap().doc.clone();
    let prior_history = app.session.active().unwrap().history.past_len();
    crate::menus::invoke(&mut app, &egui::Context::default(), "file.placeLinked", json!({})).unwrap();
    app.poll_file_dialog();
    app.poll_file_dialog();
    let dialog = dialogs(&app).into_iter().find(|dialog| owns(&dialog.fields)).expect("order prompt");
    std::fs::write(&path, fixture::historical_pixel(false)).unwrap();

    set_order(&mut app, dialog.id, json!("little-endian"));
    confirm(&mut app, dialog.id).unwrap();
    assert!(app.ui.status_error && app.ui.status.contains("linked source changed"), "{}", app.ui.status);
    let active = app.session.active().unwrap();
    assert!(Arc::ptr_eq(&before, &active.doc));
    assert_eq!(active.history.past_len(), prior_history);
    assert!(app.prompt_order.is_empty());
}

#[test]
fn accepted_historical_source_changes_run_as_cancellable_desktop_jobs() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("picked.xcf");
    let bytes = fixture::historical_pixel(true);
    std::fs::write(&path, &bytes).unwrap();
    for command in [COMMAND, "layer.smartObjects.relinkToFile"] {
        let pick = contents(path.to_str().unwrap(), bytes.clone());
        let mut app = app(vec![Some(pick)], HistoricalXcfSampleOrder::BigEndian);
        let before = app.session.active().unwrap().doc.clone();
        let prior_history = app.session.active().unwrap().history.past_len();
        crate::menus::invoke(&mut app, &egui::Context::default(), command, json!({})).unwrap();
        app.poll_file_dialog();
        app.poll_file_dialog();
        app.background_jobs = true;
        choose(&mut app, "little-endian");
        let job = app.historical_replacements.applying.expect("accepted source change starts a job");
        assert_eq!(app.session.job(job).unwrap().command, command);
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        assert!(app.session.cancel_job(job));
        tick_probe(&mut app);
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        assert_eq!(app.session.active().unwrap().history.past_len(), prior_history);
        assert!(app.historical_replacements.is_empty());
    }
}

#[test]
fn edit_contents_prompts_for_a_legacy_embedded_xcf_and_keeps_the_choice() {
    let mut app = app(Vec::new(), HistoricalXcfSampleOrder::BigEndian);
    app.session
        .edit("Legacy source", |doc, _| {
            let LayerContent::Smart(object) = &mut doc.layers[0].content else { panic!("smart object") };
            object.historical_xcf_sample_order = None;
            Ok(())
        })
        .unwrap();
    let parent = app.session.active().unwrap().doc.id;
    let ctx = egui::Context::default();
    let result = crate::menus::invoke(&mut app, &ctx, "layer.smartObjects.editContents", json!({})).unwrap();
    assert!(result["heldHistoricalXcfEdit"].is_u64());
    let id = dialogs(&app).into_iter().find(|dialog| owns(&dialog.fields)).unwrap().id;
    cancel(&mut app, id).unwrap();
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(object(&app).historical_xcf_sample_order, None);

    app.run("layer.smartObjects.editContents", json!({})).unwrap();
    choose(&mut app, "big-endian");
    assert_eq!(object(&app).historical_xcf_sample_order, Some(HistoricalXcfSampleOrder::BigEndian));
    let child = app.session.active().unwrap();
    assert_ne!(child.doc.id, parent);
    assert_eq!(child.doc.layers[0].surface().unwrap().pixel(0, 0)[0], f32::from(0x1234u16) / 65535.);
}
