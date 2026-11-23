//! Tests that run paint commands, moved from the engine-core `rasterize_style_cmds` tests: 1 test(s).

/// Sets the pixels of `area` (the whole canvas when `None`) on the active layer to `hex`, through the
/// document: the state `edit.fill` leaves, without the paint command core does not install.
fn fill_active(s: &mut Session, area: Option<photocraft_geom::Rect>, hex: &str) {
    let rgba = photocraft_engine::commands::parse_hex(hex).unwrap();
    s.edit("setup", |doc, active| {
        let area = area.unwrap_or_else(|| doc.bounds());
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let px = photocraft_raster::from_rgba(&surf.format(), rgba);
        let data: Vec<f32> = (0..(area.width() * area.height()) as usize).flat_map(|_| px.iter().copied()).collect();
        surf.write_region(area, &data);
        Ok(())
    })
    .unwrap();
}

use photocraft_doc::{Layer, LayerId};
use photocraft_engine::Session;
use serde_json::json;

use photocraft_doc::Document;

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn layer(s: &Session, id: u64) -> &Layer {
    doc(s).layer(LayerId(id)).unwrap()
}

/// A white Background and a red rectangle (partly off the right edge of the canvas) with a
/// Normal-mode drop shadow, outer glow, inner shadow, outside stroke and a half colour overlay.
fn scene(depth: u32) -> (Session, u64) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth})).unwrap();
    let id = s.execute("layer.new.layer", json!({"name": "a"})).unwrap()["layer"].as_u64().unwrap();
    fill_active(&mut s, Some(photocraft_geom::Rect::new(14, 10, 64, 30)), "#e02010");
    s.execute("layer.layerStyle.dropShadow", json!({"blend": "normal", "distance": 6, "size": 4, "color": "#102040"})).unwrap();
    s.execute("layer.layerStyle.outerGlow", json!({"blend": "normal", "size": 5, "color": "#20c040", "add": true})).unwrap();
    s.execute("layer.layerStyle.innerShadow", json!({"blend": "normal", "distance": 3, "size": 3, "add": true})).unwrap();
    s.execute("layer.layerStyle.stroke", json!({"size": 2, "position": "outside", "color": "#0000ff", "add": true})).unwrap();
    s.execute("layer.layerStyle.colorOverlay", json!({"color": "#ffff00", "opacity": 50, "add": true})).unwrap();
    (s, id)
}

#[test]
fn several_selected_layers_bake_in_one_step() {
    let (mut s, a) = scene(8);
    let b = s.execute("layer.new.layer", json!({"name": "b"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("select.rect", json!({"x": 2, "y": 34, "width": 10, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#00ff00"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.layerStyle.stroke", json!({"size": 2})).unwrap();
    let plain = s.execute("layer.new.layer", json!({"name": "plain"})).unwrap()["layer"].as_u64().unwrap();
    *s.active_mut().unwrap().selection_mut() = vec![LayerId(a), LayerId(b), LayerId(plain)];
    let steps = s.active().unwrap().history.past_len();
    let r = s.execute("layer.rasterize.layerStyle", json!({})).unwrap();
    assert_eq!(r["layers"], json!([a, b]));
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    assert!(layer(&s, a).effects.items.is_empty() && layer(&s, b).effects.items.is_empty());
}
