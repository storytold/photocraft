#![allow(clippy::unwrap_used)]

use super::*;
use std::sync::Arc;

struct SourceFolder(PathBuf);

impl SourceFolder {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("photocraft-fresh-headless-{}-{serial}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for SourceFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn fresh_headless_sources_refuse_reused_estimate_and_preserve_existing_document() {
    let folder = SourceFolder::new();
    let bytes = craft_psd::testgen::small(craft_psd::Version::Psd, craft_psd::Compression::Zip).to_bytes().unwrap();
    let source = folder.0.join("fresh.psd");
    std::fs::write(&source, bytes).unwrap();
    let gb = 1u64 << 30;
    let memory = photocraft_io::memory::MemorySnapshot { available_ram: Some(8 * gb), swap_free: None, commit_headroom: None };
    for scoped in [false, true] {
        let mut headless =
            if scoped { Headless::with_workspace(AuthorizedWorkspace::new(Some(&folder.0), Some(&folder.0)).unwrap()) } else { Headless::trusted_local() };
        headless.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        let previous = headless.session.active().unwrap().clone();
        let active = headless.session.active_index();
        let path = if scoped { Path::new("fresh.psd") } else { &source };
        photocraft_io::memory::with_mock_memory(memory, || {
            // The real tiny file is charged into (soft=4 GiB, hard=6 GiB], without
            // allocating gigabytes. This estimate capability belongs to another source.
            photocraft_io::psd_budget::with_test_estimate_charge(4 * gb + 1, || {
                let stale = photocraft_io::ImportOptions { psd_high_memory_approved: true, psd_approved_estimate: Some(u64::MAX), ..Default::default() };
                let error = headless.open_with_options(path, &stale).unwrap_err();
                assert!(
                    matches!(&error, AutomationError::Import(photocraft_io::IoError::PsdApprovalRequired { required, .. }) if *required > 4 * gb),
                    "scoped={scoped}: {error}"
                );
                assert_eq!(headless.session.documents().len(), 1);
                assert_eq!(headless.session.active_index(), active);
                let current = headless.session.active().unwrap();
                assert!(Arc::ptr_eq(&current.doc, &previous.doc));
                assert_eq!(current.path, previous.path);
                assert_eq!(current.source_read_only, previous.source_read_only);
                assert_eq!(
                    (current.revision, current.saved_revision, current.history.past_len()),
                    (previous.revision, previous.saved_revision, previous.history.past_len())
                );
                // Explicit approval for these fresh bytes has no reused estimate and
                // must remain supported on both the path and capability-read routes.
                let fresh = photocraft_io::ImportOptions { psd_high_memory_approved: true, ..Default::default() };
                headless.open_with_options(path, &fresh).unwrap();
                assert_eq!(headless.session.documents().len(), 2);
                assert!(Arc::ptr_eq(&headless.session.documents()[0].doc, &previous.doc));
            });
        });
    }
}
