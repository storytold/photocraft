//! File › Automate › Batch: an input that fails does not use up a serial number or letter (#13664).

use super::*;

fn tmp(name: &str) -> String {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("photocraft-batch-serial-{}-{name}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

fn write_png(path: &str) {
    let img = photocraft_codecs::Image::from_u8(4, 4, photocraft_codecs::ChannelLayout::Rgb, vec![200; 48]).unwrap();
    let bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// The issue's folder: `a.png`, `aa.txt`, `b.png`, `c.png`, `notes.txt`, `sub/a.png`, `sub/deeper/d.png`.
fn issue_folder(label: &str, with_text: bool) -> (String, String) {
    let dir = tmp(label);
    let input = join(&dir, "in");
    std::fs::create_dir_all(join(&input, "sub/deeper")).unwrap();
    for name in ["a.png", "b.png", "c.png", "sub/a.png", "sub/deeper/d.png"] {
        write_png(&join(&input, name));
    }
    if with_text {
        for name in ["aa.txt", "notes.txt"] {
            std::fs::write(join(&input, name), b"not an image").unwrap();
        }
    }
    (input, join(&dir, "out"))
}

fn run(input: &str, output: &str, with_text: bool, naming: Value) -> Vec<String> {
    let mut s = Session::new();
    let p = json!({
        "input": input, "output": output, "steps": [], "format": "same", "startingSerial": 5,
        "includeSubfolders": true, "includeNonImages": with_text, "flatOutput": true, "namingComponents": naming
    });
    let result = s.execute("file.automate.batch", p).unwrap();
    let failed = result["errors"].as_array().unwrap().len();
    assert_eq!(failed, if with_text { 2 } else { 0 }, "{result}");
    result["files"].as_array().unwrap().iter().map(|f| file_name(f.as_str().unwrap()).to_string()).collect()
}

#[test]
fn a_failed_file_does_not_use_up_a_serial_number() {
    let (input, output) = issue_folder("gap", true);
    let names = run(&input, &output, true, json!([{"component":"serial3"}, {"component":"extension"}, {"text":"_x"}]));
    assert_eq!(names, ["005png_x", "006png_x", "007png_x", "008png_x", "009png_x"]);
}

#[test]
fn a_batch_without_failures_keeps_its_names() {
    let (input, output) = issue_folder("clean", false);
    let names = run(&input, &output, false, json!([{"component":"serial3"}, {"component":"extension"}, {"text":"_x"}]));
    assert_eq!(names, ["005png_x", "006png_x", "007png_x", "008png_x", "009png_x"]);
}

#[test]
fn serial_letters_follow_the_same_counter() {
    let (input, output) = issue_folder("letters", true);
    let names = run(&input, &output, true, json!([{"component":"letter"}, {"text":"_x"}]));
    assert_eq!(names, ["a_x", "b_x", "c_x", "d_x", "e_x"]);
}

/// A folder of `files` (PNGs where the name ends in `.png`, text otherwise) run with `naming`.
fn run_named(label: &str, files: &[&str], in_place: bool, start: u32, naming: &str) -> (Vec<String>, usize) {
    let dir = tmp(label);
    let input = join(&dir, "in");
    std::fs::create_dir_all(&input).unwrap();
    for name in files {
        if name.ends_with(".png") {
            write_png(&join(&input, name));
        } else {
            std::fs::write(join(&input, name), b"not an image").unwrap();
        }
    }
    let output = if in_place { input.clone() } else { join(&dir, "out") };
    let mut s = Session::new();
    let p = json!({
        "input": input, "output": output, "steps": [], "format": "same", "startingSerial": start,
        "includeNonImages": true, "naming": naming
    });
    let result = s.execute("file.automate.batch", p).unwrap();
    let files = result["files"].as_array().unwrap().iter().map(|f| file_name(f.as_str().unwrap()).to_string()).collect();
    (files, result["errors"].as_array().unwrap().len())
}

#[test]
fn a_refused_input_after_the_last_serial_number_does_not_fail_the_batch() {
    let (files, errors) = run_named("last-serial", &["a.png", "z"], false, u32::MAX, "{name}_{serial}.png");
    assert_eq!((files.as_slice(), errors), (["a_4294967295.png".to_string()].as_slice(), 1));
}

#[test]
fn a_refused_input_named_like_a_source_does_not_stop_an_in_place_batch() {
    // `z` takes no number, so its name is `z_003.png`, the next input's own name: nothing is
    // written to it, so the batch still runs in place.
    let (files, errors) = run_named("in-place", &["a.png", "b", "c.png", "z", "z_003.png"], true, 1, "{name}_{serial:3}.png");
    assert_eq!(files, ["a_001.png", "c_002.png", "z_003_003.png"]);
    assert_eq!(errors, 2);
}

#[test]
fn the_error_log_may_take_a_refused_inputs_name() {
    // `d.txt` takes no number, so its name `d_003.png` is never written and the log can use it.
    let dir = tmp("log-name");
    let input = join(&dir, "in");
    std::fs::create_dir_all(&input).unwrap();
    for name in ["a.png", "c.png"] {
        write_png(&join(&input, name));
    }
    for name in ["b.txt", "d.txt"] {
        std::fs::write(join(&input, name), b"not an image").unwrap();
    }
    let output = join(&dir, "out");
    let log = join(&output, "d_003.png");
    let p = json!({
        "input": input, "output": output, "steps": [], "format": "same", "includeNonImages": true,
        "naming": "{name}_{serial:3}.png", "errorLog": log
    });
    let result = Session::new().execute("file.automate.batch", p).unwrap();
    let files: Vec<String> = result["files"].as_array().unwrap().iter().map(|f| file_name(f.as_str().unwrap()).to_string()).collect();
    assert_eq!(files, ["a_001.png", "c_002.png"]);
    assert_eq!(result["errors"].as_array().unwrap().len(), 2);
    assert_eq!(result["errorLog"], json!(log));
}
