//! A save that won't fit in memory is refused before any encoding starts, with a message that
//! says how to free memory; the document stays open and unsaved (#10024). Its own test binary:
//! the memory simulation is process-wide.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Mutex;

use photocraft_engine::Session;
use photocraft_raster::headroom;
use photocraft_ui_egui::{PhotocraftApp, Services};
use serde_json::json;

/// The simulation is process-wide: the tests take turns.
static LOCK: Mutex<()> = Mutex::new(());

fn app(writes: Rc<RefCell<u32>>) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(
        Session::new(),
        Services {
            export: Some(Box::new(|doc, path, _| {
                photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
            })),
            write: Some(Box::new(move |_, _| {
                *writes.borrow_mut() += 1;
                Ok(())
            })),
            ..Default::default()
        },
    );
    app.run("file.new", json!({"width": 8, "height": 8, "name": "Untitled-1"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app
}

fn identity(app: &PhotocraftApp) -> (String, Option<String>, u64, u64) {
    let st = app.session.active().unwrap();
    (st.doc.name.clone(), st.path.clone(), st.revision, st.saved_revision)
}

#[test]
fn a_save_that_does_not_fit_fails_and_leaves_the_document_unsaved() {
    let _turn = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let writes = Rc::new(RefCell::new(0));
    let mut app = app(writes.clone());
    let before = identity(&app);
    headroom::simulate_available(Some(1 << 20));
    let refused = app.save_as(Some("big.psd".into()));
    let refused_automation = app.save_automation(Some("auto.psd".into()));
    headroom::simulate_available(None);
    let err = refused.unwrap_err();
    assert!(err.contains("Not enough memory to save \u{201c}big.psd\u{201d}"), "{err}");
    assert!(err.contains("Edit \u{203a} Purge \u{203a} All"), "{err}");
    let automation_err = refused_automation.unwrap_err();
    assert!(automation_err.contains("Not enough memory to save \u{201c}auto.psd\u{201d}"), "{automation_err}");
    assert_eq!(*writes.borrow(), 0, "nothing was encoded or written");
    assert_eq!(identity(&app), before, "the document is unsaved and unchanged");
    app.save_as(Some("big.psd".into())).unwrap();
    assert_eq!(*writes.borrow(), 1);
}

fn warnings(app: &PhotocraftApp) -> usize {
    app.ui.notices.iter().filter(|n| n.title == photocraft_ui_egui::memory_warning::TITLE).count()
}

/// Going low posts one notice that stays; hovering low doesn't post again; recovering past the
/// higher threshold takes it down and arms the next warning.
#[test]
fn the_low_memory_warning_posts_once_and_rearms_with_hysteresis() {
    let _turn = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    use photocraft_ui_egui::memory_warning::{REARM_ABOVE, WARN_BELOW, tick};
    let mut app = app(Rc::new(RefCell::new(0)));
    let mut now = 0.0;
    let mut look = |app: &mut PhotocraftApp, available: Option<u64>| {
        headroom::simulate_available(available);
        now += 2.5;
        tick(app, now);
    };
    look(&mut app, None);
    assert_eq!(warnings(&app), 0, "plenty of memory (and always so on the desktop)");
    look(&mut app, Some(WARN_BELOW - 1));
    assert_eq!(warnings(&app), 1);
    assert!(app.ui.notices.iter().all(|n| n.sticky), "stays until dismissed");
    look(&mut app, Some(WARN_BELOW - 1));
    look(&mut app, Some(WARN_BELOW + 1));
    assert_eq!(warnings(&app), 1, "no second notice while it stays low");
    look(&mut app, Some(REARM_ABOVE));
    assert_eq!(warnings(&app), 0, "recovered: the stale notice is taken down");
    look(&mut app, Some(WARN_BELOW - 1));
    assert_eq!(warnings(&app), 1, "low again: warned again");
    // Checks are two seconds apart at most.
    headroom::simulate_available(None);
    app.ui.notices.clear();
    headroom::simulate_available(Some(0));
    tick(&mut app, now + 0.5);
    assert_eq!(warnings(&app), 0);
    headroom::simulate_available(None);
}

#[test]
fn no_warning_without_a_document() {
    let _turn = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut app = PhotocraftApp::new(Session::new(), Services::default());
    headroom::simulate_available(Some(0));
    photocraft_ui_egui::memory_warning::tick(&mut app, 100.0);
    headroom::simulate_available(None);
    assert_eq!(warnings(&app), 0);
}

/// Export As on the save worker checks the document it writes, made there at the export's size,
/// not only the one it was made from: an enlarged export that won't fit is refused before it is
/// encoded, as the inline export refuses it.
#[test]
fn a_background_export_that_does_not_fit_at_its_size_is_refused_before_encoding() {
    use photocraft_ui_egui::{FileDialogAnswer, export_dialog, jobs_ui};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    let _turn = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let encoded = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&encoded);
    let mut app = PhotocraftApp::new(
        Session::new(),
        Services {
            save_file: Some(Arc::new(move |_, _, _, _| {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(Vec::new())
            })),
            file_dialog: Some(Box::new(|_, _, reply| reply.send(Some(FileDialogAnswer::SaveTo("/tmp/photocraft-test/big.png".into()))))),
            ..Default::default()
        },
    );
    app.background_jobs = true;
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let ctx = egui::Context::default();
    // The 100 × 100 document fits, and so does resizing it to 1000 × 1000 (about 4 MB); encoding
    // that as a PNG (its layer and a composite, about 8 MB) doesn't.
    headroom::simulate_available(Some(headroom::RESERVE + (6 << 20)));
    let settings = json!({"format": "png", "scale": 1000});
    let asked = export_dialog::confirm(&mut app, settings.as_object().unwrap());
    let t = std::time::Instant::now();
    if asked.is_ok() {
        app.poll_file_dialog(&ctx, None);
        while t.elapsed() < std::time::Duration::from_secs(30)
            && !app.ui.notices.iter().any(|n| n.lines.iter().chain([&n.title]).any(|l| l.contains("Save failed")))
            && encoded.load(Ordering::SeqCst) == 0
        {
            jobs_ui::tick(&mut app, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    headroom::simulate_available(None);
    asked.unwrap();
    assert_eq!(encoded.load(Ordering::SeqCst), 0, "not encoded");
    let said: Vec<String> = app.ui.notices.iter().flat_map(|n| n.lines.iter().chain([&n.title]).cloned()).collect();
    assert!(said.iter().any(|l| l.contains("Not enough memory to save \u{201c}big.png\u{201d}")), "{said:?}");
}
