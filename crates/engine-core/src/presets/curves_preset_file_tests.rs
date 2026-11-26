//! The Curves dialog's gear menu (#14379) from the engine side: the preset file, the user preset
//! list, and the `curves.presets.*` commands the dialog, the CLI, the control channel and MCP run.

use photocraft_doc::{ColorMode, LayerContent};
use serde_json::{Value, json};

use super::PresetsExt;
use super::curves::{self, CurvePreset, EXT, MAX_FILE_BYTES, from_bytes, to_bytes};
use crate::adjust_params;

/// A composite curve with a lifted midtone and three different channel curves, so a channel lost
/// between the file and the dialog shows up.
fn curves_params() -> Value {
    json!({
        "points": [[0.0, 0.0], [100.0, 150.0], [255.0, 255.0]],
        "red": [[0.0, 20.0], [255.0, 235.0]],
        "green": [[0.0, 0.0], [128.0, 100.0], [255.0, 255.0]],
        "blue": [[0.0, 0.0], [255.0, 200.0]],
    })
}

/// A composite curve on its own: the smallest thing a preset may hold.
fn one_curve() -> Value {
    json!({"points": [[0.0, 0.0], [255.0, 200.0]]})
}

/// A 256-level pencil map, lifted by 10 %.
fn pencil_map() -> Vec<f64> {
    (0..=255).map(|k| (k as f64 * 1.1).min(255.0)).collect()
}

/// A temp folder one test owns, named after the test and this process.
fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("photocraft-curves-preset-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp folder");
    d
}

/// A path in `dir(name)`; the folder exists, the file need not.
fn path_in(name: &str, file: &str) -> String {
    dir(name).join(file).to_string_lossy().to_string()
}

fn save(s: &mut crate::Session, p: &Value) -> crate::Result<Value> {
    s.execute("curves.presets.save", p.clone())
}

fn load(s: &mut crate::Session, p: &Value) -> crate::Result<Value> {
    s.execute("curves.presets.load", p.clone())
}

fn list(s: &mut crate::Session) -> Value {
    s.execute("curves.presets.list", json!({})).expect("the list always answers")
}

fn delete(s: &mut crate::Session, p: &Value) -> crate::Result<Value> {
    s.execute("curves.presets.delete", p.clone())
}

/// The user list: (name, curves).
fn stored(s: &crate::Session) -> Vec<(String, Value)> {
    s.presets().curves_presets.iter().map(|p| (p.name.clone(), p.curves.clone())).collect()
}

/// The curves `params` hold, in the form `image.adjustments.curves` writes them back.
fn parsed(params: &Value) -> Value {
    adjust_params::to_params(&adjust_params::from_params("curves", params, None, ColorMode::Rgb).expect("the parameters parse"))
}

/// The curves of the first entry of `curves.presets.list`.
fn listed_curves(s: &mut crate::Session) -> Value {
    list(s)["presets"][0]["curves"].clone()
}

// ------------------------------------------------------------------ the file itself

#[test]
fn a_preset_file_holds_every_channel_of_the_curves() {
    let bytes = to_bytes(&CurvePreset { name: "My Curve".into(), curves: curves_params() }).expect("a preset writes");
    let text = String::from_utf8(bytes.clone()).expect("the file is UTF-8 text");
    assert!(text.contains("photoCraftCurvePreset"), "the file says what it is: {text}");
    let back = from_bytes(&bytes).expect("the file reads back");
    assert_eq!(back.name, "My Curve");
    for key in ["points", "red", "green", "blue"] {
        assert_eq!(back.curves[key], parsed(&curves_params())[key], "{key} comes back as the Curves command reads it");
    }
}

#[test]
fn a_pencil_curve_round_trips_as_a_drawn_map() {
    let params = json!({"points": pencil_map(), "pencil": true});
    let back = from_bytes(&to_bytes(&CurvePreset { name: "Drawn".into(), curves: params.clone() }).expect("writes")).expect("reads");
    assert_eq!(back.curves["pencil"], json!(true), "the file keeps the curve drawn");
    assert_eq!(back.curves["points"], parsed(&params)["points"], "a 256-level map comes back level for level");
}

#[test]
fn an_ink_preset_keeps_its_channels() {
    let cmyk = json!({"points": [[0.0, 10.0], [255.0, 245.0]], "cyan": [[0.0, 0.0], [255.0, 200.0]], "magenta": [[0.0, 30.0], [255.0, 255.0]]});
    let back = from_bytes(&to_bytes(&CurvePreset { name: "Print".into(), curves: cmyk.clone() }).expect("writes")).expect("reads");
    assert_eq!(back.curves["cyan"], parsed(&cmyk)["cyan"], "the ink channels survive the file");
    assert_eq!(back.curves, parsed(&cmyk), "and they are the curves the Curves command takes");
}

#[test]
fn a_malformed_preset_file_is_an_error_not_a_crash() {
    let good = to_bytes(&CurvePreset { name: "My Curve".into(), curves: curves_params() }).expect("writes");
    let short_map = pencil_map()[..200].to_vec();
    for (what, bytes) in [
        ("an empty file", Vec::new()),
        ("text that is not JSON", b"not a preset at all".to_vec()),
        ("truncated JSON", good[..good.len() / 2].to_vec()),
        ("no version key", br#"{"curves":{"points":[[0,0],[255,255]]}}"#.to_vec()),
        ("another version", br#"{"photoCraftCurvePreset":99,"curves":{"points":[[0,0],[255,255]]}}"#.to_vec()),
        ("curves that is a string", br#"{"photoCraftCurvePreset":1,"curves":"lift"}"#.to_vec()),
        ("no curve at all", br#"{"photoCraftCurvePreset":1,"curves":{}}"#.to_vec()),
        ("only a key that is not a curve", br#"{"photoCraftCurvePreset":1,"curves":{"smooth":2}}"#.to_vec()),
        ("a curve with one point", br#"{"photoCraftCurvePreset":1,"curves":{"points":[[0,0]]}}"#.to_vec()),
        ("a point outside 0..255", br#"{"photoCraftCurvePreset":1,"curves":{"points":[[0,0],[300,255]]}}"#.to_vec()),
        ("a point that is not a pair", br#"{"photoCraftCurvePreset":1,"curves":{"points":[[0,0],[255]]}}"#.to_vec()),
        ("a channel that is a string", br#"{"photoCraftCurvePreset":1,"curves":{"red":"bright"}}"#.to_vec()),
        ("a pencil map of the wrong length", json!({"photoCraftCurvePreset": 1, "curves": {"points": short_map, "pencil": true}}).to_string().into_bytes()),
        ("far too many points", json!({"photoCraftCurvePreset": 1, "curves": {"points": vec![[0.0, 0.0]; 5000]}}).to_string().into_bytes()),
        ("JSON that is not an object", br#"[1,2,3]"#.to_vec()),
        ("a name that is not a string", br#"{"photoCraftCurvePreset":1,"name":7,"curves":{"points":[[0,0],[255,255]]}}"#.to_vec()),
    ] {
        assert!(from_bytes(&bytes).is_err(), "{what} is refused");
    }
}

// ------------------------------------------------------------------ the commands

#[test]
fn save_writes_every_channel_to_a_file_and_load_reads_it_back() {
    let path = path_in("save-load", "My Curve.pcrv");
    let mut s = crate::session();
    let r = save(&mut s, &json!({"curves": curves_params(), "path": path})).expect("a save works");
    assert_eq!(r["name"], "My Curve", "a saved preset takes the file's name");
    assert_eq!(r["path"], path.as_str());
    assert!(std::fs::metadata(&path).is_ok(), "the file is on disk");

    // A fresh session: the curves come back from the file alone.
    let mut t = crate::session();
    let r = load(&mut t, &json!({"path": path})).expect("the file loads");
    assert_eq!(r["name"], "My Curve");
    for key in ["points", "red", "green", "blue"] {
        assert_eq!(r["curves"][key], parsed(&curves_params())[key], "{key} came back");
    }
    // The same file as inline bytes, the way the control channel and MCP pass them.
    let bytes = std::fs::read(&path).expect("the file is readable");
    let inline = load(&mut t, &json!({"data": photocraft_paint::tile::b64_encode(&bytes)})).expect("inline bytes load");
    assert_eq!(inline["curves"], r["curves"], "inline bytes read the same curves");
}

#[test]
fn a_saved_preset_without_a_path_returns_its_bytes() {
    let mut s = crate::session();
    let r = save(&mut s, &json!({"curves": curves_params(), "name": "Bytes"})).expect("a save works");
    assert_eq!(r["name"], "Bytes");
    assert!(r.get("path").is_none(), "no file was asked for, none is written");
    let bytes = photocraft_paint::tile::b64_decode(r["data"].as_str().expect("the answer carries data")).expect("the data is base64");
    assert_eq!(from_bytes(&bytes).expect("the bytes read back").curves, parsed(&curves_params()));
}

#[test]
fn the_listed_curves_run_through_the_curves_command() {
    let mut s = crate::session();
    save(&mut s, &json!({"curves": curves_params(), "name": "Mine"})).expect("a save works");
    let curves = listed_curves(&mut s);
    assert_eq!(curves, parsed(&curves_params()), "the list holds the curves in the command's own form");
    s.execute("file.new", json!({"width": 8, "height": 8})).expect("a document");
    s.execute("layer.newAdjustmentLayer.curves", curves.clone()).expect("the preset curves make a Curves layer");
    let state = s.active().expect("the document");
    let id = state.active_layer.expect("the new layer");
    let LayerContent::Adjustment(adj) = &state.doc.layer(id).expect("the layer").content else { panic!("the layer holds an adjustment") };
    let applied = adjust_params::to_params(adj);
    for key in ["points", "red", "green", "blue"] {
        assert_eq!(applied[key], parsed(&curves_params())[key], "{key} reaches the document");
    }
}

#[test]
fn a_loaded_or_saved_user_preset_is_current_and_can_be_deleted() {
    let mut s = crate::session();
    assert!(list(&mut s)["current"].is_null(), "a session that never touched a preset has no current one");
    let path = path_in("current", "Loaded.pcrv");
    save(&mut s, &json!({"curves": curves_params(), "path": path})).expect("a save writes the file");
    assert_eq!(list(&mut s)["current"], "Loaded", "the saved preset is the current one");
    save(&mut s, &json!({"curves": one_curve(), "name": "Saved"})).expect("a save works");
    assert_eq!(list(&mut s)["current"], "Saved", "the second save makes its own preset current");
    load(&mut s, &json!({"path": path})).expect("the same file loads again");
    assert_eq!(list(&mut s)["current"], "Loaded 2", "the loaded preset gets a name of its own");
    assert_eq!(stored(&s).len(), 3, "every saved and loaded preset is in the user list");
    assert_eq!(delete(&mut s, &json!({"name": "Saved"})).expect("delete by name works")["deleted"], "Saved");
    assert_eq!(list(&mut s)["current"], "Loaded 2", "deleting another preset leaves the current one alone");
}

#[test]
fn deleting_the_current_preset_leaves_nothing_current_and_keeps_the_file() {
    let mut s = crate::session();
    let path = path_in("delete-current", "Kept.pcrv");
    save(&mut s, &json!({"curves": curves_params(), "path": path})).expect("a save writes the file");
    assert_eq!(delete(&mut s, &json!({})).expect("Delete Current Preset")["deleted"], "Kept");
    assert!(list(&mut s)["current"].is_null(), "with the current preset deleted, Default is current again");
    assert!(stored(&s).is_empty(), "the list is empty");
    assert!(std::fs::metadata(&path).is_ok(), "deleting a preset leaves the user's file on disk");
}

#[test]
fn delete_of_the_default_or_an_unknown_preset_is_an_error() {
    let mut s = crate::session();
    for p in [json!({}), json!({"name": "Default"}), json!({"name": "Lighter"}), json!({"name": 3})] {
        assert!(delete(&mut s, &p).is_err(), "{p}: there is no user preset to delete");
    }
    save(&mut s, &json!({"curves": one_curve(), "name": "Mine"})).expect("a save works");
    assert_eq!(delete(&mut s, &json!({"name": "Mine"})).expect("delete works")["deleted"], "Mine");
    assert!(list(&mut s)["presets"].as_array().is_some_and(|a| a.is_empty()), "the list is empty again");
}

#[test]
fn two_presets_of_one_name_stay_addressable() {
    let mut s = crate::session();
    for at in 0..3 {
        let r = save(&mut s, &json!({"curves": one_curve(), "name": "Same"})).expect("a save works");
        assert_eq!(r["name"], if at == 0 { "Same".to_string() } else { format!("Same {}", at + 1) }, "{at}: a name of its own");
    }
    assert_eq!(list(&mut s)["count"], 3);
    assert_eq!(list(&mut s)["presets"].as_array().expect("a list").iter().filter(|p| p["name"] == "Same").count(), 1, "one entry keeps the plain name");
}

#[test]
fn a_malformed_truncated_or_oversized_file_is_refused_without_touching_the_list() {
    let mut s = crate::session();
    save(&mut s, &json!({"curves": one_curve(), "name": "Mine"})).expect("a save works");
    let before = stored(&s);
    let d = dir("malformed");
    let whole = to_bytes(&CurvePreset { name: "Whole".into(), curves: curves_params() }).expect("writes");
    for (name, bytes) in [
        ("bad.pcrv", b"{ not json".to_vec()),
        ("empty.pcrv", Vec::new()),
        ("half.pcrv", whole[..whole.len() / 2].to_vec()),
        ("big.pcrv", vec![b' '; MAX_FILE_BYTES as usize + 1]),
    ] {
        let path = d.join(name).to_string_lossy().to_string();
        std::fs::write(&path, &bytes).expect("the test can write");
        assert!(load(&mut s, &json!({"path": path})).is_err(), "{name} is refused");
        assert_eq!(stored(&s), before, "{name} leaves the user list alone");
    }
    // Inline bytes meet the same cap.
    let big = photocraft_paint::tile::b64_encode(&vec![b' '; MAX_FILE_BYTES as usize + 1]);
    assert!(load(&mut s, &json!({"data": big})).is_err(), "an over-cap inline file is refused");
    assert_eq!(stored(&s), before);
}

#[test]
fn bad_params_are_errors_not_crashes() {
    let mut s = crate::session();
    // A path whose parent is a plain file cannot hold the preset.
    let blocker = dir("save-blocked").join("blocker");
    std::fs::write(&blocker, b"not a folder").expect("the test can write");
    let under_a_file = format!("{}/Curve.pcrv", blocker.to_string_lossy());
    for p in [
        json!({}),
        json!({"curves": "lift"}),
        json!({"curves": {}}),
        json!({"curves": {"points": [[0, 0], [100, 150], [255, 255]], "red": "bright"}}),
        json!({"curves": curves_params(), "name": 3}),
        json!({"curves": curves_params(), "name": "   "}),
        json!({"curves": curves_params(), "path": ""}),
        json!({"curves": curves_params(), "path": under_a_file}),
    ] {
        assert!(save(&mut s, &p).is_err(), "{p}");
        assert!(stored(&s).is_empty(), "{p} adds nothing to the user list");
    }
    for p in [json!({}), json!({"path": "/no/such/file.pcrv"}), json!({"data": "!!not base64!!"}), json!({"data": 3}), json!({"path": 5})] {
        assert!(load(&mut s, &p).is_err(), "{p}");
    }
    assert!(list(&mut s)["presets"].as_array().is_some_and(|a| a.is_empty()), "and the list stayed empty");
}

#[test]
fn curve_presets_persist_with_the_preferences() {
    let mut s = crate::session();
    save(&mut s, &json!({"curves": curves_params(), "name": "Mine"})).expect("a save works");
    save(&mut s, &json!({"curves": one_curve(), "name": "Second"})).expect("a save works");
    let saved = s.presets().to_json(&s);

    let mut t = crate::session();
    t.load_presets_json(saved.clone());
    assert_eq!(stored(&t).len(), 2, "both presets come back");
    assert_eq!(list(&mut t)["current"], "Second", "and so does the current one");

    // A hand-edited preferences file: unusable entries are skipped, and one broken set must not
    // discard the other preset kinds (#5602).
    let presets = saved.as_object().expect("a presets object").clone();
    let mut damaged = presets["curves_presets"].as_array().expect("a saved list").clone();
    damaged.push(json!({"name": "Bad", "curves": "lift"}));
    damaged.push(json!("nope"));
    let mut u = crate::session();
    u.load_presets_json(json!({"gradients": presets["gradients"].clone(), "curves_presets": damaged, "curves_preset": presets["curves_preset"].clone()}));
    assert_eq!(stored(&u).len(), 2, "only the usable presets are kept");
    assert_eq!(stored(&u)[1].0, "Second");
    assert_eq!(u.presets().curves_preset.as_deref(), Some("Second"), "the current preset survives the damage");
    assert!(!u.presets().gradients.is_empty(), "the other preset sets survive");
}

#[test]
fn a_broken_current_preset_name_is_ignored() {
    let mut s = crate::session();
    save(&mut s, &json!({"curves": one_curve(), "name": "Mine"})).expect("a save works");
    let saved = s.presets().to_json(&s);
    let mut t = crate::session();
    t.load_presets_json(json!({"curves_presets": saved["curves_presets"], "curves_preset": 7}));
    assert_eq!(stored(&t).len(), 1, "the list still loads");
    assert!(t.presets().curves_preset.is_none(), "a name that is not a string is ignored");
    assert!(list(&mut t)["current"].is_null(), "so Default is current");
}

#[test]
fn a_preset_of_another_colour_space_is_refused_naming_the_channels() {
    let cmyk = json!({"points": [[0.0, 10.0], [255.0, 245.0]], "cyan": [[0.0, 0.0], [255.0, 200.0]], "magenta": [[0.0, 30.0], [255.0, 255.0]]});
    let mut s = crate::session();
    let bytes = to_bytes(&CurvePreset { name: "Ink".into(), curves: parsed(&cmyk) }).expect("an ink preset writes");
    let data = json!(photocraft_paint::tile::b64_encode(&bytes));

    let e = load(&mut s, &json!({"data": data, "space": "rgb"})).unwrap_err().to_string();
    for key in ["cyan", "magenta"] {
        assert!(e.contains(key), "the message names the channel that has no place here: {e}");
    }
    assert!(e.contains("rgb"), "and says which dialog they do not fit: {e}");
    assert_eq!(list(&mut s)["count"], 0, "a refused load adds nothing to the user list");
    assert!(s.presets().curves_preset.is_none(), "and selects nothing");

    // The same file in a dialog of its own space, and a preset of only the composite anywhere.
    let r = load(&mut s, &json!({"data": data, "space": "cmyk"})).expect("ink curves in a cmyk dialog load");
    assert_eq!(r["curves"]["cyan"], parsed(&cmyk)["cyan"], "the ink curves come through");
    let plain = json!({"data": photocraft_paint::tile::b64_encode(&to_bytes(&CurvePreset { name: "Plain".into(), curves: one_curve() }).expect("a preset writes")), "space": "cmyk"});
    assert!(load(&mut s, &plain).is_ok(), "a preset naming no channel fits every dialog");
    assert!(load(&mut s, &json!({"data": data, "space": "lab"})).is_err(), "and in a lab dialog they have no place either");
    assert!(load(&mut s, &json!({"data": data, "space": "not a space"})).is_err(), "a space that is not one is an error");
    assert!(load(&mut s, &json!({"data": data, "space": null})).is_ok(), "no space names no dialog: the file is read as it is");
}

#[test]
fn a_save_that_only_returns_bytes_changes_nothing() {
    let mut s = crate::session();
    save(&mut s, &json!({"curves": curves_params(), "name": "Mine"})).expect("a save works");
    let r = save(&mut s, &json!({"curves": one_curve(), "name": "Not yet", "record": false})).expect("the bytes answer");
    assert!(r["data"].is_string(), "the bytes are there to write");
    assert_eq!(r["name"], "Not yet", "and the name it would take");
    assert_eq!(list(&mut s)["count"], 1, "nothing was added to the list");
    assert_eq!(list(&mut s)["current"], "Mine", "and nothing was selected");
    // The dialog's second step: the same curves recorded for good.
    save(&mut s, &json!({"curves": one_curve(), "name": "Not yet"})).expect("a save works");
    assert_eq!(list(&mut s)["count"], 2, "and now the preset is in the list");
}

#[test]
fn a_current_name_is_kept_only_while_that_preset_is_in_the_list() {
    let mut s = crate::session();
    save(&mut s, &json!({"curves": curves_params(), "name": "Mine"})).expect("a save works");
    let saved = s.presets().to_json(&s);
    let mut t = crate::session();
    t.load_presets_json(json!({"curves_presets": [], "curves_preset": saved["curves_preset"]}));
    assert!(t.presets().curves_preset.is_none(), "a current preset that is not in the list is dropped");
    assert!(list(&mut t)["current"].is_null(), "so Default is current");
    let mut u = crate::session();
    u.load_presets_json(saved.clone());
    assert_eq!(u.presets().curves_preset.as_deref(), Some("Mine"), "a listed current preset stays");
    // The list is read before the name is checked, so a preset the file keeps keeps its selection.
    let mut v = crate::session();
    v.load_presets_json(json!({"curves_presets": saved["curves_presets"], "curves_preset": "Mine"}));
    assert_eq!(v.presets().curves_preset.as_deref(), Some("Mine"), "the name matches the loaded list");
}

#[test]
fn a_loaded_preset_replaces_the_curves_and_the_mode_the_dialog_held() {
    let drawn = json!({"target": { "layer": 3 }, "points": [0.0, 1.0, 2.0], "pencil": true, "smooth": 2});
    let preset = json!({"points": [[0.0, 0.0], [128.0, 150.0], [255.0, 255.0]], "red": [[0.0, 20.0], [255.0, 235.0]]});
    let merged = curves::over(&drawn, &preset).expect("both sides are objects");
    assert_eq!(merged["target"], drawn["target"], "the dialog keeps what it addresses");
    assert_eq!(merged["points"], preset["points"], "the preset's control points replace the drawn map");
    assert_eq!(merged["red"], preset["red"], "and so do its channels");
    assert!(merged.get("pencil").is_none(), "Pencil mode goes with the preset's own point mode");
    assert!(merged.get("smooth").is_none(), "and the smoothing passes the dialog held are not carried over");
    assert!(curves::over(&json!([3]), &preset).is_none(), "parameters that are not an object are refused");
    let drawn_now = curves::over(&json!({"points": [0.0, 1.0]}), &json!({"points": [0.0, 2.0], "pencil": true})).expect("both are objects");
    assert_eq!(drawn_now["pencil"], true, "a drawn preset sets Pencil mode on a dialog that had none");
}

#[test]
fn a_preset_that_names_channels_of_two_colour_modes_is_refused() {
    let mut s = crate::session();
    let mixed = json!({"points": [[0.0, 0.0], [100.0, 150.0], [255.0, 255.0]], "red": [[0.0, 20.0], [255.0, 235.0]], "cyan": [[0.0, 10.0], [255.0, 240.0]]});
    let err = save(&mut s, &json!({"curves": mixed.clone(), "name": "Mixed"})).unwrap_err().to_string();
    assert!(err.contains("red") && err.contains("cyan"), "the refusal names a channel of each mode: {err}");
    assert!(err.contains("one colour mode"), "and says what a preset holds: {err}");
    assert_eq!(list(&mut s)["count"], 0, "nothing is stored, because the file cannot say which half survived");
    let bytes = to_bytes(&CurvePreset { name: "Mixed".into(), curves: mixed }).expect("a preset writes");
    assert!(from_bytes(&bytes).is_err(), "reading such a file fails instead of quietly keeping one family");
    let data = photocraft_paint::tile::b64_encode(&bytes);
    let err = load(&mut s, &json!({"data": data})).unwrap_err().to_string();
    assert!(err.contains("red") && err.contains("cyan"), "and the load says the same thing: {err}");
    // What the format does hold: one mode's channels, with the composite curve beside them.
    assert!(
        save(&mut s, &json!({"curves": json!({"points": [[0.0, 0.0], [100.0, 150.0], [255.0, 255.0]], "cyan": [[0.0, 10.0], [255.0, 240.0]]}), "name": "Ink"}))
            .is_ok(),
        "one mode plus the composite saves"
    );
    assert!(
        save(&mut s, &json!({"curves": json!({"points": [[0.0, 0.0], [100.0, 150.0], [255.0, 255.0]], "red": [[0.0, 20.0], [255.0, 235.0]]}), "name": "Red"}))
            .is_ok(),
        "and so does the same shape in rgb"
    );
    assert_eq!(list(&mut s)["count"], 2, "both of those are in the list");
}

#[test]
fn a_gray_dialog_refuses_a_preset_that_names_channel_curves() {
    let mut s = crate::session();
    let rgb = to_bytes(&CurvePreset { name: "Red lift".into(), curves: curves_params() }).expect("a preset writes");
    let data = photocraft_paint::tile::b64_encode(&rgb);
    let err = load(&mut s, &json!({"data": data, "space": "gray"})).unwrap_err().to_string();
    assert!(err.contains("red"), "the refusal names the channel the dialog has no place for: {err}");
    assert!(err.contains("composite"), "and says what that dialog does edit: {err}");
    let plain = to_bytes(&CurvePreset { name: "Plain".into(), curves: one_curve() }).expect("a preset writes");
    assert!(
        load(&mut s, &json!({"data": photocraft_paint::tile::b64_encode(&plain), "space": "gray"})).is_ok(),
        "the composite curve alone fits a gray dialog"
    );
    assert!(load(&mut s, &json!({"data": data, "space": "grey"})).is_err(), "a space this command does not spell is an error");
    assert!(load(&mut s, &json!({"data": data, "space": "rgb"})).is_ok(), "the same preset fits the dialog of its own channels");
}

#[test]
fn the_saved_selection_follows_the_list_that_is_read_back() {
    let mut s = crate::session();
    save(&mut s, &json!({"curves": curves_params(), "name": "Mine"})).expect("a save works");
    let saved = s.presets().to_json(&s);
    assert_eq!(saved["curves_preset"], "Mine", "a session with a preset current writes its name");
    // The file is read back into the session it came from: the name is still there, the list is not.
    s.load_presets_json(json!({"curves_presets": [], "curves_preset": "Mine"}));
    assert!(s.presets().curves_preset.is_none(), "a selection the reloaded list does not hold is dropped, not kept");
    assert!(list(&mut s)["current"].is_null(), "so Default is current again");
    let mut t = crate::session();
    t.load_presets_json(json!({"curves_presets": saved["curves_presets"], "curves_preset": "Mine"}));
    assert_eq!(t.presets().curves_preset.as_deref(), Some("Mine"), "a named preset in the list is selected");
    // Saving with no preset current writes null: reading that back is a choice, not a silence.
    t.load_presets_json(json!({"curves_preset": null}));
    assert!(t.presets().curves_preset.is_none(), "an explicit null selects the Default curve again");
    // A file that says nothing about the selection leaves the session's choice alone.
    t.load_presets_json(json!({"curves_presets": saved["curves_presets"], "curves_preset": "Mine"}));
    t.load_presets_json(json!({"curves_presets": saved["curves_presets"]}));
    assert_eq!(t.presets().curves_preset.as_deref(), Some("Mine"), "an omitted key keeps a choice that is still listed");
    t.load_presets_json(json!({"curves_presets": []}));
    assert!(t.presets().curves_preset.is_none(), "and an omitted key still cannot leave a choice outside the list");
}

#[test]
fn the_command_docs_name_the_preset_file_type() {
    assert_eq!(EXT, "pcrv");
    let specs = curves::specs();
    let load = specs.iter().find(|c| c.id == "curves.presets.load").expect("the load command is installed");
    assert!(load.params.contains(".pcrv"), "its parameter doc names the file type: {}", load.params);
    let save = specs.iter().find(|c| c.id == "curves.presets.save").expect("the save command is installed");
    assert!(save.params.contains("curves"), "the save documents its curves key: {}", save.params);
    assert!(specs.iter().any(|c| c.id == "curves.presets.list"), "the list command is there");
    assert!(specs.iter().any(|c| c.id == "curves.presets.delete"), "the delete command is there");
    // The preset commands change the user list, like every other preset command here.
    for id in ["curves.presets.save", "curves.presets.load", "curves.presets.delete"] {
        let spec = specs.iter().find(|c| c.id == id).expect("the command is installed");
        assert!(spec.journal, "{id} is recorded in the session journal, as contours.new and contours.load are");
    }
    let list = specs.iter().find(|c| c.id == "curves.presets.list").expect("the list command is installed");
    assert!(!list.journal, "a query is not journaled");
}

#[test]
fn a_full_preset_list_is_capped_on_both_sides() {
    let mut s = crate::session();
    for at in 0..curves::MAX_PRESETS {
        let curve = json!({"points": [[0.0, 0.0], [255.0, (at % 250) as f64 + 1.0]]});
        save(&mut s, &json!({"curves": curve, "name": format!("P{at}")})).expect("a save works");
    }
    assert_eq!(list(&mut s)["count"], curves::MAX_PRESETS);
    assert!(save(&mut s, &json!({"curves": one_curve(), "name": "One too many"})).is_err(), "the list is capped");
    assert_eq!(stored(&s).len(), curves::MAX_PRESETS, "a refused save changes nothing");
    // A hand-written preferences file cannot make the list grow past the cap either.
    let mut bigger = list(&mut s)["presets"].as_array().expect("a list").clone();
    for at in 0..10 {
        bigger.push(json!({"name": format!("Extra{at}"), "curves": one_curve()}));
    }
    let mut t = crate::session();
    t.load_presets_json(json!({"curves_presets": bigger}));
    assert_eq!(stored(&t).len(), curves::MAX_PRESETS, "a longer saved list is cut to the cap");
}
