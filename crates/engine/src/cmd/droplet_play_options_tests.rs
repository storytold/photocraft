//! Create Droplet's Play options (#13709): Override Action "Open" Commands, Include All
//! Subfolders, Suppress File Open Options Dialogs and Suppress Color Profile Warnings are saved in
//! the droplet, and each run of the droplet uses them.

use super::*;

/// Saves a `w`×`h` image at `path`, in the format its extension names.
fn image(path: &str, w: u32, h: u32) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "#406080"})).unwrap();
    crate::file_cmds::save_doc(&s.active().unwrap().doc, path, None).unwrap();
}

/// The issue's test folder under `root`: `in/` with `a.png`, `b.jpg`, `c.psd`, `notes.txt`,
/// `sub/a.png` and `sub/deeper/d.png`. The two `a.png` differ in size (4×3 at the top, 6×5 in
/// `sub`), so a result tells which one it came from. Returns the `in` folder.
fn test_folder(root: &str) -> String {
    let dir = join(root, "in");
    std::fs::create_dir_all(join(&dir, "sub/deeper")).unwrap();
    image(&join(&dir, "a.png"), 4, 3);
    image(&join(&dir, "b.jpg"), 4, 3);
    image(&join(&dir, "c.psd"), 4, 3);
    std::fs::write(join(&dir, "notes.txt"), "not an image").unwrap();
    image(&join(&dir, "sub/a.png"), 6, 5);
    image(&join(&dir, "sub/deeper/d.png"), 2, 2);
    dir
}

/// Creates `root/Invert.pcdroplet`, which inverts each input into `root/out`; `extra` adds params
/// (the Play options) to `file.automate.createDroplet`.
fn droplet(root: &str, extra: Value) -> String {
    let mut p = json!({"path": join(root, "Invert"), "steps": [["image.adjustments.invert", {}]], "output": join(root, "out"), "shim": false});
    p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    Session::new().execute("file.automate.createDroplet", p).unwrap()["path"].as_str().unwrap().to_string()
}

/// The `options` object saved in the droplet file at `path`.
fn saved_options(path: &str) -> Value {
    serde_json::from_slice::<Value>(&std::fs::read(path).unwrap()).unwrap()["options"].clone()
}

/// The names of the files in `dir`, sorted.
fn names(dir: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

/// Width and height of the image at `path`.
fn size(path: &str) -> (u32, u32) {
    let d = photocraft_io::import(path, &std::fs::read(path).unwrap()).unwrap().document;
    (d.size.width, d.size.height)
}

/// Runs the droplet at `droplet` on `input` (files and folders) in a fresh session.
fn run(droplet: &str, input: Value) -> Value {
    Session::new().execute("file.automate.runDroplet", json!({"droplet": droplet, "input": input})).unwrap()
}

#[test]
fn create_droplet_saves_the_play_options() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().to_string_lossy().into_owned();
    let out = join(&root, "out");
    let on = droplet(&root, json!({"overrideOpen": true, "includeSubfolders": true, "suppressOpenOptions": true, "suppressWarnings": true}));
    assert_eq!(
        saved_options(&on),
        json!({"output": out, "overrideOpen": true, "includeSubfolders": true, "suppressOpenOptions": true, "suppressWarnings": true})
    );
    let off = droplet(&root, json!({"overrideOpen": false, "includeSubfolders": false, "suppressOpenOptions": false, "suppressWarnings": false}));
    assert_eq!(
        saved_options(&off),
        json!({"output": out, "overrideOpen": false, "includeSubfolders": false, "suppressOpenOptions": false, "suppressWarnings": false})
    );
    // Each option on its own, so one key can't stand in for another.
    for key in ["overrideOpen", "includeSubfolders", "suppressOpenOptions", "suppressWarnings"] {
        let one = droplet(&root, json!({key: true}));
        assert_eq!(saved_options(&one), json!({"output": out, key: true}), "{key}");
    }
    // Without the params (and with null) the droplet is written as before.
    assert_eq!(saved_options(&droplet(&root, json!({}))), json!({"output": out}));
    assert_eq!(saved_options(&droplet(&root, json!({"includeSubfolders": null}))), json!({"output": out}));
}

/// The issue's measurement: with Include All Subfolders, dropping `in` writes `a.png`, `b.jpg`,
/// `c.psd` and `d.png`; the files are taken depth first, so `sub/a.png` comes later and its result
/// replaces the top-level one's without an error, and `notes.txt` is skipped.
#[test]
fn include_all_subfolders_takes_every_subfolder_depth_first() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().to_string_lossy().into_owned();
    let input = test_folder(&root);
    let out = join(&root, "out");
    let r = run(&droplet(&root, json!({"includeSubfolders": true})), json!([input]));
    assert_eq!(r["errors"], json!([]), "{r}");
    let expected: Vec<String> = ["a.png", "b.jpg", "c.psd", "a.png", "d.png"].iter().map(|n| join(&out, n)).collect();
    assert_eq!(r["files"], json!(expected), "inputs in, in/sub, in/sub/deeper, each folder's files by name");
    assert_eq!(names(&out), ["a.png", "b.jpg", "c.psd", "d.png"]);
    assert_eq!(size(&join(&out, "a.png")), (6, 5), "the later a.png (from sub) wins");
    assert_eq!(size(&join(&out, "d.png")), (2, 2));
}

/// Without Include All Subfolders a dropped folder gives only its own images, as before; a droplet
/// written before the option existed (no key) runs the same way.
#[test]
fn without_include_all_subfolders_a_folder_gives_its_own_images() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().to_string_lossy().into_owned();
    let input = test_folder(&root);
    let out = join(&root, "out");
    for extra in [json!({"includeSubfolders": false}), json!({})] {
        let _ = std::fs::remove_dir_all(&out);
        let dp = droplet(&root, extra.clone());
        let r = run(&dp, json!([input]));
        assert_eq!(r["errors"], json!([]), "{extra}: {r}");
        let expected: Vec<String> = ["a.png", "b.jpg", "c.psd"].iter().map(|n| join(&out, n)).collect();
        assert_eq!(r["files"], json!(expected), "{extra}");
        assert_eq!(names(&out), ["a.png", "b.jpg", "c.psd"], "{extra}");
        assert_eq!(size(&join(&out, "a.png")), (4, 3), "{extra}");
    }
}

/// With Include All Subfolders and no output folder chosen, the results go to the default output
/// folder and a rerun doesn't take them as inputs: `<folder>/droplet-output` for a dropped folder
/// (also when all its images are in subfolders), else the first dropped file's
/// `droplet-output`, inside the dropped folder here.
#[test]
fn the_subfolder_walk_leaves_out_the_droplet_output() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().to_string_lossy().into_owned();
    let input = test_folder(&root);
    let nested = join(&root, "nested");
    std::fs::create_dir_all(join(&nested, "inner")).unwrap();
    image(&join(&nested, "inner/x.png"), 3, 3);
    let dp = Session::new()
        .execute(
            "file.automate.createDroplet",
            json!({"path": join(&root, "Invert"), "steps": [["image.adjustments.invert", {}]], "includeSubfolders": true, "shim": false}),
        )
        .unwrap()["path"]
        .as_str()
        .unwrap()
        .to_string();
    let first_file = join(&input, "sub/a.png");
    let cases = [
        (json!([input]), join(&input, "droplet-output"), vec!["a.png", "b.jpg", "c.psd", "a.png", "d.png"]),
        (json!([nested]), join(&nested, "droplet-output"), vec!["x.png"]),
        (json!([first_file, input]), join(&input, "sub/droplet-output"), vec!["a.png", "a.png", "b.jpg", "c.psd", "a.png", "d.png"]),
    ];
    for (dropped, out, files) in cases {
        let expected: Vec<String> = files.iter().map(|n| join(&out, n)).collect();
        for pass in ["first", "rerun"] {
            let r = run(&dp, dropped.clone());
            assert_eq!(r["errors"], json!([]), "{dropped} {pass}: {r}");
            assert_eq!(r["output"], json!(out), "{dropped} {pass}");
            assert_eq!(r["files"], json!(expected), "{dropped} {pass}");
        }
    }
}

/// Override Action "Open" Commands: on, the action's recorded Open reads each dropped file; off,
/// the recorded Open opens its own (here missing) file and the input fails.
#[test]
fn override_open_makes_the_recorded_open_read_each_dropped_file() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().to_string_lossy().into_owned();
    let input = join(&root, "a.png");
    image(&input, 4, 3);
    let out = join(&root, "out");
    let steps = json!([["file.open", {"path": join(&root, "recorded.png")}], ["image.imageRotation.90cw", {}]]);
    let make = |name: &str, on: bool| {
        let p = json!({"path": join(&root, name), "steps": steps, "output": out, "overrideOpen": on, "shim": false});
        Session::new().execute("file.automate.createDroplet", p).unwrap()["path"].as_str().unwrap().to_string()
    };
    let r = run(&make("On", true), json!([input]));
    assert_eq!(r["errors"], json!([]), "{r}");
    assert_eq!(size(&join(&out, "a.png")), (3, 4), "the dropped 4×3 file, rotated");
    std::fs::remove_dir_all(&out).unwrap();
    let r = run(&make("Off", false), json!([input]));
    assert_eq!(r["files"], json!([]), "{r}");
    assert_eq!(r["errors"].as_array().unwrap().len(), 1, "{r}");
    assert!(!std::path::Path::new(&join(&out, "a.png")).exists());
}

/// The two Suppress options open each dropped file as Batch opens it with them: the droplet's
/// result equals a Batch run's with the same options, for each combination. A camera raw file with
/// a sidecar shows both: Suppress File Open Options Dialogs develops it with the sidecar's
/// settings, and opening it that way asks a color profile question unless Suppress Color Profile
/// Warnings is on too.
#[test]
fn the_suppress_options_open_the_dropped_files_as_batch_does() {
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};
    let root = tempfile::tempdir().unwrap();
    let root = root.path().to_string_lossy().into_owned();
    let raw = join(&root, "raw.dng");
    let (w, h) = (24, 16);
    std::fs::write(&raw, DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 65535)).build()).unwrap();
    let xmp = photocraft_io::camera_raw_xmp::save(&photocraft_algo::camera_raw::CameraRaw { exposure: 1.0, ..Default::default() });
    std::fs::write(join(&root, "raw.xmp"), xmp).unwrap();
    let steps = json!([["image.adjustments.invert", {}]]);
    let mut results = Vec::new();
    for (open_options, warnings) in [(false, false), (true, false), (false, true), (true, true)] {
        let tag = format!("{open_options}-{warnings}");
        let play = json!({"suppressOpenOptions": open_options, "suppressWarnings": warnings});
        let mut p = json!({"path": join(&root, &tag), "steps": steps, "format": "png", "shim": false});
        p.as_object_mut().unwrap().extend(play.as_object().unwrap().clone());
        let dp = Session::new().execute("file.automate.createDroplet", p).unwrap()["path"].as_str().unwrap().to_string();
        let dropped = join(&root, &format!("droplet-{tag}"));
        let r = Session::new().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [raw], "output": dropped})).unwrap();
        let batched = join(&root, &format!("batch-{tag}"));
        let mut bp = json!({"steps": steps, "input": [raw], "output": batched, "format": "png"});
        bp.as_object_mut().unwrap().extend(play.as_object().unwrap().clone());
        let b = Session::new().execute("file.automate.batch", bp).unwrap();
        assert_eq!(r["errors"], b["errors"], "{tag}");
        let read = |dir: &str| std::fs::read(join(dir, "raw.png")).ok();
        assert_eq!(read(&dropped), read(&batched), "{tag}");
        results.push((r["errors"].as_array().unwrap().len(), read(&dropped)));
    }
    let [plain, open_options, warnings, both] = results.try_into().unwrap();
    assert_eq!((plain.0, warnings.0, both.0), (0, 0, 0));
    assert_eq!(open_options.0, 1, "developed with the sidecar's settings, the file asks a color profile question");
    assert!(open_options.1.is_none());
    assert_ne!(plain.1, both.1, "the sidecar's +1 exposure is applied only with Suppress File Open Options Dialogs");
}

/// Wrong-type Play options are errors, never a panic: Create Droplet writes no file, and a droplet
/// file edited by hand doesn't run.
#[test]
fn wrong_type_play_options_are_errors() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().to_string_lossy().into_owned();
    let input = test_folder(&root);
    let path = join(&root, "Bad.pcdroplet");
    for key in ["overrideOpen", "includeSubfolders", "suppressOpenOptions", "suppressWarnings"] {
        for bad in [json!("yes"), json!(1), json!([true]), json!({})] {
            let p = json!({"path": path, "steps": [["image.adjustments.invert", {}]], "shim": false, key: bad});
            assert!(Session::new().execute("file.automate.createDroplet", p).is_err(), "{key}: {bad}");
            assert!(!std::path::Path::new(&path).exists(), "{key}: {bad}");
        }
    }
    let good = droplet(&root, json!({}));
    let text = std::fs::read_to_string(&good).unwrap();
    for key in ["overrideOpen", "includeSubfolders", "suppressOpenOptions", "suppressWarnings"] {
        let mut v: Value = serde_json::from_str(&text).unwrap();
        v["options"][key] = json!("yes");
        std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        let r = Session::new().execute("file.automate.runDroplet", json!({"droplet": path, "input": [input]}));
        assert!(r.is_err(), "{key}: {r:?}");
    }
}
