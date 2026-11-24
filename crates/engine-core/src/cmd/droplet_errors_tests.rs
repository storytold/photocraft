//! The droplet's Errors choice (#13713): `createDroplet` stores the choice, `runDroplet` passes
//! `stopOnError` to Batch so the run ends at the first failing file, and a droplet that logs errors
//! writes the log itself, so its frame lines say `Start Droplet` / `End Droplet` (#13665's Batch
//! encoding otherwise, UTF-16 big-endian with BOM `FE FF` and CRLF). Droplets saved without the new
//! keys keep today's behaviour: go on after a failure, no log.
//!
//! Every session here comes from `crate::session()`, not a bare `Session::new()`: only the first
//! installs this crate's commands for the process, and CI runs each test on its own, so a bare
//! session reports `NotInstalled` for its first command.

use super::*;

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-droplet-errors-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

fn small_png(path: &str) {
    let mut s = crate::session();
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
    crate::session().execute("file.automate.createDroplet", p).unwrap()["path"].as_str().unwrap().to_string()
}

/// Writes `dir/Rotate.pcdroplet`, a step that succeeds on every image.
fn working_droplet(dir: &str, extra: Value) -> String {
    let mut p =
        json!({"path": format!("{dir}/Rotate"), "steps": [["image.imageRotation.90cw", {}]], "format": "png", "output": format!("{dir}/out"), "shim": false});
    if let (Some(object), Some(add)) = (p.as_object_mut(), extra.as_object()) {
        object.extend(add.clone());
    }
    crate::session().execute("file.automate.createDroplet", p).unwrap()["path"].as_str().unwrap().to_string()
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
    let error = crate::session()
        .execute(
            "file.automate.createDroplet",
            json!({"path": format!("{dir}/Bad"), "steps": failing_steps(), "output": format!("{dir}/out"), "shim": false, "errors": "quiet"}),
        )
        .unwrap_err();
    assert!(error.to_string().contains("errors"), "{error}");
}

/// A log path that is not a string is refused where it is stored, the way Batch's own policy parse
/// refuses it: a droplet never carries a log a later run would silently drop (no log, no error).
/// A null stays "not given", as everywhere else in this command.
#[test]
fn a_log_path_that_is_not_a_string_is_refused() {
    let dir = tmp("store-log-type");
    let error = crate::session()
        .execute(
            "file.automate.createDroplet",
            json!({"path": format!("{dir}/Typed"), "steps": failing_steps(), "output": format!("{dir}/out"), "shim": false, "errors": "log", "errorLog": 42}),
        )
        .unwrap_err();
    assert!(matches!(error, crate::EngineError::BadParams { .. }), "{error}");
    assert!(error.to_string().contains("errorLog"), "{error}");
    assert!(!std::path::Path::new(&format!("{dir}/Typed.pcdroplet")).exists(), "nothing was stored");
    let dp = failing_droplet(&dir, json!({"errors": "log", "errorLog": null}));
    let saved: Value = serde_json::from_slice(&std::fs::read(dp).unwrap()).unwrap();
    assert!(saved["options"].get("errorLog").is_none(), "null means not given: {saved}");
    assert_eq!(saved["options"].get("stopOnError"), Some(&json!(false)), "{saved}");
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
    let r = crate::session().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a, b]})).unwrap();
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
    let r = crate::session().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a, b]})).unwrap();
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
    let r = crate::session().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a, b]})).unwrap();
    assert_eq!(r["errors"].as_array().map(Vec::len), Some(2), "{r}");
    let text = decode_be(&std::fs::read(&log).unwrap());
    assert!(text.starts_with("Start Droplet\r\n\r\n"), "{text:?}");
    assert!(text.ends_with("\r\nEnd Droplet\r\n"), "{text:?}");
    // The `File:` lines use the platform's separators, as #13665 measured for Batch.
    let native = |path: &str| -> String { if cfg!(windows) { path.replace('/', "\\") } else { path.to_string() } };
    let first = format!("File: \"{}\"", native(&a));
    let second = format!("File: \"{}\"", native(&b));
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
    let r = crate::session().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a, b]})).unwrap();
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
    crate::session().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a.clone()]})).unwrap();
    crate::session()
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
    let r = crate::session().execute("file.automate.batch", json!({"steps": [], "input": [missing], "output": format!("{dir}/out"), "errorLog": log})).unwrap();
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
        let r = crate::session().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [txt.clone(), png.clone()]})).unwrap();
        assert_eq!(r["errors"], json!([]), "{choice}: {r}");
        assert_eq!(r["files"].as_array().map(Vec::len), Some(1), "{choice}: {r}");
        if choice == "log" {
            assert_eq!(std::fs::read(&log).unwrap().len(), 62, "{choice}: the log has no entry for notes.txt");
        } else {
            assert!(!std::path::Path::new(&log).exists(), "Stop for Errors writes no log: {r}");
        }
    }
}

// ---------------------------------------------------------------- the log never replaces an image

/// #13713 review: the droplet writes its own log, so Batch's log guard (#13665) never sees the path.
/// A log naming a dropped input, or a result the run plans to write, must leave that image alone: the
/// run finishes, the bytes stay, and the refusal reaches `errorLogError` in Batch's own wording.
#[test]
fn an_error_log_naming_an_input_or_a_planned_output_never_replaces_it() {
    let dir = tmp("log-collision");
    let (a, b) = inputs(&dir);
    // Aimed at a dropped input: the run goes on over the failing step, and a.png keeps its PNG bytes.
    let dropped = std::fs::read(&a).unwrap();
    let dp = failing_droplet(&dir, json!({"errors": "log", "errorLog": a.clone()}));
    let r = crate::session().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a.clone(), b.clone()]})).unwrap();
    assert_eq!(r["errors"].as_array().map(Vec::len), Some(2), "the run went on to both files: {r}");
    assert_eq!(std::fs::read(&a).unwrap(), dropped, "{a} still holds the dropped image, not the log");
    let refused = r["errorLogError"].as_str().unwrap_or_default().to_owned();
    assert!(refused.contains("errorLog") && refused.contains("must not replace an input or batch output"), "{r}");
    // Aimed at a planned output: run once without a log to learn the names Batch writes, then aim there.
    let plain = working_droplet(&dir, json!({}));
    let planned: Vec<String> =
        crate::session().execute("file.automate.runDroplet", json!({"droplet": plain, "input": [a.clone(), b.clone()]})).unwrap()["files"]
            .as_array()
            .expect("the run's results")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
    assert_eq!(planned.len(), 2, "{planned:?}");
    let out = planned.first().expect("one planned output").clone();
    let result = std::fs::read(&out).unwrap();
    let dp = working_droplet(&dir, json!({"errors": "log", "errorLog": out.clone()}));
    let r = crate::session().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a, b]})).unwrap();
    assert_eq!(r["files"].as_array().map(Vec::len), Some(2), "the run still wrote its results: {r}");
    assert_eq!(std::fs::read(&out).unwrap(), result, "{out} still holds the result, not the log");
    let refused = r["errorLogError"].as_str().unwrap_or_default().to_owned();
    assert!(refused.contains("errorLog") && refused.contains("must not replace an input or batch output"), "{r}");
}

/// #13713 review: the guard above missed one file of its own run, the `.pcdroplet` being executed.
/// It is never a dropped input (a folder scan takes images only) and never a batch result, so a
/// droplet saved with its log aimed at its own name erased its own definition: the next run of that
/// droplet found UTF-16 log bytes where the steps were. The definition's bytes must stay.
#[test]
fn an_error_log_naming_the_droplet_file_itself_never_replaces_it() {
    let dir = tmp("log-self");
    let (a, b) = inputs(&dir);
    let dp = failing_droplet(&dir, json!({"errors": "log", "errorLog": format!("{dir}/Fails.pcdroplet")}));
    let definition = std::fs::read(&dp).unwrap();
    assert_eq!(dp, format!("{dir}/Fails.pcdroplet"), "the log was aimed at the droplet's own path");
    let r = crate::session().execute("file.automate.runDroplet", json!({"droplet": dp.clone(), "input": [a, b]})).unwrap();
    assert_eq!(std::fs::read(&dp).unwrap(), definition, "{dp} still holds the droplet definition, not the log");
    let refused = r["errorLogError"].as_str().unwrap_or_default().to_owned();
    assert!(refused.contains("errorLog") && refused.contains("must not replace an input or batch output"), "{r}");
    let again = crate::session().execute("file.automate.runDroplet", json!({"droplet": dp.clone(), "input": [a]})).unwrap();
    assert_eq!(again["errors"].as_array().map(Vec::len), Some(1), "the droplet is still runnable: {again}");
}

/// #13713 review: the guard above only saw the results the run returned in `files`. A failed entry
/// is not in that list, yet Batch still planned its output name and refuses its own log there
/// (#13665), so the droplet has to protect it too: the name belongs to the run whether or not this
/// entry wrote it. A log aimed there replaced the result an earlier run left under that name.
#[test]
fn an_error_log_naming_the_planned_output_of_a_failed_entry_never_replaces_it() {
    let dir = tmp("log-failed-planned");
    let (a, b) = inputs(&dir);
    // What the run plans to write for the two dropped images, left there by an earlier run.
    std::fs::create_dir_all(format!("{dir}/out")).unwrap();
    let (first, second) = (format!("{dir}/out/a.png"), format!("{dir}/out/b.png"));
    std::fs::write(&first, b"earlier result for a").unwrap();
    std::fs::write(&second, b"earlier result for b").unwrap();
    let dp = failing_droplet(&dir, json!({"errors": "log", "errorLog": first.clone()}));
    let r = crate::session().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a, b]})).unwrap();
    assert_eq!(r["errors"].as_array().map(Vec::len), Some(2), "both entries failed: {r}");
    assert_eq!(r["files"], json!([]), "the failed entries wrote nothing: {r}");
    assert_eq!(std::fs::read(&first).unwrap(), b"earlier result for a", "{first} still holds the earlier result, not the log");
    assert_eq!(std::fs::read(&second).unwrap(), b"earlier result for b", "{second} still holds the earlier result");
    let refused = r["errorLogError"].as_str().unwrap_or_default().to_owned();
    assert!(refused.contains("errorLog") && refused.contains("must not replace an input or batch output"), "{r}");
}

// ---------------------------------------------------------------- the launcher next to the droplet

/// #15869 review: the guard above still missed one file of the run, the `.command` launcher
/// `createDroplet` writes next to the droplet (`write_shim`). It is neither the definition, a dropped
/// input nor a batch result, so a droplet saved with its log aimed at the launcher replaced the
/// launcher with UTF-16 log bytes and the next launch had no script to run. The rule below keys on the
/// launcher being there, not on the platform: `shim` defaults to `cfg!(unix)`, but a droplet saved
/// with `"shim": true` on Windows carries the very same file and needs the same protection.
#[test]
fn an_error_log_naming_the_droplet_launcher_never_replaces_it() {
    let dir = tmp("log-launcher");
    let (a, b) = inputs(&dir);
    let launcher = format!("{dir}/Fails.command");
    let dp = failing_droplet(&dir, json!({"errors": "log", "errorLog": launcher.clone(), "shim": true}));
    let script = std::fs::read(&launcher).expect("`\"shim\": true` writes the launcher next to the droplet");
    assert_eq!(&script[..2], b"#!", "{launcher} is a shell script");
    let r = crate::session().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a, b]})).unwrap();
    assert_eq!(r["errors"].as_array().map(Vec::len), Some(2), "the run went on to both files: {r}");
    assert_eq!(std::fs::read(&launcher).unwrap(), script, "{launcher} still holds the launcher, not the log");
    assert_eq!(&script[..2], b"#!", "the launcher is still launchable");
    let refused = r["errorLogError"].as_str().unwrap_or_default().to_owned();
    assert!(refused.contains("errorLog") && refused.contains("must not replace an input or batch output"), "{r}");
}

/// The other half of the same rule: a launcher that was never written is nothing to protect, so a log
/// that merely carries the `.command` suffix stays writable - that is the destination a user picks in
/// the Errors row, and refusing it would break the log the choice asks for.
#[test]
fn a_log_named_like_a_launcher_writes_when_no_launcher_was_generated() {
    let dir = tmp("log-launcher-none");
    let (a, b) = inputs(&dir);
    let log = format!("{dir}/Fails.command");
    let dp = failing_droplet(&dir, json!({"errors": "log", "errorLog": log.clone()}));
    assert!(!std::path::Path::new(&log).exists(), "the default here writes no launcher to protect");
    let r = crate::session().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a, b]})).unwrap();
    assert!(r.get("errorLogError").is_none(), "a log name that is not a launcher is no collision: {r}");
    let text = decode_be(&std::fs::read(&log).unwrap());
    assert!(text.starts_with("Start Droplet\r\n\r\n") && text.ends_with("\r\nEnd Droplet\r\n"), "{text:?}");
}

/// The default, saved with no `shim` key at all, on the platform the run happens to be on: Unix wrote
/// a launcher, so the log aimed at that name is refused and the script stays; elsewhere the default
/// wrote none, so the very same droplet writes its log to that name. One rule, two answers, and both
/// are pinned here so the difference is the launcher's existence and never a platform test.
#[test]
fn a_default_droplet_protects_its_launcher_only_where_one_was_written() {
    let dir = tmp("log-launcher-default");
    let (a, b) = inputs(&dir);
    let launcher = format!("{dir}/Default.command");
    let dp = crate::session()
        .execute(
            "file.automate.createDroplet",
            json!({"path": format!("{dir}/Default"), "steps": failing_steps(), "format": "png", "output": format!("{dir}/out"), "errors": "log", "errorLog": launcher}),
        )
        .unwrap()["path"]
        .as_str()
        .unwrap()
        .to_string();
    let script = std::fs::read(&launcher).ok();
    let r = crate::session().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a, b]})).unwrap();
    let refused = r["errorLogError"].as_str().unwrap_or_default().to_owned();
    match script {
        Some(script) => {
            assert_eq!(std::fs::read(&launcher).unwrap(), script, "{launcher} still holds the launcher, not the log");
            assert!(refused.contains("errorLog") && refused.contains("must not replace an input or batch output"), "{r}");
        }
        None => {
            assert!(refused.is_empty(), "no launcher was written, so nothing blocks the log: {r}");
            let text = decode_be(&std::fs::read(&launcher).unwrap());
            assert!(text.starts_with("Start Droplet\r\n\r\n"), "{text:?}");
        }
    }
}
