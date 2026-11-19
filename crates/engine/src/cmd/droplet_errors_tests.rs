//! The droplet's Errors choice (#13713): `createDroplet` stores the choice, `runDroplet` passes
//! `stopOnError` to Batch so the run ends at the first failing file, and a droplet that logs errors
//! writes the log itself, so its frame lines say `Start Droplet` / `End Droplet` (#13665's Batch
//! encoding otherwise, UTF-16 big-endian with BOM `FE FF` and CRLF). Droplets saved without the new
//! keys keep today's behaviour: go on after a failure, no log.

use super::*;

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-droplet-errors-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

fn small_png(path: &str) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 3, "background": "#808080"})).unwrap();
    crate::file_cmds::save_doc(&s.active().unwrap().doc, path, None).unwrap();
}

/// The two dropped images, in this order: `a.png` first, `b.jpg` second.
fn inputs(dir: &str) -> (String, String) {
    let (a, b) = (format!("{dir}/a.png"), format!("{dir}/b.jpg"));
    small_png(&a);
    small_png(&b);
    (a, b)
}

/// A step that fails on every document: it names a layer that is not there.
fn failing_steps() -> Value {
    json!([["layer.delete", {"layer": 999}]])
}

/// Writes `dir/Fails.pcdroplet` with the failing step and the extra create params in `extra`.
fn failing_droplet(dir: &str, extra: Value) -> String {
    let mut p = json!({"path": format!("{dir}/Fails"), "steps": failing_steps(), "format": "png", "output": format!("{dir}/out"), "shim": false});
    if let (Some(object), Some(add)) = (p.as_object_mut(), extra.as_object()) {
        object.extend(add.clone());
    }
    Session::new().execute("file.automate.createDroplet", p).unwrap()["path"].as_str().unwrap().to_string()
}

/// Writes `dir/Rotate.pcdroplet`, a step that succeeds on every image.
fn working_droplet(dir: &str, extra: Value) -> String {
    let mut p =
        json!({"path": format!("{dir}/Rotate"), "steps": [["image.imageRotation.90cw", {}]], "format": "png", "output": format!("{dir}/out"), "shim": false});
    if let (Some(object), Some(add)) = (p.as_object_mut(), extra.as_object()) {
        object.extend(add.clone());
    }
    Session::new().execute("file.automate.createDroplet", p).unwrap()["path"].as_str().unwrap().to_string()
}

fn utf16_be(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xfe, 0xff];
    for word in text.encode_utf16() {
        bytes.extend_from_slice(&word.to_be_bytes());
    }
    bytes
}

fn decode_be(bytes: &[u8]) -> String {
    assert_eq!(&bytes[..2], &[0xfe, 0xff], "BOM FE FF");
    let (pairs, remainder) = bytes[2..].as_chunks::<2>();
    assert!(remainder.is_empty(), "an even number of bytes");
    String::from_utf16(&pairs.iter().map(|pair| u16::from_be_bytes(*pair)).collect::<Vec<_>>()).unwrap()
}

// ---------------------------------------------------------------- the choice in the droplet

/// Stop for Errors: the droplet carries Batch's own `stopOnError`, so a run needs no new key.
#[test]
fn stop_for_errors_is_stored_in_the_droplet() {
    let dir = tmp("store-stop");
    let dp = failing_droplet(&dir, json!({"errors": "stop"}));
    let saved: Value = serde_json::from_slice(&std::fs::read(&dp).unwrap()).unwrap();
    assert_eq!(saved["options"]["stopOnError"], json!(true), "{saved}");
    assert!(saved["options"].get("errorLog").is_none(), "Stop for Errors writes no log: {saved}");
}

/// Log Errors to File: the chosen log path is stored, and the run goes on after a failure.
#[test]
fn log_errors_to_file_stores_the_log_path() {
    let dir = tmp("store-log");
    let log = format!("{dir}/droplog.txt");
    let dp = failing_droplet(&dir, json!({"errors": "log", "errorLog": log}));
    let saved: Value = serde_json::from_slice(&std::fs::read(&dp).unwrap()).unwrap();
    assert_eq!(saved["options"]["errorLog"], json!(log), "{saved}");
    assert_eq!(saved["options"].get("stopOnError"), Some(&json!(false)), "{saved}");
}

/// A droplet saved without the Errors choice keeps today's parameters.
#[test]
fn a_droplet_saved_without_the_choice_stores_neither_key() {
    let dir = tmp("store-none");
    let dp = failing_droplet(&dir, json!({}));
    let saved: Value = serde_json::from_slice(&std::fs::read(&dp).unwrap()).unwrap();
    assert_eq!(saved["options"], json!({"format": "png", "output": format!("{dir}/out")}), "{saved}");
}

#[test]
fn the_errors_choice_is_only_stop_or_log() {
    let dir = tmp("store-bad");
    let error = Session::new()
        .execute(
            "file.automate.createDroplet",
            json!({"path": format!("{dir}/Bad"), "steps": failing_steps(), "output": format!("{dir}/out"), "shim": false, "errors": "quiet"}),
        )
        .unwrap_err();
    assert!(error.to_string().contains("errors"), "{error}");
}

// ---------------------------------------------------------------- stop at the first failure

/// Stop for Errors reaches Batch: the failing file is the last one the run touches.
#[test]
fn stop_for_errors_ends_the_run_at_the_first_failing_file() {
    let dir = tmp("stop");
    let (a, b) = (format!("{dir}/a.png"), format!("{dir}/b.jpg"));
    small_png(&a);
    small_png(&b);
    let dp = failing_droplet(&dir, json!({"errors": "stop"}));
    let r = Session::new().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a, b]})).unwrap();
    assert_eq!(r["errors"].as_array().map(Vec::len), Some(1), "{r}");
    assert_eq!(r["skipped"], json!([format!("{dir}/b.jpg")]), "{r}");
    assert_eq!(r["files"], json!([]), "{r}");
    assert!(!std::path::Path::new(&format!("{dir}/out/b.jpg")).exists(), "the second file was not processed");
}

/// Without the choice, as saved before #13713, the run goes on and fails on both files: today's
/// behaviour, pinned as a guard.
#[test]
fn a_droplet_without_the_new_keys_goes_on_after_a_failure() {
    let dir = tmp("old-go-on");
    let (a, b) = inputs(&dir);
    let dp = failing_droplet(&dir, json!({}));
    let log = format!("{dir}/droplog.txt");
    let r = Session::new().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a, b]})).unwrap();
    assert_eq!(r["errors"].as_array().map(Vec::len), Some(2), "{r}");
    assert!(r.get("skipped").is_none(), "{r}");
    assert!(!std::path::Path::new(&log).exists(), "a droplet saved without the choice writes no log");
}

/// Log Errors to File goes on: both failures are listed, in dropped order.
#[test]
fn log_errors_to_file_runs_every_file() {
    let dir = tmp("log-go-on");
    let (a, b) = inputs(&dir);
    let log = format!("{dir}/droplog.txt");
    let dp = failing_droplet(&dir, json!({"errors": "log", "errorLog": log}));
    let r = Session::new().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a, b]})).unwrap();
    assert_eq!(r["errors"].as_array().map(Vec::len), Some(2), "{r}");
    let text = decode_be(&std::fs::read(&log).unwrap());
    assert!(text.starts_with("Start Droplet\r\n\r\n"), "{text:?}");
    assert!(text.ends_with("\r\nEnd Droplet\r\n"), "{text:?}");
    let first = format!("File: \"{}\"", a.replace('/', "\\"));
    let second = format!("File: \"{}\"", b.replace('/', "\\"));
    assert!(text.contains(&first) && text.contains(&second), "{text:?}");
}

// ---------------------------------------------------------------- the droplet log

/// An error-free run still writes the log: 62 bytes, BOM plus `Start Droplet`, blank, `End Droplet`.
#[test]
fn an_error_free_run_writes_exactly_62_bytes() {
    let dir = tmp("log-clean");
    let log = format!("{dir}/droplog.txt");
    let dp = working_droplet(&dir, json!({"errors": "log", "errorLog": log}));
    let (a, b) = inputs(&dir);
    let r = Session::new().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a, b]})).unwrap();
    assert_eq!(r["errors"], json!([]), "{r}");
    let bytes = std::fs::read(&log).unwrap();
    assert_eq!(bytes.len(), 62, "{:?}", decode_be(&bytes));
    assert_eq!(bytes, utf16_be("Start Droplet\r\n\r\nEnd Droplet\r\n"));
}

/// The frame lines say Droplet, never Batch: the same failing step logged by Batch still writes
/// #13665's Batch lines next to the droplet's Droplet lines.
#[test]
fn the_droplet_log_says_droplet_where_the_batch_log_says_batch() {
    let dir = tmp("frames");
    let (a, _) = inputs(&dir);
    let droplet_log = format!("{dir}/droplet.txt");
    let dp = failing_droplet(&dir, json!({"errors": "log", "errorLog": droplet_log}));
    Session::new().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a.clone()]})).unwrap();
    Session::new()
        .execute(
            "file.automate.batch",
            json!({"steps": failing_steps(), "input": [a], "output": format!("{dir}/batch-out"), "format": "png", "errorLog": format!("{dir}/batch.txt")}),
        )
        .unwrap();
    assert!(decode_be(&std::fs::read(&droplet_log).unwrap()).starts_with("Start Droplet\r\n\r\n"), "the droplet log's first line");
    assert!(decode_be(&std::fs::read(format!("{dir}/batch.txt")).unwrap()).starts_with("Start Batch\r\n\r\n"), "the batch log's first line");
}

/// The same rule driven through the command, so a Batch run keeps #13665's frame lines.
#[test]
fn a_batch_run_still_writes_batch_frame_lines() {
    let dir = tmp("batch-neighbour");
    let log = format!("{dir}/batchlog.txt");
    let missing = format!("{dir}/in/missing.png");
    let r = Session::new().execute("file.automate.batch", json!({"steps": [], "input": [missing], "output": format!("{dir}/out"), "errorLog": log})).unwrap();
    assert_eq!(r["errors"].as_array().map(Vec::len), Some(1), "{r}");
    let text = decode_be(&std::fs::read(&log).unwrap());
    assert!(text.starts_with("Start Batch\r\n\r\n"), "{text:?}");
    assert!(text.ends_with("\r\nEnd Batch\r\n"), "{text:?}");
}

/// A dropped file that is not an image stays silent in both modes (#13717): no reported error, and
/// nothing in the log of a droplet that logs.
#[test]
fn a_dropped_text_file_stays_silent_in_both_modes() {
    let dir = tmp("log-non-image");
    let log = format!("{dir}/droplog.txt");
    let txt = format!("{dir}/notes.txt");
    std::fs::write(&txt, "just some notes").unwrap();
    let png = format!("{dir}/a.png");
    small_png(&png);
    for choice in ["log", "stop"] {
        let _ = std::fs::remove_file(&log);
        let dp = working_droplet(&dir, json!({"errors": choice, "errorLog": log}));
        let r = Session::new().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [txt.clone(), png.clone()]})).unwrap();
        assert_eq!(r["errors"], json!([]), "{choice}: {r}");
        assert_eq!(r["files"].as_array().map(Vec::len), Some(1), "{choice}: {r}");
        if choice == "log" {
            assert_eq!(std::fs::read(&log).unwrap().len(), 62, "{choice}: the log has no entry for notes.txt");
        } else {
            assert!(!std::path::Path::new(&log).exists(), "Stop for Errors writes no log: {r}");
        }
    }
}
