//! A live stroke whose coverage a GPU draws (`LiveStroke::begin_gpu`) commits through the code a
//! CPU stroke's coverage goes through. Here a CPU-built stroke stands in for the GPU: its coverage
//! of each tile the dabs touched is fed back at the end, in any order and more than once, and the
//! commit must be the pixels `paint.stroke` writes, bit for bit. That guards the bookkeeping
//! (which tiles are touched, the bounds, refusing a tile left unfed), not the GPU.

use super::overlay_tests::{PTS, brush, points, same_pixels, scene, surface};
use super::*;
use crate::brush_cmds::LiveStrokeExt;

const W: i32 = 120;
const H: i32 = 80;

/// The stroke on a scene, its dabs generated as the canvas would, and the CPU stroke that stands
/// in for the GPU's coverage.
fn strokes(p: &mut Value) -> (Session, LiveStroke, LiveStroke) {
    let s = scene(8, &|_| ());
    p["points"] = json!([PTS[0]]);
    let (mut gpu, _) = LiveStroke::begin_gpu(&s, "paint.stroke", p).unwrap();
    let mut cpu = LiveStroke::begin_overlay(&s, "paint.stroke", p).unwrap();
    for c in PTS[1..].chunks(2) {
        gpu.push_dabs(&points(c)).unwrap();
        cpu.push(&points(c)).unwrap();
    }
    gpu.finish_dabs();
    cpu.renderer.finish();
    (s, gpu, cpu)
}

fn feed(gpu: &mut LiveStroke, cpu: &LiveStroke, keys: &[(i32, i32)]) {
    for &(kx, ky) in keys {
        gpu.feed_coverage((kx, ky), &cpu.renderer.coverage_in(Rect::new(kx * 64, ky * 64, kx * 64 + 64, ky * 64 + 64)));
    }
}

#[test]
fn fed_coverage_commits_what_paint_stroke_writes() {
    for (name, order) in [("in order", 0), ("reversed", 1), ("stale first, then current", 2)] {
        let mut p = brush(json!({}));
        let (mut s, mut gpu, cpu) = strokes(&mut p);
        let keys = gpu.touched_keys();
        assert!(keys.len() > 1, "{name}: the stroke spans several coverage tiles");
        match order {
            1 => feed(&mut gpu, &cpu, &keys.iter().rev().copied().collect::<Vec<_>>()),
            2 => {
                // A tile fed once with something else, as a read-back of an earlier state would
                // be, is replaced by the next feed.
                for &k in &keys {
                    gpu.feed_coverage(k, &vec![0.25; 64 * 64]);
                }
                feed(&mut gpu, &cpu, &keys);
            }
            _ => feed(&mut gpu, &cpu, &keys),
        }
        let seed = gpu.seed;
        s.offer_live_stroke(gpu).unwrap();
        p["points"] = json!(PTS);
        p["seed"] = json!(seed);
        let taken = s.execute("paint.stroke", p.clone()).unwrap();
        assert!(taken.get("fromLive").is_some(), "{name}: the commit took the live pixels");
        let mut plain = scene(8, &|_| ());
        plain.execute("paint.stroke", p).unwrap();
        assert!(same_pixels(&surface(&s), &surface(&plain), Rect::new(0, 0, W, H)), "{name}: the commit is paint.stroke's pixels");
    }
}

#[test]
fn a_commit_with_coverage_still_to_read_back_is_refused() {
    let mut p = brush(json!({}));
    let (_, mut gpu, cpu) = strokes(&mut p);
    let keys = gpu.touched_keys();
    feed(&mut gpu, &cpu, &keys[1..]);
    assert!(gpu.into_commit().is_err(), "a tile was never fed");
}

#[test]
fn a_gpu_stroke_draws_no_coverage_itself() {
    let mut p = brush(json!({}));
    let (_, gpu, _) = strokes(&mut p);
    assert!(gpu.bounds().width() > 0, "the dabs' rectangles are the stroke's bounds");
    assert_eq!(gpu.renderer.coverage_at(60, 40), 0.0, "nothing was rasterised");
}

#[test]
fn a_stroke_the_gpu_cannot_draw_is_refused_with_a_reason() {
    let s = scene(8, &|_| ());
    let mut p = brush(json!({"erase": true}));
    p["points"] = json!(PTS);
    assert!(LiveStroke::begin_gpu(&s, "paint.stroke", &p).is_err());
}
