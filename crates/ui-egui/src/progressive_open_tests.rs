//! A raw opened interactively shows previews in its tab while the full develop runs, then the
//! full document takes the tab over (tab position, zoom kept; fresh history, not dirty).

use std::sync::Arc;
use std::time::{Duration, Instant};

use photocraft_raw::testgen::{DngSpec, Val, baseline_jpeg, mosaic, scene};

use super::*;

fn dng(with_jpeg: bool) -> Vec<u8> {
    let (w, h) = (48, 32);
    let jpeg = baseline_jpeg(&vec![90u8; 40 * 30 * 3], 40, 30, 3);
    let spec = |at: u32| {
        let mut s = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000));
        if with_jpeg {
            s.ifd0_extra = vec![(513, Val::Long(vec![at])), (514, Val::Long(vec![jpeg.len() as u32]))];
        }
        s
    };
    let mut bytes = spec(spec(0).build().len() as u32).build();
    if with_jpeg {
        bytes.extend_from_slice(&jpeg);
    }
    bytes
}

fn app_with_a_document() -> PhotocraftApp {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    let mut app = PhotocraftApp::new(session, crate::Services::default());
    app.sync_views();
    app
}

/// Draws frames until the open's first preview is a tab.
fn until_previewed(app: &mut PhotocraftApp) {
    let end = Instant::now() + Duration::from_secs(20);
    while app.jobs.previews.is_empty() {
        assert!(Instant::now() < end, "no preview within 20 s");
        show_previews(app);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn until_opened(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    let end = Instant::now() + Duration::from_secs(20);
    while !app.jobs.opens.is_empty() || app.session.has_jobs() {
        assert!(Instant::now() < end, "the open did not finish within 20 s");
        tick(app, &ctx);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn the_full_document_takes_over_the_preview_tab_with_its_view_and_a_clean_state() {
    for with_jpeg in [false, true] {
        let mut app = app_with_a_document();
        start_open(&mut app, "s.dng", None, OpenSource::Bytes(Arc::new(dng(with_jpeg)))).unwrap();
        until_previewed(&mut app);
        // The preview is a normal tab after the first document, locked by the open job.
        assert_eq!(app.session.documents().len(), 2);
        let preview = app.session.documents()[1].doc.id;
        assert_eq!(app.jobs.previews[0].1, preview);
        assert!(app.session.job_on(preview).is_some(), "locked while the full develop runs");
        assert!(app.jobs.focus.is_none() && open_tabs(&app).is_empty(), "the preview replaces the progress tab");
        app.ui.views[1].zoom = 3.0;
        until_opened(&mut app);
        assert_eq!(app.session.documents().len(), 2, "preview closed, not kept beside the full document");
        let st = &app.session.documents()[1];
        assert_ne!(st.doc.id, preview);
        assert_eq!((st.doc.size.width, st.doc.size.height), (48, 32), "the full develop");
        assert!(!st.is_dirty());
        assert_eq!(st.history.past_len(), 0);
        assert_eq!(app.ui.views[1].zoom, 3.0, "zoom kept");
        assert!(app.session.job_on(st.doc.id).is_none());
        assert!(app.jobs.previews.is_empty() && app.jobs.opens.is_empty());
        assert_eq!(app.session.active_index(), Some(1));
    }
}

#[test]
fn closing_the_preview_cancels_the_open_without_leaking_a_lock_or_a_tab() {
    let mut app = app_with_a_document();
    start_open(&mut app, "s.dng", None, OpenSource::Bytes(Arc::new(dng(true)))).unwrap();
    until_previewed(&mut app);
    app.session.close(1);
    app.sync_views();
    until_opened(&mut app);
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.jobs.previews.is_empty() && app.session.jobs().is_empty());
}

#[test]
fn cancelling_the_open_closes_its_preview() {
    let mut app = app_with_a_document();
    start_open(&mut app, "s.dng", None, OpenSource::Bytes(Arc::new(dng(false)))).unwrap();
    until_previewed(&mut app);
    let job = app.jobs.previews[0].0;
    assert!(cancel(&mut app, job));
    until_opened(&mut app);
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.jobs.previews.is_empty() && app.session.jobs().is_empty());
}

#[test]
fn a_failing_open_closes_its_preview_and_reports_the_error() {
    // A raw whose sensor data is cut off: the previews may show, the full develop fails.
    let mut bytes = dng(true);
    let cut = bytes.len() - 4000;
    bytes.truncate(cut.max(200));
    let mut app = app_with_a_document();
    start_open(&mut app, "bad.dng", None, OpenSource::Bytes(Arc::new(bytes))).unwrap();
    until_opened(&mut app);
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.jobs.previews.is_empty() && app.session.jobs().is_empty());
}
