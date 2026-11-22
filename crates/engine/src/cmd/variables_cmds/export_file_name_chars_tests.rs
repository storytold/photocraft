//! #13758: Data Sets as Files keeps a data set name's spaces and other legal characters in the file
//! name, maps only the nine characters Windows refuses, and offers the data set number unpadded
//! (`{number}`) and padded to the set count (`{paddedNumber}`). `{index}` keeps its three digits.

use serde_json::{Value, json};

use crate::Session;

/// A document "trail-map" whose data sets are `names`, each showing or hiding its one layer.
fn session(names: &[&str]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "white", "name": "trail-map"})).unwrap();
    let layer = s.active().unwrap().doc.layers[0].id;
    s.execute("image.variables.define", json!({"defs": [{"name": "show", "layer": layer.0, "type": "visibility"}]})).unwrap();
    let sets: Vec<Value> =
        names.iter().enumerate().map(|(i, name)| json!({"name": name, "values": [{"variable": "show", "kind": "visibility", "value": i % 2 == 0}]})).collect();
    s.execute("image.variables.dataSets", json!({"dataSets": sets})).unwrap();
    s
}

/// The file names `naming` gives, in order, written as PNGs into a fresh folder.
fn exported(names: &[&str], naming: &str) -> Vec<String> {
    let mut s = session(names);
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().to_string_lossy().into_owned();
    let r = s.execute("file.export.dataSetsAsFiles", json!({"dir": dir, "format": "png", "naming": naming})).unwrap();
    let files: Vec<String> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().strip_prefix(&format!("{dir}/")).unwrap().to_string()).collect();
    for f in &files {
        assert!(std::path::Path::new(&format!("{dir}/{f}")).is_file(), "{f} was written inside the folder");
    }
    files
}

#[test]
fn spaces_and_other_legal_characters_are_kept() {
    assert_eq!(exported(&["Route 1", "Route 2", "Route 3"], "{document}_{name}"), ["trail-map_Route 1.png", "trail-map_Route 2.png", "trail-map_Route 3.png"]);
    assert_eq!(exported(&["Mr. O'Neil (draft)", "Café #2"], "{name}"), ["Mr. O'Neil (draft).png", "Café #2.png"]);
}

#[test]
fn the_nine_illegal_characters_are_mapped_in_data_set_names() {
    assert_eq!(exported(&[r#"Mid/Year:Sale*?"<>|x\y"#], "{name}"), [r#"Mid_Year-Sale--'()_x_y.png"#]);
}

#[test]
fn illegal_characters_typed_in_the_naming_are_mapped_too() {
    assert_eq!(exported(&["Lake"], r#"v:2*?"<>|-{name}"#), [r#"v-2--'()_-Lake.png"#]);
}

#[test]
fn the_number_comes_unpadded_or_padded_to_the_set_count() {
    let three = ["Ash", "Birch", "Cedar"];
    assert_eq!(exported(&three, "{number}_{name}"), ["1_Ash.png", "2_Birch.png", "3_Cedar.png"]);
    assert_eq!(exported(&three, "{paddedNumber}"), ["1.png", "2.png", "3.png"]);
    let eleven = ["s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11"];
    let padded = exported(&eleven, "{paddedNumber}");
    assert_eq!(padded[0], "01.png");
    assert_eq!(padded[1], "02.png");
    assert_eq!(padded[10], "11.png");
    assert_eq!(exported(&eleven, "{number}")[1], "2.png");
}

#[test]
fn index_keeps_three_digits() {
    assert_eq!(exported(&["Ash", "Birch"], "row-{index}"), ["row-001.png", "row-002.png"]);
}

#[test]
fn names_stay_inside_the_folder_and_off_device_names() {
    assert_eq!(exported(&["../up", "nul"], "{name}"), [".._up.png", "nul_.png"]);
}

/// A name of only dots and spaces would name the folder or its parent on Windows: it becomes `_` (CodeRabbit on #16239).
#[test]
fn names_of_only_dots_and_spaces_become_an_underscore() {
    assert_eq!(exported(&["..."], "{name}"), ["_.png"]);
    assert_eq!(exported(&[". ."], "{name}"), ["_.png"]);
    assert_eq!(exported(&[".."], "{name}"), ["_.png"]);
}

/// Two data sets with the same name keep their own places: `{number}` tells them apart (CodeRabbit on #16239).
#[test]
fn repeated_data_set_names_get_their_own_numbers() {
    assert_eq!(exported(&["A", "A"], "{number}"), ["1.png", "2.png"]);
}

/// A token typed in a data set's name stays literal; only the template's own tokens expand (CodeRabbit on #16239).
#[test]
fn a_token_inside_a_data_set_name_stays_literal() {
    assert_eq!(exported(&["{number}", "1"], "{name}"), ["{number}.png", "1.png"]);
}

/// Windows drops the trailing dots of a folder name: `A./out.png` and `A/out.png` are one file, so the batch is
/// refused before anything is written (CodeRabbit on #16239).
#[test]
fn folders_that_differ_only_by_a_trailing_dot_collide() {
    let mut s = session(&["A", "A."]);
    let dir = tempfile::tempdir().unwrap();
    let err = s.execute("file.export.dataSetsAsFiles", json!({"dir": dir.path().to_string_lossy(), "format": "png", "naming": "{name}/out"})).unwrap_err();
    assert!(err.to_string().contains("same output"), "{err}");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "nothing was written");
}
