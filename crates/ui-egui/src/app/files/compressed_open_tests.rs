use super::*;
use std::cell::Cell;
use std::rc::Rc;

#[test]
fn wrapped_inline_opens_register_real_jobs_before_attempting_header_decode() {
    for signature in [&[0x1f, 0x8b][..], &b"BZh"[..], &b"\xfd7zXZ\0"[..]] {
        // Deliberately truncated real transport signatures: a synchronous probe
        // would reject these before a pending tab could be registered. The real
        // engine/IO worker must report that failure only after startup returns.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = false;
        let order = jobs_ui::OpenOrder { batch: 7, seq: 2 };
        let warnings = app.open_bytes_placed("renamed.bin", signature, Some("/source/renamed.bin"), Some(3), Some(order)).unwrap();
        assert!(warnings.is_empty());
        assert!(!app.background_jobs, "routing does not change the user's general inline preference");
        assert!(app.session.documents().is_empty());
        assert_eq!(app.jobs.opens.len(), 1, "recognized wrapper must become an actual pending open");
        let tab = &app.jobs.opens[0];
        assert_eq!(tab.name, "renamed.bin");
        assert_eq!(tab.path.as_deref(), Some("/source/renamed.bin"));
        assert_eq!(tab.slot, Some(3));
        assert_eq!(tab.order, Some(order));
        let job = tab.job;
        assert!(app.session.jobs().iter().any(|info| info.id == job));
        assert!(app.historical_opens.is_empty());
        assert!(app.session.wait_job(job).is_err(), "the worker still enforces malformed transport framing");
        assert!(app.session.documents().is_empty());
    }
}

#[test]
fn unwrapped_ordinary_open_keeps_the_inline_import_service() {
    let calls = Rc::new(Cell::new(0));
    let observed = Rc::clone(&calls);
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        crate::Services {
            import: Some(Box::new(move |name, bytes, _options| {
                assert_eq!(name, "ordinary.bin");
                assert_eq!(bytes, b"unwrapped ordinary source");
                observed.set(observed.get() + 1);
                Ok(photocraft_io::ImportResult {
                    document: photocraft_doc::Document::new(
                        "Inline document",
                        photocraft_geom::Size::new(1, 1),
                        photocraft_color::ColorMode::Rgb,
                        photocraft_color::SampleType::U8,
                    ),
                    warnings: Vec::new(),
                    initial_view: None,
                    source_read_only: false,
                    preview_only: false,
                })
            })),
            ..Default::default()
        },
    );
    app.background_jobs = false;
    app.open_bytes("ordinary.bin", b"unwrapped ordinary source").unwrap();
    assert_eq!(calls.get(), 1);
    assert!(app.jobs.opens.is_empty());
    assert_eq!(app.session.documents().len(), 1);
}
