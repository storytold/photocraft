use super::*;
use photocraft_doc::{Color, SampleType, Size};
use photocraft_engine::Session;
use photocraft_io::{NativeImportTarget, NativeInitialView};

/// Create a tiny original native document so opening-error tests need no importer or external fixture.
fn document(name: &str) -> Document {
    Document::with_background(name, Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE)
}

/// Build a protected decoded queue entry with supplied view hints and no additional profile-policy work.
fn held(name: &str, view: NativeInitialView) -> Held {
    Held {
        doc: document(name),
        initial_view: Some(view),
        read_only: true,
        damage_pending: false,
        name: name.into(),
        path: None,
        warnings: Vec::new(),
        bytes: None,
        slot: None,
        question: None,
        policy_applied: true,
        color_settings: None,
        ignore_exif_profile_tag: None,
    }
}

/// Hold original specification-built v7 bytes; the optional truncation fails only after a sample order is supplied.
fn historical_app(broken: bool) -> (PhotocraftApp, serde_json::Map<String, Value>, u64) {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    app.session.add_document(document("Existing"), None);
    app.session.color.settings.ask_on_missing = false;
    app.session.color.settings.ask_on_mismatch = false;
    app.background_jobs = false;
    let mut bytes = b"gimp xcf v007\0".to_vec();
    for value in [1u32, 1, 0, 250, 0, 0, 0, 0] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    if broken {
        bytes.truncate(30);
    }
    app.open_bytes("A.xcf", &bytes).unwrap();
    let mut fields = app.ui.dialogs.iter().find(|dialog| owns_historical(&dialog.fields)).unwrap().fields.clone();
    fields.insert("historicalXcfSampleOrder".into(), json!("little-endian"));
    let id = fields[HISTORICAL_FIELD].as_u64().unwrap();
    assert!(app.session.holds_historical_xcf_open(id));
    assert!(!app.session.holds_historical_xcf_open(0));
    (app, fields, id)
}

#[test]
/// Either answer discards a consumed source and admits the next file, through direct helpers and the actual control handler.
fn historical_inline_failure_drops_consumed_identity_and_advances_queue() {
    use crate::control::{ControlRequest, Outcome};
    /// Refuse an answer before dispatch; a consumed identity must still not leave a dead UI prompt.
    fn deny_answer(id: &str, _: &Value) -> photocraft_engine::Result<()> {
        if id == "file.open" { Err(photocraft_engine::EngineError::Other("historical answer denied".into())) } else { Ok(()) }
    }
    for consume_by_cancel in [false, true] {
        for accept in [true, false] {
            for control in [false, true] {
                for denied in [false, true] {
                    let (mut app, fields, id) = historical_app(false);
                    let existing = app.session.active().unwrap().doc.clone();
                    let dialog = app.ui.dialogs.iter_mut().find(|dialog| owns_historical(&dialog.fields)).unwrap();
                    dialog.fields = fields.clone();
                    let dialog_id = dialog.id;
                    arrive(&mut app, held("B.pcraft", NativeInitialView::default()));
                    // Another accepted caller can either open or cancel the exact capability before the UI answers.
                    let consumed = if consume_by_cancel {
                        json!({"heldHistoricalXcfOpen":id,"cancel":true})
                    } else {
                        json!({"heldHistoricalXcfOpen":id,"historicalXcfSampleOrder":"little-endian"})
                    };
                    app.session.execute("file.open", consumed).unwrap();
                    assert!(!app.session.holds_historical_xcf_open(id));
                    app.session.authorize = if denied { Some(deny_answer) } else { None };
                    let error = if control {
                        let method = if accept { "ui.dialog.confirm" } else { "ui.dialog.cancel" };
                        let (request, _receiver) = ControlRequest::new(method, json!({"dialog":dialog_id}));
                        let Outcome::Done(response) = crate::control::handle(&mut app, &egui::Context::default(), &request) else {
                            panic!("inline historical answer must finish immediately")
                        };
                        assert_eq!(response["ok"], false);
                        response["error"].as_str().unwrap().to_string()
                    } else {
                        answer_historical(&mut app, &fields, accept).unwrap_err()
                    };
                    assert!(error.contains(if denied { "historical answer denied" } else { "no longer available" }));
                    assert!(
                        app.held_opens.is_empty(),
                        "a consumed source must not trap B behind a dead question: cancel={consume_by_cancel}, accept={accept}, control={control}"
                    );
                    assert!(!app.ui.dialogs.iter().any(|dialog| dialog.fields.get(HISTORICAL_FIELD).and_then(Value::as_u64) == Some(id)));
                    assert_eq!(app.session.active().unwrap().doc.name, "B.pcraft");
                    assert!(app.session.active().unwrap().source_read_only);
                    assert_eq!(app.session.documents().len(), if consume_by_cancel { 2 } else { 3 });
                    assert!(std::sync::Arc::ptr_eq(&existing, &app.session.documents()[0].doc));
                }
            }
        }
    }
}

#[test]
/// A dispatch denial after successful preflight must preserve a live source, its prompt and the waiting next file.
fn historical_cancel_dispatch_denial_retains_live_choice_until_retry() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    /// Admit the preflight but reject the subsequent engine dispatch, leaving the held capability live.
    fn deny_dispatch(id: &str, _: &Value) -> photocraft_engine::Result<()> {
        if id == "file.open" && CALLS.fetch_add(1, Ordering::SeqCst) > 0 {
            Err(photocraft_engine::EngineError::Other("cancel dispatch denied".into()))
        } else {
            Ok(())
        }
    }
    let (mut app, fields, id) = historical_app(false);
    let existing = app.session.active().unwrap().doc.clone();
    let dialogs = app.ui.dialogs.clone();
    arrive(&mut app, held("B.pcraft", NativeInitialView::default()));
    CALLS.store(0, Ordering::SeqCst);
    app.session.authorize = Some(deny_dispatch);
    assert!(answer_historical(&mut app, &fields, false).unwrap_err().contains("cancel dispatch denied"));
    assert!(CALLS.load(Ordering::SeqCst) >= 2);
    assert!(app.session.holds_historical_xcf_open(id));
    assert_eq!(app.ui.dialogs, dialogs);
    assert_eq!(app.held_opens.iter().map(Pending::name).collect::<Vec<_>>(), ["A.xcf", "B.pcraft"]);
    assert!(std::sync::Arc::ptr_eq(&existing, &app.session.active().unwrap().doc));
    app.session.authorize = None;
    answer_historical(&mut app, &fields, false).unwrap();
    assert!(!app.session.holds_historical_xcf_open(id));
    assert!(app.held_opens.is_empty());
    assert_eq!(app.session.active().unwrap().doc.name, "B.pcraft");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
/// Delay worker event delivery to distinguish a consumed source with a failed UI handoff from a retryable decode failure.
fn historical_worker_failure_recovers_only_a_still_held_source() {
    use photocraft_engine::jobs::JobId;
    for broken in [false, true] {
        let (mut app, fields, id) = historical_app(broken);
        let existing = app.session.active().unwrap().doc.clone();
        arrive(&mut app, held("B.pcraft", NativeInitialView::default()));
        app.background_jobs = true;
        let pending = answer_historical(&mut app, &fields, true).unwrap();
        let job = JobId(pending["job"].as_u64().unwrap());
        let completed = app.session.wait_job(job);
        if broken {
            assert!(completed.is_err());
            assert!(app.session.holds_historical_xcf_open(id));
        } else {
            let completed = completed.unwrap();
            assert!(!app.session.holds_historical_xcf_open(id));
            // The engine has applied the success, but its shell event has not run yet.
            // Closing that document makes the real finish_open_tab handoff fail.
            let document = completed["docId"].as_u64().unwrap();
            let index = app.session.documents().iter().position(|state| state.doc.id.0 == document).unwrap();
            app.session.close(index).unwrap();
        }
        crate::jobs_ui::tick(&mut app, &egui::Context::default());
        assert!(app.jobs.opens.is_empty());
        if broken {
            assert_eq!(app.held_opens.len(), 2, "a source still owned by the engine remains retryable ahead of B");
            assert_eq!(app.session.documents().len(), 1);
            let retry = app.ui.dialogs.iter().find(|dialog| owns_historical(&dialog.fields)).unwrap().fields.clone();
            assert_eq!(retry[HISTORICAL_FIELD], id);
            answer_historical(&mut app, &retry, false).unwrap();
        }
        assert!(!app.session.holds_historical_xcf_open(id));
        assert!(app.held_opens.is_empty(), "a consumed source cannot be revived after the UI handoff fails");
        assert!(!app.ui.dialogs.iter().any(|dialog| dialog.fields.get(HISTORICAL_FIELD).and_then(Value::as_u64) == Some(id)));
        assert_eq!(app.session.documents().len(), 2);
        assert_eq!(app.session.active().unwrap().doc.name, "B.pcraft");
        assert!(app.session.active().unwrap().source_read_only);
        assert!(std::sync::Arc::ptr_eq(&existing, &app.session.documents()[0].doc));
    }
}

#[test]
/// Require swallowed queue errors to post a named notice while preserving existing documents and their active
/// index.
fn queued_invalid_initial_view_is_visible_and_preserves_existing_documents() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    app.session.add_document(document("Existing active"), None);
    app.session.add_document(document("Existing other"), None);
    app.session.set_active(0);
    let before: Vec<_> = app.session.documents().iter().map(|state| state.doc.clone()).collect();
    let invalid = NativeInitialView { samplers: vec![[f64::NAN, 0.5]], ..Default::default() };

    // advance() deliberately ignores the returned Result. The failure still
    // has to reach the normal error status and agent-visible notice stack.
    arrive(&mut app, held("invalid-view.xcf", invalid));

    assert!(app.held_opens.is_empty());
    assert_eq!(app.session.active_index(), Some(0));
    assert_eq!(app.session.documents().iter().map(|state| state.doc.clone()).collect::<Vec<_>>(), before);
    assert!(app.ui.status_error);
    assert!(app.ui.status.contains("invalid-view.xcf"));
    assert!(app.ui.notices.iter().any(|notice| notice.error && notice.title.contains("invalid-view.xcf") && notice.title.contains("sampler")));
}

#[test]
/// Reject invalid profile-prompt hints visibly, then admit the valid next file with its sampler state and source
/// protection.
fn invalid_profile_prompt_view_reports_failure_and_next_queued_file_opens_protected() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    app.session.add_document(document("Existing"), None);
    let existing = app.session.active().unwrap().doc.clone();
    let mut invalid = held("bad-channel-view.xcf", NativeInitialView { target: NativeImportTarget::AlphaChannel(0), ..Default::default() });
    invalid.question = Some(json!({"missing":true,"action":"preserve"}));
    arrive(&mut app, invalid);
    arrive(&mut app, held("next-valid.xcf", NativeInitialView { samplers: vec![[0.5, 0.75]], ..Default::default() }));
    assert_eq!(app.session.documents().len(), 1, "both files wait behind the profile question");

    let error = answer(&mut app, Some("preserve")).unwrap_err();

    assert!(error.contains("target alpha channel is absent"));
    assert!(app.held_opens.is_empty());
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(app.session.documents()[0].doc, existing);
    assert!(app.session.documents().iter().all(|state| state.doc.name != "bad-channel-view.xcf"));
    let admitted = app.session.active().unwrap();
    assert_eq!(admitted.doc.name, "next-valid.xcf");
    assert!(admitted.source_read_only, "successful queue continuation retains source protection");
    assert_eq!(admitted.color_samplers, vec![[0.5, 0.75]]);
    assert!(app.ui.notices.iter().any(|notice| notice.error && notice.title.contains("bad-channel-view.xcf")));
}

#[test]
/// A deferred policy keeps both its working space and EXIF preference when live settings change.
fn held_policy_without_question_uses_captured_working_space() {
    for ignore_exif in [false, true] {
        let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
        app.session.execute("edit.colorSettings", json!({"workingRgb":"display-p3","askOnMissing":false,"policyRgb":"preserve"})).unwrap();
        let mut pending = held("captured-policy.xcf", NativeInitialView::default());
        // Original little-endian TIFF: IFD0 points to an Exif IFD with SHORT ColorSpace = sRGB.
        let mut exif = b"Exif\0\0II\x2a\0\x08\0\0\0".to_vec();
        exif.extend([1, 0, 0x69, 0x87, 4, 0, 1, 0, 0, 0, 26, 0, 0, 0, 0, 0, 0, 0]);
        exif.extend([1, 0, 0x01, 0xA0, 3, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
        pending.doc.metadata.exif = Some(Arc::new(exif));
        pending.policy_applied = false;
        pending.color_settings = Some(app.session.color.settings.clone());
        pending.ignore_exif_profile_tag = Some(ignore_exif);
        app.session.execute("edit.colorSettings", json!({"workingRgb":"adobe-rgb-compat"})).unwrap();
        app.session.edit_prefs(|prefs| prefs.file_handling.ignore_exif_profile_tag = !ignore_exif);
        arrive(&mut app, pending);
        assert!(app.held_opens.is_empty());
        let expected = if ignore_exif { photocraft_cms::Builtin::DisplayP3 } else { photocraft_cms::Builtin::Srgb };
        assert_eq!(app.session.active().unwrap().doc.icc_profile.as_ref().map(|profile| profile.as_slice()), Some(expected.profile().to_bytes().as_slice()));
        assert_eq!(app.session.color.settings.working_rgb, "adobe-rgb-compat");
        assert_eq!(app.session.prefs().file_handling.ignore_exif_profile_tag, !ignore_exif);
    }
}
