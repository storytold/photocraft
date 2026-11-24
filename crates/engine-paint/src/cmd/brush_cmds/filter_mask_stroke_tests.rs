//! FILE-215-3: a brush stroke into a smart object's filter mask (`"target":"filterMask"`) hides
//! the smart filters where it is black, in one undo step.

use photocraft_doc::{LayerContent, SmartObject};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

const DEPTHS: [u64; 3] = [8, 16, 32];
const W: i32 = 64;
const H: i32 = 48;

fn session(depth: u64) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": W, "height": H, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    // A textured block on the new layer, so the blur and the mask show.
    s.edit("setup", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let r = Rect::new(-6, 5, 40, 38);
        let mut data = Vec::new();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let v = ((x * 7 + y * 13) % 23) as f32 / 22.0;
                let a = if (x + y) % 5 == 0 { 0.5 } else { 1.0 };
                data.extend(photocraft_raster::from_rgba(&fmt, [v, 1.0 - v, ((x * y) % 7) as f32 / 6.0, a]));
            }
        }
        surf.write_region(r, &data);
        Ok(())
    })
    .unwrap();
    s
}

fn flat(s: &Session) -> Vec<[f32; 4]> {
    photocraft_compose::flatten(&s.active().unwrap().doc).into_px()
}

fn active_smart(s: &Session) -> SmartObject {
    let d = s.active().unwrap();
    match &d.doc.layer(d.active_layer.unwrap()).unwrap().content {
        LayerContent::Smart(sm) => sm.clone(),
        other => panic!("not smart: {}", other.kind_name()),
    }
}

/// A vertical black stroke over x = 8, as the Brush tool sends it.
fn stroke(s: &mut Session, target: &str) -> Result<Value> {
    s.execute("paint.stroke", json!({"points": [[8, 2], [8, 46]], "size": 8, "hardness": 1, "color": "#000000", "target": target}))
}

#[test]
fn painting_the_filter_mask_hides_the_filters_there_in_one_undo_step() {
    for depth in DEPTHS {
        let mut s = session(depth);
        let original = flat(&s);
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        // Without smart filters there is no filter mask to paint.
        let before = s.active().unwrap().doc.clone();
        let steps = s.active().unwrap().history.past_len();
        assert!(matches!(stroke(&mut s, "filterMask"), Err(EngineError::Disabled(..))), "depth {depth}");
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
        assert_eq!(s.active().unwrap().history.past_len(), steps);
        s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
        let blurred = flat(&s);
        assert!(active_smart(&s).filter_mask.is_none());
        let steps = s.active().unwrap().history.past_len();

        stroke(&mut s, "filterMask").unwrap();
        let mask = active_smart(&s).filter_mask.expect("painting starts a white filter mask");
        assert_eq!((mask.value(8, 20), mask.value(30, 20)), (0.0, 1.0), "depth {depth}");
        let px = flat(&s);
        let at = |v: &[[f32; 4]], x: i32, y: i32| v[(y * W + x) as usize];
        let hidden = at(&px, 8, 20).iter().zip(at(&original, 8, 20)).all(|(a, b)| (a - b).abs() < 1e-6);
        assert!(hidden, "depth {depth}: black hides the blur");
        assert_eq!(at(&px, 30, 20), at(&blurred, 30, 20), "depth {depth}: white keeps it");
        assert_eq!(active_smart(&s).smart_filters.len(), 1, "the stroke is no smart filter");

        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one step");
        s.execute("edit.undo", json!({})).unwrap();
        assert!(active_smart(&s).filter_mask.is_none());
        assert_eq!(flat(&s), blurred, "depth {depth}: undo brings the blur back");
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(flat(&s), px);
    }
}
