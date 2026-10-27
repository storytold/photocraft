use super::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_engine::Session;
use photocraft_engine::automate_cmds::ScriptBinding;
use photocraft_geom::Size;

fn scan_app() -> (PhotocraftApp, Value) {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 8, "height": 9, "name": "Previous"})).unwrap();
    session.execute("edit.colorSettings", json!({"askOnMissing": false})).unwrap();
    session.edit_prefs(|prefs| {
        prefs.script_events.enabled = true;
        prefs.script_events.bindings = vec![ScriptBinding { event: "openDocument".into(), script: None, steps: Some(json!([])), name: "Open scan".into() }];
    });
    let mut scan = Document::new("Scan", Size::new(3, 2), ColorMode::Grayscale, SampleType::U16);
    scan.resolution_dpi = 300.0;
    let (index, color) = session.open_document(scan, None);
    session.active_mut().unwrap().saved_revision = 0;
    let report = json!({"available": true, "devices": [], "acquired": true, "cancelled": false,
        "document": index, "docId": session.active().unwrap().doc.id.0, "color": color, "warnings": ["Scanner import note"]});
    (PhotocraftApp::new(session, crate::Services::default()), report)
}

#[test]
fn acquired_scan_shows_warnings_without_a_save_or_recent_path() {
    let (mut app, report) = scan_app();
    let previous = app.session.documents()[0].doc.id;
    let opening = JobId(41);
    app.jobs.opens.push(OpenTab { job: opening, name: "Other image".into(), path: None, slot: None, order: None });
    app.jobs.focus = Some(opening);
    app.ui.chrome.home = Some(app.session.documents().len());
    finish_wia(&mut app, &report).unwrap();
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(app.session.documents()[0].doc.id, previous);
    let active = app.session.active().unwrap();
    assert_eq!(active.doc.name, "Scan");
    assert_eq!(active.doc.depth, SampleType::U16);
    assert_eq!(active.doc.mode, ColorMode::Grayscale);
    assert_eq!(active.doc.resolution_dpi, 300.0);
    assert!(active.path.is_none());
    assert!(active.is_dirty());
    assert_eq!(app.ui.views.len(), 2);
    assert_eq!(app.session.active_index(), Some(1));
    assert!(app.jobs.focus.is_none());
    assert!(app.ui.chrome.home.is_none());
    assert_eq!(app.jobs.opens.len(), 1, "other background opening tabs remain available");
    assert!(app.session.prefs().file_handling.recent_files.is_empty());
    assert!(app.ui.dialogs.is_empty());
    assert!(app.ui.notices.iter().any(|notice| notice.lines.iter().any(|line| line == "Scanner import note")));
    assert_eq!(app.session.file_menu.event_log.len(), 1, "one Open Document hook for the completed scan");
}

#[test]
fn background_scan_completion_keeps_captured_automation_script_suppression() {
    for automation in [false, true] {
        let (mut app, report) = scan_app();
        let job = JobId(37);
        if automation {
            app.jobs.wia_without_script_events.push(job);
        }
        // The control request's flag and preferences have already been restored by the
        // time its background result arrives.
        app.automation_input = false;
        on_event(
            &mut app,
            &egui::Context::default(),
            JobEvent { id: job, command: "file.import.wiaSupport".into(), label: "WIA scanner".into(), document: None, outcome: JobOutcome::Done(report) },
        );
        assert_eq!(app.session.file_menu.event_log.len(), usize::from(!automation));
        assert!(app.session.prefs().script_events.enabled, "restore the current preference after completion");
        assert!(app.jobs.wia_without_script_events.is_empty());
        assert!(app.ui.dialogs.is_empty());
        assert!(app.session.active().unwrap().is_dirty());
    }
}

#[test]
fn synchronous_automation_scan_does_not_fire_user_script_hooks() {
    let (mut app, report) = scan_app();
    app.automation_input = true;
    finish_wia(&mut app, &report).unwrap();
    assert!(app.session.file_menu.event_log.is_empty());
    assert!(app.session.prefs().script_events.enabled);
}

#[test]
fn scanner_cancellation_and_discovery_do_not_open_or_change_documents() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 9})).unwrap();
    let before = app.session.active().unwrap().doc.id;
    app.ui.status = "Ready".into();
    finish_wia(&mut app, &json!({"available": true, "devices": [], "acquired": false, "cancelled": false})).unwrap();
    assert_eq!(app.ui.status, "Ready", "discovery has no shell effects");
    finish_wia(&mut app, &json!({"available": true, "devices": [], "acquired": false, "cancelled": true})).unwrap();
    assert!(app.ui.status.contains("Cancelled"));
    assert!(!app.ui.status_error);
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.active().unwrap().doc.id, before);
    assert!(app.ui.dialogs.is_empty());
    assert!(app.ui.notices.is_empty());
}

#[test]
fn scan_stays_unsaved_after_the_standard_missing_profile_prompt() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 2, "height": 2})).unwrap();
    let previous = app.session.active().unwrap().doc.id;
    app.session.execute("edit.colorSettings", json!({"askOnMissing": true})).unwrap();
    let mut image = photocraft_codecs::Image::new(3, 2, photocraft_codecs::ChannelLayout::Gray, photocraft_codecs::SampleType::U16).unwrap();
    image.meta.dpi = Some((300.0, 300.0));
    let bytes = photocraft_codecs::encode(&image, photocraft_codecs::Format::Tiff, &photocraft_codecs::EncodeOptions::default()).unwrap();
    let mut report = match app.session.start_open("Scan.tif", OpenSource::Bytes(Arc::new(bytes))).unwrap() {
        Started::Done(result) => result,
        Started::Job(job) => app.session.wait_job(job).unwrap(),
    };
    report["acquired"] = json!(true);
    finish_wia(&mut app, &report).unwrap();
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.active().unwrap().doc.id, previous);
    let prompt = app.ui.dialogs.first().unwrap();
    assert_eq!(prompt.fields["__prefsui"], "mismatch");
    assert_eq!(prompt.fields["missing"], true);
    crate::open_prompt::answer(&mut app, Some("preserve")).unwrap();
    assert_eq!(app.session.documents().len(), 2);
    let scan = app.session.active().unwrap();
    assert!(scan.path.is_none());
    assert!(scan.is_dirty(), "accepting the profile prompt must still offer to save the scan");
    assert_eq!(scan.doc.depth, SampleType::U16);
    assert_eq!(scan.doc.resolution_dpi, 300.0);
}
