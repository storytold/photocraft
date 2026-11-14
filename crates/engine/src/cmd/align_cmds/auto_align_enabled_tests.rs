//! #14198: Auto-Align Layers enabled state and layer collection across pixel, Background,
//! adjustment, group, type, smart object, and shape layers.

use super::*;

fn new_doc_with_two_layers() -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 60, "depth": 8})).unwrap();
    let bg = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({"name": "Pixel"})).unwrap();
    let p = s.active().unwrap().active_layer.unwrap();
    for (id, c) in [(bg, [0.4, 0.4, 0.4, 1.0]), (p, [0.7, 0.2, 0.2, 1.0])] {
        s.edit("paint", |doc, _| {
            doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 10, 50, 40), &c);
            Ok(())
        })
        .unwrap();
    }
    (s, bg, p)
}

#[test]
fn disabled_with_no_doc_or_single_layer() {
    let s = Session::new();
    assert!(!s.is_enabled("edit.autoAlignLayers"));

    let mut s2 = Session::new();
    s2.execute("file.new", json!({"width": 64, "height": 64, "depth": 8})).unwrap();
    assert!(!s2.is_enabled("edit.autoAlignLayers"));
}

#[test]
fn disabled_when_type_or_shape_layer_is_selected_with_pixel_layers() {
    let (mut s, bg, p) = new_doc_with_two_layers();
    s.execute("type.create", json!({"text": "Hi", "x": 10, "y": 20})).unwrap();
    let text_id = s.active().unwrap().active_layer.unwrap();

    s.execute("layer.select", json!({"layer": bg.0})).unwrap();
    s.execute("layer.select", json!({"layer": p.0, "mode": "add"})).unwrap();
    s.execute("layer.select", json!({"layer": text_id.0, "mode": "add"})).unwrap();
    assert!(!s.is_enabled("edit.autoAlignLayers"));
    assert!(s.is_enabled("edit.autoBlendLayers"));
    assert!(s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).is_err());

    s.execute("shape.create", json!({"kind": "rect", "rect": [5, 5, 20, 20], "fill": "#ff0000"})).unwrap();
    let shape_id = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.select", json!({"layer": bg.0})).unwrap();
    s.execute("layer.select", json!({"layer": p.0, "mode": "add"})).unwrap();
    s.execute("layer.select", json!({"layer": shape_id.0, "mode": "add"})).unwrap();
    assert!(!s.is_enabled("edit.autoAlignLayers"));
}

#[test]
fn enabled_and_runs_for_group_and_adjustment_layers_with_pixel_layers() {
    let (mut s, bg, p) = new_doc_with_two_layers();
    s.execute("layer.new.group", json!({"name": "G"})).unwrap();
    let g = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.moveTo", json!({"layer": p.0, "target": g.0, "position": "into"})).unwrap();

    s.execute("layer.select", json!({"layer": bg.0})).unwrap();
    s.execute("layer.select", json!({"layer": g.0, "mode": "add"})).unwrap();
    assert!(s.is_enabled("edit.autoAlignLayers"));
    assert!(!s.is_enabled("edit.autoBlendLayers"));
    s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).unwrap();

    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let adj = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.select", json!({"layer": bg.0})).unwrap();
    s.execute("layer.select", json!({"layer": adj.0, "mode": "add"})).unwrap();
    assert!(s.is_enabled("edit.autoAlignLayers"));
    assert!(s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).is_err());

    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let adj2 = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.select", json!({"layer": adj.0})).unwrap();
    s.execute("layer.select", json!({"layer": adj2.0, "mode": "add"})).unwrap();
    assert!(s.is_enabled("edit.autoAlignLayers"));
    assert!(s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).is_err());

    s.execute("layer.select", json!({"layer": bg.0})).unwrap();
    s.execute("layer.select", json!({"layer": g.0, "mode": "add"})).unwrap();
    s.execute("layer.select", json!({"layer": adj.0, "mode": "add"})).unwrap();
    assert!(s.is_enabled("edit.autoAlignLayers"));
    s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).unwrap();
}
