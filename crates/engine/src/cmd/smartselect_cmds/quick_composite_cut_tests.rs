//! #12921 (CodeRabbit on #14824): transparency is an edge for the cut only on the layer path
//! (Sample All Layers off). With Sample All Layers on, a composite that has transparent parts (a
//! document with no opaque background) is cut as before: the mid-grey behind it, not an edge.

use super::*;

const W: u32 = 800;
const H: u32 = 600;

/// The issue's layer, alone in a document with no background: the grey ramp (100, 105, …, 145 from
/// x 100, 40 px blocks at y 350–450) and a red rectangle on transparency.
fn document_without_background() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": 8, "background": "transparent"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    paint_new_layer(&mut s, W, H, |x, y| {
        let (x, y) = (x as i32, y as i32);
        if (100..300).contains(&x) && (100..250).contains(&y) {
            return [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0];
        }
        if (100..500).contains(&x) && (350..450).contains(&y) {
            let v = (100 + 5 * ((x - 100) / 40)) as f32 / 255.0;
            return [v, v, v, 1.0];
        }
        [0.0, 0.0, 0.0, 0.0]
    });
    s
}

/// Writes `px` (straight RGBA) over the whole surface of the last layer, which `file.new` plus
/// `layer.new.layer` leaves transparent.
fn paint_new_layer(s: &mut Session, w: u32, h: u32, px: impl Fn(u32, u32) -> [f32; 4]) {
    s.edit("paint", |doc, _| {
        let Some(surf) = doc.layers.last_mut().and_then(Layer::surface_mut) else {
            return Err(EngineError::Other("the new layer has no surface".into()));
        };
        let mut data = Vec::with_capacity((w * h) as usize * surf.channels());
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&px(x, y));
            }
        }
        surf.write_region(Rect::new(0, 0, w as i32, h as i32), &data);
        Ok(())
    })
    .unwrap();
}

fn bounds(r: &Value) -> Vec<i64> {
    r["bounds"].as_array().unwrap().iter().filter_map(Value::as_i64).collect()
}

#[test]
fn sample_all_layers_keeps_its_cut_on_a_transparent_composite() {
    let mut s = document_without_background();
    let on = s.execute("select.quick", json!({"points": [[120.0, 400.0]], "size": 30, "mode": "new", "sampleAllLayers": true})).unwrap();
    // The ramp is cut at its 5-level steps as before: the 100 block alone, as with the plain pixels
    // (the layer path's cut, with transparency as an edge, takes the whole ramp: bounds 100,350,400,100).
    assert_eq!(bounds(&on), [99, 349, 42, 102], "a click on the 100 block selects that block");
    s.execute("select.deselect", json!({})).unwrap();
    let off = s.execute("select.quick", json!({"points": [[120.0, 400.0]], "size": 30, "mode": "new", "sampleAllLayers": false})).unwrap();
    let whole = bounds(&off);
    assert!(whole[2] > 300, "Sample All Layers off takes the whole ramp: {whole:?}");
}

/// CodeRabbit on #14824: with Sample All Layers off but an adjustment layer active, there is no
/// layer surface to sample, so the cut reads the composite and keeps its cut as before.
#[test]
fn an_active_adjustment_layer_keeps_the_composite_cut() {
    let mut s = document_without_background();
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let active = s.active().unwrap().active_layer.and_then(|id| s.active().unwrap().doc.layer(id).map(|l| l.surface().is_none())).unwrap_or(false);
    assert!(active, "the adjustment layer is active and has no pixels");
    let r = s.execute("select.quick", json!({"points": [[120.0, 400.0]], "size": 30, "mode": "new", "sampleAllLayers": false})).unwrap();
    assert_eq!(bounds(&r), [99, 349, 42, 102], "the composite is cut as before: the 100 block alone");
}

/// CodeRabbit on #14824: 16-bit documents were not measured and keep their cut.
#[test]
fn a_16_bit_layer_keeps_its_cut() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": 16, "background": "transparent"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    paint_new_layer(&mut s, W, H, |x, y| {
        let (x, y) = (x as i32, y as i32);
        if (100..500).contains(&x) && (350..450).contains(&y) {
            let v = (100 + 5 * ((x - 100) / 40)) as f32 / 255.0;
            return [v, v, v, 1.0];
        }
        [0.0, 0.0, 0.0, 0.0]
    });
    let r = s.execute("select.quick", json!({"points": [[120.0, 400.0]], "size": 30, "mode": "new", "sampleAllLayers": false})).unwrap();
    assert_eq!(bounds(&r), [99, 349, 42, 102], "the 100 block alone, as before this change");
}
