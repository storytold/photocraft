use super::*;
use std::sync::Arc;

#[test]
fn fresh_automation_bytes_refuse_reused_estimate_and_preserve_existing_document() {
    let bytes = craft_psd::testgen::small(craft_psd::Version::Psd, craft_psd::Compression::Zip).to_bytes().unwrap();
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        crate::Services {
            // Exercise the normal host decoder, not a fabricated approval error.
            import: Some(Box::new(|name, bytes, options| photocraft_io::import_with_options(name, bytes, options).map_err(|error| error.to_string()))),
            ..Default::default()
        },
    );
    app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
    let previous = app.session.active().unwrap().clone();
    let active = app.session.active_index();
    let events = app.session.prefs().script_events.enabled;
    let gb = 1u64 << 30;
    let memory = photocraft_io::memory::MemorySnapshot { available_ram: Some(8 * gb), swap_free: None, commit_headroom: None };
    photocraft_io::memory::with_mock_memory(memory, || {
        // Only the estimate grows: the real imported PSD stays tiny.
        photocraft_io::psd_budget::with_test_estimate_charge(4 * gb + 1, || {
            let stale = photocraft_io::ImportOptions { psd_high_memory_approved: true, psd_approved_estimate: Some(u64::MAX), ..Default::default() };
            let error = app.open_automation_bytes_with_options("fresh.psd", &bytes, &stale).unwrap_err();
            assert!(error.contains("approval"), "the fresh file must request approval: {error}");
            assert_eq!(app.session.documents().len(), 1);
            assert_eq!(app.session.active_index(), active);
            assert_eq!(app.session.prefs().script_events.enabled, events);
            let current = app.session.active().unwrap();
            assert!(Arc::ptr_eq(&current.doc, &previous.doc));
            assert_eq!(current.path, previous.path);
            assert_eq!(current.source_read_only, previous.source_read_only);
            assert_eq!(
                (current.revision, current.saved_revision, current.history.past_len()),
                (previous.revision, previous.saved_revision, previous.history.past_len())
            );
            // Fresh explicit consent remains valid when no saved estimate is reused.
            let fresh = photocraft_io::ImportOptions { psd_high_memory_approved: true, ..Default::default() };
            app.open_automation_bytes_with_options("fresh.psd", &bytes, &fresh).unwrap();
            assert_eq!(app.session.documents().len(), 2);
            assert!(Arc::ptr_eq(&app.session.documents()[0].doc, &previous.doc));
            assert_eq!(app.session.prefs().script_events.enabled, events);
        });
    });
}
