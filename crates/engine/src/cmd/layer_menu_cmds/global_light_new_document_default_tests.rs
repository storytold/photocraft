use photocraft_doc::GlobalLight;
use serde_json::{Value, json};

use crate::Session;

fn active_global_light(s: &Session) -> GlobalLight {
    s.active().expect("active document").doc.global_light
}

#[test]
fn no_document_global_light_command_is_enabled() {
    let s = Session::new();
    assert!(s.active().is_none());
    assert!(s.is_enabled("layer.layerStyle.globalLight"), "layer.layerStyle.globalLight should be enabled without an open document");
    assert_eq!(s.disabled_reason("layer.layerStyle.globalLight"), None);
}

#[test]
fn no_document_global_light_sets_and_applies_new_document_default() {
    let mut s = Session::new();

    // Without a stored override, new documents start at 120 / 30.
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    assert_eq!(active_global_light(&s), GlobalLight { angle: 120.0, altitude: 30.0 });
    s.execute("file.close", json!({"save": "discard"})).unwrap();

    // Setting Global Light with no document open updates the stored default and adds no history state.
    let res = s.execute("layer.layerStyle.globalLight", json!({"angle": 75.0, "altitude": 20.0})).unwrap();
    assert_eq!(res["angle"].as_f64(), Some(75.0));
    assert_eq!(res["altitude"].as_f64(), Some(20.0));
    assert!(s.documents().is_empty(), "no document or history state should be created");

    // Partial updates preserve the other field from the stored default, with the same wrap/clamp rules.
    let res = s.execute("layer.layerStyle.globalLight", json!({"angle": 270.0})).unwrap();
    assert_eq!(res["angle"].as_f64(), Some(-90.0));
    assert_eq!(res["altitude"].as_f64(), Some(20.0));

    let res = s.execute("layer.layerStyle.globalLight", json!({"altitude": 150.0})).unwrap();
    assert_eq!(res["angle"].as_f64(), Some(-90.0));
    assert_eq!(res["altitude"].as_f64(), Some(90.0));

    let res = s.execute("layer.layerStyle.globalLight", json!({"altitude": -10.0})).unwrap();
    assert_eq!(res["altitude"].as_f64(), Some(0.0));

    // Invalid numbers return Err without mutating the stored default.
    assert!(s.execute("layer.layerStyle.globalLight", json!({"angle": 1e39_f64})).is_err());
    assert!(s.execute("layer.layerStyle.globalLight", json!({"altitude": 1e39_f64})).is_err());
    let preserved = s.execute("layer.layerStyle.globalLight", json!({})).unwrap();
    assert_eq!(preserved["angle"].as_f64(), Some(-90.0));
    assert_eq!(preserved["altitude"].as_f64(), Some(0.0));

    // Set 75 / 20 and verify new documents across transparent/background/artboard start with it.
    s.execute("layer.layerStyle.globalLight", json!({"angle": 75.0, "altitude": 20.0})).unwrap();
    for params in
        [json!({"width": 32, "height": 32}), json!({"width": 32, "height": 32, "background": "white"}), json!({"width": 32, "height": 32, "artboard": true})]
    {
        s.execute("file.new", params).unwrap();
        assert_eq!(active_global_light(&s), GlobalLight { angle: 75.0, altitude: 20.0 });
        s.execute("file.close", json!({"save": "discard"})).unwrap();
    }
}

#[test]
fn open_document_global_light_stays_independent_of_new_document_default() {
    let mut s = Session::new();
    s.execute("layer.layerStyle.globalLight", json!({"angle": 75.0, "altitude": 20.0})).unwrap();

    // Open two documents (both start from the default 75 / 20) and change each independently.
    s.execute("file.new", json!({"width": 32, "height": 32, "name": "Doc A"})).unwrap();
    s.execute("layer.layerStyle.globalLight", json!({"angle": 0.0, "altitude": 15.0})).unwrap();
    assert_eq!(active_global_light(&s), GlobalLight { angle: 0.0, altitude: 15.0 });

    s.execute("file.new", json!({"width": 32, "height": 32, "name": "Doc B"})).unwrap();
    s.execute("layer.layerStyle.globalLight", json!({"angle": 52.0, "altitude": 40.0})).unwrap();
    assert_eq!(active_global_light(&s), GlobalLight { angle: 52.0, altitude: 40.0 });

    // Both existing documents kept their own Global Light independently.
    assert_eq!(s.documents()[0].doc.global_light, GlobalLight { angle: 0.0, altitude: 15.0 });
    assert_eq!(s.documents()[1].doc.global_light, GlobalLight { angle: 52.0, altitude: 40.0 });

    // Changing Global Light on an open document did not alter the stored default for new documents.
    s.execute("file.new", json!({"width": 32, "height": 32, "name": "Doc C"})).unwrap();
    assert_eq!(active_global_light(&s), GlobalLight { angle: 75.0, altitude: 20.0 });
}

#[test]
fn preferences_round_trip_and_missing_field_keeps_120_30() {
    let mut s = Session::new();
    s.execute("layer.layerStyle.globalLight", json!({"angle": 75.0, "altitude": 20.0})).unwrap();
    let saved = s.prefs_to_json();
    let parsed: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(parsed["globalLight"]["angle"].as_f64(), Some(75.0));
    assert_eq!(parsed["globalLight"]["altitude"].as_f64(), Some(20.0));

    let mut restored = Session::new();
    restored.load_prefs_json(&saved).unwrap();
    restored.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    assert_eq!(active_global_light(&restored), GlobalLight { angle: 75.0, altitude: 20.0 });

    let mut empty = Session::new();
    empty.load_prefs_json("{}").unwrap();
    empty.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    assert_eq!(active_global_light(&empty), GlobalLight { angle: 120.0, altitude: 30.0 });
}

#[test]
fn opened_files_keep_saved_global_light_regardless_of_default() {
    let dir = std::env::temp_dir().join(format!("photocraft-global-light-default-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pcraft = dir.join("saved.pcraft");
    let psd = dir.join("saved.psd");

    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    s.execute("layer.layerStyle.globalLight", json!({"angle": 60.0, "altitude": 45.0})).unwrap();
    s.execute("file.saveACopy", json!({"path": pcraft.to_str().unwrap()})).unwrap();
    s.execute("file.saveACopy", json!({"path": psd.to_str().unwrap()})).unwrap();
    s.execute("file.close", json!({"save": "discard"})).unwrap();

    // Change the new-document default to something else before reopening the saved files.
    s.execute("layer.layerStyle.globalLight", json!({"angle": 75.0, "altitude": 20.0})).unwrap();

    for path in [&pcraft, &psd] {
        s.execute("file.openAs", json!({"path": path.to_str().unwrap()})).unwrap();
        assert_eq!(active_global_light(&s), GlobalLight { angle: 60.0, altitude: 45.0 }, "path {path:?} should keep saved Global Light");
        s.execute("file.close", json!({"save": "discard"})).unwrap();
    }

    let _ = std::fs::remove_dir_all(&dir);
}
