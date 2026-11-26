//! The live stroke mask (`StrokeRenderer::composite_live_mask`) is what a full redraw of the
//! stroke so far writes.

use super::*;
use serde_json::json;

const W: i32 = 320;
const H: i32 = 140;

fn brush(extra: serde_json::Value) -> BrushSettings {
    let mut b = json!({"size": 36, "hardness": 0.4, "flow": 0.6, "opacity": 0.8, "color": [0.8, 0.1, 0.3, 1.0], "seed": 5});
    for (k, v) in extra.as_object().unwrap() {
        b[k] = v.clone();
    }
    serde_json::from_value(b).unwrap()
}

fn path() -> Vec<StrokePoint> {
    (0..48).map(|i| StrokePoint::new(30.0 + 5.6 * f64::from(i), 70.0 + 30.0 * (f64::from(i) / 7.0).sin(), 0.4 + 0.6 * ((i * 5) % 7) as f32 / 6.0)).collect()
}

/// A mask after every live step is what a full redraw of the stroke so far writes, so a step that
/// adds Dual Brush dabs over tiles the stroke already covers shows them too.
#[test]
fn a_dual_brush_only_step_redraws_the_live_mask() {
    // Far-apart stroke dabs and close Dual Brush dabs: most pushes add only the latter.
    let b = brush(json!({"size": 40, "spacing": 2.0, "dualBrush": {"enabled": true, "size": 6, "spacing": 0.2}}));
    let mut r = StrokeRenderer::new(&b, None, 1.0);
    let canvas = Rect::new(0, 0, W, H);
    let mut live = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
    let (mut old, mut dual_only) = (Rect::EMPTY, 0);
    for p in path() {
        r.push(&[p]);
        let covered = r.dual.as_ref().is_some_and(|d| d.dirty.iter().any(|k| r.cov.tiles.contains_key(&tile_key(*k))));
        dual_only += usize::from(r.cov.dirty.is_empty() && covered);
        old = r.composite_live_mask(old, None, &mut live, false).1;
        let mut again = r.clone();
        let mut fresh = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        again.composite_mask(None, &mut fresh, false, true);
        if let Some(mut tail) = again.tail_preview() {
            tail.composite_mask(None, &mut fresh, false, false);
        }
        assert_eq!(live.read_region(canvas), fresh.read_region(canvas), "after the push at x = {}", p.x);
    }
    assert!(dual_only > 0, "no push added only Dual Brush dabs, so the test did not cover them");
}
