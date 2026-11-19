//! #14598: Image › Duplicate with "Duplicate Merged Layers Only" keeps the source's layer comps,
//! as the reference application does and as a full copy already did. Their layers are gone in
//! the flattened copy, so applying one reports them as missing instead of failing.

use serde_json::json;

use crate::Session;

/// A document with one layer and two comps, the second with a comment and only visibility.
fn source() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
    s.execute("layerComp.new", json!({"name": "Comp A"})).unwrap();
    s.execute("layerComp.new", json!({"name": "Comp B", "comment": "only visibility", "position": false, "appearance": false})).unwrap();
    s
}

#[test]
fn a_merged_copy_keeps_every_comp_with_its_name_comment_and_options() {
    let mut s = source();
    let src: Vec<_> = s.active().unwrap().doc.layer_comps.clone();
    assert_eq!(src.len(), 2);
    s.execute("image.duplicate", json!({"mergedOnly": true})).unwrap();
    let copy = &s.active().unwrap().doc;
    assert_eq!(copy.layers.len(), 1, "the merged copy has one layer");
    assert_eq!(copy.layer_comps.len(), src.len(), "every comp is kept");
    for (kept, original) in copy.layer_comps.iter().zip(&src) {
        assert_eq!(kept.name, original.name);
        assert_eq!(kept.comment, original.comment);
        let options = |c: &photocraft_doc::LayerComp| (c.apply_visibility, c.apply_position, c.apply_appearance);
        assert_eq!(options(kept), options(original), "{}: options kept", kept.name);
    }
    let listed = s.execute("layerComp.list", json!({})).unwrap();
    assert_eq!(listed["comps"].as_array().map(Vec::len), Some(2), "layerComp.list shows the kept comps");
}

#[test]
fn applying_a_kept_comp_in_the_merged_copy_reports_its_missing_layers() {
    let mut s = source();
    s.execute("image.duplicate", json!({"mergedOnly": true})).unwrap();
    let applied = s.execute("layerComp.apply", json!({"comp": "Comp A"})).expect("applying a kept comp does not fail");
    assert!(applied["missingLayers"].as_u64().unwrap_or(0) > 0, "the source's layers are reported missing: {applied}");
    let warnings = s.execute("layerComp.updateWarnings", json!({})).unwrap();
    assert!(warnings["count"].as_u64().unwrap_or(0) > 0, "the comps warn about their missing layers: {warnings}");
}

#[test]
fn the_source_and_a_full_copy_keep_their_comps() {
    let mut s = source();
    s.execute("image.duplicate", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer_comps.len(), 2, "a full copy keeps the comps, as before");
    s.set_active(0);
    s.execute("image.duplicate", json!({"mergedOnly": true})).unwrap();
    s.set_active(0);
    assert_eq!(s.active().unwrap().doc.layer_comps.len(), 2, "the source keeps its comps");
}
