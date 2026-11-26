use super::*;
use crate::{HistoricalXcfSourceCapture, import_file_for_open_verified_with_historical_source};
use photocraft_raster::Interrupt;

struct Directory(std::path::PathBuf);

impl Directory {
    #[allow(clippy::unwrap_used, reason = "the test fixture requires a temporary directory")]
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("photocraft-historical-source-{}-{id}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn require() -> ImportOptions {
    ImportOptions { historical_xcf_order: HistoricalXcfOrder::RequireExplicit, ..Default::default() }
}

#[test]
fn retained_verified_path_retries_original_encoded_bytes_after_path_replacement_once() {
    let directory = Directory::new();
    let path = directory.path().join("source.xcf.gz");
    let original = gzip(&fixture(11, SampleOrder::LittleEndian, false));
    std::fs::write(&path, &original).unwrap();
    let expected_identity = crate::FileIdentity::of_path(&path).unwrap();
    let opts = require();
    let mut identity = None;
    let mut capture = HistoricalXcfSourceCapture::new(1 << 20);
    let error =
        import_file_for_open_verified_with_historical_source("source.xcf.gz", &path, None, &opts, &Interrupt::NONE, &mut identity, &mut capture).unwrap_err();
    assert!(matches!(error, IoError::HistoricalXcfSampleOrderRequired(crate::HistoricalXcfSampleOrderRequest { version: 11, bits_per_sample: 16 })));
    assert_eq!(identity, Some(expected_identity));
    let retained = capture.take().unwrap();
    assert_eq!(retained.as_slice(), original);
    assert!(capture.take().is_none(), "the path handoff is single-consume");
    // The source path now names a different inode and different byte order.
    // Confirmation uses the admitted Arc, not a second lookup of that path.
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, gzip(&fixture(11, SampleOrder::BigEndian, false))).unwrap();
    let retried = crate::import_with_options("source.xcf.gz", &retained, &options(SampleOrder::LittleEndian)).unwrap();
    assert_eq!(first_pixel(&retried.document), vec![0x1234 as f32 / 65535., 0xabcd as f32 / 65535., 42. / 65535., 1.]);
    let reopened = crate::import_with_options("source.xcf.gz", &std::fs::read(&path).unwrap(), &options(SampleOrder::LittleEndian)).unwrap();
    assert_ne!(first_pixel(&reopened.document), first_pixel(&retried.document), "the replacement would decode different pixels");
    assert_eq!(opts, require(), "the caller's captured options remain unchanged");
}

#[test]
fn historical_capture_refuses_capacity_cancellation_stale_identity_and_bad_wrapper_without_publishing() {
    let directory = Directory::new();
    let path = directory.path().join("source.xcf");
    let original = fixture(11, SampleOrder::LittleEndian, false);
    std::fs::write(&path, &original).unwrap();
    let mut identity = None;
    let mut capture = HistoricalXcfSourceCapture::new(0);
    let error =
        import_file_for_open_verified_with_historical_source("source.xcf", &path, None, &require(), &Interrupt::NONE, &mut identity, &mut capture).unwrap_err();
    assert!(matches!(error, IoError::OutOfMemory(_)), "{error}");
    assert!(capture.take().is_none());
    let stale = crate::FileIdentity::of_path(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, gzip(&original)).unwrap();
    let mut capture = HistoricalXcfSourceCapture::new(1 << 20);
    let error =
        import_file_for_open_verified_with_historical_source("source.xcf", &path, Some(&stale), &require(), &Interrupt::NONE, &mut identity, &mut capture)
            .unwrap_err();
    assert!(matches!(error, IoError::SourceChanged), "{error}");
    assert!(capture.take().is_none());
    let cancel = || true;
    let error = import_file_for_open_verified_with_historical_source(
        "source.xcf",
        &path,
        None,
        &require(),
        &Interrupt::cancel_only(&cancel),
        &mut identity,
        &mut capture,
    )
    .unwrap_err();
    assert!(matches!(error, IoError::Cancelled), "{error}");
    assert!(capture.take().is_none());
    let mut damaged = gzip(&original);
    let crc = damaged.len() - 8;
    damaged[crc] ^= 1;
    std::fs::write(&path, damaged).unwrap();
    let error =
        import_file_for_open_verified_with_historical_source("source.xcf", &path, None, &require(), &Interrupt::NONE, &mut identity, &mut capture).unwrap_err();
    assert!(matches!(error, IoError::Xcf(_)), "full wrapper integrity precedes the question: {error}");
    assert!(capture.take().is_none());
}

#[test]
fn historical_capture_rechecks_the_opened_handle_before_publishing() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let directory = Directory::new();
    let path = directory.path().join("source.xcf");
    let original = fixture(11, SampleOrder::LittleEndian, false);
    std::fs::write(&path, &original).unwrap();
    let rewritten = AtomicBool::new(false);
    let progress = |_fraction: f32| {};
    let polls = std::sync::atomic::AtomicUsize::new(0);
    let check = || {
        // First poll is before opening; the next poll is import_for_open_with,
        // after its caller has read the original source from that same handle.
        if polls.fetch_add(1, Ordering::Relaxed) == 1 {
            let mut changed = original.clone();
            changed.push(0);
            std::fs::write(&path, changed).unwrap();
            rewritten.store(true, Ordering::Relaxed);
        }
        false
    };
    let mut identity = None;
    let mut capture = HistoricalXcfSourceCapture::new(1 << 20);
    let result = import_file_for_open_verified_with_historical_source(
        "source.xcf",
        &path,
        None,
        &require(),
        &Interrupt::new(&check, &progress),
        &mut identity,
        &mut capture,
    );
    assert!(rewritten.load(Ordering::Relaxed));
    assert!(matches!(result, Err(IoError::SourceChanged)), "{result:?}");
    assert!(capture.take().is_none());
}
