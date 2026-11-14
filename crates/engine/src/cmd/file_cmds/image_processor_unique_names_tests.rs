use std::{
    fs,
    path::{Path, PathBuf},
    sync::Barrier,
    thread,
};

use serde_json::{Value, json};

use crate::Session;

const COMMAND: &str = "file.scripts.imageProcessor";
const PIXELS: &[u8] = b"P6\n2 1\n255\n\xff\0\0\0\xff\0";

fn files(result: &Value) -> Result<Vec<PathBuf>, serde_json::Error> {
    serde_json::from_value(result["files"].clone())
}

#[test]
fn selects_the_first_free_suffix_and_reports_every_written_format() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.b.ppm");
    fs::write(&source, PIXELS).unwrap();
    let output = root.path().join("out");
    fs::create_dir(&output).unwrap();
    for name in ["a.b.jpg", "a.b_1.jpg", "a.b_3.jpg", "a.b.tif"] {
        fs::write(output.join(name), name.as_bytes()).unwrap();
    }
    let result = Session::new().execute(COMMAND, json!({"input":[source],"output":output,"jpeg":{},"psd":{},"tiff":{}})).unwrap();
    let expected = ["a.b_2.jpg", "a.b.psd", "a.b_1.tif"];
    assert_eq!(files(&result).unwrap(), expected.map(|name| output.join(name)));
    assert_eq!(result["errors"], json!([]));
    let next = Session::new().execute(COMMAND, json!({"input":[source],"output":output,"format":"jpg"})).unwrap();
    assert_eq!(files(&next).unwrap(), [output.join("a.b_4.jpg")]);
    assert_eq!(next["errors"], json!([]));
    for name in ["a.b.jpg", "a.b_1.jpg", "a.b_3.jpg", "a.b.tif"] {
        assert_eq!(fs::read(output.join(name)).unwrap(), name.as_bytes());
    }
    assert_eq!(fs::read(&source).unwrap(), PIXELS);
    for name in expected {
        let file = output.join(name);
        let result = photocraft_io::import(name, &fs::read(file).unwrap()).unwrap();
        assert_eq!((result.document.size.width, result.document.size.height), (2, 1));
    }
}

#[test]
fn same_stem_inputs_get_separate_results_without_changing_shared_processing() {
    let source = tempfile::tempdir().unwrap();
    let output = source.path().join("output");
    let a = source.path().join("a/same.ppm");
    let b = source.path().join("b/same.ppm");
    for path in [&a, &b] {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, PIXELS).unwrap();
    }
    let mut session = Session::new();
    let result = session.execute(COMMAND, json!({"input":[a,b],"output":output,"format":"png"})).unwrap();
    assert_eq!(files(&result).unwrap(), [output.join("same.png"), output.join("same_1.png")]);
    assert_eq!(result["errors"], json!([]));

    let result = session.execute(COMMAND, json!({"input":source.path(),"output":output,"format":"png","includeSubfolders":true})).unwrap();
    assert_eq!(files(&result).unwrap(), [output.join("a/same.png"), output.join("b/same.png")]);
    assert_eq!(result["errors"], json!([]));

    let legacy = source.path().join("legacy");
    let inputs = [a.to_string_lossy().into_owned(), b.to_string_lossy().into_owned()];
    let result = super::super::batch_run::process_files(&session, &inputs, &legacy.to_string_lossy(), "png", Default::default(), "", &|_| Ok(()));
    assert_eq!(files(&result).unwrap(), [legacy.join("same.png")]);
    let errors = result["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert!(errors[0]["error"].as_str().unwrap().contains("same output name"));
}

#[test]
fn concurrent_folder_runs_cannot_replace_each_others_results() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("same.ppm");
    let output = root.path().join("out");
    fs::write(&source, PIXELS).unwrap();
    let params = json!({"input": [source], "output": output, "format": "png"});
    let gate = Barrier::new(4);
    let results = thread::scope(|scope| {
        let workers: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    let mut session = Session::new();
                    gate.wait();
                    session.execute(COMMAND, params.clone())
                })
            })
            .collect();
        workers.into_iter().map(|worker| worker.join()).collect::<Vec<_>>()
    });
    let mut written = Vec::new();
    for result in results {
        let result = result.unwrap().unwrap();
        assert_eq!(result["errors"], json!([]));
        written.push(result["files"][0].as_str().unwrap().to_owned());
    }
    written.sort();
    let expected = ["same.png", "same_1.png", "same_2.png", "same_3.png"].map(|file| output.join(file));
    assert_eq!(written.iter().map(Path::new).collect::<Vec<_>>(), expected.iter().map(|path| path.as_path()).collect::<Vec<_>>());
    assert_eq!(fs::read_dir(&output).unwrap().count(), 4);
    for path in &expected {
        assert!(photocraft_io::import(&path.to_string_lossy(), &fs::read(path).unwrap()).is_ok());
    }
}

#[test]
fn directory_and_dangling_symlink_names_stay_occupied() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("example.ppm");
    fs::write(&source, PIXELS).unwrap();
    let output = root.path().join("out");
    fs::create_dir_all(output.join("example.png")).unwrap();
    let counter = if cfg!(unix) { 2 } else { 1 };
    #[cfg(unix)]
    std::os::unix::fs::symlink(output.join("missing.png"), output.join("example_1.png")).unwrap();
    let result = Session::new().execute(COMMAND, json!({"input":[source],"output":output,"format":"png"})).unwrap();
    assert_eq!(files(&result).unwrap(), [output.join(format!("example_{counter}.png"))]);
    assert_eq!(result["errors"], json!([]));
    assert!(output.join("example.png").is_dir());
    #[cfg(unix)]
    assert_eq!(fs::read_link(output.join("example_1.png")).unwrap(), output.join("missing.png"));
    assert!(!output.join("missing.png").exists());
}
