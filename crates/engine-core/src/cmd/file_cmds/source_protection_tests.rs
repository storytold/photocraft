use super::*;
use crate::color_cmds::ColorExt;
use std::path::Path;

/// An original, tiny Krita wire fixture: one uncompressed 4x4 BGRA tile.
/// Its import-only policy comes from the real reader, regardless of the filename.
#[allow(clippy::unwrap_used, reason = "An error writing the in-memory test fixture must fail the test")]
pub(crate) fn import_only_bytes() -> Vec<u8> {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?><DOC xmlns="http://www.calligra.org/DTD/krita" syntaxVersion="2.0"><IMAGE name="img" width="4" height="4" colorspacename="RGBA" x-res="72" y-res="72"><layers><layer nodetype="paintlayer" name="Paint" colorspacename="RGBA" filename="layer0" compositeop="normal" visible="1" opacity="255"/></layers></IMAGE></DOC>"#;
    let mut tile = b"VERSION 2\nTILEWIDTH 4\nTILEHEIGHT 4\nPIXELSIZE 4\nDATA 1\n0,0,LZF,65\n".to_vec();
    tile.push(0); // Raw interleaved pixels, rather than LZF planes.
    tile.extend_from_slice(&[0, 255, 0, 255].repeat(16));
    let mut zip = photocraft_format::zip::ZipWriter::new();
    zip.add("mimetype", b"application/x-krita").unwrap();
    zip.add("maindoc.xml", xml.as_bytes()).unwrap();
    zip.add("img/layers/layer0", &tile).unwrap();
    zip.finish().unwrap()
}

fn session() -> Session {
    let mut s = crate::session();
    s.color_mut().settings.ask_on_missing = false;
    s.color_mut().settings.ask_on_mismatch = false;
    s
}

#[allow(clippy::unwrap_used, reason = "The test requires the supplied fixture to open and become active")]
fn open_import(path: &Path, bytes: &[u8]) -> Session {
    let mut s = session();
    let path = path.to_str().unwrap();
    open_bytes_as(&mut s, path, bytes, None, Some(path.into())).unwrap();
    let state = s.active().unwrap();
    assert!(state.source_read_only);
    assert!(state.path.is_none());
    assert_eq!(state.protected_import_source.as_deref(), Some(path));
    assert_eq!(photocraft_compose::flatten(&state.doc).get(0, 0), [0., 1., 0., 1.]);
    s
}

#[test]
fn save_a_copy_cannot_replace_import_only_sources_or_aliases_even_with_overwrite_consent() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.pcraft");
    let bytes = import_only_bytes();
    std::fs::write(&source, &bytes).unwrap();
    let alias = directory.path().join("alias.psd");
    std::fs::hard_link(&source, &alias).unwrap();
    let destinations = vec![source.clone(), alias.clone(), directory.path().join(".").join("source.pcraft")];
    #[cfg(unix)]
    let destinations = {
        let link = directory.path().join("symbolic.pcraft");
        std::os::unix::fs::symlink(&source, &link).unwrap();
        let mut destinations = destinations;
        destinations.push(link);
        destinations
    };
    let mut s = open_import(&source, &bytes);
    let before = s.active().unwrap().clone();
    for destination in destinations {
        for overwrite in [false, true] {
            let error = s.execute("file.saveACopy", json!({"path":destination,"overwrite":overwrite})).unwrap_err().to_string();
            assert!(error.contains("import-only source"), "{error}");
            assert_eq!(std::fs::read(&source).unwrap(), bytes);
            assert_eq!(std::fs::read(&alias).unwrap(), bytes);
            let after = s.active().unwrap();
            assert!(Arc::ptr_eq(&before.doc, &after.doc));
            assert_eq!(after.history.past_len(), before.history.past_len());
            assert_eq!((after.revision, after.saved_revision), (before.revision, before.saved_revision));
            assert_eq!(after.path, before.path);
            assert_eq!(after.protected_import_source, before.protected_import_source);
        }
    }
}

#[test]
fn a_different_native_copy_keeps_pixels_and_reopens_without_runtime_source_protection() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.kra");
    let native = directory.path().join("copy.pcraft");
    let bytes = import_only_bytes();
    std::fs::write(&source, &bytes).unwrap();
    let mut s = open_import(&source, &bytes);
    let before = s.active().unwrap().clone();
    s.execute("file.saveACopy", json!({"path":native})).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    assert!(Arc::ptr_eq(&before.doc, &s.active().unwrap().doc));
    assert_eq!(s.active().unwrap().path, None);
    assert_eq!(s.active().unwrap().saved_revision, before.saved_revision);
    assert_eq!(s.active().unwrap().history.past_len(), before.history.past_len());
    // This is the normal successful Save As identity handoff. It must not forget
    // the original import-only file while this session remains open.
    s.saved_to(native.to_str().unwrap().into());
    assert!(check_import_destination(s.active().unwrap(), source.to_str().unwrap()).is_err());
    let native_bytes = std::fs::read(&native).unwrap();
    let mut reopened = session();
    open_bytes_as(&mut reopened, native.to_str().unwrap(), &native_bytes, None, Some(native.to_str().unwrap().into())).unwrap();
    let fresh = reopened.active().unwrap();
    assert!(!fresh.source_read_only);
    assert!(fresh.protected_import_source.is_none());
    assert_eq!(fresh.path.as_deref(), native.to_str());
    assert_eq!(photocraft_compose::flatten(&fresh.doc).get(0, 0), [0., 1., 0., 1.]);
}

#[test]
fn source_origin_survives_template_policy_and_failed_later_admission() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.psdt");
    let bytes = import_only_bytes();
    std::fs::write(&source, &bytes).unwrap();
    let mut s = open_import(&source, &bytes);
    assert!(s.active().unwrap().doc.name.starts_with("Untitled-"));
    let before = s.active().unwrap().clone();
    assert!(open_bytes_as(&mut s, "broken.png", b"not an image", None, Some("unrelated.png".into())).is_err());
    assert_eq!(s.documents().len(), 1);
    assert!(Arc::ptr_eq(&before.doc, &s.active().unwrap().doc));
    assert_eq!(s.active().unwrap().protected_import_source, before.protected_import_source);
    assert!(check_import_destination(s.active().unwrap(), source.to_str().unwrap()).is_err());
}

#[test]
fn byte_only_imports_do_not_infer_a_filesystem_origin_from_the_display_name() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("display-only.pcraft");
    let bytes = import_only_bytes();
    std::fs::write(&source, &bytes).unwrap();
    let mut s = session();
    open_bytes_as(&mut s, source.to_str().unwrap(), &bytes, None, None).unwrap();
    let state = s.active().unwrap();
    assert!(state.source_read_only);
    assert!(state.path.is_none());
    assert!(state.protected_import_source.is_none());
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
}
