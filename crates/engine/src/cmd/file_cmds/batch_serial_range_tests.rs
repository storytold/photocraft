//! File › Automate › Batch: Starting Serial # takes 0 (#13662). Serial 0 with 3 Digit Serial
//! Number gives `000`, and a serial wider than its digits keeps every digit (1000 gives `1000`).

use super::*;
use serde_json::json;

/// Two inputs, so each check sees the starting serial and the one after it.
fn inputs() -> Vec<BatchInput> {
    ["in/first.png", "in/second.png"].iter().map(|path| BatchInput { path: (*path).into(), rel_dir: String::new() }).collect()
}

/// The file names `plan` gives the two inputs for `params`.
fn names(params: serde_json::Value) -> Vec<String> {
    plan(&inputs(), "out", "png", &params).unwrap().unwrap().iter().map(|path| crate::file_cmds::file_name(path).to_owned()).collect()
}

#[test]
fn serial_zero_counts_from_000_with_three_digits() {
    assert_eq!(names(json!({"namingComponents": [{"component": "serial3"}], "startingSerial": 0})), ["000", "001"]);
    assert_eq!(names(json!({"namingComponents": [{"component": "serial1"}], "startingSerial": 0})), ["0", "1"]);
    assert_eq!(names(json!({"naming": "{serial:3}_{name}", "startingSerial": 0})), ["000_first", "001_second"]);
}

/// Serial 0 has no letter of its own: from 0, template and structured letters both start at `a`,
/// never at an empty name. From 1 up, template letters still follow the serial.
#[test]
fn letters_from_serial_zero_start_at_a() {
    assert_eq!(names(json!({"naming": "{letter}{ext}", "startingSerial": 0})), ["a.png", "b.png"]);
    assert_eq!(names(json!({"naming": "{name}_{LETTER}", "startingSerial": 0})), ["first_A", "second_B"]);
    assert_eq!(names(json!({"namingComponents": [{"component": "letter"}], "startingSerial": 0})), ["a", "b"]);
    assert_eq!(names(json!({"naming": "{letter}", "startingSerial": 1})), ["a", "b"]);
    assert_eq!(names(json!({"naming": "{letter}", "startingSerial": 26})), ["z", "aa"]);
}

#[test]
fn a_serial_wider_than_its_digits_keeps_every_digit() {
    assert_eq!(names(json!({"namingComponents": [{"component": "serial3"}], "startingSerial": 1000})), ["1000", "1001"]);
    assert_eq!(names(json!({"namingComponents": [{"component": "serial2"}], "startingSerial": 9999})), ["9999", "10000"]);
    assert_eq!(names(json!({"naming": "{serial:3}", "startingSerial": 1000})), ["1000", "1001"]);
}

#[test]
fn no_starting_serial_still_starts_at_one() {
    assert_eq!(names(json!({"namingComponents": [{"component": "serial3"}]})), ["001", "002"]);
}

#[test]
fn a_starting_serial_that_is_no_whole_number_from_zero_is_refused() {
    for start in [json!(-1), json!(1.5), json!("1"), json!(null), json!(4_294_967_296_u64)] {
        let error = plan(&inputs(), "out", "png", &json!({"namingComponents": [{"component": "serial3"}], "startingSerial": start})).unwrap_err();
        assert!(error.to_string().contains("startingSerial must be an integer from 0 to 4294967295"), "{start}: {error}");
    }
}

/// The run itself writes the planned names: serial 0 reaches the files on disk as `000` and `001`.
#[test]
fn a_batch_run_from_serial_zero_writes_000_and_001() {
    let dir = std::env::temp_dir().join(format!("photocraft-batch-serial-range-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.to_string_lossy().into_owned();
    let mut s = crate::Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "background": "#ff0000"})).unwrap();
    let inputs: Vec<String> = ["a.png", "b.png"].iter().map(|name| join(&dir, name)).collect();
    for path in &inputs {
        crate::file_cmds::save_doc(&s.active().unwrap().doc, path, None).unwrap();
    }
    let out = join(&dir, "out");
    let result = s
        .execute(
            "file.automate.batch",
            json!({"input": inputs, "output": out, "steps": [], "format": "png", "namingComponents": [{"component": "serial3"}], "startingSerial": 0}),
        )
        .unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["files"], json!([join(&out, "000"), join(&out, "001")]));
    for name in ["000", "001"] {
        assert!(std::path::Path::new(&join(&out, name)).is_file(), "{name} was written");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
