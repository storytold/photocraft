//! A droplet stores the Create Droplet File Naming rows, the starting serial and the compatibility
//! flags, and `file.automate.runDroplet` passes them to `file.automate.batch` (#13712).

use super::*;

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-droplet-naming-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

fn small_image(path: &str) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 3, "background": "#808080"})).unwrap();
    crate::file_cmds::save_doc(&s.active().unwrap().doc, path, None).unwrap();
}

/// The naming options of the measured case: 3 Digit Serial Number + text `_x` + extension.
fn naming_options() -> Value {
    json!({
        "namingComponents": [{"component": "serial3"}, {"text": "_x"}, {"component": "extension"}],
        "startingSerial": 7,
        "compatibility": {"windows": true, "macintosh": false, "unix": false},
    })
}

/// Saves a rotate-90 droplet in `dir` with `extra` merged into its options, and returns its path.
fn droplet(dir: &str, extra: &Value) -> String {
    let mut p =
        json!({"path": format!("{dir}/Rotate"), "steps": [["image.imageRotation.90cw", {}]], "format": "same", "output": format!("{dir}/out"), "shim": false});
    if let Some(map) = extra.as_object() {
        for (k, v) in map {
            p[k] = v.clone();
        }
    }
    let r = Session::new().execute("file.automate.createDroplet", p).unwrap();
    r["path"].as_str().unwrap().to_string()
}

/// Inputs `dir/a.png` and `dir/b.jpg`, run, and return the run result.
fn run_on_inputs(dp: &str, dir: &str) -> Value {
    let (a, b) = (format!("{dir}/a.png"), format!("{dir}/b.jpg"));
    small_image(&a);
    small_image(&b);
    Session::new().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a, b]})).unwrap()
}

/// `createDroplet` stores the naming rows, the starting serial and the compatibility flags in the droplet.
#[test]
fn create_droplet_stores_the_rows_serial_and_compatibility() {
    let dir = tmp("stored");
    let dp = droplet(&dir, &naming_options());
    let v: Value = serde_json::from_slice(&std::fs::read(&dp).unwrap()).unwrap();
    assert_eq!(v["options"]["namingComponents"], naming_options()["namingComponents"], "{v}");
    assert_eq!(v["options"]["startingSerial"], json!(7), "{v}");
    assert_eq!(v["options"]["compatibility"], naming_options()["compatibility"], "{v}");
}

/// The measured case: 3 Digit Serial + `_x` + extension with Starting Serial 7 on `a.png` and
/// `b.jpg` writes `007_x.png` and `008_x.jpg` (#13712 "Done when").
#[test]
fn droplet_naming_rows_serial_and_extension_name_the_outputs() {
    let dir = tmp("measured");
    let dp = droplet(&dir, &naming_options());
    let r = run_on_inputs(&dp, &dir);
    assert_eq!(r["errors"], json!([]), "{r}");
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(files.iter().any(|f| f.ends_with("007_x.png")), "{files:?}");
    assert!(files.iter().any(|f| f.ends_with("008_x.jpg")), "{files:?}");
    assert!(std::path::Path::new(&format!("{dir}/out/007_x.png")).exists(), "007_x.png was written");
    assert!(std::path::Path::new(&format!("{dir}/out/008_x.jpg")).exists(), "008_x.jpg was written");
    assert!(!std::path::Path::new(&format!("{dir}/out/a.png")).exists(), "the input name is not kept when naming is stored");
}

/// A droplet written before this change (no naming stored) keeps each input's name.
#[test]
fn a_droplet_without_stored_naming_keeps_each_input_name() {
    let dir = tmp("old");
    let dp = droplet(&dir, &json!({}));
    let v: Value = serde_json::from_slice(&std::fs::read(&dp).unwrap()).unwrap();
    assert!(v["options"].get("namingComponents").is_none(), "{v}");
    let r = run_on_inputs(&dp, &dir);
    assert_eq!(r["errors"], json!([]), "{r}");
    assert!(std::path::Path::new(&format!("{dir}/out/a.png")).exists(), "a.png keeps its name");
    assert!(std::path::Path::new(&format!("{dir}/out/b.jpg")).exists(), "b.jpg keeps its name");
}

/// Independent-review MF1: the Create Droplet form removes `namingComponents` when every row is
/// None but always sends the Starting Serial #, whose range includes 0. A droplet with a stored
/// 0 and no stored rows keeps each input's name instead of dying in Batch's serial validation,
/// which runs before Batch settles for keeping names (batch_naming.rs plan()).
#[test]
fn all_none_rows_with_a_stored_zero_serial_keep_each_input_name() {
    let dir = tmp("all-none-serial-0");
    let dp = droplet(&dir, &json!({"startingSerial": 0}));
    let v: Value = serde_json::from_slice(&std::fs::read(&dp).unwrap()).unwrap();
    assert!(v["options"].get("namingComponents").is_none(), "all-None rows store no naming: {v}");
    let r = run_on_inputs(&dp, &dir);
    assert_eq!(r["errors"], json!([]), "{r}");
    assert!(std::path::Path::new(&format!("{dir}/out/a.png")).exists(), "a.png keeps its name");
    assert!(std::path::Path::new(&format!("{dir}/out/b.jpg")).exists(), "b.jpg keeps its name");
}

/// An old or hand-written droplet can store the option keys as null; a null key counts as not
/// stored, so the run keeps each input's name instead of forwarding nulls that Batch rejects.
#[test]
fn null_valued_option_keys_behave_like_absent_ones() {
    let dir = tmp("null-keys");
    let dp = format!("{dir}/Null.pcdroplet");
    let droplet_json = json!({
        "photocraftDroplet": 1,
        "name": "Null",
        "action": {"name": "Null", "steps": [["image.imageRotation.90cw", {}]]},
        "options": {"output": format!("{dir}/out"), "format": "same", "namingComponents": null, "startingSerial": null, "compatibility": null},
    });
    std::fs::write(&dp, serde_json::to_vec(&droplet_json).unwrap()).unwrap();
    let r = run_on_inputs(&dp, &dir);
    assert_eq!(r["errors"], json!([]), "{r}");
    assert!(std::path::Path::new(&format!("{dir}/out/a.png")).exists(), "a.png keeps its name");
    assert!(std::path::Path::new(&format!("{dir}/out/b.jpg")).exists(), "b.jpg keeps its name");
}

/// The engine keeps Batch's own starting-serial validation: a stored 0 is refused by the batch
/// the droplet runs (the 0..9999 dialog range is #13662's engine question, not clamped here).
#[test]
fn a_stored_zero_serial_runs_batch_to_its_range_error() {
    let dir = tmp("zero");
    let dp = droplet(&dir, &json!({"namingComponents": [{"component": "serial3"}], "startingSerial": 0}));
    let (a, b) = (format!("{dir}/a.png"), format!("{dir}/b.jpg"));
    small_image(&a);
    small_image(&b);
    let r = Session::new().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a, b]}));
    let e = r.expect_err("the batch range error surfaces");
    assert!(e.to_string().contains("startingSerial"), "{e}");
}

/// The compatibility flags reach Batch: with Mac OS checked a colon in a text row is refused as
/// platform-incompatible instead of written.
#[test]
fn compatibility_flags_reach_batch_and_refuse_a_mac_incompatible_name() {
    let dir = tmp("compat");
    let dp = droplet(&dir, &json!({"namingComponents": [{"text": "a:b"}], "compatibility": {"macintosh": true, "windows": false, "unix": false}}));
    let (a, b) = (format!("{dir}/a.png"), format!("{dir}/b.jpg"));
    small_image(&a);
    small_image(&b);
    let r = Session::new().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [a, b]}));
    let e = r.expect_err("the naming plan refuses the name before any file is written");
    assert!(e.to_string().contains("incompatible with the requested platform"), "{e}");
}

/// A serial above the dialog's 9999 can only come from a hand-written droplet; Batch takes it
/// as it does today, so the run numbers on from the stored value.
#[test]
fn a_stored_serial_above_the_dialog_range_is_not_clamped_by_the_engine() {
    let dir = tmp("high");
    let dp = droplet(&dir, &json!({"namingComponents": [{"component": "serial3"}, {"component": "extension"}], "startingSerial": 12345}));
    let r = run_on_inputs(&dp, &dir);
    assert_eq!(r["errors"], json!([]), "{r}");
    assert!(std::path::Path::new(&format!("{dir}/out/12345.png")).exists(), "12345.png was written");
    assert!(std::path::Path::new(&format!("{dir}/out/12346.jpg")).exists(), "12346.jpg was written");
}

/// A droplet that stored only compatibility flags and no naming rows keeps each input's name, as
/// it does today: `runDroplet` forwards the flags only along with stored naming rows, and Batch
/// would refuse a `compatibility` without them. Guard test: passes before and after the gating.
#[test]
fn a_droplet_with_only_compatibility_flags_still_keeps_each_input_name() {
    let dir = tmp("compat-only");
    let dp = droplet(&dir, &json!({"compatibility": {"macintosh": true, "windows": true, "unix": true}}));
    let r = run_on_inputs(&dp, &dir);
    assert_eq!(r["errors"], json!([]), "{r}");
    assert!(std::path::Path::new(&format!("{dir}/out/a.png")).exists(), "a.png keeps its name");
    assert!(std::path::Path::new(&format!("{dir}/out/b.jpg")).exists(), "b.jpg keeps its name");
}
