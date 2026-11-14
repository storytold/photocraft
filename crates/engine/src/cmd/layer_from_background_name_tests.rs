//! #14821: Layer > New > Layer from Background names the converted layer "Layer N", with the
//! lowest free N counting from 0. Today `unlock_background` (`extra_cmds.rs`) always writes
//! "Layer 0", so a document that already has a layer of that name ends up with two layers called
//! "Layer 0".
//!
//! The five cases below are the ones measured in the issue. `Document::next_layer_name` counts from
//! 1, so the command needs its own 0-based rule.
//!
//! Not measured in the issue: whether a name inside a group takes part in the search, and whether a
//! differently cased name ("LAYER 1") counts as taken. Both keep the reading the measured cases
//! support: top-level layer names only, compared exactly, as `next_layer_name` compares them. The
//! group case is asked about on the issue.

use photocraft_doc::LayerId;
use serde_json::json;

use crate::Session;

fn doc(s: &Session) -> &photocraft_doc::Document {
    &s.active().unwrap().doc
}

/// A document with the white Background `file.new` makes and top-level layers named `others` above
/// it, then Layer from Background: the name the converted Background ends with.
fn converted_name(others: &[&str]) -> String {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    let bg = doc(&s).layers[0].id;
    for n in others {
        let id = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
        s.execute("layer.setProps", json!({"layer": id.0, "name": n})).unwrap();
    }
    assert_eq!(doc(&s).layers.iter().skip(1).map(|l| l.name.as_str()).collect::<Vec<_>>(), others, "the other layers are named as the issue's test document");
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    doc(&s).layer(bg).unwrap().name.clone()
}

#[test]
fn the_name_is_the_lowest_layer_n_that_no_other_layer_uses() {
    let cases: Vec<(Vec<&str>, &str)> = vec![
        (vec!["Top"], "Layer 0"),
        (vec!["Layer 0"], "Layer 1"),
        (vec!["Layer 1"], "Layer 0"),
        (vec!["Layer 5"], "Layer 0"),
        (vec!["Layer 0", "Layer 1"], "Layer 2"),
    ];
    for (others, want) in cases {
        assert_eq!(converted_name(&others), want, "other layers: {others:?}");
    }
}

#[test]
fn a_document_that_is_only_a_background_takes_layer_0() {
    assert_eq!(converted_name(&[]), "Layer 0");
}

#[test]
fn undo_restores_the_background_with_its_old_name() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    let bg = doc(&s).layers[0].id;
    s.execute("layer.new.layer", json!({"name": "Layer 0"})).unwrap();
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    assert_eq!(doc(&s).layer(bg).unwrap().name, "Layer 1", "the taken name is skipped");
    assert!(s.undo(), "one history step");
    let l = doc(&s).layer(bg).unwrap();
    assert!(doc(&s).is_background(bg) && l.name == "Background", "the Background is back, named as before");
}

#[test]
fn the_other_callers_of_unlock_background_still_name_it_layer_0() {
    // #14821 changes only Layer from Background. An edit the Background cannot take converts it
    // through `background_to_layer`, which names it "Layer 0" as today, even when that name is
    // already taken.
    for (key, value) in [("opacity", json!(0.5)), ("fill", json!(0.5)), ("blend", json!("Multiply"))] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
        let bg = doc(&s).layers[0].id;
        s.execute("layer.new.layer", json!({"name": "Layer 0"})).unwrap();
        s.execute("layer.setProps", json!({"layer": bg.0, key: value})).unwrap();
        let l = doc(&s).layer(bg).unwrap();
        assert!(!doc(&s).is_background(bg) && l.name == "Layer 0", "{key}: still named Layer 0 as today");
    }
}
