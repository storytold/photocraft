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
