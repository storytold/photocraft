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

/// A white Background and a red rectangle (partly off the right edge of the canvas) with a
/// Normal-mode drop shadow, outer glow, inner shadow, outside stroke and a half colour overlay.
fn scene(depth: u32) -> (Session, u64) {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth})).unwrap();
    let id = s.execute("layer.new.layer", json!({"name": "a"})).unwrap()["layer"].as_u64().unwrap();
    crate::tests::fill_active(&mut s, Some(photocraft_geom::Rect::new(14, 10, 64, 30)), "#e02010");
    s.execute("layer.layerStyle.dropShadow", json!({"blend": "normal", "distance": 6, "size": 4, "color": "#102040"})).unwrap();
    s.execute("layer.layerStyle.outerGlow", json!({"blend": "normal", "size": 5, "color": "#20c040", "add": true})).unwrap();
    s.execute("layer.layerStyle.innerShadow", json!({"blend": "normal", "distance": 3, "size": 3, "add": true})).unwrap();
    s.execute("layer.layerStyle.stroke", json!({"size": 2, "position": "outside", "color": "#0000ff", "add": true})).unwrap();
    s.execute("layer.layerStyle.colorOverlay", json!({"color": "#ffff00", "opacity": 50, "add": true})).unwrap();
    (s, id)
}

/// #4974: a layer that fails to bake takes back the ones baked before it.
#[test]
fn a_later_layer_failing_takes_back_the_baked_ones() {
    let (mut s, id) = scene(8);
    let st = s.active_mut().unwrap();
    st.saved_revision = st.revision;
    let before = s.active().unwrap().doc.clone();
    let steps = s.active().unwrap().history.past_len();
    let journal = s.journal.len();
    let calls = std::cell::Cell::new(0);
    let r = bake_layers(&mut s, vec![LayerId(id), LayerId(id)], |s, id, key| {
        calls.set(calls.get() + 1);
        if calls.get() == 1 { bake_one(s, id, key) } else { Err(EngineError::Other("second layer failed".into())) }
    });
    assert!(r.is_err());
    assert_eq!(calls.get(), 2);
    assert_eq!(layer(&s, id).effects.items.len(), 5);
    assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    assert!(!s.active().unwrap().history.can_redo());
    assert_eq!(s.journal.len(), journal);
    assert!(!s.active().unwrap().is_dirty());
}

/// A layer with no effects is refused before a step starts: the revision does not move.
#[test]
fn a_layer_without_effects_changes_nothing() {
    let (mut s, _) = scene(8);
    let plain = s.execute("layer.new.layer", json!({"name": "plain"})).unwrap()["layer"].as_u64().unwrap();
    let st = s.active_mut().unwrap();
    st.saved_revision = st.revision;
    let (revision, steps) = (s.active().unwrap().revision, s.active().unwrap().history.past_len());
    let err = bake_layers(&mut s, vec![LayerId(plain)], bake_one).unwrap_err().to_string();
    assert!(err.contains("no layer effects"), "{err}");
    assert_eq!(s.active().unwrap().revision, revision);
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    assert!(!s.active().unwrap().is_dirty());
}
