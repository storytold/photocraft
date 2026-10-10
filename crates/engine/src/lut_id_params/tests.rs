use std::fs;
use std::path::PathBuf;

use photocraft_cms::lutfile::{LutFile, write_cube};
use photocraft_doc::{Adjustment, LayerContent};
use serde_json::{Value, json};

use crate::lut_library::{LutLibrary, MAX_LUT_BYTES};
use crate::{EngineError, Session};

const ENTRY_POINTS: [&str; 3] = ["image.adjustments.colorLookup", "layer.newAdjustmentLayer.colorLookup", "layer.setAdjustment"];

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("photocraft-lutids-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A 2-point table that inverts every channel (red varies fastest).
fn invert_cube() -> String {
    let mut lut = LutFile::identity(2);
    for v in &mut lut.data {
        *v = 1.0 - *v;
    }
    write_cube(&lut)
}

/// `<base>/lib/Pack/Invert.cube` plus `<base>/outside.cube`, and a session using the library.
fn fixture(name: &str) -> (Session, PathBuf) {
    let base = temp(name);
    fs::create_dir_all(base.join("lib").join("Pack").join("Film")).unwrap();
    fs::write(base.join("lib").join("Pack").join("Invert.cube"), invert_cube()).unwrap();
    fs::write(base.join("lib").join("Pack").join("Film").join("Nested.cube"), invert_cube()).unwrap();
    fs::write(base.join("outside.cube"), invert_cube()).unwrap();
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
    s.lut_library = Some(LutLibrary::new(base.join("lib")));
    (s, base)
}

fn px(s: &mut Session) -> Vec<f32> {
    serde_json::from_value(s.execute("document.pixel", json!({"x": 1, "y": 1})).unwrap()).unwrap()
}

/// The Color Lookup layer's embedded table name and size.
fn lookup(s: &Session) -> (String, u32) {
    let st = s.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        LayerContent::Adjustment(Adjustment::ColorLookup { name, size, .. }) => (name.clone(), *size),
        other => panic!("{other:?}"),
    }
}

fn run(s: &mut Session, entry: &str, p: Value) -> Result<Value, EngineError> {
    if entry == "layer.setAdjustment" {
        s.execute("layer.newAdjustmentLayer.colorLookup", json!({"lut": "none"})).unwrap();
    }
    s.execute(entry, p)
}

#[test]
fn a_library_id_applies_through_each_entry_point_and_is_embedded() {
    let (mut s, _) = fixture("apply");
    // New adjustment layer: the table is in the layer, named after the file.
    s.execute("layer.newAdjustmentLayer.colorLookup", json!({"lut": "Pack/Invert.cube"})).unwrap();
    assert_eq!(lookup(&s), ("Invert.cube".to_string(), 2));
    assert!(px(&mut s)[0] < 0.01, "the inverting table turned white to black");

    // Editing the layer: a nested id works, and the table replaces the earlier one.
    s.execute("layer.setAdjustment", json!({"lut": "warm"})).unwrap();
    assert_ne!(lookup(&s).1, 2);
    s.execute("layer.setAdjustment", json!({"lut": "Pack/Film/Nested.cube", "dither": true})).unwrap();
    assert_eq!(lookup(&s), ("Nested.cube".to_string(), 2));

    // Destructively on pixels.
    let (mut s2, _) = fixture("apply-image");
    assert!(px(&mut s2)[0] > 0.99);
    s2.execute("image.adjustments.colorLookup", json!({"lut": "Pack/Invert.cube"})).unwrap();
    assert!(px(&mut s2)[0] < 0.01);
}

#[test]
fn the_applied_layer_does_not_depend_on_the_library_afterwards() {
    let (mut s, base) = fixture("embedded");
    s.execute("layer.newAdjustmentLayer.colorLookup", json!({"lut": "Pack/Invert.cube"})).unwrap();
    fs::remove_dir_all(base.join("lib")).unwrap();
    s.lut_library = None;
    assert!(px(&mut s)[0] < 0.01, "the table lives in the layer");
}

#[test]
fn applying_by_id_leaves_recent_and_the_recorded_params_alone() {
    let (mut s, _) = fixture("journal");
    s.execute("layer.newAdjustmentLayer.colorLookup", json!({"lut": "Pack/Invert.cube"})).unwrap();
    assert!(s.lut_library.as_ref().unwrap().recent().is_empty(), "only lut.used records a recent LUT");
    let (id, params) = s.journal.last().unwrap();
    assert_eq!(id, "layer.newAdjustmentLayer.colorLookup");
    assert_eq!(params["lut"], "Pack/Invert.cube");
    assert!(params.get("data").is_none(), "the journal keeps the id, not the table");
}

#[test]
fn unknown_and_traversal_style_ids_are_refused_by_every_entry_point() {
    let (mut s, base) = fixture("hostile");
    let outside = base.join("outside.cube").to_string_lossy().into_owned();
    let other_pack = base.join("lib").join("Pack").join("Invert.cube").to_string_lossy().into_owned();
    let ids: Vec<String> = [
        "Pack/Missing.cube",
        "Missing/Invert.cube",
        "../outside.cube",
        "../../x.cube",
        "Pack/../outside.cube",
        "Pack/../Pack/Invert.cube",
        "Pack/./Invert.cube",
        "/etc/x.cube",
        "/Pack/Invert.cube",
        "C:\\x.cube",
        "C:/x.cube",
        "Pack\\Invert.cube",
        "Pack//Invert.cube",
        "Pack/",
        "Pack",
        "/",
        "pack/invert.cube",
        "Pack/Invert",
        "Pack/Invert.cube ",
        "Pack/Invert.cube\0",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([outside, other_pack])
    .collect();
    for entry in ENTRY_POINTS {
        for id in &ids {
            let r = run(&mut s, entry, json!({"lut": id}));
            let e = r.expect_err(&format!("{entry} accepted {id:?}")).to_string();
            assert!(e.contains("unknown look"), "{entry} {id:?}: {e}");
        }
    }
}

#[test]
fn without_a_library_an_id_is_an_unknown_look_that_points_at_library_ids() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    for entry in ENTRY_POINTS {
        let e = run(&mut s, entry, json!({"lut": "Pack/Invert.cube"})).unwrap_err().to_string();
        assert!(e.contains("unknown look") && e.contains("library id"), "{entry}: {e}");
    }
    // Built-ins and "none" are unaffected.
    s.execute("layer.newAdjustmentLayer.colorLookup", json!({"lut": "warm"})).unwrap();
    s.execute("layer.setAdjustment", json!({"lut": "none"})).unwrap();
}

#[test]
fn a_hand_added_oversize_file_is_refused() {
    let (mut s, base) = fixture("oversize");
    let big = fs::File::create(base.join("lib").join("Pack").join("Big.cube")).unwrap();
    big.set_len(MAX_LUT_BYTES + 1).unwrap();
    drop(big);
    for entry in ENTRY_POINTS {
        let e = run(&mut s, entry, json!({"lut": "Pack/Big.cube"})).unwrap_err().to_string();
        assert!(e.contains("larger than"), "{entry}: {e}");
    }
}

#[test]
fn a_library_lut_that_is_not_utf8_is_refused_rather_than_changed() {
    let (mut s, base) = fixture("not-utf8");
    // A valid table with one stray Latin-1 byte in a comment: lossy decoding would accept it.
    let mut bytes = b"# caf\xe9\n".to_vec();
    bytes.extend_from_slice(invert_cube().as_bytes());
    fs::write(base.join("lib").join("Pack").join("Latin1.cube"), bytes).unwrap();
    for entry in ENTRY_POINTS {
        let e = run(&mut s, entry, json!({"lut": "Pack/Latin1.cube"})).unwrap_err().to_string();
        assert!(e.contains("UTF-8"), "{entry}: {e}");
    }
}

#[test]
fn a_broken_installed_file_is_a_parse_error_not_a_panic() {
    let (mut s, base) = fixture("broken");
    fs::write(base.join("lib").join("Pack").join("Junk.cube"), b"\xff\xfe not a lut").unwrap();
    for entry in ENTRY_POINTS {
        assert!(run(&mut s, entry, json!({"lut": "Pack/Junk.cube"})).is_err(), "{entry}");
    }
}

#[test]
fn only_color_lookup_layers_look_at_the_lut_param() {
    let (mut s, _) = fixture("other-kind");
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    // Nothing resolves (or reads the library) for an adjustment that has no `lut`.
    let p = json!({"lut": "Pack/Invert.cube"});
    assert_eq!(super::resolve(&s, "layer.setAdjustment", p.clone()).unwrap(), p);
    assert_eq!(super::resolve(&s, "layer.newAdjustmentLayer.levels", p.clone()).unwrap(), p);
}

#[test]
fn resolving_swaps_the_id_for_the_embedded_text_and_ignores_other_params() {
    let (s, _) = fixture("resolve");
    let out = super::resolve(&s, "layer.newAdjustmentLayer.colorLookup", json!({"lut": "Pack/Invert.cube", "dither": true})).unwrap();
    assert!(out.get("lut").is_none());
    assert_eq!(out["fileName"], "Invert.cube");
    assert_eq!(out["data"], invert_cube());
    assert_eq!(out["dither"], true);
    // A non-string `lut` or a missing one is left for the command to judge.
    for p in [json!({}), json!({"lut": 5}), json!({"lut": null}), json!({"lut": "warm"}), json!({"lut": "none"}), Value::Null] {
        assert_eq!(super::resolve(&s, "layer.newAdjustmentLayer.colorLookup", p.clone()).unwrap(), p);
    }
}
