#![allow(clippy::unwrap_used)]

use super::*;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

fn wrapped(corrupt_tail: bool) -> Vec<u8> {
    let mut plain = b"gimp xcf v007\0".to_vec();
    for word in [1u32, 1, 0, 250, 0, 0, 0, 0] {
        plain.extend_from_slice(&word.to_be_bytes());
    }
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&plain).unwrap();
    let mut bytes = encoder.finish().unwrap();
    if corrupt_tail {
        let crc = bytes.len() - 8;
        bytes[crc] ^= 1;
    }
    bytes
}

struct Source {
    directory: std::path::PathBuf,
    path: std::path::PathBuf,
}
impl Source {
    fn new(name: &str, bytes: &[u8]) -> Self {
        let directory = std::env::temp_dir().join(format!("photocraft-historical-capture-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("source.xcf.gz");
        std::fs::write(&path, bytes).unwrap();
        Self { directory, path }
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.directory);
    }
}

#[test]
fn verified_historical_capture_probes_header_before_corrupt_tail_and_retains_exact_encoded_bytes() {
    for corrupt in [false, true] {
        let bytes = wrapped(corrupt);
        let source = Source::new(if corrupt { "corrupt" } else { "valid" }, &bytes);
        let mut identity = None;
        let mut captured = None;
        let error = import_file_for_open_verified_with_historical_source(
            "source.xcf.gz",
            &source.path,
            None,
            &ImportOptions::default(),
            &photocraft_raster::Interrupt::NONE,
            &mut identity,
            &mut captured,
        )
        .unwrap_err();
        assert!(matches!(error, IoError::HistoricalXcfSampleOrderRequired(_)));
        assert_eq!(identity, Some(FileIdentity::of_path(&source.path).unwrap()));
        let captured = captured.unwrap();
        assert_eq!(captured.as_slice(), bytes);
        let options = ImportOptions { historical_xcf_sample_order: Some(HistoricalXcfSampleOrder::BigEndian), ..Default::default() };
        let resumed = import_for_open_with("source.xcf.gz", &captured, &options, &photocraft_raster::Interrupt::NONE);
        if corrupt {
            assert!(resumed.is_err(), "the explicit-order import must validate the stream checksum");
            assert!(!matches!(resumed, Err(IoError::HistoricalXcfSampleOrderRequired(_))));
            let mut legacy_identity = None;
            assert!(
                !matches!(
                    import_file_for_open_verified(
                        "source.xcf.gz",
                        &source.path,
                        None,
                        &ImportOptions::default(),
                        &photocraft_raster::Interrupt::NONE,
                        &mut legacy_identity
                    ),
                    Err(IoError::HistoricalXcfSampleOrderRequired(_))
                ),
                "legacy noninteractive import still validates the whole wrapper"
            );
        } else {
            let (opened, damaged) = resumed.unwrap();
            assert_eq!(opened.document.size, photocraft_geom::Size::new(1, 1));
            assert!(!damaged);
        }
    }
}

#[test]
fn historical_capture_rechecks_opened_identity_before_publishing_encoded_source() {
    let source = Source::new("changed", &wrapped(false));
    let calls = AtomicUsize::new(0);
    let cancel = || {
        // The first check precedes opening; the next occurs during the prefix probe,
        // after the verified handle has read its encoded bytes.
        if calls.fetch_add(1, Ordering::SeqCst) == 1 {
            let mut file = std::fs::OpenOptions::new().append(true).open(&source.path).unwrap();
            file.write_all(&[0]).unwrap();
        }
        false
    };
    let mut identity = None;
    let mut captured = None;
    let result = import_file_for_open_verified_with_historical_source(
        "source.xcf.gz",
        &source.path,
        None,
        &ImportOptions::default(),
        &photocraft_raster::Interrupt::cancel_only(&cancel),
        &mut identity,
        &mut captured,
    );
    assert!(matches!(result, Err(IoError::SourceChanged)));
    assert!(captured.is_none());
    assert!(identity.is_some());
}

#[test]
fn cancelled_historical_capture_does_not_publish_encoded_source() {
    let source = Source::new("cancelled", &wrapped(false));
    let calls = AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, Ordering::SeqCst) > 0;
    let mut identity = None;
    let mut captured = Some(Arc::new(vec![1]));
    let result = import_file_for_open_verified_with_historical_source(
        "source.xcf.gz",
        &source.path,
        None,
        &ImportOptions::default(),
        &photocraft_raster::Interrupt::cancel_only(&cancel),
        &mut identity,
        &mut captured,
    );
    assert!(matches!(result, Err(IoError::Cancelled)));
    assert!(captured.is_none());
}
