//! Public engine workflows over a tiny original XCF fixture, without a corpus dependency.
use photocraft_engine::{
    Session,
    color_cmds::ColorExt,
    file_cmds::open_bytes_as,
    jobs::{OpenSource, Started},
};
use serde_json::json;
use std::{path::Path, sync::Arc};

fn source_xcf() -> Vec<u8> {
    /// Append a big-endian v3 fixture field without relying on the importer or a corpus file.
    fn word(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_be_bytes());
    }
    /// Resolve a reserved 32-bit fixture pointer to its independently written section.
    fn patch(out: &mut [u8], at: usize, value: usize) {
        out[at..at + 4].copy_from_slice(&(value as u32).to_be_bytes());
    }
    let mut out = b"gimp xcf v003\0".to_vec();
    for value in [1, 1, 0, 17, 1] {
        word(&mut out, value);
    }
    out.push(0); // Image compression property: raw tiles.
    word(&mut out, 0);
    word(&mut out, 0);
    let layer_pointer = out.len();
    for _ in 0..3 {
        word(&mut out, 0); // Layer pointer, end of layers, end of channels.
    }
    let layer = out.len();
    for value in [1, 1, 1, 6] {
        word(&mut out, value);
    }
    out.extend_from_slice(b"Pixel\0");
    // Independently modified text pixels remain authoritative while the small
    // original configuration supplies real retained report evidence.
    let text = br#"(text "Saved pixels")"#;
    let mut parasite = Vec::new();
    word(&mut parasite, 16);
    parasite.extend_from_slice(b"gimp-text-layer\0");
    word(&mut parasite, 0);
    word(&mut parasite, text.len() as u32);
    parasite.extend_from_slice(text);
    word(&mut out, 21);
    word(&mut out, parasite.len() as u32);
    out.extend_from_slice(&parasite);
    word(&mut out, 26);
    word(&mut out, 4);
    word(&mut out, 2);
    word(&mut out, 0);
    word(&mut out, 0);
    let hierarchy_pointer = out.len();
    word(&mut out, 0);
    word(&mut out, 0);
    let hierarchy = out.len();
    for value in [1, 1, 4] {
        word(&mut out, value);
    }
    let level_pointer = out.len();
    word(&mut out, 0);
    word(&mut out, 0);
    let level = out.len();
    word(&mut out, 1);
    word(&mut out, 1);
    let tile_pointer = out.len();
    word(&mut out, 0);
    word(&mut out, 0);
    let tile = out.len();
    out.extend_from_slice(&[128, 64, 32, 255]);
    patch(&mut out, layer_pointer, layer);
    patch(&mut out, hierarchy_pointer, hierarchy);
    patch(&mut out, level_pointer, level);
    patch(&mut out, tile_pointer, tile);
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

/// Wrap a small original fixture in one stored DEFLATE block with independent gzip length and CRC fields.
fn stored_gzip(bytes: &[u8]) -> Vec<u8> {
    assert!(bytes.len() <= u16::MAX as usize);
    let len = bytes.len() as u16;
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255, 1];
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(bytes);
    out.extend_from_slice(&crc32(bytes).to_le_bytes());
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out
}

fn session() -> Session {
    let mut session = Session::new();
    session.color_mut().settings.ask_on_missing = false;
    session.color_mut().settings.ask_on_mismatch = false;
    session
}

fn open(path: &Path, bytes: &[u8], background: bool) -> Session {
    let mut session = session();
    if background {
        match session.start_open(path.file_name().unwrap().to_str().unwrap(), OpenSource::Path(path.to_str().unwrap().into())).unwrap() {
            Started::Job(job) => {
                session.wait_job(job).unwrap();
            }
            Started::Done(_) => panic!("native Open worker expected"),
        }
    } else {
        open_bytes_as(&mut session, path.to_str().unwrap(), bytes, None, Some(path.to_str().unwrap().into())).unwrap();
    }
    session
}

#[test]
fn misnamed_plain_and_gzip_xcf_keep_source_protection_and_native_reopen_independence() {
    let plain = source_xcf();
    for bytes in [plain.clone(), stored_gzip(&plain)] {
        for background in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let input = directory.path().join("misnamed.pcraft");
            let alias = directory.path().join("alias.psd");
            let native = directory.path().join("saved-native.pcraft");
            std::fs::write(&input, &bytes).unwrap();
            std::fs::hard_link(&input, &alias).unwrap();
            let mut session = open(&input, &bytes, background);
            let state = session.active().unwrap();
            assert!(state.source_read_only && state.path.is_none());
            assert_eq!(state.protected_import_source.as_deref(), input.to_str());
            assert!(state.doc.metadata.import_conversion.is_some());
            let original = state.doc.clone();
            for destination in [&input, &alias] {
                assert!(session.execute("file.saveACopy", json!({"path":destination,"overwrite":true})).is_err());
                assert!(Arc::ptr_eq(&session.active().unwrap().doc, &original));
                assert_eq!(std::fs::read(destination).unwrap(), bytes);
            }
            session.execute("layer.setProps", json!({"opacity":0.5})).unwrap();
            let rendered = photocraft_compose::flatten(&session.active().unwrap().doc);
            let report = session.active().unwrap().doc.metadata.import_conversion.clone();
            session.execute("file.saveACopy", json!({"path":native})).unwrap();
            session.saved_to(native.to_str().unwrap().into());
            let state = session.active().unwrap();
            assert!(state.source_read_only, "the import origin stays protected after the writable save path is adopted");
            assert_eq!(state.path.as_deref(), native.to_str());
            assert_eq!(state.protected_import_source.as_deref(), input.to_str(), "Save As keeps the source guard for this session");
            assert!(photocraft_engine::file_cmds::check_import_destination(state, input.to_str().unwrap()).is_err());
            assert_eq!(std::fs::read(&input).unwrap(), bytes);
            std::fs::remove_file(&input).unwrap();
            std::fs::remove_file(&alias).unwrap();
            let mut reopened = open(&native, &std::fs::read(&native).unwrap(), false);
            let state = reopened.active().unwrap();
            assert!(!state.source_read_only && state.protected_import_source.is_none());
            assert_eq!(state.path.as_deref(), native.to_str());
            assert_eq!(state.doc.metadata.import_conversion, report);
            assert_eq!(photocraft_compose::flatten(&state.doc).px, rendered.px);
            reopened.execute("layer.setProps", json!({"opacity":0.75})).unwrap();
            reopened.execute("edit.undo", json!({})).unwrap();
            assert_eq!(photocraft_compose::flatten(&reopened.active().unwrap().doc).px, rendered.px);
            reopened.execute("edit.redo", json!({})).unwrap();
            assert!(reopened.active().unwrap().is_dirty());
        }
    }
}

#[test]
fn cancelled_and_failed_xcf_open_preserve_the_existing_document_and_source() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.xcf");
    for cancel in [false, true] {
        let bytes = if cancel { source_xcf() } else { b"gimp xcf v003\0broken".to_vec() };
        std::fs::write(&input, &bytes).unwrap();
        let mut session = session();
        session.execute("file.new", json!({"width":2,"height":2,"background":"#00ff00"})).unwrap();
        session.execute("layer.setProps", json!({"opacity":0.75})).unwrap();
        let before = session.active().unwrap().clone();
        let Started::Job(job) = session.start_open("source.xcf", OpenSource::Path(input.to_str().unwrap().into())).unwrap() else {
            panic!("native Open worker")
        };
        if cancel {
            assert!(session.cancel_job(job));
            session.join_cancelled_jobs();
        }
        let error = session.wait_job(job).unwrap_err();
        if cancel {
            assert!(matches!(error, photocraft_engine::EngineError::Cancelled));
        }
        assert_eq!(session.documents().len(), 1);
        let state = session.active().unwrap();
        assert!(Arc::ptr_eq(&state.doc, &before.doc));
        assert_eq!((state.revision, state.saved_revision, state.history.past_len()), (before.revision, before.saved_revision, before.history.past_len()));
        assert_eq!(std::fs::read(&input).unwrap(), bytes);
    }
}

#[test]
fn cancelled_or_stale_xcf_raster_place_never_changes_the_accepted_target() {
    for cancel in [false, true] {
        let mut session = session();
        session.execute("file.new", json!({"width":4,"height":4})).unwrap();
        let bytes = source_xcf();
        let Started::Job(job) = session.start_place_bytes("source.xcf", bytes.clone(), None, json!({"smartObject":false})).unwrap() else {
            panic!("native Place worker")
        };
        if cancel {
            assert!(session.cancel_job(job));
            session.join_cancelled_jobs();
        } else {
            session
                .edit("Concurrent native edit", |doc, _| {
                    doc.name = "accepted newer state".into();
                    Ok(())
                })
                .unwrap();
        }
        let before = session.active().unwrap().clone();
        let error = session.wait_job(job).unwrap_err();
        if cancel {
            assert!(matches!(error, photocraft_engine::EngineError::Cancelled));
        } else {
            assert!(error.to_string().contains("document changed"), "{error}");
        }
        let state = session.active().unwrap();
        assert!(Arc::ptr_eq(&state.doc, &before.doc));
        assert_eq!((state.revision, state.history.past_len()), (before.revision, before.history.past_len()));
        assert_eq!(source_xcf(), bytes);
    }
}
