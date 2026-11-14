use super::*;
use crate::file_dialog::FileDialogAnswer;
use photocraft_doc::{ColorMode, Document, HistoricalXcfSampleOrder, Layer, SampleType, SmartObject, SmartSource};
use photocraft_geom::{Affine, Size};
use std::sync::Arc;

use crate::historical_xcf_fixture as fixture;

fn app(answers: Vec<Option<FileDialogAnswer>>, old_order: HistoricalXcfSampleOrder) -> PhotocraftApp {
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
            file_dialog: Some(crate::file_dialog::fake(answers).0),
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
    app.poll_file_dialog(&ctx, None);
    app.poll_file_dialog(&ctx, None);
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

fn choose(app: &mut PhotocraftApp, choice: &str) {
    let dialog = app.ui.dialogs.iter_mut().find(|dialog| owns(&dialog.fields)).unwrap();
    dialog.fields.insert("historicalXcfSampleOrder".into(), json!(choice));
    let id = dialog.id;
    crate::dialogs::confirm(app, id).unwrap();
}

fn object(app: &PhotocraftApp) -> &SmartObject {
    let state = app.session.documents().iter().find(|state| state.doc.name == "Parent").unwrap();
    let LayerContent::Smart(smart) = &state.doc.layers[0].content else { panic!("smart object") };
    smart
}

fn modern_pick(wrapped: bool) -> FileDialogAnswer {
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
        FileDialogAnswer::Contents("modern.xcf.gz".into(), encoder.finish().unwrap())
    } else {
        FileDialogAnswer::Contents("modern.xcf".into(), bytes)
    }
}

fn tick_probe(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |ui| crate::jobs_ui::tick(app, ui.ctx()));
    output.textures_delta.clear();
}

fn cancel_choice(app: &mut PhotocraftApp) {
    let id = app.ui.dialogs.iter().find(|dialog| owns(&dialog.fields)).unwrap().id;
    crate::dialogs::cancel(app, id).unwrap();
}

#[test]
fn plain_and_wrapped_modern_picks_wait_then_reject_a_changed_same_target() {
    for wrapped in [false, true] {
        let first = fixture::historical_pixel(true);
        let mut app =
            app(vec![Some(FileDialogAnswer::Contents("first.xcf".into(), first.clone())), Some(modern_pick(wrapped))], HistoricalXcfSampleOrder::BigEndian);
        app.session.set_cooperative_jobs(true);
        app.session.active_mut().unwrap().source_read_only = true;
        let before = app.session.active().unwrap().clone();
        pick_from_menu(&mut app);
        let dialog = app.ui.dialogs[0].id;
        pick_from_menu(&mut app);
        assert!(Arc::ptr_eq(&before.doc, &app.session.active().unwrap().doc), "a later plain or wrapped pick cannot bypass the earlier reservation");
        assert_eq!(app.session.active().unwrap().revision, before.revision);
        assert_eq!(app.session.active().unwrap().history.past_len(), before.history.past_len());
        tick_probe(&mut app);
        assert_eq!(app.historical_replacements.len(), 2);
        assert_eq!(app.ui.dialogs.len(), 1);
        assert_eq!(app.ui.dialogs[0].id, dialog);
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
        assert!(app.ui.dialogs.is_empty());
    }
}

#[test]
fn cancelling_earlier_choice_allows_unchanged_plain_and_wrapped_modern_targets() {
    for wrapped in [false, true] {
        let modern = modern_pick(wrapped);
        let FileDialogAnswer::Contents(_, expected) = &modern else { panic!("modern fixture") };
        let expected = expected.clone();
        let mut app = app(
            vec![Some(FileDialogAnswer::Contents("earlier.xcf".into(), fixture::historical_pixel(true))), Some(modern)],
            HistoricalXcfSampleOrder::BigEndian,
        );
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
        let FileDialogAnswer::Contents(_, expected) = &modern else { panic!("modern fixture") };
        let expected = expected.clone();
        let mut app = app(vec![Some(FileDialogAnswer::Contents("first.xcf".into(), first.clone())), Some(modern)], HistoricalXcfSampleOrder::BigEndian);
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
        let invalid = if wrapped {
            FileDialogAnswer::Contents("broken.xcf.gz".into(), vec![0x1f, 0x8b, 0])
        } else {
            FileDialogAnswer::Contents("broken.pcraft".into(), b"not a native document".to_vec())
        };
        let mut app = app(
            vec![Some(FileDialogAnswer::Contents("first.xcf".into(), fixture::historical_pixel(true))), Some(invalid)],
            HistoricalXcfSampleOrder::BigEndian,
        );
        app.session.set_cooperative_jobs(true);
        let before = app.session.active().unwrap().clone();
        pick_from_menu(&mut app);
        let earlier = app.ui.dialogs[0].id;
        pick_from_menu(&mut app);
        assert_eq!(app.historical_replacements.len(), 2);
        assert!(Arc::ptr_eq(&before.doc, &app.session.active().unwrap().doc));
        assert_eq!(app.ui.dialogs[0].id, earlier);
        if wrapped {
            tick_probe(&mut app);
            assert!(app.ui.status_error, "the failed header job reports only its own source");
            assert_eq!(app.historical_replacements.len(), 1);
            assert_eq!(app.ui.dialogs[0].id, earlier);
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
        assert!(app.ui.dialogs.is_empty());
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
        let mut app = app(vec![Some(FileDialogAnswer::Paths(vec![path.to_string_lossy().into_owned()]))], previous);
        let before = app.session.active().unwrap().doc.clone();
        pick_from_menu(&mut app);
        assert_eq!(app.historical_replacements.len(), 1);
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        let fields = &app.ui.dialogs.iter().find(|dialog| owns(&dialog.fields)).unwrap().fields;
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
    let mut app = app(vec![Some(FileDialogAnswer::Contents("picked.xcf".into(), fixture::historical_pixel(true)))], HistoricalXcfSampleOrder::BigEndian);
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    pick_from_menu(&mut app);
    for choice in [Value::Null, json!("native"), json!(false), json!(1)] {
        let dialog = app.ui.dialogs.iter_mut().find(|dialog| owns(&dialog.fields)).unwrap();
        dialog.fields.insert("historicalXcfSampleOrder".into(), choice);
        let id = dialog.id;
        assert!(crate::dialogs::confirm(&mut app, id).is_err());
        assert!(app.ui.dialogs.iter().any(|dialog| dialog.id == id));
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    }
    let id = app.ui.dialogs.iter().find(|dialog| owns(&dialog.fields)).unwrap().id;
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    assert_eq!(app.historical_replacements.len(), 0);
    assert!(app.prompt_order.is_empty());
}

#[test]
fn replacement_queue_keeps_other_open_prompts_and_rejects_stale_second_pick() {
    let bytes = fixture::historical_pixel(true);
    let mut app = app(
        vec![Some(FileDialogAnswer::Contents("first.xcf".into(), bytes.clone())), Some(FileDialogAnswer::Contents("second.xcf".into(), bytes))],
        HistoricalXcfSampleOrder::BigEndian,
    );
    pick_from_menu(&mut app);
    let first = app.ui.dialogs.iter().find(|dialog| owns(&dialog.fields)).unwrap().id;
    pick_from_menu(&mut app);
    assert_eq!(app.ui.dialogs.iter().filter(|dialog| owns(&dialog.fields)).count(), 1);
    assert_eq!(app.historical_replacements.len(), 2);
    let later = Document::new("Later", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let native = photocraft_format::save_to_bytes(&later, &Default::default()).unwrap();
    app.open_bytes("later.pcraft", &native).unwrap();
    assert_eq!(app.session.documents().len(), 1, "the unrelated open waits behind replacement choices");
    choose(&mut app, "little-endian");
    assert!(!app.ui.dialogs.iter().any(|dialog| dialog.id == first));
    let after_first = app.session.active().unwrap().doc.clone();
    let dialog = app.ui.dialogs.iter_mut().find(|dialog| owns(&dialog.fields)).unwrap();
    dialog.fields.insert("historicalXcfSampleOrder".into(), json!("little-endian"));
    let second = dialog.id;
    assert!(crate::dialogs::confirm(&mut app, second).unwrap_err().contains("changed"));
    assert!(Arc::ptr_eq(&after_first, &app.session.active().unwrap().doc));
    crate::dialogs::cancel(&mut app, second).unwrap();
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(app.session.active().unwrap().doc.name, "Later");
    assert!(app.prompt_order.is_empty());
    assert_eq!(app.historical_replacements.len(), 0);
}

#[test]
fn changed_or_closed_replacement_target_can_only_be_cancelled_without_affecting_another_document() {
    for close in [false, true] {
        let mut app = app(vec![Some(FileDialogAnswer::Contents("picked.xcf".into(), fixture::historical_pixel(true)))], HistoricalXcfSampleOrder::BigEndian);
        pick_from_menu(&mut app);
        if close {
            app.session.close(0);
        } else {
            app.session.execute("layer.new.layer", json!({})).unwrap();
        }
        app.session.add_document(Document::new("Other", Size::new(1, 1), ColorMode::Rgb, SampleType::U8), None);
        app.sync_views();
        let other = app.session.active().unwrap().doc.clone();
        let dialog = app.ui.dialogs.iter_mut().find(|dialog| owns(&dialog.fields)).unwrap();
        dialog.fields.insert("historicalXcfSampleOrder".into(), json!("little-endian"));
        let id = dialog.id;
        assert!(crate::dialogs::confirm(&mut app, id).is_err());
        assert!(Arc::ptr_eq(&other, &app.session.active().unwrap().doc));
        crate::dialogs::cancel(&mut app, id).unwrap();
        assert!(Arc::ptr_eq(&other, &app.session.active().unwrap().doc));
        assert_eq!(app.historical_replacements.len(), 0);
    }
}

#[test]
fn replacement_choices_wait_behind_an_existing_historical_open_reservation() {
    let mut app = app(vec![Some(FileDialogAnswer::Contents("replacement.xcf".into(), fixture::historical_pixel(true)))], HistoricalXcfSampleOrder::BigEndian);
    app.open_bytes("earlier.xcf", &fixture::historical_pixel(false)).unwrap();
    let earlier = app.ui.dialogs.iter().find(|dialog| crate::dialogs::historical_xcf::owns(&dialog.fields)).unwrap().id;
    pick_from_menu(&mut app);
    assert_eq!(app.ui.dialogs.len(), 1);
    assert_eq!(app.ui.dialogs[0].id, earlier);
    assert_eq!(app.historical_opens.len(), 1);
    app.ui.dialogs[0].fields.insert("historicalXcfSampleOrder".into(), json!("big-endian"));
    crate::dialogs::confirm(&mut app, earlier).unwrap();
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
    let mut app = app(vec![Some(FileDialogAnswer::Contents("modern.pcraft".into(), bytes.clone()))], HistoricalXcfSampleOrder::LittleEndian);
    pick_from_menu(&mut app);
    assert_eq!(object(&app).historical_xcf_sample_order, None);
    let SmartSource::Embedded { bytes: retained, .. } = &object(&app).source else { panic!("modern embedded source") };
    assert_eq!(retained.as_slice(), bytes);
    assert!(app.historical_replacements.is_empty());
    assert!(app.ui.dialogs.is_empty());
}

#[test]
fn wrapped_picker_defers_codec_work_and_can_cancel_before_the_cooperative_probe() {
    // This invalid wrapper would synchronously fail a header probe. A cooperative
    // job must be registered without entering the codec in the picker callback.
    for cancel in [false, true] {
        let mut app = app(vec![Some(FileDialogAnswer::Contents("broken.xcf.gz".into(), vec![0x1f, 0x8b, 0]))], HistoricalXcfSampleOrder::BigEndian);
        app.session.set_cooperative_jobs(true);
        let before = app.session.active().unwrap().doc.clone();
        pick_from_menu(&mut app);
        assert_eq!(app.historical_replacements.len(), 1);
        let held = &app.historical_replacements.held[0];
        assert!(held.question.is_none());
        let job = held.job.expect("deferred header job");
        assert!(app.session.has_jobs());
        assert!(app.ui.dialogs.is_empty(), "no choice is guessed before probing");
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
        assert!(app.ui.dialogs.is_empty());
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
    let mut app = app(vec![Some(FileDialogAnswer::Contents("picked.xcf.gz".into(), wrapped.clone()))], HistoricalXcfSampleOrder::BigEndian);
    app.session.set_cooperative_jobs(true);
    let before = app.session.active().unwrap().doc.clone();
    pick_from_menu(&mut app);
    assert_eq!(app.historical_replacements.len(), 1);
    assert!(app.ui.dialogs.is_empty());
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |ui| crate::jobs_ui::tick(&mut app, ui.ctx()));
    output.textures_delta.clear();
    assert_eq!(app.ui.dialogs.iter().filter(|dialog| owns(&dialog.fields)).count(), 1);
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
