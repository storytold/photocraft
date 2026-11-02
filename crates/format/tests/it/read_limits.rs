//! Caller-specific file limits reuse the regular-file reader and preserve the default ceiling.

/// Exact, over-limit, empty, and default reads use tiny regular files rather than large allocations.
#[test]
fn picked_read_limit_preserves_exact_default_and_regular_file_behavior() {
    let dir = super::common::temp_dir("picked-read-limit");
    let path = dir.join("small.atn");
    std::fs::write(&path, b"12345").unwrap();
    assert_eq!(photocraft_format::read::read_file_limited(&path, 5).unwrap(), b"12345");
    assert_eq!(photocraft_format::read::read_file_limited(&path, 4).unwrap_err().kind(), std::io::ErrorKind::FileTooLarge);
    assert_eq!(photocraft_format::read_file(&path).unwrap(), b"12345");
    std::fs::write(&path, b"").unwrap();
    assert!(photocraft_format::read::read_file_limited(&path, 0).unwrap().is_empty());
    assert!(photocraft_format::read::read_file_limited(&dir, 5).is_err());
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(dir).unwrap();
}
