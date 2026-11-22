//! Which words in an imported Visibility cell mean visible (#13933): only `true`.

use super::*;
use photocraft_doc::Layer;

/// Import one data set per `cells` entry into a document with a Visibility variable `show`, and
/// return each set's stored visibility.
fn imported_visibility(cells: &[&str]) -> Vec<bool> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let badge = s
        .edit("setup", |doc, _| {
            let layer = Layer::raster("badge", doc.pixel_format());
            let id = layer.id;
            doc.layers.push(layer);
            Ok(id)
        })
        .unwrap();
    s.execute("image.variables.define", json!({"defs": [{"name": "show", "layer": badge.0, "type": "visibility"}]})).unwrap();
    let csv = format!("show\n{}\n", cells.join("\n"));
    let r = import_data_sets_text(&mut s, &csv, &json!({})).unwrap();
    assert_eq!(r["imported"], cells.len());
    s.active()
        .unwrap()
        .doc
        .variables
        .data_sets
        .iter()
        .map(|set| match set.values.as_slice() {
            [DataValue { value: VarValue::Visibility(v), .. }] => *v,
            other => panic!("expected one visibility value, got {other:?}"),
        })
        .collect()
}

#[test]
fn only_true_means_visible() {
    assert_eq!(imported_visibility(&["true"]), [true]);
}

#[test]
fn other_words_mean_invisible() {
    let cells = ["false", "FALSE", "yes", "1", "visible", "on"];
    assert_eq!(imported_visibility(&cells), [false; 6]);
}

#[test]
fn other_capitalisations_of_true_stay_visible() {
    assert_eq!(imported_visibility(&["TRUE", "True", " true "]), [true; 3]);
}
