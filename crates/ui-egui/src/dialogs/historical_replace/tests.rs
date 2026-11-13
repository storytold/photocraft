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
