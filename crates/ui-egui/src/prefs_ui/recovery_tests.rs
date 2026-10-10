use super::*;
use crate::{Recoverable, Services};
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use photocraft_engine::Session;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

fn document(name: &str) -> Document {
    Document::with_background(name, Size::new(4, 3), ColorMode::Rgb, SampleType::U16, Color::WHITE)
}

fn recovered(doc: Document) -> crate::RecoveredDocument {
    crate::RecoveredDocument { key: "pending".into(), path: Some("original.psb".into()), document: doc, history: None, context: json!({}) }
}

fn entry(key: &str, doc: Document) -> Recoverable {
    Recoverable { key: key.into(), name: doc.name.clone(), path: Some("original.psb".into()), load: Box::new(move || Ok(recovered(doc))) }
}

type RecoveryLog = Arc<Mutex<Vec<String>>>;

fn services(entries: Vec<Recoverable>) -> (Services, RecoveryLog) {
    let mut entries = Some(entries);
    let log: RecoveryLog = Arc::default();
    let discarded = log.clone();
    (
        Services {
            discover_recovery: Some(Box::new(move || {
                let entries = entries.take().unwrap_or_default();
                Box::new(move || (entries, Vec::new()))
            })),
            discard_autosave: Some(Box::new(move |id, _| {
                discarded.lock().unwrap().push(format!("discard {id}"));
                Ok(())
            })),
            ..Default::default()
        },
        log,
    )
}

// Dropping `release` on an assertion failure also unblocks the worker.
fn gated(doc: Document) -> (Recoverable, mpsc::Receiver<()>, mpsc::Sender<()>, mpsc::Receiver<()>) {
    let (started, start) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let (finished, finish) = mpsc::channel();
    let mut item = entry("pending", doc.clone());
    item.load = Box::new(move || {
        let _ = started.send(());
        wait.recv_timeout(Duration::from_secs(10)).map_err(|e| e.to_string())?;
        let _ = finished.send(());
        Ok(recovered(doc))
    });
    (item, start, release, finish)
}

fn frame(app: &mut PhotocraftApp, ctx: &egui::Context) {
    ctx.run_ui(egui::RawInput::default(), |ui| {
        tick(app, ui.ctx());
        crate::jobs_ui::tick(app, ui.ctx());
    })
    .textures_delta
    .clear();
}

fn settle(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        frame(app, ctx);
        if !app.session.has_jobs() && app.prefs_rt.recovery_queue.is_empty() {
            return;
        }
        assert!(Instant::now() < deadline, "recovery did not finish");
        std::thread::yield_now();
    }
}

#[test]
fn recovery_is_deferred_until_after_app_construction() {
    let calls = Arc::new(AtomicUsize::new(0));
    let called = calls.clone();
    let services = Services {
        discover_recovery: Some(Box::new(move || {
            called.fetch_add(1, Ordering::SeqCst);
            Box::new(|| (Vec::new(), Vec::new()))
        })),
        ..Default::default()
    };
    let app = PhotocraftApp::new(Session::new(), services);
    assert!(app.session.prefs().file_handling.recover_on_launch);
    assert_eq!(calls.load(Ordering::SeqCst), 0, "recovery must not hold up creation of the application window");
}

#[test]
fn recovery_keeps_frames_and_editing_live_and_adopts_the_admitted_document() {
    let mut doc = document("Recovered");
    doc.layers[0].surface_mut().unwrap().write_pixel(1, 2, &[1.0, 0.0, 0.0, 1.0]);
    let original = doc.clone();
    let (pending, started, release, finished) = gated(doc);
    let (services, log) = services(vec![pending]);
    let mut app = PhotocraftApp::new(Session::new(), services);
    let ctx = egui::Context::default();
    assert!(app.session.documents().is_empty());
    frame(&mut app, &ctx);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        frame(&mut app, &ctx);
        if started.try_recv().is_ok() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(app.session.has_jobs());
    assert!(log.lock().unwrap().is_empty());
    // Opening a copy while decoding exercises ID collision handling as well as focus retention.
    app.session.add_document(original.clone(), Some("working.psb".into()));
    let working_id = app.session.active().unwrap().doc.id;
    app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
    for _ in 0..3 {
        frame(&mut app, &ctx);
        assert_eq!(app.session.documents().len(), 1);
    }
    assert_eq!(app.session.active().unwrap().doc.layers[0].surface().unwrap().pixel(1, 2), vec![0.0, 0.0, 1.0, 1.0]);
    release.send(()).unwrap();
    finished.recv_timeout(Duration::from_secs(10)).unwrap();
    settle(&mut app, &ctx);

    assert_eq!(app.session.active().unwrap().doc.id, working_id, "finishing recovery must not steal focus");
    let recovered = app.session.documents().iter().find(|st| st.doc.id != working_id).unwrap();
    assert_eq!(recovered.path.as_deref(), Some("original.psb"));
    assert!(recovered.is_dirty());
    assert_eq!(recovered.doc.layers, original.layers);
    assert_eq!(app.prefs_rt.autosaved.get(&recovered.doc.id).map(|stamp| stamp.revision), Some(recovered.revision));
    assert_eq!(app.prefs_rt.recovery_keys.get(&recovered.doc.id).map(String::as_str), Some("pending"));
}

#[test]
fn cancelling_recovery_skips_the_queue_and_never_adopts_a_late_result() {
    let (pending, started, release, finished) = gated(document("First"));
    let second_calls = Arc::new(AtomicUsize::new(0));
    let calls = second_calls.clone();
    let second = Recoverable {
        key: "second".into(),
        name: "Second".into(),
        path: None,
        load: Box::new(move || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(recovered(document("Second")))
        }),
    };
    let (services, log) = services(vec![pending, second]);
    let mut app = PhotocraftApp::new(Session::new(), services);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        frame(&mut app, &ctx);
        if started.try_recv().is_ok() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let job = app.session.jobs()[0].id;
    crate::jobs_ui::cancel(&mut app, job);
    frame(&mut app, &ctx);
    assert!(!app.session.has_jobs());
    assert!(app.prefs_rt.recovery_queue.is_empty());
    release.send(()).unwrap();
    finished.recv_timeout(Duration::from_secs(10)).unwrap();
    for _ in 0..3 {
        frame(&mut app, &ctx);
    }
    assert!(app.session.documents().is_empty());
    assert!(log.lock().unwrap().is_empty(), "cancelled recovery must not adopt or discard saved entries");
    assert_eq!(second_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn failed_recovery_reports_the_error_and_continues_to_the_next_entry() {
    let broken = Recoverable { key: "broken".into(), name: "Broken".into(), path: None, load: Box::new(|| Err("corrupt tile".into())) };
    let (services, log) = services(vec![broken, entry("good", document("Good"))]);
    let mut app = PhotocraftApp::new(Session::new(), services);
    let ctx = egui::Context::default();
    settle(&mut app, &ctx);
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.ui.chrome.home.is_none(), "the first recovered document should be visible");
    assert!(app.ui.notices.iter().any(|n| n.error && (n.title.contains("corrupt tile") || n.lines.iter().any(|l| l.contains("corrupt tile")))));
    assert_eq!(app.prefs_rt.recovery_keys.get(&app.session.active().unwrap().doc.id).map(String::as_str), Some("good"));
    assert!(log.lock().unwrap().is_empty());
}

#[test]
fn disabled_recovery_never_discovers_or_decodes_entries() {
    let calls = Arc::new(AtomicUsize::new(0));
    let called = calls.clone();
    let mut session = Session::new();
    session.execute("prefs.set", json!({"path": "fileHandling.recoverOnLaunch", "value": false})).unwrap();
    let mut app = PhotocraftApp::new(
        session,
        Services {
            discover_recovery: Some(Box::new(move || {
                called.fetch_add(1, Ordering::SeqCst);
                Box::new(|| (vec![entry("disabled", document("Disabled"))], Vec::new()))
            })),
            ..Default::default()
        },
    );
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&mut app, &ctx);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(!app.session.has_jobs());
    assert!(app.session.documents().is_empty());
}

#[test]
fn delayed_admission_event_never_acknowledges_later_edit_or_history_purge() {
    for purge in [false, true] {
        let queued = Arc::new(Mutex::new(Vec::new()));
        let saves = queued.clone();
        let services = Services {
            autosave: Some(Box::new(move |_, _, revision, _, _, _| {
                saves.lock().unwrap().push(revision);
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(Session::new(), services);
        app.session.add_document(document("Recovered"), None);
        app.session.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
        app.sync_views();
        let st = app.session.active().unwrap();
        let id = st.doc.id;
        let captured = CheckpointStamp::of(st, json!({}));
        let revision = st.revision;
        if purge {
            app.session.active_mut().unwrap().history.clear();
        } else {
            app.session.execute("edit.fill", json!({"color":"#0000ff"})).unwrap();
        }
        finish_recovery(&mut app, "durable", &json!({"documentId":id.0,"revision":revision,"admittedStamp":captured,"context":{}}));
        assert!(!app.prefs_rt.autosaved.contains_key(&id));
        assert!(app.prefs_rt.catch_up.contains(&id));
        autosave_now(&mut app);
        tick(&mut app, &egui::Context::default());
        assert!(!queued.lock().unwrap().is_empty(), "changed state requires a new durable checkpoint");
    }
}

#[test]
fn descriptor_discovery_runs_on_worker_and_frames_remain_live() {
    let ui_thread = std::thread::current().id();
    let (started, start) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let mut task = Some(Box::new(move || {
        assert_ne!(std::thread::current().id(), ui_thread);
        started.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(10)).unwrap();
        (Vec::new(), Vec::new())
    }) as crate::RecoveryDiscovery);
    let services = Services { discover_recovery: Some(Box::new(move || task.take().unwrap())), ..Default::default() };
    let mut app = PhotocraftApp::new(Session::new(), services);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    start.recv_timeout(Duration::from_secs(10)).unwrap();
    for _ in 0..3 {
        frame(&mut app, &ctx);
        assert!(app.session.has_jobs());
    }
    release.send(()).unwrap();
    settle(&mut app, &ctx);
}
