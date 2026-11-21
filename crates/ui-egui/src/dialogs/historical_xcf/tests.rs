use super::*;
use crate::dialogs::historical_sample_order::tests_support::{cancel, confirm, dialogs, profile_id, set_order};
use crate::historical_xcf_fixture as fixture;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        crate::Services {
            import: Some(Box::new(|name, bytes, opts| photocraft_io::import_with_options(name, bytes, opts).map_err(|error| error.to_string()))),
            ..Default::default()
        },
    );
    app.background_jobs = false;
    app
}

fn choose(app: &mut PhotocraftApp, choice: &str) {
    let id = dialogs(app).into_iter().find(|dialog| owns(&dialog.fields)).unwrap().id;
    set_order(app, id, json!(choice));
    confirm(app, id).unwrap();
}

fn assert_pixel(app: &PhotocraftApp) {
    let state = app.session.active().unwrap();
    let photocraft_doc::LayerContent::Raster(surface) = &state.doc.layers[0].content else { panic!("native raster") };
    let mut pixel = [0.; 4];
    surface.read_pixel(0, 0, &mut pixel);
    assert_eq!(pixel, [0x1234 as f32 / 65535., 0x4567 as f32 / 65535., 0x89ab as f32 / 65535., 1.]);
    assert!(state.source_read_only);
    assert!(state.path.is_none());
    assert_eq!(state.history.past_len(), 0);
}

#[test]
fn explicit_historical_orders_open_exact_native_samples_and_never_preselect() {
    for (little, order) in [(true, "little-endian"), (false, "big-endian")] {
        let mut app = app();
        app.open_file("/source/renamed.dat", &fixture::historical_pixel(little)).unwrap();
        assert!(app.session.documents().is_empty());
        let dialog = dialogs(&app).into_iter().find(|dialog| owns(&dialog.fields)).unwrap();
        assert_eq!(dialog.fields["historicalXcfSampleOrder"], Value::Null);
        assert!(!can_confirm(&dialog.fields));
        assert!(app.show_if_open("/source/renamed.dat"));
        assert_eq!(app.historical_opens.len(), 1, "the pending source is focused, not opened twice");
        choose(&mut app, order);
        assert_pixel(&app);
        assert!(app.historical_opens.is_empty());
        assert!(app.prompt_order.is_empty());
        assert_eq!(app.ui.recent_files.first().map(String::as_str), Some("/source/renamed.dat"));
    }
}

#[test]
fn missing_or_invalid_choice_keeps_dialog_source_and_document_unchanged_then_cancel_releases_it() {
    let mut app = app();
    app.run("file.new", json!({"width":2,"height":2})).unwrap();
    let control = app.session.active().unwrap().doc.clone();
    app.open_bytes("old.xcf", &fixture::historical_pixel(true)).unwrap();
    let dialog = dialogs(&app).into_iter().find(|dialog| owns(&dialog.fields)).unwrap().id;
    for invalid in [Value::Null, json!("native"), json!(true)] {
        set_order(&mut app, dialog, invalid);
        assert!(confirm(&mut app, dialog).is_err());
        assert!(app.framework.dialogs.stack().contains(dialog));
        assert_eq!(app.historical_opens.len(), 1);
        assert_eq!(app.session.documents().len(), 1);
        assert!(std::sync::Arc::ptr_eq(&control, &app.session.active().unwrap().doc));
    }
    let source = app.historical_opens[0].id;
    cancel(&mut app, dialog).unwrap();
    assert!(app.historical_opens.is_empty());
    assert!(app.prompt_order.is_empty());
    assert!(!app.session.cancel_historical_xcf_open(source));
    assert!(std::sync::Arc::ptr_eq(&control, &app.session.active().unwrap().doc));
}

fn later_document() -> crate::open_prompt::Held {
    crate::open_prompt::Held {
        doc: photocraft_doc::Document::new("later", photocraft_geom::Size::new(2, 2), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8),
        initial_view: None,
        read_only: false,
        damage_pending: false,
        name: "later".into(),
        path: None,
        warnings: Vec::new(),
        bytes: None,
        slot: None,
        order: None,
        question: None,
        policy_applied: true,
        smart_object: false,
    }
}

fn resuming_before_historical_and_decoded(app: &mut PhotocraftApp) -> crate::jobs_ui::OpenTab {
    for id in [101, 102] {
        app.historical_opens.push(Historical {
            id,
            name: format!("historical-{id}.xcf"),
            path: None,
            slot: None,
            order: None,
            version: 4,
            bits: 16,
            job: (id == 101).then_some(JobId(201)),
            place: None,
        });
    }
    app.held_opens.push_back(later_document());
    app.prompt_order.extend([PromptKind::Resuming(101), PromptKind::Historical(102), PromptKind::Open]);
    crate::jobs_ui::OpenTab { job: JobId(201), name: "resumed.xcf".into(), path: None, slot: None, order: None }
}

#[test]
fn direct_historical_completion_leaves_next_historical_ahead_of_unrelated_decoded_open() {
    let mut app = app();
    app.run("file.new", json!({"width":2,"height":2})).unwrap();
    let document = app.session.active().unwrap().doc.id;
    let admission = crate::jobs_ui::finish_open_with_held(&mut app, "resumed.xcf", None, &json!({"docId":document.0,"warnings":[]}), None, None).unwrap();
    assert_eq!(admission, None, "a direct admission appended no held document");
    // Completion may only replace its reservation when its admission actually
    // produced a held document. B's presence is not evidence that A produced one.
    resuming_before_historical_and_decoded(&mut app);
    complete(&mut app, 101, admission);
    assert_eq!(app.prompt_order.iter().copied().collect::<Vec<_>>(), [PromptKind::Historical(102), PromptKind::Open]);
    assert_eq!(app.held_opens.len(), 1);
    assert_eq!(app.held_opens[0].name, "later");
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.active().unwrap().doc.id, document);
    assert_eq!(dialogs(&app)[0].fields[FIELD], 102);
}

#[test]
fn resumed_held_completion_replaces_only_its_own_reservation() {
    let mut app = app();
    app.run("file.new", json!({"width":2,"height":2})).unwrap();
    let document = app.session.active().unwrap().doc.id;
    let tab = resuming_before_historical_and_decoded(&mut app);
    // With the reservation present finish_open re-holds this exact admitted
    // document, and its completion must put A before C while leaving B after C.
    crate::jobs_ui::finish_open_tab(&mut app, &tab, &json!({"docId":document.0,"warnings":[]})).unwrap();
    assert_eq!(app.prompt_order.iter().copied().collect::<Vec<_>>(), [PromptKind::Historical(102), PromptKind::Open]);
    assert_eq!(app.held_opens.len(), 1);
    assert_eq!(app.held_opens[0].name, "later");
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.active().unwrap().doc.id, document);
    assert_eq!(dialogs(&app)[0].fields[FIELD], 102);
}

#[test]
fn failed_resumed_completion_does_not_claim_an_unrelated_held_open() {
    let mut app = app();
    let tab = resuming_before_historical_and_decoded(&mut app);
    assert!(crate::jobs_ui::finish_open_tab(&mut app, &tab, &json!({"warnings":[]})).is_err());
    assert_eq!(app.prompt_order.iter().copied().collect::<Vec<_>>(), [PromptKind::Historical(102), PromptKind::Open]);
    assert_eq!(app.held_opens.len(), 1);
    assert_eq!(app.held_opens[0].name, "later");
    assert!(app.session.documents().is_empty());
    assert_eq!(dialogs(&app)[0].fields[FIELD], 102);
}

#[test]
fn historical_resume_keeps_its_position_before_later_file_and_profile_prompt() {
    for background in [false, true] {
        let mut app = app();
        // An implicit nonlinear XCF stays untagged. Ask about its missing profile so
        // this real color-policy prompt must remain ahead of the later queued file.
        app.run("edit.colorSettings", json!({"policyRgb":"preserve","askOnMissing":true})).unwrap();
        let bytes = fixture::historical_pixel(true);
        let options =
            photocraft_io::ImportOptions { historical_xcf_sample_order: Some(photocraft_io::HistoricalXcfSampleOrder::LittleEndian), ..Default::default() };
        let imported = photocraft_io::import_with_options("first.xcf", &bytes, &options).unwrap();
        let question = app.session.color.open_question(&imported.document).expect("the fixture must require the profile prompt this test orders");
        assert_eq!(question["missing"], true);
        app.open_bytes("first.xcf", &bytes).unwrap();
        crate::open_prompt::arrive(&mut app, later_document());
        app.background_jobs = background;
        choose(&mut app, "little-endian");
        if background {
            let ctx = egui::Context::default();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !app.jobs.opens.is_empty() && std::time::Instant::now() < deadline {
                let mut output = ctx.run_ui(Default::default(), |ui| crate::jobs_ui::tick(&mut app, ui.ctx()));
                output.textures_delta.clear();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(app.jobs.opens.is_empty(), "historical worker finished");
        }
        assert!(app.session.documents().is_empty());
        assert_eq!(app.held_opens.front().unwrap().name, "first.xcf");
        let prompt = profile_id(&app);
        confirm(&mut app, prompt).unwrap();
        assert_eq!(app.session.documents().iter().map(|state| state.doc.name.as_str()).collect::<Vec<_>>(), ["first.xcf", "later"]);
        app.session.set_active(0);
        assert_pixel(&app);
        assert!(app.historical_opens.is_empty());
        assert!(app.prompt_order.is_empty());
    }
}

#[test]
fn cancelled_historical_resume_does_not_steal_later_decoded_entry() {
    let mut app = app();
    app.open_bytes("first.xcf", &fixture::historical_pixel(true)).unwrap();
    crate::open_prompt::arrive(&mut app, later_document());
    app.background_jobs = true;
    choose(&mut app, "little-endian");
    let job = app.jobs.opens[0].job;
    app.session.cancel_job(job);
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |ui| crate::jobs_ui::tick(&mut app, ui.ctx()));
    output.textures_delta.clear();
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.active().unwrap().doc.name, "later");
    assert!(app.historical_opens.is_empty());
    assert!(app.prompt_order.is_empty());
}

#[test]
fn earlier_profile_prompt_stays_ahead_of_historical_choice() {
    let mut app = app();
    let mut first = later_document();
    first.name = "earlier".into();
    first.question = Some(json!({"missing":true,"working":"sRGB"}));
    crate::open_prompt::arrive(&mut app, first);
    let earlier = dialogs(&app)[0].id;
    app.open_bytes("historical.xcf", &fixture::historical_pixel(false)).unwrap();
    assert_eq!(dialogs(&app).len(), 1);
    assert!(!owns(&dialogs(&app)[0].fields));
    confirm(&mut app, earlier).unwrap();
    assert_eq!(app.session.documents().len(), 1);
    assert!(owns(&dialogs(&app)[0].fields));
    choose(&mut app, "big-endian");
    assert_pixel(&app);
    assert_eq!(app.session.documents().len(), 2);
}

#[test]
fn automation_open_forwards_choice_before_read_and_preserves_source_protection() {
    use std::cell::Cell;
    use std::rc::Rc;
    for (little, order) in [(true, "little-endian"), (false, "big-endian")] {
        let mut app = app();
        let reads = Rc::new(Cell::new(0));
        let count = reads.clone();
        app.services.automation_read = Some(Box::new(move |_| {
            count.set(count.get() + 1);
            Ok(crate::AutomationReadResult { name: "old.xcf".into(), bytes: fixture::historical_pixel(little), save_path: Some("source.xcf".into()) })
        }));
        let ctx = egui::Context::default();
        for choice in [json!("invalid"), json!(order)] {
            let (request, _) = crate::control::ControlRequest::new("app.open", json!({"path":"source.xcf","historicalXcfSampleOrder":choice}));
            let crate::control::Reply::Done(reply) = crate::control::handle(&mut app, &ctx, &request) else { panic!("immediate") };
            if choice == "invalid" {
                assert_eq!(reply["ok"], false);
                assert_eq!(reads.get(), 0);
                assert!(app.session.documents().is_empty());
            } else {
                assert_eq!(reply["ok"], true, "{reply}");
                assert_eq!(reads.get(), 1);
                assert_pixel(&app);
            }
        }
    }
}

#[test]
fn refused_historical_start_restores_choice_ahead_of_later_open_then_retries_retained_bytes() {
    let mut app = app();
    app.run("file.new", json!({"width":2,"height":2})).unwrap();
    let original = app.session.active().unwrap().doc.clone();
    app.session.color.settings.ask_on_missing = false;
    app.session.color.settings.ask_on_mismatch = false;
    app.open_file("/source/retained.xcf", &fixture::historical_pixel(false)).unwrap();
    let source = app.historical_opens[0].id;
    crate::open_prompt::arrive(&mut app, later_document());
    let dialog_id = dialogs(&app).into_iter().find(|dialog| owns(&dialog.fields)).unwrap().id;
    set_order(&mut app, dialog_id, json!("big-endian"));
    let cancelled = photocraft_engine::jobs::JobCtx::new();
    cancelled.cancel();
    app.session.set_inline_job_ctx(Some(cancelled));
    confirm(&mut app, dialog_id).unwrap();
    assert!(app.ui.status_error);
    assert!(app.session.holds_historical_xcf_open(source));
    assert_eq!(app.historical_opens.len(), 1);
    assert_eq!(app.historical_opens[0].id, source);
    assert_eq!(app.prompt_order.iter().copied().collect::<Vec<_>>(), [PromptKind::Historical(source), PromptKind::Open]);
    assert_eq!(app.held_opens.len(), 1, "the later decoded file cannot pass the retry choice");
    assert_eq!(app.session.documents().len(), 1);
    assert!(std::sync::Arc::ptr_eq(&original, &app.session.active().unwrap().doc));
    let retry = dialogs(&app).into_iter().find(|dialog| owns(&dialog.fields)).unwrap();
    assert_eq!(retry.fields[FIELD], source);
    assert_eq!(retry.fields["historicalXcfSampleOrder"], Value::Null, "a recreated choice requires an explicit answer");
    app.session.set_inline_job_ctx(None);
    choose(&mut app, "big-endian");
    assert!(!app.session.holds_historical_xcf_open(source));
    assert!(app.historical_opens.is_empty());
    assert!(app.held_opens.is_empty());
    assert!(app.prompt_order.is_empty());
    assert_eq!(app.session.documents().len(), 3);
    assert_eq!(app.session.active().unwrap().doc.name, "later");
    app.session.set_active(1);
    assert_pixel(&app);
    assert!(std::sync::Arc::ptr_eq(&original, &app.session.documents()[0].doc));
}

fn owns(fields: &serde_json::Map<String, Value>) -> bool {
    fields.contains_key(FIELD)
}

fn can_confirm(fields: &serde_json::Map<String, Value>) -> bool {
    matches!(fields.get("historicalXcfSampleOrder").and_then(Value::as_str), Some("little-endian" | "big-endian"))
}
