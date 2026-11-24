use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};

use crate::EngineError;

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
    let mut session = crate::session();

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
    let mut session = crate::session();

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
        let files: Vec<PathBuf> = serde_json::from_value(result["files"].clone()).unwrap();
        assert_eq!(files, expected);
        assert_eq!(result["errors"], json!([]));
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
    let mut session = crate::session();
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
    let mut session = crate::session();
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
    let mut session = crate::session();
    for per_format in [false, true] {
        let output = root.path().join(if per_format { "multi" } else { "single" });
        let result = session.execute(COMMAND, settings(&input, &output, per_format)).unwrap();
        let mut expected = vec![output.join("test.jpg")];
        if per_format {
            expected.push(output.join("test.tif"));
        }
        let files: Vec<PathBuf> = serde_json::from_value(result["files"].clone()).unwrap();
        assert_eq!(files, expected);
        let errors = result["errors"].as_array().unwrap();
        assert_eq!(errors.len(), if per_format { 2 } else { 1 });
        assert!(errors.iter().all(|error| error["file"] == json!(corrupt)));
    }
}

#[test]
fn empty_explicit_inputs_error_and_invalid_recursion_keeps_its_diagnostic() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    let mut session = crate::session();
    for per_format in [false, true] {
        let mut params = settings(Path::new(""), &output, per_format);
        params["includeSubfolders"] = json!("true");
        let error = session.execute(COMMAND, params.clone()).unwrap_err();
        assert!(matches!(error, EngineError::BadParams { msg, .. } if msg.contains("includeSubfolders")));
        params["includeSubfolders"] = json!(false);
        params["input"] = json!([]);
        assert_eq!(session.execute(COMMAND, params), Err(EngineError::Other(NO_SOURCE.into())));
    }
    assert!(!output.exists());
}

#[test]
fn a_source_removed_between_formats_keeps_the_written_file_and_read_error() {
    use super::{ProcessorInputs, image_processor_folder_inputs, processor_format, run_processor_formats};

    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    let input = root.path().join("test.ppm");
    let jpeg = output.join("test.jpg");
    fs::create_dir(&output).unwrap();
    write_image(&input).unwrap();
    let session = crate::session();
    let output = output.to_str().unwrap();
    let sources = image_processor_folder_inputs(&json!({"files": [input]}), output).unwrap();
    let inputs = ProcessorInputs::Folder(sources);
    let formats =
        [processor_format(&session, "jpeg", "jpg", &json!({})).unwrap().unwrap(), processor_format(&session, "tiff", "tif", &json!({})).unwrap().unwrap()];
    let result = run_processor_formats(
        &session,
        &inputs,
        output,
        formats.into_iter().inspect(|format| {
            if format.extension == "tif" {
                assert!(jpeg.is_file());
                fs::remove_file(&input).unwrap();
            }
        }),
    )
    .unwrap();
    let files = result["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(Path::new(files[0].as_str().unwrap()), jpeg);
    let errors = result["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["file"], json!(input));
    assert_eq!(errors[0]["format"], "tif");
}
