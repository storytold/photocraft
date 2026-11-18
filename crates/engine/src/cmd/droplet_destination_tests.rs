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
    let dp = droplet(&dir, json!({"destination": "saveAndClose", "overrideSaveAs": true, "output": format!("{dir}/out")}));
    let v: Value = serde_json::from_slice(&std::fs::read(&dp).unwrap()).unwrap();
    assert_eq!(v["options"]["destination"], json!("saveAndClose"), "the destination rides in the droplet");
    assert_eq!(v["options"]["overrideSaveAs"], json!(true), "the tick rides in the droplet");
    assert!(v["options"].get("output").is_none(), "a saveAndClose droplet stores no output folder");

    let none_dp = droplet(&dir, json!({"path": format!("{dir}/NoneDest"), "destination": "none", "output": format!("{dir}/out")}));
    let v: Value = serde_json::from_slice(&std::fs::read(&none_dp).unwrap()).unwrap();
    assert_eq!(v["options"]["destination"], json!("none"));
    assert!(v["options"].get("output").is_none(), "a none droplet stores no output folder");

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
/// folder) still run as Folder. The JSON is the old on-disk shape kept alive by main's own
/// pre-`calledActions` pin (`old.pcdroplet` inside
/// droplets_carry_the_actions_their_play_steps_call, crates/engine/src/cmd/automate_cmds/tests.rs,
/// as it stands at the base c337d6e4e). Guard test: passes before and after.
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
    let listing = |dir: &str| -> Vec<std::path::PathBuf> {
        let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
        files.sort();
        files
    };
    let dp = droplet(&dir, json!({"destination": "none", "steps": [["image.imageRotation.90cw", {}], ["file.save", {}], ["image.adjustments.invert", {}]]}));
    let files_before = listing(&dir);
    let mut s = app_session();
    let r = s.execute("file.automate.runDroplet", json!({"droplet": &dp, "input": [&input]})).unwrap();
    assert_eq!(r["errors"], json!([]), "{r}");
    assert_eq!(r["files"], json!([]));
    // The step recorded after the Save ran too: a run that merely stopped at the save point would
    // end with the rotation as its last state.
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Invert"), "the steps after the recorded Save ran");
    assert_eq!(std::fs::read(&input).unwrap(), before, "the recorded Save wrote nothing over the input");
    // No new file of ANY kind appeared in the test folder (a written copy would be one).
    let fresh: Vec<_> = listing(&dir).into_iter().filter(|p| !files_before.contains(p)).collect();
    assert!(fresh.is_empty(), "the run created no new file anywhere in the folder, got {fresh:?}");
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

/// #13711 review HIGH-1: the None run plays steps in the caller's session, so it must raise and
/// hand back `actions.batch_depth` like Batch raises it for its scratch run. A droplet whose steps
/// run the same droplet then stops at the nesting limit with the shared error text, and the stop
/// reaches the outermost answer marked like Batch marks a child's stop - instead of opening one
/// document per level until the stack or memory runs out. None twin of
/// `a_droplet_that_runs_itself_stops_at_the_nesting_limit` (file_cmds::tests, the folder path).
#[test]
fn a_none_droplet_that_runs_itself_stops_at_the_nesting_limit() {
    let dir = tmp("none-recursion");
    let input = format!("{dir}/a.png");
    input_picture(&input);
    let before = std::fs::read(&input).unwrap();
    let dp = format!("{dir}/self.pcdroplet");
    let step = json!(["file.automate.runDroplet", {"droplet": dp, "input": [input]}]);
    std::fs::write(&dp, json!({"photocraftDroplet": 1, "name": "self", "action": {"steps": [step]}, "options": {"destination": "none"}}).to_string()).unwrap();
    let mut s = app_session();
    let r = s.execute("file.automate.runDroplet", json!({"droplet": &dp, "input": [&input]})).unwrap();
    assert!(r["errors"][0]["error"].as_str().unwrap().contains("nested more than"), "{r}");
    assert_eq!(r["errors"][0]["limit"], json!(true), "the stop is marked, as Batch marks its children's stops");
    assert_eq!(r["files"], json!([]), "{r}");
    assert_eq!(s.actions.batch_depth, 0, "the None run hands the caller's nesting depth back");
    assert_eq!(std::fs::read(&input).unwrap(), before, "nothing was written at any level");
}

/// #13711 review HIGH-2: the None run plays from the actions the droplet captured (its
/// `calledActions`, #2794), installing them for its steps as the folder path forwards them to
/// Batch. The app's live list here holds an unrelated action of the same name - it must not play.
#[test]
fn a_none_droplet_plays_the_action_it_captured_not_the_live_list() {
    use crate::actions_cmds::{Action, ActionStep};

    let dir = tmp("captured");
    let input = format!("{dir}/a.png");
    input_picture(&input);
    let mut maker = Session::new();
    maker.actions.list.push(Action::new("Inner", vec![ActionStep::new("image.imageRotation.90cw", json!({}))])).unwrap();
    maker.actions.list.push(Action::new("Top", vec![ActionStep::new("actions.play", json!({"action": "Inner"}))])).unwrap();
    let r = maker.execute("file.automate.createDroplet", json!({"path": format!("{dir}/Top"), "action": "Top", "destination": "none", "shim": false})).unwrap();
    let dp = r["path"].as_str().unwrap().to_string();
    let v: Value = serde_json::from_slice(&std::fs::read(&dp).unwrap()).unwrap();
    assert!(v["calledActions"].as_array().is_some_and(|a| a.iter().any(|ac| ac["name"] == "Inner")), "the droplet captured Inner: {v}");

    let mut s = app_session();
    s.actions.list.push(Action::new("Inner", vec![ActionStep::new("edit.fill", json!({"color": "#000000"}))])).unwrap();
    let r = s.execute("file.automate.runDroplet", json!({"droplet": &dp, "input": [&input]})).unwrap();
    assert_eq!(r["errors"], json!([]), "{r}");
    let st = s.active().unwrap();
    assert_eq!((st.doc.size.width, st.doc.size.height), (2, 4), "the captured Inner rotated; the live same-named fill did not play");
}

/// #13711 review MEDIUM-3: the None run opens each input through the shared open core (as File >
/// Open and Batch's scratch run do), not a bare import: a developed raw takes Preferences >
/// Raw Defaults > Resolution. (Chosen cheap honest neighbour: the raw ppi; the profiled-JPEG
/// colour policy rides in the same call - `open_bytes_as` runs the Color Settings policy through
/// `Session::open_document` - but its pin would need an ICC fixture, so it is not re-pinned here.)
#[test]
fn a_none_droplet_opens_a_raw_at_the_raw_defaults_resolution() {
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};

    let dir = tmp("raw-ppi");
    let input = format!("{dir}/IMG.dng");
    std::fs::write(&input, DngSpec::cfa(24, 16, mosaic(&scene(24, 16), 24, [0, 1, 1, 2], 0, 65535)).build()).unwrap();
    let dp = droplet(&dir, json!({"path": format!("{dir}/Invert"), "destination": "none", "steps": [["image.adjustments.invert", {}]]}));
    let mut s = app_session();
    s.execute("prefs.set", json!({"path": "rawDefaults.resolution", "value": 240})).unwrap();
    let r = s.execute("file.automate.runDroplet", json!({"droplet": &dp, "input": [&input]})).unwrap();
    assert_eq!(r["errors"], json!([]), "{r}");
    assert_eq!(r["files"], json!([]));
    let st = s.active().unwrap();
    assert_eq!(st.doc.resolution_dpi, 240.0, "the raw opened at the Raw Defaults ppi, as File > Open does");
    assert!(st.is_dirty(), "the opened raw stays open and changed");
}

/// #13711 review MEDIUM-4: a none droplet gets no Unix `.command` shim, even one asked for: the
/// shim runs `photocraft-cli droplet`, and a headless run of a none droplet is always refused, so
/// the launcher could only ever fail. A folder droplet still gets the shim as today.
#[test]
fn a_none_droplet_gets_no_shim_even_when_one_is_asked_for() {
    let dir = tmp("shim-none");
    let r = Session::new()
        .execute(
            "file.automate.createDroplet",
            json!({"path": format!("{dir}/NoShim"), "steps": [["image.adjustments.invert", {}]], "destination": "none", "shim": true}),
        )
        .unwrap();
    assert_eq!(r["shim"], Value::Null, "no launcher for a run that can only be refused: {r}");
    assert!(!std::path::Path::new(&format!("{dir}/NoShim.command")).exists());

    let r = Session::new()
        .execute(
            "file.automate.createDroplet",
            json!({"path": format!("{dir}/WithShim"), "steps": [["image.adjustments.invert", {}]], "destination": "folder", "output": format!("{dir}/out"), "shim": true}),
        )
        .unwrap();
    assert!(r["shim"].is_string(), "a folder droplet still gets the shim: {r}");
    assert!(std::path::Path::new(r["shim"].as_str().unwrap()).exists(), "the shim file is written");
}
