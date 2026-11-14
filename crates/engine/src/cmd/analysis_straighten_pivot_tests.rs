//! Straighten Layer turns the layer about the centre of its content bounds and records one History
//! step named Free Transform, as the reference application does (#11992). The Background / crop
//! path keeps its own step.

use photocraft_doc::{LayerContent, LayerId};

use super::*;

const CMD: &str = "image.analysis.straightenLayer";

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    s
}

fn layer_of(s: &Session, id: LayerId) -> &photocraft_doc::Layer {
    s.active().unwrap().doc.layer(id).unwrap()
}

fn bounds(s: &Session, id: LayerId) -> Rect {
    crate::layer_multi_cmds::layer_bounds(layer_of(s, id)).unwrap()
}

fn centre(r: Rect) -> [f64; 2] {
    [f64::from(r.x0 + r.x1) / 2.0, f64::from(r.y0 + r.y1) / 2.0]
}

fn last_step(s: &Session) -> String {
    s.active().unwrap().history.entries().last().cloned().unwrap_or_default()
}

/// "Colors": 200 x 150 at 100,100, ruler from 100,100 to 300,250 (A -36.9 degrees).
fn colors() -> (Session, LayerId) {
    let mut s = session();
    let l = LayerId(s.execute("layer.new.layer", json!({"name": "Colors"})).unwrap()["layer"].as_u64().unwrap());
    s.edit("paint", |doc, _| {
        doc.layer_mut(l).unwrap().surface_mut().unwrap().fill_rect(Rect::new(100, 100, 300, 250), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("image.analysis.rulerTool", json!({"start": [100, 100], "end": [300, 250]})).unwrap();
    (s, l)
}

#[test]
fn the_layer_turns_about_its_own_centre_as_one_free_transform_step() {
    let (mut s, l) = colors();
    let steps = s.active().unwrap().history.past_len();
    let r = s.execute(CMD, json!({})).unwrap();
    assert_eq!(r["cropped"], false, "{r}");
    let b = bounds(&s, l);
    // Measured in the reference: 75,54 with size 250 x 241, within 1 px.
    let got = [b.x0, b.y0, b.x1 - b.x0, b.y1 - b.y0];
    for (g, want) in got.iter().zip([75, 54, 250, 241]) {
        assert!((g - want).abs() <= 1, "bounds {got:?}, want about [75, 54, 250, 241]");
    }
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one History step");
    assert_eq!(last_step(&s), "Free Transform");
    let d = &s.active().unwrap().doc;
    assert!(d.measurement.ruler.is_none(), "the ruler is cleared");
    assert_eq!((d.size.width, d.size.height), (800, 600), "the canvas keeps its size");
}

#[test]
fn text_and_smart_object_layers_stay_what_they_are_and_turn_in_place() {
    let mut s = session();
    let text = LayerId(s.execute("type.create", json!({"x": 500, "y": 400, "text": "Hello", "size": 40})).unwrap()["layer"].as_u64().unwrap());
    let (mut t, pixels) = colors();
    t.execute("layer.smartObjects.convertToSmartObject", json!({"layer": pixels.0})).unwrap();
    // The smart object is a new layer, and the active one.
    let smart = t.active().unwrap().active_layer.unwrap();
    s.execute("image.analysis.rulerTool", json!({"start": [100, 100], "end": [300, 250]})).unwrap();
    t.execute("image.analysis.rulerTool", json!({"start": [100, 100], "end": [300, 250]})).unwrap();
    for (what, s, id) in [("text", &mut s, text), ("smart object", &mut t, smart)] {
        let c0 = centre(bounds(s, id));
        s.execute(CMD, json!({})).unwrap_or_else(|e| panic!("{what}: {e}"));
        let c1 = centre(bounds(s, id));
        // Type turns about its Free Transform frame (the type box), so only the smart object's ink
        // centre is pinned; the reference did not measure a text pivot.
        let tol = if what == "text" { 10.0 } else { 2.0 };
        assert!((c0[0] - c1[0]).abs() <= tol && (c0[1] - c1[1]).abs() <= tol, "{what}: centre {c0:?} moved to {c1:?}");
        let kept = match &layer_of(s, id).content {
            LayerContent::Text(_) => what == "text",
            LayerContent::Smart(_) => what == "smart object",
            _ => false,
        };
        assert!(kept, "{what}: transformed, not rasterized");
        assert_eq!(last_step(s), "Free Transform", "{what}");
    }
}

#[test]
fn the_crop_path_keeps_its_straighten_step() {
    let (mut s, _) = colors();
    let r = s.execute(CMD, json!({"crop": true})).unwrap();
    assert_eq!(r["cropped"], true, "{r}");
    assert_eq!(last_step(&s), "Straighten");
}
