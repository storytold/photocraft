//! The Create Droplet Destination (#13711): the droplet stores `destination` (none, folder or
//! saveAndClose), and `file.automate.runDroplet` runs it: Save and Close overwrites each input in
//! its own format, Folder writes its copy as before, None keeps each document open and changed in
//! the app and writes nothing (a headless session refuses it). A droplet written before the change
//! (no `destination` stored, an `output` folder) still runs as Folder.

use super::*;

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-droplet-dest-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

/// A 4x2 gray PNG at `path` (or a JPEG of it, by extension); the run's steps rotate it, so the
/// written shape tells what ran.
fn input_picture(path: &str) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 2, "background": "#808080"})).unwrap();
    crate::file_cmds::save_doc(&s.active().unwrap().doc, path, None).unwrap();
}

/// `dir/Rotate.pcdroplet` for one rotation step, with `extra` merged into the command params.
fn droplet(dir: &str, extra: Value) -> String {
    let mut p = json!({"path": format!("{dir}/Rotate"), "steps": [["image.imageRotation.90cw", {}]], "shim": false});
    if let Value::Object(map) = extra {
        p.as_object_mut().unwrap().extend(map);
    }
    let r = Session::new().execute("file.automate.createDroplet", p).unwrap();
    r["path"].as_str().unwrap().to_string()
}

/// A session with an attached preset store: the desktop and web apps attach one once loaded
/// (`crate::preset_store`); headless CLI/MCP sessions and plain tests never do.
fn app_session() -> Session {
    let mut s = Session::new();
    s.attach_preset_store(crate::preset_store::open(Box::new(crate::preset_store::MemBackend::default())));
    s
}

/// #13711: createDroplet stores the Destination the form chose, and an Override Action "Save As"
/// Commands tick, in the droplet; a call that names neither keeps storing no such keys (as before).
#[test]
fn create_droplet_stores_destination_and_the_override_tick() {
    let dir = tmp("store");
    let dp = droplet(&dir, json!({"destination": "saveAndClose", "overrideSaveAs": true}));
    let v: Value = serde_json::from_slice(&std::fs::read(&dp).unwrap()).unwrap();
    assert_eq!(v["options"]["destination"], json!("saveAndClose"), "the destination rides in the droplet");
    assert_eq!(v["options"]["overrideSaveAs"], json!(true), "the tick rides in the droplet");

    let plain = droplet(&dir, json!({}));
    let v: Value = serde_json::from_slice(&std::fs::read(&plain).unwrap()).unwrap();
    assert!(v["options"].get("destination").is_none(), "an omitted destination stores no key, as before");
    assert!(v["options"].get("overrideSaveAs").is_none(), "an omitted box stores no key, as before");

    let e = Session::new()
        .execute(
            "file.automate.createDroplet",
            json!({"path": format!("{dir}/Bad"), "steps": [["image.adjustments.invert", {}]], "destination": "banana", "shim": false}),
        )
        .unwrap_err();
    assert!(e.to_string().contains("destination"), "{e}");
}

/// #13711 Done-when: droplets written before this change (no destination stored, an `output`
/// folder) still run as Folder. The JSON is the shape main's createDroplet wrote at 0fb40c534a
/// (the stored-old-droplet case in automate_cmds::tests). Guard test: passes before and after.
#[test]
fn a_droplet_without_a_destination_key_still_runs_as_folder() {
    let dir = tmp("legacy");
    std::fs::create_dir_all(format!("{dir}/in")).unwrap();
    let input = format!("{dir}/in/a.png");
    input_picture(&input);
    let before = std::fs::read(&input).unwrap();
    let dp = format!("{dir}/old.pcdroplet");
    std::fs::write(&dp, json!({"photocraftDroplet": 1, "action": {"steps": [["image.imageRotation.90cw", {}]]}, "options": {"output": format!("{dir}/out-old"), "format": "png"}}).to_string()).unwrap();
    let r = Session::new().execute("file.automate.runDroplet", json!({"droplet": &dp, "input": [format!("{dir}/in")]})).unwrap();
    assert_eq!(r["errors"], json!([]), "{r}");
    assert_eq!(r["output"], json!(format!("{dir}/out-old")), "{r}");
    assert_eq!(std::fs::read(&input).unwrap(), before, "the input keeps its bytes: the copy went to the output folder");
    let out = photocraft_codecs::decode(&std::fs::read(format!("{dir}/out-old/a.png")).unwrap()).unwrap();
    assert_eq!(out.dimensions(), (2, 4), "the rotation ran and wrote into the stored output folder");
}

/// #13711: Destination Folder with the key stored behaves exactly like the key-less droplet: the
/// copy is written to the stored output folder, the input untouched.
#[test]
fn a_droplet_with_destination_folder_writes_its_copy_as_before() {
    let dir = tmp("folder");
    let input = format!("{dir}/a.png");
    input_picture(&input);
    let before = std::fs::read(&input).unwrap();
    let dp = droplet(&dir, json!({"destination": "folder", "output": format!("{dir}/out"), "format": "png"}));
    let r = Session::new().execute("file.automate.runDroplet", json!({"droplet": &dp, "input": [&input]})).unwrap();
    assert_eq!(r["errors"], json!([]), "{r}");
    assert_eq!(r["output"], json!(format!("{dir}/out")), "{r}");
    assert_eq!(std::fs::read(&input).unwrap(), before, "the input is not touched");
    let out = photocraft_codecs::decode(&std::fs::read(format!("{dir}/out/a.png")).unwrap()).unwrap();
    assert_eq!(out.dimensions(), (2, 4));
}

/// #13711: Destination Save and Close overwrites each input in its own format (as Batch's Save and
/// Close, #13659): each input file itself becomes the rotated picture, and no output folder appears.
#[test]
fn destination_save_and_close_overwrites_each_input_in_its_own_format() {
    let dir = tmp("save-close");
    let (a, b) = (format!("{dir}/a.png"), format!("{dir}/b.jpg"));
    input_picture(&a);
    input_picture(&b);
    let dp = droplet(&dir, json!({"destination": "saveAndClose"}));
    let r = Session::new().execute("file.automate.runDroplet", json!({"droplet": &dp, "input": [&a, &b]})).unwrap();
    assert_eq!(r["errors"], json!([]), "{r}");
    for (path, name) in [(&a, "a.png"), (&b, "b.jpg")] {
        let out = photocraft_codecs::decode(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(out.dimensions(), (2, 4), "{name}: the input itself holds the rotated picture");
    }
    assert!(!std::path::Path::new(&format!("{dir}/out")).exists(), "Save and Close takes no output folder");
}

/// #13711: a saveAndClose droplet whose stored format is not the source's own fails on Batch's own
/// Save and Close rule, before any file is written (the rule measured for #13659, passed on).
#[test]
fn destination_save_and_close_refuses_a_different_stored_format_without_writing() {
    let dir = tmp("save-close-format");
    let input = format!("{dir}/a.png");
    input_picture(&input);
    let before = std::fs::read(&input).unwrap();
    let dp = droplet(&dir, json!({"destination": "saveAndClose", "format": "jpg"}));
    let e = Session::new().execute("file.automate.runDroplet", json!({"droplet": &dp, "input": [&input]})).unwrap_err();
    assert!(e.to_string().contains("format"), "{e}");
    assert_eq!(std::fs::read(&input).unwrap(), before, "refused before writing anything");
}

/// #13711: Destination None, run in the app: each input opens as a document that stays open and
/// changed, its last History state is the action's step, and nothing is written anywhere.
#[test]
fn destination_none_keeps_each_opened_document_changed_and_writes_nothing() {
    let dir = tmp("none-app");
    let input = format!("{dir}/a.png");
    input_picture(&input);
    let before = std::fs::read(&input).unwrap();
    let dp = droplet(&dir, json!({"destination": "none"}));
    let mut s = app_session();
    let r = s.execute("file.automate.runDroplet", json!({"droplet": &dp, "input": [&input]})).unwrap();
    assert_eq!(r["errors"], json!([]), "{r}");
    assert_eq!(r["files"], json!([]), "nothing is written: {r}");
    assert_eq!(r["output"], Value::Null, "the run names no output folder: {r}");
    assert_eq!(std::fs::read(&input).unwrap(), before, "the input file keeps its bytes");
    assert_eq!(s.documents().len(), 1, "the document stays open");
    let st = s.active().unwrap();
    assert!(st.is_dirty(), "the tab shows the changed mark");
    assert_eq!(st.history.undo_label(), Some("Rotate Canvas"), "the last History state is the action's step");
    assert!(
        !std::path::Path::new(&format!("{dir}/out")).exists() && !std::path::Path::new(&format!("{dir}/droplet-output")).exists(),
        "None writes no folder at all"
    );
}

/// #13711: None with recorded Save steps still writes nothing: with no destination they mark
/// nothing, so they are skipped and the document stays changed (as Batch with no destination).
#[test]
fn destination_none_skips_the_recorded_save_and_still_writes_nothing() {
    let dir = tmp("none-save");
    let input = format!("{dir}/a.png");
    input_picture(&input);
    let before = std::fs::read(&input).unwrap();
    let dp = droplet(&dir, json!({"destination": "none", "steps": [["image.imageRotation.90cw", {}], ["file.saveAs", {}], ["file.saveACopy", {}]]}));
    let mut s = app_session();
    let r = s.execute("file.automate.runDroplet", json!({"droplet": &dp, "input": [&input]})).unwrap();
    assert_eq!(r["errors"], json!([]), "{r}");
    assert_eq!(r["files"], json!([]));
    assert_eq!(std::fs::read(&input).unwrap(), before, "the recorded Save wrote nothing over the input");
    let written = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| matches!(e.path().extension().and_then(|x| x.to_str()), Some("jpg") | Some("psd") | Some("pcraft")));
    assert!(!written, "the recorded Save wrote no copy anywhere in the test folder");
    assert!(s.active().unwrap().is_dirty(), "the document stays changed");
}

/// #13711: a run that names an output override still writes nothing when the droplet's Destination
/// is None: the stored destination decides, the override cannot reintroduce a write.
#[test]
fn destination_none_ignores_a_call_output_override_and_writes_nothing() {
    let dir = tmp("none-override");
    let input = format!("{dir}/a.png");
    input_picture(&input);
    let dp = droplet(&dir, json!({"destination": "none"}));
    let mut s = app_session();
    let r = s.execute("file.automate.runDroplet", json!({"droplet": &dp, "input": [&input], "output": format!("{dir}/forced")})).unwrap();
    assert_eq!(r["files"], json!([]), "{r}");
    assert!(!std::path::Path::new(&format!("{dir}/forced")).exists(), "the override folder was not even created");
}

/// #13711: headless (the `photocraft-cli droplet` kind: a bare session, no preset store attached)
/// the run returns an error saying None needs the app, instead of writing anything.
#[test]
fn destination_none_is_refused_when_no_app_is_involved() {
    let dir = tmp("none-headless");
    let input = format!("{dir}/a.png");
    input_picture(&input);
    let before = std::fs::read(&input).unwrap();
    let dp = droplet(&dir, json!({"destination": "none"}));
    let e = Session::new().execute("file.automate.runDroplet", json!({"droplet": &dp, "input": [&input]})).unwrap_err();
    assert!(e.to_string().contains("app"), "the error names the app: {e}");
    assert_eq!(std::fs::read(&input).unwrap(), before, "nothing was written");
    let droplets_and_images =
        std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.path().extension().is_some_and(|x| x == "pcdroplet" || x == "png")).count();
    assert_eq!(droplets_and_images, 2, "only the droplet and its one input sit in the folder");
}

/// #13711: a droplet file carrying an unknown destination is refused instead of guessed (fail
/// closed), and nothing is written.
#[test]
fn an_unknown_stored_destination_is_refused_without_writing() {
    let dir = tmp("bad-dest");
    let input = format!("{dir}/a.png");
    input_picture(&input);
    let before = std::fs::read(&input).unwrap();
    let dp = format!("{dir}/bad.pcdroplet");
    std::fs::write(&dp, json!({"photocraftDroplet": 1, "action": {"steps": [["image.imageRotation.90cw", {}]]}, "options": {"destination": "banana", "output": format!("{dir}/out"), "format": "png"}}).to_string()).unwrap();
    let e = Session::new().execute("file.automate.runDroplet", json!({"droplet": &dp, "input": [&input]})).unwrap_err();
    assert!(e.to_string().contains("destination"), "{e}");
    assert_eq!(std::fs::read(&input).unwrap(), before, "refused before writing");
    assert!(!std::path::Path::new(&format!("{dir}/out")).exists());
}

/// #13711: an Override Action "Save As" Commands tick stored in the droplet does not change the
/// run yet (not measured for droplets; for Batch see #13659): the folder run writes its copy as
/// today, exactly like the same droplet without the tick.
#[test]
fn a_stored_override_tick_does_not_change_the_run() {
    let dir = tmp("override-run");
    let (ticked, plain) = (format!("{dir}/a.png"), format!("{dir}/b.png"));
    input_picture(&ticked);
    input_picture(&plain);
    let with = droplet(
        &dir,
        json!({"path": format!("{dir}/With"), "destination": "folder", "output": format!("{dir}/out-with"), "format": "png", "overrideSaveAs": true}),
    );
    let without = droplet(&dir, json!({"path": format!("{dir}/Without"), "destination": "folder", "output": format!("{dir}/out-without"), "format": "png"}));
    let mut s = Session::new();
    let a = s.execute("file.automate.runDroplet", json!({"droplet": &with, "input": [&ticked]})).unwrap();
    assert_eq!(a["errors"], json!([]), "{a}");
    let b = s.execute("file.automate.runDroplet", json!({"droplet": &without, "input": [&plain]})).unwrap();
    assert_eq!(b["errors"], json!([]), "{b}");
    let x = photocraft_codecs::decode(&std::fs::read(format!("{dir}/out-with/a.png")).unwrap()).unwrap();
    let y = photocraft_codecs::decode(&std::fs::read(format!("{dir}/out-without/b.png")).unwrap()).unwrap();
    assert_eq!((x.dimensions(), y.dimensions()), ((2, 4), (2, 4)), "both write the folder copy as before");
}
