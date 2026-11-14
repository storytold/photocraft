use std::{fs, path::Path};

use serde_json::{Value, json};

use crate::{EngineError, Session};

const COMMAND: &str = "file.scripts.imageProcessor";
const NO_SOURCE: &str = "There were no source files that could be opened by PhotoCraft.";

fn settings(input: &Path, output: &Path, per_format: bool) -> Value {
    let mut params = json!({"input": input, "output": output});
    if per_format {
        params["jpeg"] = json!({});
        params["tiff"] = json!({});
    }
    params
}

fn write_image(path: &Path) -> std::io::Result<()> {
    fs::write(path, b"P6\n2 1\n255\n\xff\0\0\0\xff\0")
}

#[test]
fn blank_empty_non_image_and_corrupt_sources_report_no_openable_files() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    let output = root.path().join("output");
    fs::create_dir(&input).unwrap();
    let mut session = Session::new();

    for per_format in [false, true] {
        assert_eq!(session.execute(COMMAND, settings(Path::new(""), &output, per_format)), Err(EngineError::Other(NO_SOURCE.into())));
    }

    for file in [None, Some("notes.txt"), Some("broken.jpg")] {
        if let Some(file) = file {
            fs::write(input.join(file), b"not an image").unwrap();
        }
        for per_format in [false, true] {
            assert_eq!(session.execute(COMMAND, settings(&input, &output, per_format)), Err(EngineError::Other(NO_SOURCE.into())));
        }
        assert!(!output.exists());
    }
}

#[test]
fn image_processor_skips_non_images_and_can_find_images_in_subfolders() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    let nested = input.join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(input.join("notes.txt"), b"not an image").unwrap();
    fs::write(nested.join("manifest.txt"), b"not an image").unwrap();
    write_image(&nested.join("test.ppm")).unwrap();
    let mut session = Session::new();

    for per_format in [false, true] {
        let output = root.path().join(if per_format { "multi" } else { "single" });
        let mut params = settings(&input, &output, per_format);
        assert_eq!(session.execute(COMMAND, params.clone()), Err(EngineError::Other(NO_SOURCE.into())));
        params["includeSubfolders"] = json!(true);
        let result = session.execute(COMMAND, params).unwrap();
        let mut expected = vec![output.join("nested/test.jpg")];
        if per_format {
            expected.push(output.join("nested/test.tif"));
        }
        assert_eq!(result, json!({"files": expected, "errors": []}));
        for path in expected {
            let image = photocraft_io::import(&path.to_string_lossy(), &fs::read(&path).unwrap()).unwrap();
            assert_eq!((image.document.size.width, image.document.size.height), (2, 1));
        }
    }
}

#[test]
fn an_opened_image_can_still_report_a_processing_or_write_failure() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    let output = root.path().join("output");
    fs::create_dir(&input).unwrap();
    write_image(&input.join("test.ppm")).unwrap();
    fs::write(&output, b"not a directory").unwrap();
    let mut session = Session::new();
    for per_format in [false, true] {
        let result = session.execute(COMMAND, settings(&input, &output, per_format)).unwrap();
        assert_eq!(result["files"], json!([]));
        let errors = result["errors"].as_array().unwrap();
        assert_eq!(errors.len(), if per_format { 2 } else { 1 });
        assert!(errors.iter().all(|error| error["file"] == json!(input.join("test.ppm"))));
    }
    assert_eq!(fs::read(&output).unwrap(), b"not a directory");
}

#[test]
fn missing_paths_and_missing_open_documents_keep_their_own_errors() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing");
    let output = root.path().join("output");
    let mut session = Session::new();
    for per_format in [false, true] {
        let params = settings(&missing, &output, per_format);
        let error = session.execute(COMMAND, params.clone()).unwrap_err().to_string();
        assert!(error.contains(&missing.to_string_lossy().to_string()), "{error}");
        assert_ne!(error, NO_SOURCE);
        for source in ["current", "open"] {
            let mut params = params.clone();
            params["source"] = json!(source);
            assert_eq!(session.execute(COMMAND, params), Err(EngineError::NoDocument));
        }
    }
}

#[test]
fn mixed_valid_and_corrupt_images_keep_the_corrupt_file_report() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    fs::create_dir(&input).unwrap();
    let corrupt = input.join("broken.jpg");
    fs::write(&corrupt, b"not an image").unwrap();
    write_image(&input.join("test.ppm")).unwrap();
    let mut session = Session::new();
    for per_format in [false, true] {
        let output = root.path().join(if per_format { "multi" } else { "single" });
        let result = session.execute(COMMAND, settings(&input, &output, per_format)).unwrap();
        let mut expected = vec![output.join("test.jpg")];
        if per_format {
            expected.push(output.join("test.tif"));
        }
        assert_eq!(result["files"], json!(expected));
        let errors = result["errors"].as_array().unwrap();
        assert_eq!(errors.len(), if per_format { 2 } else { 1 });
        assert!(errors.iter().all(|error| error["file"] == json!(corrupt)));
    }
}

#[test]
fn empty_explicit_inputs_error_and_invalid_recursion_keeps_its_diagnostic() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    let mut session = Session::new();
    for per_format in [false, true] {
        let mut params = settings(Path::new(""), &output, per_format);
        params["includeSubfolders"] = json!("true");
        let error = session.execute(COMMAND, params.clone()).unwrap_err();
        assert!(matches!(error, EngineError::BadParams { msg, .. } if msg.contains("includeSubfolders")));
        params.as_object_mut().unwrap().remove("input");
        params["includeSubfolders"] = json!(false);
        params["files"] = json!([]);
        assert_eq!(session.execute(COMMAND, params), Err(EngineError::Other(NO_SOURCE.into())));
    }
    assert!(!output.exists());
}
