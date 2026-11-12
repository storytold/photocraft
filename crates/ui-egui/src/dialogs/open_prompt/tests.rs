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
        order: None,
        question: None,
        policy_applied: true,
        smart_object: false,
        color_settings: None,
        ignore_exif_profile_tag: None,
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
    assert!(!app.ui.dialogs.iter().any(|dialog| owns(&dialog.fields)), "the answered invalid file cannot leave a modal blocking queue recovery");
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
