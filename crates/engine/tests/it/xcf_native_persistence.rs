//! Published XCF input, ordinary native edits/copies/reopen, and import-only source safety.
#![cfg(feature = "corpus")]

use std::path::{Path, PathBuf};

use photocraft_engine::{Session, file_cmds::open_bytes_as};
use serde_json::json;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("pc-xcf-persistence-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        for name in ["source.xcf", "source.xcf.gz", "source.xcf.bz2", "source.xcf.xz", "source.xcf.psd", "source.pcraft", "alias.pcraft", "converted.pcraft"] {
            let _ = std::fs::remove_file(self.path(name));
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

#[test]
fn misnamed_and_gzip_inputs_keep_source_protection_and_native_reopen_independence() {
    use photocraft_engine::jobs::{OpenSource, Started};
    use std::io::Write;

    let plain = source();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&plain).unwrap();
    let compressed = encoder.finish().unwrap();
    for name in ["source.pcraft", "source.xcf.gz", "source.xcf.bz2", "source.xcf.xz"] {
        for bytes in [&plain, &compressed] {
            for background in [false, true] {
                let files = Files::new();
                let input = files.path(name);
                let alias = files.path("alias.pcraft");
                let native = files.path("converted.pcraft");
                std::fs::write(&input, bytes).unwrap();
                std::fs::hard_link(&input, &alias).unwrap();
                let mut session = if background {
                    let mut session = Session::new();
                    session.color.settings.ask_on_missing = false;
                    if let Started::Job(id) = session.start_open(name, OpenSource::Path(input.to_string_lossy().into_owned())).unwrap() {
                        session.wait_job(id).unwrap();
                    }
                    session
                } else {
                    open(&input, bytes)
                };
                let state = session.active().unwrap();
                assert!(state.source_read_only && state.path.is_none());
                assert_eq!(state.protected_import_source.as_deref(), input.to_str());
                for destination in [&input, &alias] {
                    assert!(session.execute("file.saveACopy", json!({"path": destination, "overwrite": true})).is_err());
                    assert_eq!(std::fs::read(destination).unwrap(), *bytes);
                }
                session.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
                let pixels = photocraft_compose::flatten(&session.active().unwrap().doc).px;
                let report = session.active().unwrap().doc.metadata.import_conversion.clone();
                session.execute("file.saveACopy", json!({"path": native})).unwrap();
                std::fs::remove_file(&input).unwrap();
                std::fs::remove_file(&alias).unwrap();
                let mut reopened = open(&native, &std::fs::read(&native).unwrap());
                let state = reopened.active().unwrap();
                assert!(!state.source_read_only && state.protected_import_source.is_none());
                assert_eq!(state.doc.metadata.import_conversion, report);
                assert_eq!(photocraft_compose::flatten(&state.doc).px, pixels);
                reopened.execute("layer.setProps", json!({"opacity": 0.75})).unwrap();
                reopened.execute("edit.undo", json!({})).unwrap();
                reopened.execute("edit.redo", json!({})).unwrap();
                assert!(reopened.active().unwrap().is_dirty());
            }
        }
    }
}

#[test]
fn batch_discovers_compound_names_and_same_format_writes_a_supported_copy() {
    let bytes = source();
    for name in ["source.xcf", "source.xcf.gz", "source.xcf.bz2", "source.xcf.xz"] {
        let files = Files::new();
        let input = files.path(name);
        // Content, independently from the misleading wrapper suffix, must still win.
        std::fs::write(&input, &bytes).unwrap();
        assert_eq!(photocraft_engine::file_cmds::list_images(files.0.to_str().unwrap()).unwrap(), vec![input.to_string_lossy().into_owned()]);
        let mut session = Session::new();
        let result = session.execute("file.automate.batch", json!({"input": [input], "output": files.0, "format": "same", "steps": []})).unwrap();
        assert_eq!(result["errors"], json!([]), "{result}");
        let output = result["files"][0].as_str().unwrap();
        assert!(output.ends_with(".psd"), "{output}");
        let saved = std::fs::read(output).unwrap();
        assert!(saved.starts_with(b"8BPS"));
        assert_eq!(photocraft_io::import(output, &saved).unwrap().document.layers.len(), 4);
        assert_eq!(std::fs::read(&input).unwrap(), bytes);
        std::fs::remove_file(output).unwrap();
    }
}

fn source() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/gimp/layers-8bit.xcf");
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}; run cargo xtask corpus --gimp", path.display()))
}

fn open(path: &Path, bytes: &[u8]) -> Session {
    let mut s = Session::new();
    s.color.settings.ask_on_missing = false;
    open_bytes_as(&mut s, path.to_str().unwrap(), bytes, None, Some(path.to_string_lossy().into_owned())).unwrap();
    s
}

#[test]
fn native_work_reopens_and_remains_editable_after_its_source_copy_is_removed() {
    let files = Files::new();
    let bytes = source();
    let input = files.path("source.xcf");
    let native = files.path("converted.pcraft");
    std::fs::write(&input, &bytes).unwrap();
    let mut s = open(&input, &bytes);
    assert!(s.active().unwrap().source_read_only && s.active().unwrap().path.is_none());
    let report = s.active().unwrap().doc.metadata.import_conversion.clone();
    let screen = s.active().unwrap().doc.layers.iter().find(|l| l.name == "Screen off-canvas").unwrap().id;
    s.execute("layer.select", json!({"layer": screen.0})).unwrap();
    s.execute("paint.stroke", json!({"points": [[2, 22], [8, 25]], "size": 3, "color": "#ff0000"})).unwrap();
    let edited = photocraft_compose::flatten(&s.active().unwrap().doc).px;
    s.execute("edit.undo", json!({})).unwrap();
    assert_ne!(photocraft_compose::flatten(&s.active().unwrap().doc).px, edited);
    s.execute("edit.redo", json!({})).unwrap();
    s.execute("file.saveACopy", json!({"path": native})).unwrap();
    assert_eq!(std::fs::read(&input).unwrap(), bytes);
    std::fs::remove_file(&input).unwrap(); // Only this test's owned copy, never the public corpus.
    let mut reopened = open(&native, &std::fs::read(&native).unwrap());
    let state = reopened.active().unwrap();
    assert!(!state.source_read_only && state.path.is_some());
    assert_eq!(state.doc.metadata.import_conversion, report);
    assert_eq!(photocraft_compose::flatten(&state.doc).px, edited);
    reopened.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
    reopened.execute("edit.undo", json!({})).unwrap();
    reopened.execute("edit.redo", json!({})).unwrap();
    assert!(reopened.active().unwrap().is_dirty());
}

#[test]
fn save_a_copy_cannot_replace_an_import_only_source_or_its_existing_alias() {
    let files = Files::new();
    let bytes = source();
    let input = files.path("source.xcf");
    let alias = files.path("alias.pcraft");
    std::fs::write(&input, &bytes).unwrap();
    std::fs::hard_link(&input, &alias).unwrap();
    for background in [false, true] {
        let mut s = if background {
            use photocraft_engine::jobs::{OpenSource, Started};
            let mut s = Session::new();
            s.color.settings.ask_on_missing = false;
            if let Started::Job(id) = s.start_open("source.xcf", OpenSource::Path(input.to_string_lossy().into_owned())).unwrap() {
                s.wait_job(id).unwrap();
            }
            s
        } else {
            open(&input, &bytes)
        };
        assert_eq!(s.active().unwrap().protected_import_source.as_deref(), input.to_str());
        let before = s.active().unwrap().doc.clone();
        for destination in [&input, &alias] {
            assert!(
                s.execute("file.saveACopy", json!({"path": destination, "overwrite": true})).is_err(),
                "import-only source must not be replaced even under a native suffix"
            );
            assert_eq!(std::fs::read(&input).unwrap(), bytes);
            assert_eq!(std::fs::read(&alias).unwrap(), bytes);
            assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
        }
    }
}

#[test]
fn batch_output_cannot_replace_an_import_only_input_through_a_native_suffix_alias() {
    let files = Files::new();
    let bytes = source();
    let input = files.path("source.xcf");
    let alias = files.path("source.pcraft");
    std::fs::write(&input, &bytes).unwrap();
    std::fs::hard_link(&input, &alias).unwrap();
    let mut s = Session::new();
    let result = s.execute("file.automate.batch", json!({"input": [input], "output": files.0, "format": "pcraft", "steps": []})).unwrap();
    assert_eq!(result["files"], json!([]));
    assert!(result["errors"][0]["error"].as_str().unwrap().contains("import-only source"), "{result}");
    assert_eq!(std::fs::read(&input).unwrap(), bytes);
    assert_eq!(std::fs::read(&alias).unwrap(), bytes);
    // A different native destination still produces ordinary editable native data.
    std::fs::remove_file(&alias).unwrap();
    let result = s.execute("file.automate.batch", json!({"input": [input], "output": files.0, "format": "pcraft", "steps": []})).unwrap();
    assert_eq!(result["errors"], json!([]));
    assert_eq!(result["files"].as_array().unwrap().len(), 1);
    let reopened = photocraft_format::load_from_bytes(&std::fs::read(&alias).unwrap()).unwrap();
    assert!(reopened.metadata.import_conversion.is_some());
    assert_eq!(std::fs::read(&input).unwrap(), bytes);
}
