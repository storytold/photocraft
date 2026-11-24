//! Import-source protection through the real desktop save writers.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// Original tiny Krita ZIP wire fixture, matching the public reader's 4×4 raw BGRA tile format.
fn import_only_bytes() -> Vec<u8> {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?><DOC xmlns="http://www.calligra.org/DTD/krita" syntaxVersion="2.0"><IMAGE name="img" width="4" height="4" colorspacename="RGBA" x-res="72" y-res="72"><layers><layer nodetype="paintlayer" name="Paint" colorspacename="RGBA" filename="layer0" compositeop="normal" visible="1" opacity="255"/></layers></IMAGE></DOC>"#;
    let mut tile = b"VERSION 2\nTILEWIDTH 4\nTILEHEIGHT 4\nPIXELSIZE 4\nDATA 1\n0,0,LZF,65\n".to_vec();
    tile.push(0);
    tile.extend_from_slice(&[0, 255, 0, 255].repeat(16));
    let mut zip = photocraft_format::zip::ZipWriter::new();
    zip.add("mimetype", b"application/x-krita").unwrap();
    zip.add("maindoc.xml", xml.as_bytes()).unwrap();
    zip.add("img/layers/layer0", &tile).unwrap();
    zip.finish().unwrap()
}

fn imported_app(source: &std::path::Path, bytes: &[u8]) -> Box<PhotocraftApp> {
    let imported = photocraft_io::import(source.to_str().unwrap(), bytes).unwrap();
    assert!(imported.source_read_only);
    let mut app = app();
    app.session.close(0);
    let (_, warning) =
        app.session.admit_import_with_initial_view(imported.document, Some(source.to_str().unwrap().into()), imported.source_read_only, imported.initial_view);
    assert!(warning.is_none());
    let state = app.session.active().unwrap();
    assert!(state.source_read_only);
    assert!(state.path.is_none());
    assert_eq!(state.protected_import_source.as_deref(), source.to_str());
    assert_eq!(photocraft_compose::flatten(&state.doc).get(0, 0), [0., 1., 0., 1.]);
    app
}

#[test]
fn desktop_save_as_and_copy_refuse_import_sources_and_hardlink_aliases_before_encoding() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.pcraft");
    let alias = directory.path().join("alias.pcraft");
    let bytes = import_only_bytes();
    std::fs::write(&source, &bytes).unwrap();
    std::fs::hard_link(&source, &alias).unwrap();
    let mut app = imported_app(&source, &bytes);
    app.background_jobs = false;
    let encoded = Rc::new(std::cell::Cell::new(0));
    let count = encoded.clone();
    app.services.export = Some(Box::new(move |doc, path, _| {
        count.set(count.get() + 1);
        photocraft_io::export(doc, path, &Default::default()).map(|out| (out.bytes, out.warnings)).map_err(|error| error.to_string())
    }));
    app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|error| error.to_string())));
    let before = app.session.active().unwrap().clone();
    for destination in [&source, &alias, &directory.path().join(".").join("source.pcraft")] {
        for copy in [false, true] {
            let error = app.write_document(destination.to_str().unwrap().into(), &ExportSettings::default(), copy).unwrap_err();
            assert!(error.contains("import-only source"), "{error}");
            assert_eq!(std::fs::read(&source).unwrap(), bytes);
            assert_eq!(std::fs::read(&alias).unwrap(), bytes);
            let after = app.session.active().unwrap();
            assert!(Arc::ptr_eq(&before.doc, &after.doc));
            assert_eq!((after.revision, after.saved_revision), (before.revision, before.saved_revision));
            assert_eq!(after.history.past_len(), before.history.past_len());
            assert_eq!(after.path, before.path);
            assert_eq!(after.protected_import_source, before.protected_import_source);
        }
    }
    assert_eq!(encoded.get(), 0);
    let native = directory.path().join("safe.pcraft");
    app.save_as(Some(native.to_str().unwrap().into())).unwrap();
    assert!(encoded.get() > 0);
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    assert_eq!(std::fs::read(&alias).unwrap(), bytes);
    let saved = photocraft_format::load_from_bytes(&std::fs::read(&native).unwrap()).unwrap();
    assert_eq!(photocraft_compose::flatten(&saved).get(0, 0), [0., 1., 0., 1.]);
    assert_eq!(app.session.active().unwrap().path.as_deref(), native.to_str());
    assert_eq!(app.session.active().unwrap().protected_import_source.as_deref(), source.to_str());
    assert!(app.save_as(Some(alias.to_str().unwrap().into())).unwrap_err().contains("import-only source"));
}

#[test]
fn desktop_background_save_refuses_an_import_alias_before_starting_a_worker() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.pcraft");
    let alias = directory.path().join("alias.pcraft");
    let bytes = import_only_bytes();
    std::fs::write(&source, &bytes).unwrap();
    std::fs::hard_link(&source, &alias).unwrap();
    let mut app = imported_app(&source, &bytes);
    app.background_jobs = true;
    let writes = Arc::new(AtomicUsize::new(0));
    let called = writes.clone();
    let saver: crate::SaveFileFn = Arc::new(move |_, path, _, _| {
        called.fetch_add(1, Ordering::Relaxed);
        std::fs::write(path, b"must never replace an import").map_err(|error| error.to_string())?;
        Ok(Vec::new())
    });
    app.services.save_file = Some(saver.clone());
    let before = app.session.active().unwrap().clone();
    assert!(app.write_document(alias.to_str().unwrap().into(), &ExportSettings::default(), false).unwrap_err().contains("import-only source"));
    // Direct callers of the worker entry point must receive the same protection.
    assert!(
        crate::jobs_ui::start_save(&mut app, source.to_str().unwrap().into(), ExportSettings::default(), true, saver, false)
            .unwrap_err()
            .contains("import-only source")
    );
    assert_eq!(writes.load(Ordering::Relaxed), 0);
    assert!(app.jobs.saves.is_empty());
    assert!(app.session.jobs().is_empty());
    assert!(Arc::ptr_eq(&before.doc, &app.session.active().unwrap().doc));
    assert_eq!(identity(&app), (before.doc.name.clone(), before.path, before.revision, before.saved_revision));
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    assert_eq!(std::fs::read(&alias).unwrap(), bytes);
}
