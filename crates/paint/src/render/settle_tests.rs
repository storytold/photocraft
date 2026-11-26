//! A stroke composited tile by tile while it is drawn (`StrokeRenderer::settle`) ends as the
//! pixels one composite of the whole stroke writes.

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

fn pre() -> Surface {
    let mut s = Surface::with_default(PixelFormat::RGBA8, &[0.9, 0.85, 0.7, 1.0]);
    let r = Rect::new(20, 10, 260, 110);
    let data: Vec<f32> = (r.y0..r.y1)
        .flat_map(|y| {
            (r.x0..r.x1).flat_map(move |x| [((x * 7 + y * 3) % 31) as f32 / 30.0, 0.5, ((x + y) % 11) as f32 / 10.0, if (x + y) % 6 == 0 { 0.4 } else { 1.0 }])
        })
        .collect();
    s.write_region(r, &data);
    s
}

fn path() -> Vec<StrokePoint> {
    (0..48).map(|i| StrokePoint::new(30.0 + 5.6 * f64::from(i), 70.0 + 30.0 * (f64::from(i) / 7.0).sin(), 0.4 + 0.6 * ((i * 5) % 7) as f32 / 6.0)).collect()
}

fn same(a: &Surface, b: &Surface) -> bool {
    (0..H).all(|y| (0..W).all(|x| a.rgba(x, y).map(f32::to_bits) == b.rgba(x, y).map(f32::to_bits)))
}

/// Draws the stroke one point at a time, settling `max` tiles every `every` points, then compares
/// with one composite of the finished stroke. Returns how many tiles `settle` wrote mid-stroke.
fn check(name: &str, b: &BrushSettings, every: usize, max: usize, lock: bool, selection: Option<&Surface>) -> usize {
    let pre = pre();
    let mut shadow = pre.clone();
    let mut r = StrokeRenderer::new(b, Some(pre.format()), 1.0);
    let mut early = 0;
    for (i, p) in path().iter().enumerate() {
        r.push(std::slice::from_ref(p));
        if i % every == 0 {
            let before = r.unsettled_keys(false).len();
            let left = r.settle(&pre, &mut shadow, selection, lock, max);
            early += before - left;
            assert!(before - left <= max, "{name}: settled {} tiles for a limit of {max}", before - left);
        }
    }
    let mut whole = StrokeRenderer::new(b, Some(pre.format()), 1.0);
    whole.push(&path());
    whole.finish();
    let mut expected = pre.clone();
    whole.composite(&pre, &mut expected, selection, lock, true);
    r.finish();
    r.settle_all(&pre, &mut shadow, selection, lock);
    assert!(same(&shadow, &expected), "{name}: the settled stroke differs from one composite");
    assert!(r.unsettled_keys(true).is_empty(), "{name}: tiles left unsettled");
    early
}

#[test]
fn a_stroke_settled_along_the_way_equals_one_composite() {
    let mut sel = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
    sel.fill_rect(Rect::new(60, 20, 250, 120), &[1.0]);
    let cases = [
        ("plain", brush(json!({}))),
        ("dual brush", brush(json!({"dualBrush": {"enabled": true, "size": 11, "spacing": 0.4}}))),
        ("wet edges", brush(json!({"wetEdges": true}))),
        ("texture", brush(json!({"texture": {"enabled": true, "depth": 0.7}}))),
        ("smoothing", brush(json!({"smoothing": {"amount": 0.6}}))),
        ("erase", brush(json!({"erase": true}))),
    ];
    for (name, b) in &cases {
        for (every, max) in [(1, 1), (3, 2), (7, 1000)] {
            let early = check(name, b, every, max, false, None);
            assert!(early > 0, "{name}: nothing was settled while drawing (every {every}, {max} at a time)");
        }
        check(name, b, 2, 3, true, Some(&sel));
    }
}

#[test]
fn settle_leaves_the_tiles_the_latest_push_touched() {
    let pre = pre();
    let mut shadow = pre.clone();
    let b = brush(json!({}));
    let mut r = StrokeRenderer::new(&b, Some(pre.format()), 1.0);
    r.push(&[StrokePoint::new(60.0, 60.0, 1.0), StrokePoint::new(110.0, 60.0, 1.0)]);
    assert_eq!(r.settle(&pre, &mut shadow, None, false, 1000), 0);
    assert!(same(&shadow, &pre), "everything was touched by the only push");
    r.push(&[StrokePoint::new(240.0, 100.0, 1.0)]);
    let left = r.settle(&pre, &mut shadow, None, false, 1);
    assert!(left > 0, "more than one tile was left from the first push");
    r.settle(&pre, &mut shadow, None, false, 1000);
    assert!(!same(&shadow, &pre), "the first push's tiles settled");
    assert_eq!(shadow.rgba(240, 100), pre.rgba(240, 100), "the latest push's tiles did not");
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
