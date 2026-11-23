use super::*;
use photocraft_engine::color_cmds::ColorExt;
use photocraft_engine::jobs::{OpenSource, PendingPsdPreflight, SourceFingerprint};
use std::sync::Arc;

#[path = "../../../../engine-core/src/jobs/historical_open/tests/fixture.rs"]
mod fixture;

fn app() -> Box<PhotocraftApp> {
    let mut session = photocraft_engine::Session::new();
    session.color_mut().settings.ask_on_missing = false;
    session.color_mut().settings.ask_on_mismatch = false;
    let mut app = PhotocraftApp::new(session, crate::Services::default());
    app.background_jobs = true;
    app
}

#[allow(clippy::unwrap_used, reason = "test fixture requires the background Open to complete successfully")]
fn finish_background(app: &mut PhotocraftApp) {
    let tab = app.jobs.opens.remove(0);
    let result = app.session.wait_job(tab.job).unwrap();
    jobs_ui::finish_open_tab(app, &tab, &result).unwrap();
}

#[allow(clippy::unwrap_used, reason = "test fixture requires a retained historical Open request")]
fn start(app: &mut PhotocraftApp, name: &str) -> u64 {
    jobs_ui::start_open(app, name, None, OpenSource::Bytes(Arc::new(fixture::historical_rgba16())), None, None).unwrap();
    finish_background(app);
    app.held_historical_opens.back().unwrap().id
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn ordinary_background_path_open_keeps_request_slot_source_identity_and_exact_pixels() {
    let mut app = app();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.xcf");
    let original = fixture::historical_rgba16();
    std::fs::write(&path, &original).unwrap();
    let source_path = path.to_str().unwrap();
    let batch = jobs_ui::begin_batch(&mut app);
    let order = jobs_ui::OpenOrder { batch, seq: 0 };
    jobs_ui::start_open(&mut app, "source.xcf", Some(source_path.into()), OpenSource::Path(source_path.into()), Some(0), Some(order)).unwrap();
    finish_background(&mut app);
    let pending = app.held_historical_opens.front().unwrap().clone();
    assert_eq!(pending.slot, Some(0));
    assert_eq!(pending.order, Some(order));
    assert!(app.session.documents().is_empty());
    assert_eq!(app.jobs.batches.len(), 1);
    assert!(app.show_if_open(source_path));
    assert_eq!(app.held_historical_opens.len(), 1, "another Open reuses the same prompt");
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"another source now occupies the path").unwrap();
    answer(&mut app, pending.id, Some("big-endian")).unwrap();
    assert!(!app.session.holds_historical_xcf_open(pending.id));
    assert!(app.held_historical_opens.is_empty());
    finish_background(&mut app);
    let state = app.session.active().unwrap();
    assert!(state.source_read_only && state.path.is_none());
    assert_eq!(state.protected_import_source.as_deref(), Some(source_path));
    assert_eq!(state.doc.layers[0].surface().unwrap().read_region(state.doc.bounds())[0], f32::from(0x1234u16) / 65535.);
    assert_eq!(std::fs::read(&path).unwrap(), b"another source now occupies the path");
    assert!(app.jobs.batches.is_empty());
}

#[test]
fn historical_dialog_starts_unselected_and_checks_identity_and_order() {
    let mut app = app();
    let request = start(&mut app, "unmarked.xcf");
    let ctx = app.ctx().clone();
    let dialog = app.framework.dialogs.get_mut::<HistoricalOpenDialog>().unwrap();
    assert_eq!(dialog.json_fields().unwrap()["historicalXcfSampleOrder"], "");
    assert!(dialog.confirm(&ctx).is_none(), "Enter/programmatic OK cannot guess an order");
    assert!(dialog.set_json_field(&ctx, "request", json!(request + 1)).is_err());
    assert!(dialog.set_json_field(&ctx, "historicalXcfSampleOrder", json!("native")).is_err());
    dialog.set_json_field(&ctx, "historicalXcfSampleOrder", json!("little-endian")).unwrap();
    assert_eq!(dialog.confirm(&ctx).as_deref(), Some("little-endian"));
    assert!(answer(&mut app, request + 1, Some("big-endian")).is_err());
    assert!(app.session.holds_historical_xcf_open(request));
    answer(&mut app, request, None).unwrap();
    assert!(!app.session.holds_historical_xcf_open(request));
    assert!(app.session.documents().is_empty());
}

#[test]
fn refused_answer_keeps_exact_request_and_external_cancel_releases_stale_dialog_only() {
    let mut app = app();
    let first = start(&mut app, "first.xcf");
    let second = start(&mut app, "second.xcf");
    app.automation_input = true;
    app.services.automation_command = Some(Box::new(|command, _| if command == COMMAND { Err("source decision denied".into()) } else { Ok(()) }));
    assert!(answer(&mut app, first, Some("big-endian")).unwrap_err().contains("denied"));
    assert!(app.session.holds_historical_xcf_open(first));
    assert_eq!(app.held_historical_opens.front().unwrap().id, first);
    assert!(app.jobs.opens.is_empty());
    app.automation_input = false;
    assert!(app.session.cancel_historical_xcf_open(first));
    open_prompt::advance(&mut app);
    assert_eq!(app.held_historical_opens.front().unwrap().id, second);
    assert!(app.framework.dialogs.stack().contains(id(second)));
    assert!(!app.framework.dialogs.stack().contains(id(first)));
    assert!(answer(&mut app, first, None).is_err(), "late answers cannot cancel the next source");
    assert!(app.session.holds_historical_xcf_open(second));
    answer(&mut app, second, None).unwrap();
}

#[test]
fn memory_historical_and_profile_prompts_keep_existing_fifo_and_bulk_cancel_releases_sources() {
    let mut app = app();
    let source = OpenSource::Bytes(Arc::new(vec![1, 2, 3]));
    let memory = app
        .session
        .store_pending_preflight(PendingPsdPreflight {
            id: 0,
            name: "large.psd".into(),
            fingerprint: SourceFingerprint::of(&source).unwrap(),
            source,
            required: 10,
            soft_limit: 5,
            hard_limit: 20,
            available_ram: None,
            confirm_damage: true,
        })
        .unwrap();
    open_prompt::arrive_memory(&mut app, memory, "large.psd".into(), None, None, None, 10, None, true, false);
    let historical = start(&mut app, "historical.xcf");
    assert_eq!(app.prompt_order.front(), Some(&open_prompt::PromptKind::Memory));
    assert!(!is_open(&app));
    app.session.color_mut().settings.ask_on_missing = true;
    let doc = photocraft_doc::Document::new("profile", photocraft_doc::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    let question = app.session.color().open_question(&doc);
    assert!(question.is_some());
    open_prompt::arrive(
        &mut app,
        open_prompt::Held {
            doc,
            read_only: false,
            initial_view: None,
            protected_import_source: None,
            damage_pending: false,
            name: "profile".into(),
            path: None,
            warnings: Vec::new(),
            bytes: None,
            slot: None,
            order: None,
            question,
            policy_applied: false,
            smart_object: false,
        },
    );
    let ctx = app.ctx().clone();
    let memory_dialog = crate::dialogs::alerts::prompt_tagged::<open_prompt::MemoryTag>(&app).unwrap();
    crate::dialogs::cancel(&mut app, &ctx, memory_dialog).unwrap();
    assert_eq!(app.prompt_order.front(), Some(&open_prompt::PromptKind::Historical(historical)));
    assert!(app.session.documents().is_empty());
    answer(&mut app, historical, None).unwrap();
    assert_eq!(app.prompt_order.front(), Some(&open_prompt::PromptKind::Open));
    let profile_dialog = app.framework.dialogs.get::<craft::color_ui::ui::ProfileMismatchDialog>().unwrap().id();
    crate::dialogs::cancel(&mut app, &ctx, profile_dialog).unwrap();
    assert!(app.prompt_order.is_empty());
    assert!(app.session.documents().is_empty());
    let next = start(&mut app, "bulk.xcf");
    open_prompt::answer(&mut app, None).unwrap();
    assert!(!app.session.holds_historical_xcf_open(next));
    assert!(app.held_historical_opens.is_empty());
}

#[test]
fn historical_prompt_waits_behind_existing_place_gate() {
    let mut app = app();
    app.jobs.place = Some(jobs_ui::PlaceJob { job: JobId(999), name: "placing".into(), doc: photocraft_doc::DocId(1) });
    let request = start(&mut app, "after-place.xcf");
    assert!(open_prompt::waiting(&app));
    assert!(!is_open(&app), "the existing 9e Place gate suppresses this dialog too");
    assert!(app.session.holds_historical_xcf_open(request));
    app.jobs.place = None;
    open_prompt::advance(&mut app);
    assert!(is_open(&app));
    answer(&mut app, request, None).unwrap();
}

#[test]
fn profile_answers_cannot_consume_a_damaged_open_behind_memory() {
    let mut app = app();
    app.prompt_order.extend([open_prompt::PromptKind::Memory, open_prompt::PromptKind::Open]);
    app.held_opens.push_back(open_prompt::Held {
        doc: photocraft_doc::Document::new("Damaged", photocraft_geom::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8),
        read_only: false,
        initial_view: None,
        protected_import_source: None,
        damage_pending: true,
        name: "damaged.psd".into(),
        path: None,
        warnings: vec![],
        bytes: None,
        slot: None,
        order: None,
        question: None,
        policy_applied: false,
        smart_object: false,
    });
    let settings = app.session.color().settings.clone();
    assert!(open_prompt::answer(&mut app, Some("convert")).unwrap_err().contains("damaged PSD"));
    assert!(open_prompt::answer_remembering(&mut app, "convert").unwrap_err().contains("damaged PSD"));
    assert_eq!(app.session.color().settings, settings);
    assert_eq!(app.held_opens.len(), 1);
    assert_eq!(app.prompt_order.iter().copied().collect::<Vec<_>>(), [open_prompt::PromptKind::Memory, open_prompt::PromptKind::Open]);
    assert!(app.session.documents().is_empty());
}
