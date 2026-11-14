//! #11736: while an adjustment layer is selected, each Color Sampler reading also reports the
//! value before that adjustment (the composite with the selected adjustment layer left out);
//! with any other layer selected the list reports single values, as before.
//!
//! The issue's document: 800 × 600 RGB/8, red (220,30,30) at 100,100–300,250 on a pixel layer, an
//! Invert adjustment layer above it, sampler #1 on the red (pixel 199,149).

use super::*;

fn document() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Colors"})).unwrap();
    s.execute("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 150})).unwrap();
    s.execute("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    s.execute("view.colorSamplers.add", json!({"x": 199.5, "y": 149.5})).unwrap();
    s
}

fn first(s: &mut Session) -> Value {
    s.execute("view.colorSamplers.list", json!({})).unwrap()["samplers"][0].clone()
}

fn select(s: &mut Session, name: &str) {
    let d = s.active_mut().unwrap();
    let id = d.doc.layers.iter().find(|l| l.name == name).map(|l| l.id);
    d.active_layer = id;
}

#[test]
fn an_adjustment_layer_selected_gives_before_and_after() {
    let mut s = document();
    let kind = {
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap().content.kind_name()
    };
    assert_eq!(kind, "Adjustment", "the new Invert layer is selected");
    let p = first(&mut s);
    assert_eq!(p["rgb"], json!([35, 225, 225]), "after: the composite as shown");
    assert_eq!(p["before"]["rgb"], json!([220, 30, 30]), "before: without the Invert");
    assert!(p["before"]["cmyk"].is_array());
}

#[test]
fn a_pixel_layer_selected_gives_single_values() {
    let mut s = document();
    select(&mut s, "Colors");
    let p = first(&mut s);
    assert_eq!(p["rgb"], json!([35, 225, 225]), "the visible composite, not the active layer");
    assert!(p.get("before").is_none());
}

#[test]
fn a_hidden_adjustment_layer_keeps_single_values() {
    let mut s = document();
    s.edit("hide", |doc, active| {
        let id = active.ok_or(EngineError::NoDocument)?;
        doc.layer_mut(id).ok_or(EngineError::NoDocument)?.visible = false;
        Ok(())
    })
    .unwrap();
    let p = first(&mut s);
    assert_eq!(p["rgb"], json!([220, 30, 30]));
    assert!(p.get("before").is_none(), "not measured: kept as today");
}

#[test]
fn listing_changes_nothing() {
    let mut s = document();
    let before = s.active().unwrap().history.past_len();
    let rev = s.active().unwrap().revision;
    first(&mut s);
    assert_eq!(s.active().unwrap().history.past_len(), before);
    assert_eq!(s.active().unwrap().revision, rev);
    let d = s.active().unwrap();
    assert!(d.doc.layers.iter().all(|l| l.visible), "the adjustment layer stays visible");
}
