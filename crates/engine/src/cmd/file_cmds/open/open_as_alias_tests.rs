use super::*;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::jobs::historical_xcf_fixture as fixture;

struct SourceFile(PathBuf);

impl SourceFile {
    fn new() -> std::io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-open-as-xcf-alias-{}-{}.bin", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
        let source = Self(path);
        file.write_all(&fixture::historical_pixel(false))?;
        Ok(source)
    }
}

impl Drop for SourceFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn compressed_xcf_open_as_aliases_accept_leading_dot_and_uppercase() {
    let source = SourceFile::new().unwrap();
    // Plain XCF is intentionally valid for every forced XCF alias. Transport
    // framing is independently tested by IO; this exercises actual command
    // validation, file reading and forced XCF dispatch without extra codecs.
    for alias in ["xcf.bz2", "xcfbz2", "xcf.xz", "xcfxz"] {
        for normalized in [alias.to_string(), format!(".{alias}"), alias.to_ascii_uppercase(), format!(".{}", alias.to_ascii_uppercase())] {
            for key in ["as", "format"] {
                let mut params = json!({"path":source.0,"historicalXcfSampleOrder":"big-endian"});
                params[key] = json!(normalized);
                let mut session = Session::new();
                session.execute("file.openAs", params).unwrap();
                assert_eq!(session.documents().len(), 1);
                let state = session.active().unwrap();
                assert!(state.source_read_only);
                assert!(state.path.is_none());
                assert_eq!(state.doc.depth, photocraft_color::SampleType::U16);
                let pixel = state.doc.layers[0].surface().unwrap().pixel(0, 0);
                let expected = [0x1234 as f32 / 65535., 0x4567 as f32 / 65535., 0x89ab as f32 / 65535., 1.];
                assert_eq!(pixel, expected, "{key}={normalized}");
            }
        }
    }
    assert_eq!(std::fs::read(&source.0).unwrap(), fixture::historical_pixel(false), "Open As must not replace its source");
}

#[test]
fn open_as_compressed_xcf_aliases_reach_file_read() {
    let source = SourceFile::new().unwrap();
    let missing = source.0.with_extension("missing");
    assert!(!missing.exists());
    for alias in ["xcf.bz2", "xcfbz2", "xcf.xz", "xcfxz"] {
        let mut session = Session::new();
        let error = session.execute("file.openAs", json!({"path":missing,"as":alias})).unwrap_err();
        assert!(matches!(&error, EngineError::Other(message) if message.contains(missing.to_str().unwrap())), "{alias}: {error}");
        assert!(session.documents().is_empty());
    }
}

#[test]
fn open_as_existing_formats_and_unknown_aliases_keep_their_validation_policy() {
    let source = SourceFile::new().unwrap();
    let missing = source.0.with_extension("missing");
    assert!(!missing.exists());
    for alias in ["xcf", ".XCF.GZ", "xcfgz", "png"] {
        let mut session = Session::new();
        let error = session.execute("file.openAs", json!({"path":missing,"as":alias})).unwrap_err();
        assert!(matches!(&error, EngineError::Other(message) if message.contains(missing.to_str().unwrap())), "{alias}: {error}");
        assert!(session.documents().is_empty());
    }
    for alias in ["xcf.zz", "xcf.bz", "xcf.xzzz", "not-a-format"] {
        let mut session = Session::new();
        let error = session.execute("file.openAs", json!({"path":missing,"as":alias})).unwrap_err();
        assert!(matches!(&error, EngineError::BadParams { cmd, msg } if cmd == "file.openAs" && msg.contains(alias)), "{alias}: {error}");
        assert!(session.documents().is_empty());
    }
}
