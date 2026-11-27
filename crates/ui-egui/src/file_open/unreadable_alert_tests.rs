//! #14059: unreadable File > Open inputs use one modal alert per file, while callers that
//! request an error result still receive it.

use super::*;
use crate::control::{ControlRequest, Reply, handle};
use crate::{Services, menus};
use craft::host::{Picked, ScriptedFiles};
use craft::ui::dialog::{DialogId, OpenDialog};
use craft::ui::runtime::drag::OsFile;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use std::sync::Arc;

fn app_with(answer: Option<Picked>) -> Box<PhotocraftApp> {
    let services = Services {
        files: Some(Arc::new(ScriptedFiles::answering(vec![answer.unwrap_or(Picked::Cancelled)]))),
        import: Some(Box::new(|name, bytes, opts| {
            photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| format!("{name}: {e}"))
        })),
        ..Default::default()
    };
    PhotocraftApp::new(photocraft_engine::Session::new(), services)
}

fn pick(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    menus::invoke(app, &ctx, "file.open", json!({})).unwrap();
    app.poll_file_dialog();
}

fn error_dialogs(app: &PhotocraftApp) -> Vec<OpenDialog> {
    app.framework.dialogs.open_dialogs().into_iter().filter(|d| d.key.as_deref() == Some("alert")).collect()
}

fn messages(app: &PhotocraftApp) -> Vec<String> {
    error_dialogs(app).into_iter().map(|d| d.fields.unwrap()["message"].as_str().unwrap().to_owned()).collect()
}

fn no_dialogs(app: &PhotocraftApp) -> bool {
    app.framework.dialogs.open_dialogs().is_empty()
}

fn memory_id(app: &PhotocraftApp) -> DialogId {
    crate::dialogs::alerts::prompt_tagged::<crate::open_prompt::MemoryTag>(app).unwrap()
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
        let mut app = app_with(Some(Picked::Contents(vec![(name.into(), bytes)])));
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
    assert!(no_dialogs(&interactive), "the caller owns presentation of a returned error");

    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut app = app_with(Some(Picked::Contents(vec![("unknown.psdt".into(), b"ordinary text".to_vec())])));
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
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app| crate::dialogs::run(app, ui), app);
    h.run_steps(3);
    assert!(h.query_by_label(&expected("notes.txt")).is_some());
    assert!(h.query_by_label("Cancel").is_none());
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(no_dialogs(h.state()));
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
    let mut app = app_with(Some(Picked::Contents(files)));
    pick(&mut app);
    assert_eq!(messages(&app), [expected("a.txt"), expected("c.png")]);
    assert_eq!(app.session.documents().iter().map(|d| d.doc.name.as_str()).collect::<Vec<_>>(), ["b.png", "d.png"]);
    assert!(app.ui.notices.iter().all(|n| n.tone != craft::theme::variant::Tone::Bad));
    let app_ctx = app.ctx().clone();
    let first = error_dialogs(&app)[0].id;
    crate::dialogs::confirm(&mut app, &app_ctx, first).unwrap();
    assert_eq!(messages(&app), [expected("c.png")]);
    let second = error_dialogs(&app)[0].id;
    assert_ne!(first, second, "each failed file has its own instance");
    let (request, _rx) = ControlRequest::new("ui.dialog.confirm", json!({"dialog": second.value()}));
    let Reply::Done(reply) = handle(&mut app, &app_ctx, &request) else { panic!("alert confirmation is immediate") };
    assert_eq!(reply["ok"], true);
    app_ctx.run_ui(Default::default(), |ui| crate::dialogs::run(&mut app, ui)).textures_delta.clear();
    assert!(messages(&app).is_empty());
    assert_eq!(app.session.documents().len(), 2);
    assert!(app.framework.dialogs.take_keyed::<OpenFailureAlert>().is_none(), "the consumer drains both instance-bound acknowledgements");

    // The per-file consumer owns only its wrapper, even though a plain alert also answers ().
    let plain = app.framework.dialogs.open(HostedAlert::new(Alert::new("PhotoCraft", "unrelated alert")));
    assert!(app.framework.dialogs.confirm_by_id(&app_ctx, plain));
    take_open_alerts(&mut app, &app_ctx);
    assert!(matches!(app.framework.dialogs.take::<HostedAlert>(), Some(DialogResult::Ok(()))));
    assert!(app.framework.dialogs.take_keyed::<OpenFailureAlert>().is_none());
}

#[test]
fn background_open_failures_are_reported_once_when_the_jobs_finish() {
    let mut app = app_with(None);
    app.background_jobs = true;
    app.open_bytes("first.txt", b"text").unwrap();
    app.open_bytes("second.png", b"text").unwrap();
    app.open_bytes("valid.png", &png()).unwrap();
    let ctx = app.ctx().clone();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !app.jobs.opens.is_empty() {
        assert!(std::time::Instant::now() < deadline, "open jobs did not finish");
        crate::jobs_ui::tick(&mut app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let mut actual = messages(&app);
    actual.sort();
    let mut wanted = vec![expected("first.txt"), expected("second.png")];
    wanted.sort();
    assert_eq!(actual, wanted);
    assert_eq!(app.session.documents().iter().map(|d| d.doc.name.as_str()).collect::<Vec<_>>(), ["valid.png"]);
    crate::jobs_ui::tick(&mut app, &ctx);
    assert_eq!(messages(&app).len(), 2, "a consumed failure event cannot show twice");
    assert!(app.ui.notices.iter().all(|n| n.tone != craft::theme::variant::Tone::Bad));
}

#[test]
fn cancelling_file_open_does_not_show_an_error() {
    let mut app = app_with(None);
    pick(&mut app);
    assert!(!app.file_dialog_open());
    assert!(app.session.documents().is_empty());
    assert!(no_dialogs(&app));
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
        assert!(no_dialogs(&app));
        let read_name = name.to_string();
        app.services.automation_read =
            Some(Box::new(move |_| Ok(crate::AutomationReadResult { name: read_name.clone(), bytes: b"text".to_vec(), save_path: None })));
        let (request, _rx) = ControlRequest::new("app.open", json!({"path": name}));
        let Reply::Done(reply) = handle(&mut app, &egui::Context::default(), &request) else { panic!("open should return immediately") };
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error"], expected_control_error);
        assert!(app.session.documents().is_empty());
        assert!(no_dialogs(&app));
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

/// A native path plus tempting valid bytes: choosing bytes instead of reading the path would
/// open a document and fail the original missing-file and raw-error assertions below.
#[cfg(not(target_arch = "wasm32"))]
fn unreadable_native_drop(path: std::path::PathBuf) -> OsFile {
    let mut file = OsFile::default();
    file.name = display_name(&path.to_string_lossy());
    file.path = Some(path);
    file.bytes = Some(png().into());
    file
}

/// Apply actual worker events and check the unmodified error sent to a waiting control client.
#[cfg(not(target_arch = "wasm32"))]
fn finish_read_failure(app: &mut PhotocraftApp, source: &str, reason: &str, worker_error: &str) {
    let job = app.jobs.opens.first().map(|tab| tab.job);
    let reply = job.map(|job| {
        let (request, reply) = ControlRequest::new("engine.execute", json!({}));
        let channel = app.control.get_or_insert_with(|| craft::automation::ControlChannel::new(std::sync::mpsc::channel().1));
        channel.settle(request, Reply::AfterJob(job.0), None, 0.0);
        reply
    });
    let ctx = app.ctx().clone();
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
            app.open_dropped(&egui::Context::default(), vec![unreadable_native_drop(path.clone())], crate::file_open::DropTo::Window);
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
    let (path, source, reason) = missing_source("recent", "missing.psd");
    for grid in [false, true] {
        let mut app = app_with(None);
        app.background_jobs = false;
        app.ui.chrome.home_recent_view = if grid { craft::ui::shell::RecentView::Grid } else { craft::ui::shell::RecentView::List };
        app.push_recent(&source);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 760.0)).with_context(app.ctx().clone()).build_ui_state(
            |ui, app: &mut Box<PhotocraftApp>| {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| crate::canvas::document_area(app, ui));
            },
            app,
        );
        h.run_steps(3);
        let label = if grid { "missing.psd".to_owned() } else { format!("missing.psd, {}", path.parent().unwrap().display()) };
        h.get_by_role_and_label(egui::accesskit::Role::ListItem, &label).click();
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
        let mut app = app_with(Some(Picked::Paths(vec![source.clone().into()])));
        app.background_jobs = background;
        app.open_dialog_as_smart_object().unwrap();
        app.poll_file_dialog();
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
    let mut app = app_with(Some(Picked::Contents(vec![("corrupt.psdt".into(), bytes.clone())])));
    pick(&mut app);
    assert_eq!(messages(&app), ["Couldn't open corrupt.psdt: PSD: unsupported file version 0"]);
    assert!(app.session.documents().is_empty());
    assert!(app.ui.recent_files.is_empty());
    assert!(app.ui.notices.is_empty());
    let mut direct = app_with(None);
    // The typed PSD path returns its raw IO error without adding a source-name prefix.
    assert_eq!(direct.open_bytes("corrupt.psdt", &bytes).unwrap_err(), "PSD: unsupported file version 0");
    assert!(no_dialogs(&direct));

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

#[cfg(not(target_arch = "wasm32"))]
fn template_psd_for_memory_prompt() -> Vec<u8> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 4, "height": 3, "background": "white"})).unwrap();
    photocraft_io::export(&s.active().unwrap().doc, "small.psd", &Default::default()).unwrap().bytes
}

#[cfg(not(target_arch = "wasm32"))]
fn memory_dialog_reply(app: &mut PhotocraftApp, dialog: DialogId, accept: bool) -> serde_json::Value {
    let method = if accept { "ui.dialog.confirm" } else { "ui.dialog.cancel" };
    let (request, _rx) = ControlRequest::new(method, json!({"dialog": dialog.value()}));
    let ctx = app.ctx().clone();
    let Reply::Done(reply) = handle(app, &ctx, &request) else { panic!("memory prompt reply is immediate") };
    ctx.run_ui(Default::default(), |ui| crate::dialogs::run(app, ui)).textures_delta.clear();
    reply
}

#[cfg(not(target_arch = "wasm32"))]
fn finish_successful_template_jobs(app: &mut PhotocraftApp) {
    let jobs: Vec<_> = app.jobs.opens.iter().map(|tab| tab.job).collect();
    for job in jobs {
        app.session.wait_job(job).unwrap();
    }
    crate::jobs_ui::tick(app, &egui::Context::default());
}

/// A real picked byte template reaches preflight through both immediate and worker completion.
/// Its original source is metadata only: approval still opens Untitled-1 without a save path.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn byte_template_memory_prompt_keeps_the_source_through_real_open_approve_and_cancel() {
    if cfg!(target_pointer_width = "32") {
        return; // The scoped 3 GiB estimate charge exceeds the 32-bit product cap.
    }
    let gb = 1u64 << 30;
    let psd = template_psd_for_memory_prompt();
    let memory = photocraft_io::memory::MemorySnapshot { available_ram: Some(5 * gb), swap_free: None, commit_headroom: None };
    for background in [false, true] {
        for accept in [false, true] {
            let mut app = app_with(Some(Picked::Contents(vec![("card.psdt".into(), psd.clone())])));
            app.background_jobs = background;
            photocraft_io::psd_budget::with_test_estimate_charge(3 * gb, || {
                photocraft_io::memory::with_mock_memory(memory, || {
                    pick(&mut app);
                    assert_eq!(app.jobs.opens.len(), usize::from(background));
                    if background {
                        finish_successful_template_jobs(&mut app);
                    }
                    assert!(app.session.documents().is_empty());
                    assert_eq!(app.held_memory_prompts.len(), 1, "use the real high-memory preflight");
                    let prompt = &app.held_memory_prompts[0];
                    assert_eq!(prompt.name, "Untitled-1");
                    assert!(prompt.path.is_none());
                    assert_eq!(prompt.error_source.as_deref(), Some("card.psdt"));
                    assert!(prompt.estimated > 3 * gb);
                    let preflight = prompt.id;
                    let dialog = memory_id(&app);
                    assert!(crate::dialogs::alerts::prompt_tag::<crate::open_prompt::MemoryTag>(&app).is_some());
                    let reply = memory_dialog_reply(&mut app, dialog, accept);
                    assert_eq!(reply["ok"], true, "{reply}");
                    if background && accept {
                        let tab = app.jobs.opens.first().unwrap();
                        assert_eq!(tab.name, "Untitled-1");
                        assert!(tab.path.is_none());
                        assert_eq!(tab.error_source.as_deref(), Some("card.psdt"));
                        finish_successful_template_jobs(&mut app);
                    }
                    assert_eq!(app.session.documents().len(), usize::from(accept));
                    if accept {
                        let opened = &app.session.documents()[0];
                        assert_eq!(opened.doc.name, "Untitled-1");
                        assert!(opened.path.is_none(), "error source is never a save destination");
                    }
                    assert!(app.ui.recent_files.is_empty(), "byte input does not acquire a Recent path");
                    assert!(no_dialogs(&app));
                    assert!(app.held_memory_prompts.is_empty());
                    assert!(app.prompt_order.is_empty());
                    assert!(app.jobs.opens.is_empty());
                    assert!(app.jobs.batches.is_empty());
                    assert!(app.session.take_pending_preflight(preflight).is_none());
                    assert_eq!(app.session.pending_preflight_bytes().unwrap(), 0);
                    assert!(!app.ui.status_error);
                });
            });
        }
    }
}

/// A stored byte preflight with a valid identity but invalid PSD version drives the real
/// approval decoder, so both the immediate error and the later worker failure keep raw replies.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn byte_template_memory_approval_decode_failures_name_the_source_and_keep_raw_replies() {
    let header = vec![
        b'8', b'B', b'P', b'S', 0, 0, // unsupported version
        0, 0, 0, 0, 0, 0, // reserved
        0, 3, // channels
        0, 0, 0, 1, // height
        0, 0, 0, 1, // width
        0, 8, // depth
        0, 3, // RGB
    ];
    // The typed UI confirmation is an acknowledgement. The underlying approval result keeps
    // its original raw wrapper, checked independently through the same real stored preflight.
    {
        let mut direct = app_with(None);
        direct.background_jobs = false;
        let source = photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::new(header.clone()));
        let fingerprint = photocraft_engine::jobs::SourceFingerprint::of(&source).unwrap();
        let id = direct
            .session
            .store_pending_preflight(photocraft_engine::jobs::PendingPsdPreflight {
                id: 0,
                name: "Untitled-1".into(),
                source,
                fingerprint,
                required: 1000,
                soft_limit: 500,
                hard_limit: 2000,
                available_ram: Some(4000),
                confirm_damage: true,
            })
            .unwrap();
        crate::open_prompt::arrive_memory(&mut direct, id, "Untitled-1".into(), None, Some("card.psdt".into()), None, None, 1000, Some(4000), false, false);
        let error = crate::open_prompt::answer_memory(&mut direct, true).unwrap_err();
        assert_eq!(error, "Untitled-1: Untitled-1: PSD: unsupported file version 0", "preserve the approval's original wrapper");
    }
    for background in [false, true] {
        let mut app = app_with(None);
        app.background_jobs = background;
        let source = photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::new(header.clone()));
        let fingerprint = photocraft_engine::jobs::SourceFingerprint::of(&source).unwrap();
        let preflight = app
            .session
            .store_pending_preflight(photocraft_engine::jobs::PendingPsdPreflight {
                id: 0,
                name: "Untitled-1".into(),
                source,
                fingerprint,
                required: 1000,
                soft_limit: 500,
                hard_limit: 2000,
                available_ram: Some(4000),
                confirm_damage: true,
            })
            .unwrap();
        crate::open_prompt::arrive_memory(&mut app, preflight, "Untitled-1".into(), None, Some("card.psdt".into()), None, None, 1000, Some(4000), false, false);
        let dialog = memory_id(&app);
        let reply = memory_dialog_reply(&mut app, dialog, true);
        let reason = "PSD: unsupported file version 0";
        if background {
            assert_eq!(reply["ok"], true, "the actual approval starts a worker: {reply}");
            let tab = app.jobs.opens.first().unwrap();
            assert_eq!(tab.name, "Untitled-1");
            assert!(tab.path.is_none());
            assert_eq!(tab.error_source.as_deref(), Some("card.psdt"));
            finish_read_failure(&mut app, "card.psdt", reason, &format!("Untitled-1: {reason}"));
        } else {
            assert_eq!(reply["ok"], true, "typed confirmation acknowledges its receipt; the approval error is presented by the real prompt consumer: {reply}");
            assert_eq!(messages(&app), [format!("Couldn't open card.psdt: {reason}")]);
        }
        assert_eq!(app.ui.status, format!("Couldn't open card.psdt: {reason}"));
        assert_eq!(app.ui.status.matches("card.psdt").count(), 1);
        assert!(!app.ui.status.contains("Untitled-1"));
        assert!(app.ui.status_error);
        assert!(app.ui.notices.is_empty());
        assert!(app.session.documents().is_empty());
        assert!(app.ui.recent_files.is_empty());
        assert!(app.jobs.opens.is_empty());
        assert!(app.held_memory_prompts.is_empty());
        assert!(app.prompt_order.is_empty());
        assert!(app.jobs.batches.is_empty());
        assert_eq!(app.session.pending_preflight_bytes().unwrap(), 0);
        assert!(app.session.take_pending_preflight(preflight).is_none());
        let alert = error_dialogs(&app)[0].id;
        let ctx = app.ctx().clone();
        crate::dialogs::confirm(&mut app, &ctx, alert).unwrap();
        assert!(no_dialogs(&app), "the one error alert closes normally");
    }
}
