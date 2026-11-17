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
    for (name, bytes) in [
        ("notes.txt", b"ordinary text".to_vec()),
        ("broken.png", b"ordinary text".to_vec()),
        ("zz-bad.png", (1..=17).collect()),
        ("unknown.psd", b"ordinary text".to_vec()),
        ("unknown.psb", b"ordinary text".to_vec()),
        ("unknown.psdt", b"ordinary text".to_vec()),
    ] {
        let mut app = app_with(Some(FileDialogAnswer::Contents(name.into(), bytes)));
        pick(&mut app);
        let wanted = expected(name);
        assert_eq!(messages(&app), [wanted.as_str()]);
        assert!(app.session.documents().is_empty());
        assert!(app.ui.recent_files.is_empty());
        assert!(app.ui.notices.is_empty(), "the modal replaces the corner notice");
        assert!(app.ui.status_error);
        assert_eq!(app.ui.status, wanted);
        assert_eq!(app.ui.status.matches(name).count(), 1);
    }
    // Interactive foreground errors now retain the source name instead of a generated Untitled-N.
    let mut interactive = app_with(None);
    assert_eq!(
        interactive.open_bytes("unknown.psdt", b"ordinary text").unwrap_err(),
        "unknown.psdt: PSD: invalid signature [111, 114, 100, 105], expected 8BPS"
    );
    assert!(interactive.ui.dialogs.is_empty(), "the caller owns presentation of a returned error");

    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut app = app_with(Some(FileDialogAnswer::Contents("unknown.psdt".into(), b"ordinary text".to_vec())));
        app.background_jobs = true;
        pick(&mut app);
        assert_eq!(app.jobs.opens.len(), 1);
        let tab = &app.jobs.opens[0];
        assert_eq!(tab.name, "Untitled-1");
        assert!(tab.path.is_none());
        assert_eq!(tab.error_source.as_deref(), Some("unknown.psdt"));
        let worker_error = format!("{}: codec: unrecognized image format", tab.name);
        finish_read_failure(&mut app, "unknown.psdt", "codec: unrecognized image format", &worker_error);
        assert!(app.ui.recent_files.is_empty());
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
    for (err, reason) in [
        (format!("{source}: file not found"), "file not found"),
        ("missing.png: permission denied".into(), "permission denied"),
        (format!("missing.png: {source}: file not found"), "file not found"),
        ("missing.png: elsewhere/missing.png: keep this reason".into(), "elsewhere/missing.png: keep this reason"),
        ("the directory is missing.png: keep this reason".into(), "the directory is missing.png: keep this reason"),
    ] {
        let mut app = app_with(None);
        app.open_failed(&source, &err);
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
    // app.open reserves the template's untitled name before its separate automation decode.
    for (name, expected_error, expected_control_error) in [
        ("notes.txt", "notes.txt: codec: unrecognized image format", "notes.txt: codec: unrecognized image format"),
        ("unknown.psdt", "unknown.psdt: PSD: invalid signature [116, 101, 120, 116], expected 8BPS", "Untitled-1: codec: unrecognized image format"),
    ] {
        let mut app = app_with(None);
        let err = app.open_automation_bytes(name, b"text").unwrap_err();
        assert_eq!(err, expected_error);
        assert!(app.ui.dialogs.is_empty());
        let read_name = name.to_string();
        app.services.automation_read =
            Some(Box::new(move |_| Ok(crate::AutomationReadResult { name: read_name.clone(), bytes: b"text".to_vec(), save_path: None })));
        let (request, _rx) = ControlRequest::new("app.open", json!({"path": name}));
        let Outcome::Done(reply) = handle(&mut app, &egui::Context::default(), &request) else { panic!("open should return immediately") };
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error"], expected_control_error);
        assert!(app.session.documents().is_empty());
        assert!(app.ui.dialogs.is_empty());
        assert!(app.ui.notices.is_empty());
    }
}

/// This path must remain absent: both the foreground and worker open really fail to read it.
#[cfg(not(target_arch = "wasm32"))]
fn missing_source(tag: &str, name: &str) -> (std::path::PathBuf, String, String) {
    let path = std::env::temp_dir().join(format!("photocraft-unreadable-{tag}-{}", std::process::id())).join(name);
    assert!(!path.exists());
    let reason = photocraft_format::read_file(&path).unwrap_err().to_string();
    let source = path.to_string_lossy().into_owned();
    (path, source, reason)
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
struct UnreadableNativeDrop(std::path::PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl egui::DroppedFile for UnreadableNativeDrop {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        panic!("a native absolute-path drop must read from disk")
    }
}

/// Apply actual worker events and check the unmodified error sent to a waiting control client.
#[cfg(not(target_arch = "wasm32"))]
fn finish_read_failure(app: &mut PhotocraftApp, source: &str, reason: &str, worker_error: &str) {
    let job = app.jobs.opens.first().map(|tab| tab.job);
    let reply = job.map(|job| {
        let (tx, rx) = std::sync::mpsc::channel();
        app.jobs.waiters.push((job, tx));
        rx
    });
    let ctx = egui::Context::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !app.jobs.opens.is_empty() {
        assert!(std::time::Instant::now() < deadline, "open job did not finish");
        crate::jobs_ui::tick(app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    if let Some(reply) = reply {
        assert_eq!(reply.try_recv().unwrap(), json!({"ok": false, "error": worker_error}));
    }
    let name = display_name(source);
    let wanted = if reason == "codec: unrecognized image format" { expected(&name) } else { format!("Couldn't open {name}: {reason}") };
    assert_eq!(messages(app), [wanted]);
    assert_eq!(app.ui.status, messages(app)[0]);
    assert_eq!(app.ui.status.matches(name.as_str()).count(), 1);
    assert!(app.session.documents().is_empty());
    assert!(app.ui.notices.is_empty());
    crate::jobs_ui::tick(app, &ctx);
    assert_eq!(messages(app).len(), 1, "the finished event is consumed once");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_drop_read_failures_name_the_source_once_in_foreground_and_background() {
    for name in ["missing.psd", "missing.psdt"] {
        let (path, source, reason) = missing_source("drop", name);
        for background in [false, true] {
            let mut app = app_with(None);
            app.background_jobs = background;
            app.open_dropped(&egui::Context::default(), vec![std::sync::Arc::new(UnreadableNativeDrop(path.clone()))], None);
            assert_eq!(app.jobs.opens.len(), usize::from(background), "exercise the actual selected execution route");
            let worker_name = app.jobs.opens.first().map(|tab| tab.name.as_str()).unwrap_or(name);
            if background {
                assert_eq!(worker_name, if name == "missing.psdt" { "Untitled-1" } else { name });
            }
            let worker_error = format!("{worker_name}: {source}: {reason}");
            finish_read_failure(&mut app, &source, &reason, &worker_error);
            assert!(app.ui.recent_files.is_empty());
        }
    }

    // The public path-plus-bytes API retains the source while the actual template worker is untitled.
    let (_, source, _) = missing_source("template-codec", "unknown.psdt");
    let mut app = app_with(None);
    app.background_jobs = true;
    app.open_file(&source, b"ordinary text").unwrap();
    assert_eq!(app.jobs.opens.len(), 1);
    let tab = &app.jobs.opens[0];
    assert_eq!(tab.name, "Untitled-1");
    assert_eq!(tab.path.as_deref(), Some(source.as_str()));
    let worker_error = format!("{}: codec: unrecognized image format", tab.name);
    finish_read_failure(&mut app, &source, "codec: unrecognized image format", &worker_error);
    assert!(app.ui.recent_files.is_empty());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn recent_grid_and_list_read_failures_keep_the_reason_without_repeating_the_path() {
    let (_, source, reason) = missing_source("recent", "missing.psd");
    for grid in [false, true] {
        let mut app = app_with(None);
        app.background_jobs = false;
        app.ui.chrome.home_recent_grid = grid;
        app.push_recent(&source);
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 760.0))
            .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ALL[0]))
            .build_ui_state(
                |ui, app: &mut PhotocraftApp| {
                    let recent = app.ui.recent_files.clone();
                    crate::home_recent_ui::show_home_recent(app, ui, &recent, 600.0);
                },
                app,
            );
        h.run_steps(3);
        h.get_by_label("missing.psd").click();
        h.run_steps(3);
        finish_read_failure(h.state_mut(), &source, &reason, "no worker in this foreground control");
        assert_eq!(h.state().ui.recent_files.as_slice(), std::slice::from_ref(&source));
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn os_open_read_failures_keep_the_existing_full_source_route() {
    let (_, source, reason) = missing_source("os", "missing.psd");
    for background in [false, true] {
        let mut app = app_with(None);
        app.background_jobs = background;
        let mut events = vec![OsEvent::Open(vec![source.clone()])];
        app.services.os_events = Some(Box::new(move || std::mem::take(&mut events)));
        app.drain_os_events(&egui::Context::default());
        assert_eq!(app.jobs.opens.len(), usize::from(background), "exercise the actual selected execution route");
        let worker_error = format!("{}: {source}: {reason}", display_name(&source));
        finish_read_failure(&mut app, &source, &reason, &worker_error);
        assert!(app.ui.recent_files.is_empty());
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn smart_object_open_read_failures_retain_the_read_source_without_a_save_or_recent_path() {
    let (_, source, reason) = missing_source("smart-object", "missing.psd");
    for background in [false, true] {
        let mut app = app_with(Some(FileDialogAnswer::Paths(vec![source.clone()])));
        app.background_jobs = background;
        app.open_dialog_as_smart_object().unwrap();
        app.poll_file_dialog(&egui::Context::default(), None);
        assert_eq!(app.jobs.opens.len(), usize::from(background), "exercise the actual selected execution route");
        assert!(app.jobs.opens.iter().all(|tab| tab.path.is_none()), "a Smart Object remains untitled");
        assert!(app.jobs.opens.iter().all(|tab| tab.error_source.as_deref() == Some(source.as_str())));
        finish_read_failure(&mut app, &source, &reason, &format!("{source}: {reason}"));
        assert!(app.ui.recent_files.is_empty());
    }
}

#[test]
fn recognized_psd_header_failures_keep_their_diagnostic_and_raw_error() {
    // The file signature is recognized: a bad version is a PSD error, not a codec miss.
    let bytes = b"8BPS\0\0".to_vec();
    let mut app = app_with(Some(FileDialogAnswer::Contents("corrupt.psdt".into(), bytes.clone())));
    pick(&mut app);
    assert_eq!(messages(&app), ["Couldn't open corrupt.psdt: PSD: unsupported file version 0"]);
    assert!(app.session.documents().is_empty());
    assert!(app.ui.recent_files.is_empty());
    assert!(app.ui.notices.is_empty());
    let mut direct = app_with(None);
    assert_eq!(direct.open_bytes("corrupt.psdt", &bytes).unwrap_err(), "corrupt.psdt: PSD: unsupported file version 0");
    assert!(direct.ui.dialogs.is_empty());

    // A damaged internal block or an unrelated/malformed diagnostic retains its reason.
    for reason in [
        "PSD: invalid signature [0, 0, 0, 0], expected 8BIM (layer blend mode)",
        "PSD: invalid signature [56, 66, 80, 83], expected 8BPS",
        "PSD: invalid signature [0, 0, 0], expected 8BPS",
        "PSD: invalid signature [256, 0, 0, 0], expected 8BPS",
        "PSD: invalid signature [+1, 0, 0, 0], expected 8BPS",
        "PSD: invalid signature [00, 0, 0, 0], expected 8BPS",
        "PSD: invalid signature [0, 0, 0, 0], expected 8BPS; keep this detail",
        "embedded PSD: invalid signature [0, 0, 0, 0], expected 8BPS",
    ] {
        let mut app = app_with(None);
        app.open_failed("corrupt.psdt", reason);
        assert_eq!(messages(&app), [format!("Couldn't open corrupt.psdt: {reason}")]);
    }
}
