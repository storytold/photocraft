use super::*;
use photocraft_doc::Document;

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn layer(s: &Session, id: u64) -> &Layer {
    doc(s).layer(LayerId(id)).unwrap()
}

fn composite(s: &Session) -> photocraft_compose::Buffer {
    let d = doc(s);
    photocraft_compose::render(d, d.bounds())
}

fn max_diff(a: &photocraft_compose::Buffer, b: &photocraft_compose::Buffer) -> f32 {
    a.px.iter().zip(&b.px).flat_map(|(p, q)| p.iter().zip(q).map(|(x, y)| (x - y).abs())).fold(0.0, f32::max)
}

/// The quantisation step of one stored sample: the baked layer is stored at the document's depth.
fn tolerance(depth: u32) -> f32 {
    match depth {
        8 => 2.5 / 255.0,
        16 => 2.0 / 32768.0 + 1e-4,
        _ => 1e-4,
    }
}

#[test]
fn rasterizes_fill_content_first() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    s.execute("layer.newFillLayer.solidColor", json!({"color": "#3366cc"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap().0;
    let square = json!({"subpaths": [{"closed": true, "knots": [[8, 8], [28, 8], [28, 20], [8, 20]]}]});
    s.execute("layer.vectorMask.add", json!({"layer": id, "path": square})).unwrap();
    s.execute("layer.layerStyle.stroke", json!({"size": 3, "position": "outside", "color": "#ff0000"})).unwrap();
    let before = composite(&s);
    let steps = s.active().unwrap().history.past_len();
    s.execute(ID, json!({})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "rasterize and bake are one step");
    assert!(matches!(layer(&s, id).content, LayerContent::Raster(_)));
    assert!(layer(&s, id).effects.items.is_empty());
    assert!(max_diff(&before, &composite(&s)) <= tolerance(8));
}
