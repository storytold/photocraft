use super::*;
use photocraft_doc::{Color, SampleType, Size};
use photocraft_engine::Session;
use photocraft_io::{NativeImportTarget, NativeInitialView};

fn document(name: &str) -> Document {
    Document::with_background(name, Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE)
}

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
    }
}

#[test]
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
