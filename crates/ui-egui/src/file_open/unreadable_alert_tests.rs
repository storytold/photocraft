//! #14059: unreadable File > Open inputs use one modal alert per file, while callers that
//! request an error result still receive it.

use super::*;
use crate::control::{ControlRequest, Outcome, handle};
use crate::file_dialog::{self, FileDialogAnswer};
use crate::state::DialogKind;
use crate::{Services, menus};
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn app_with(answer: Option<FileDialogAnswer>) -> PhotocraftApp {
    let services = Services {
        file_dialog: Some(file_dialog::fake(vec![answer]).0),
        import: Some(Box::new(|name, bytes, opts| {
            photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| format!("{name}: {e}"))
        })),
        ..Default::default()
    };
    PhotocraftApp::new(photocraft_engine::Session::new(), services)
}

fn pick(app: &mut PhotocraftApp) {
    menus::invoke(app, &egui::Context::default(), "file.open", json!({})).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
}

fn messages(app: &PhotocraftApp) -> Vec<&str> {
    app.ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).map(|d| d.fields["message"].as_str().unwrap()).collect()
}

fn expected(name: &str) -> String {
    format!("Could not open \"{name}\" because PhotoCraft does not recognize this type of file.")
}

fn png() -> Vec<u8> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 2, "height": 2, "background": "white"})).unwrap();
    photocraft_io::export(&s.active().unwrap().doc, "valid.png", &Default::default()).unwrap().bytes
}

#[test]
fn unrecognized_and_mislabeled_files_show_the_exact_alert_without_opening_documents() {
    for (name, bytes) in [("notes.txt", b"ordinary text".to_vec()), ("broken.png", b"ordinary text".to_vec()), ("zz-bad.png", (1..=17).collect())] {
        let mut app = app_with(Some(FileDialogAnswer::Contents(name.into(), bytes)));
        pick(&mut app);
        assert_eq!(messages(&app), [expected(name)]);
        assert!(app.session.documents().is_empty());
        assert!(app.ui.recent_files.is_empty());
        assert!(app.ui.notices.is_empty(), "the modal replaces the corner notice");
        assert!(app.ui.status_error);
        assert_eq!(app.ui.status, expected(name));
        assert_eq!(app.ui.status.matches(name).count(), 1);
    }
}

#[test]
fn an_open_failure_alert_has_one_ok_button_and_closes_when_clicked() {
    let mut app = app_with(None);
    app.open_failed("notes.txt", "notes.txt: codec: unrecognized image format");
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ALL[0]))
        .build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    h.run_steps(3);
    assert!(h.query_by_label(&expected("notes.txt")).is_some());
    assert!(h.query_by_label("Cancel").is_none());
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty());
    assert!(h.state().session.documents().is_empty());
}

#[test]
fn other_read_errors_keep_their_reason_and_only_exact_leading_source_prefixes_are_removed() {
    let source = std::path::Path::new("pictures").join("missing.png").to_string_lossy().into_owned();
    for err in [format!("{source}: file not found"), "missing.png: permission denied".into(), "the directory is missing.png: keep this reason".into()] {
        let mut app = app_with(None);
        app.open_failed(&source, &err);
        let reason = if err.starts_with(&source) {
            "file not found"
        } else if err.starts_with("missing.png: ") {
            "permission denied"
        } else {
            &err
        };
        assert_eq!(messages(&app), [format!("Couldn't open missing.png: {reason}")]);
        assert!(app.ui.notices.is_empty());
    }
}

#[test]
fn several_failed_files_each_get_one_alert_and_valid_files_still_open() {
    let files = vec![("a.txt".into(), b"text".to_vec()), ("b.png".into(), png()), ("c.png".into(), b"text".to_vec()), ("d.png".into(), png())];
    let mut app = app_with(Some(FileDialogAnswer::ContentsMany(files)));
    pick(&mut app);
    assert_eq!(messages(&app), [expected("a.txt"), expected("c.png")]);
    assert_eq!(app.session.documents().iter().map(|d| d.doc.name.as_str()).collect::<Vec<_>>(), ["b.png", "d.png"]);
    assert!(app.ui.notices.iter().all(|n| !n.error));
    let first = app.ui.dialogs[0].id;
    crate::dialogs::confirm(&mut app, first).unwrap();
    assert_eq!(messages(&app), [expected("c.png")]);
    let second = app.ui.dialogs[0].id;
    crate::dialogs::confirm(&mut app, second).unwrap();
    assert!(messages(&app).is_empty());
    assert_eq!(app.session.documents().len(), 2);
}

#[test]
fn background_open_failures_are_reported_once_when_the_jobs_finish() {
    let mut app = app_with(None);
    app.background_jobs = true;
    app.open_bytes("first.txt", b"text").unwrap();
    app.open_bytes("second.png", b"text").unwrap();
    app.open_bytes("valid.png", &png()).unwrap();
    let ctx = egui::Context::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !app.jobs.opens.is_empty() {
        assert!(std::time::Instant::now() < deadline, "open jobs did not finish");
        crate::jobs_ui::tick(&mut app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let mut actual = messages(&app).into_iter().map(str::to_owned).collect::<Vec<_>>();
    actual.sort();
    let mut wanted = vec![expected("first.txt"), expected("second.png")];
    wanted.sort();
    assert_eq!(actual, wanted);
    assert_eq!(app.session.documents().iter().map(|d| d.doc.name.as_str()).collect::<Vec<_>>(), ["valid.png"]);
    crate::jobs_ui::tick(&mut app, &ctx);
    assert_eq!(messages(&app).len(), 2, "a consumed failure event cannot show twice");
    assert!(app.ui.notices.iter().all(|n| !n.error));
}

#[test]
fn cancelling_file_open_does_not_show_an_error() {
    let mut app = app_with(None);
    pick(&mut app);
    assert!(!app.file_dialog_open());
    assert!(app.session.documents().is_empty());
    assert!(app.ui.dialogs.is_empty());
    assert!(app.ui.notices.is_empty());
    assert!(!app.ui.status_error);
}

#[test]
fn automation_and_control_open_still_return_the_original_error() {
    let mut app = app_with(None);
    app.services.import = Some(Box::new(|name, bytes, opts| {
        photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| format!("{name}: {e}"))
    }));
    let err = app.open_automation_bytes("notes.txt", b"text").unwrap_err();
    assert_eq!(err, "notes.txt: codec: unrecognized image format");
    assert!(app.ui.dialogs.is_empty());
    app.services.automation_read = Some(Box::new(|_| Ok(crate::AutomationReadResult { name: "notes.txt".into(), bytes: b"text".to_vec(), save_path: None })));
    let (request, _rx) = ControlRequest::new("app.open", json!({"path": "notes.txt"}));
    let Outcome::Done(reply) = handle(&mut app, &egui::Context::default(), &request) else { panic!("open should return immediately") };
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"], err);
    assert!(app.session.documents().is_empty());
    assert!(app.ui.dialogs.is_empty());
    assert!(app.ui.notices.is_empty());
}
