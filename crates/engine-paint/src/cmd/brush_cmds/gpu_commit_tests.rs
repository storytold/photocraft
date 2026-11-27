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

/// The stand-in GPU's coverage of tile `key` as the CPU stroke has it now.
fn tile_of(cpu: &LiveStroke, key: (i32, i32)) -> Vec<f32> {
    cpu.renderer.coverage_in(Rect::new(key.0 * 64, key.1 * 64, key.0 * 64 + 64, key.1 * 64 + 64))
}

/// Feed `keys` as of `as_of`.
fn feed(gpu: &mut LiveStroke, cpu: &LiveStroke, keys: &[(i32, i32)], as_of: u64) {
    for &k in keys {
        gpu.feed_coverage(k, &tile_of(cpu, k), as_of);
    }
}

#[test]
fn fed_coverage_commits_what_paint_stroke_writes() {
    for (name, order) in [("in order", 0), ("reversed", 1), ("stale first, then current", 2)] {
        let mut p = brush(json!({}));
        let (mut s, mut gpu, cpu) = strokes(&mut p);
        let (keys, e) = gpu.unfed_keys();
        assert!(keys.len() > 1, "{name}: the stroke spans several coverage tiles");
        match order {
            1 => feed(&mut gpu, &cpu, &keys.iter().rev().copied().collect::<Vec<_>>(), e),
            2 => {
                // A tile fed once with something else, as a read-back of an earlier state would
                // be, is replaced by the next feed.
                for &k in &keys {
                    gpu.feed_coverage(k, &vec![0.25; 64 * 64], e);
                }
                feed(&mut gpu, &cpu, &keys, e);
            }
            _ => feed(&mut gpu, &cpu, &keys, e),
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
    let (keys, e) = gpu.unfed_keys();
    feed(&mut gpu, &cpu, &keys[1..], e);
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

/// A symmetric stroke paints mirrored passes the GPU does not stamp: it is refused, so the caller
/// draws it on the CPU, and a CPU overlay stroke of the same session reports the same refusal.
#[test]
fn a_symmetric_stroke_is_refused() {
    let mut s = scene(8, &|_| ());
    s.execute("paint.setSymmetry", json!({"mode": "vertical"})).unwrap();
    let mut p = brush(json!({}));
    p["points"] = json!(PTS);
    assert!(LiveStroke::begin_gpu(&s, "paint.stroke", &p).is_err());
    let cpu = LiveStroke::begin_overlay(&s, "paint.stroke", &p).unwrap();
    assert!(cpu.gpu_preview().is_err(), "no GPU preview of a symmetric stroke");
}

/// A read-back asked for: the tiles' coverage as it was, and the epoch it was asked at.
type AskedFor = (Vec<((i32, i32), Vec<f32>)>, u64);

/// The strokes of `PTS` one point at a time with a small brush, so most tiles go calm while the
/// stroke is still drawn; `deliver` decides when the calm tiles' coverage arrives. Returns the
/// session, the GPU stroke after its last read-back and the tiles that were asked for mid-stroke.
fn drawn_with_calm_reads(p: &mut Value, delayed: bool) -> (Session, LiveStroke, Vec<(i32, i32)>) {
    let s = scene(8, &|_| ());
    p["points"] = json!([PTS[0]]);
    let (mut gpu, _) = LiveStroke::begin_gpu(&s, "paint.stroke", p).unwrap();
    let mut cpu = LiveStroke::begin_overlay(&s, "paint.stroke", p).unwrap();
    let mut asked: Vec<(i32, i32)> = Vec::new();
    // Read-backs asked for and not yet delivered: the tiles' coverage as it was, and when.
    let mut in_flight: Vec<AskedFor> = Vec::new();
    for c in PTS[1..].chunks(1) {
        gpu.push_dabs(&points(c)).unwrap();
        cpu.push(&points(c)).unwrap();
        let (keys, e) = gpu.calm_keys(1);
        asked.extend(&keys);
        let tiles: Vec<_> = keys.iter().map(|k| (*k, tile_of(&cpu, *k))).collect();
        if delayed {
            in_flight.push((tiles, e));
        } else {
            for (k, v) in &tiles {
                gpu.feed_coverage(*k, v, e);
            }
        }
    }
    gpu.finish_dabs();
    cpu.renderer.finish();
    // Delayed read-backs arrive after the dabs that came since.
    for (tiles, e) in &in_flight {
        for (k, v) in tiles {
            gpu.feed_coverage(*k, v, *e);
        }
    }
    let (rest, e) = gpu.unfed_keys();
    // A tile asked for during the stroke and touched again afterwards is still to read: what was
    // fed for it, current when it was asked for, no longer includes every dab.
    assert!(rest.iter().any(|k| asked.contains(k)), "a tile read back mid-stroke was touched again and is unfed (asked {asked:?}, left {rest:?})");
    assert!(!asked.is_empty(), "some tiles went calm and were asked for during the stroke");
    feed(&mut gpu, &cpu, &rest, e);
    (s, gpu, asked)
}

fn small_brush() -> Value {
    brush(json!({"brush": {"size": 14, "hardness": 0.3, "flow": 0.6}}))
}

#[test]
fn coverage_read_back_during_the_stroke_commits_what_paint_stroke_writes() {
    for (name, delayed) in [("fed as asked for", false), ("fed late, after later dabs", true)] {
        let mut p = small_brush();
        let (mut s, gpu, _) = drawn_with_calm_reads(&mut p, delayed);
        let seed = gpu.seed;
        s.offer_live_stroke(gpu).unwrap();
        p["points"] = json!(PTS);
        p["seed"] = json!(seed);
        assert!(s.execute("paint.stroke", p.clone()).unwrap().get("fromLive").is_some());
        let mut plain = scene(8, &|_| ());
        plain.execute("paint.stroke", p).unwrap();
        assert!(same_pixels(&surface(&s), &surface(&plain), Rect::new(0, 0, W, H)), "{name}: the commit is paint.stroke's pixels");
    }
}

/// A feed that predates a touch of its tile does not make the tile current, whatever the order
/// the tiles arrive in.
#[test]
fn a_feed_older_than_a_touch_does_not_count() {
    let mut p = small_brush();
    p["points"] = json!([PTS[0]]);
    let s = scene(8, &|_| ());
    let (mut gpu, _) = LiveStroke::begin_gpu(&s, "paint.stroke", &p).unwrap();
    let mut cpu = LiveStroke::begin_overlay(&s, "paint.stroke", &p).unwrap();
    let (keys, early) = gpu.unfed_keys();
    feed(&mut gpu, &cpu, &keys, early);
    assert_eq!(gpu.unfed_keys().0.len(), 0, "every tile is current when nothing touched it since");
    gpu.push_dabs(&points(&PTS[1..2])).unwrap();
    cpu.push(&points(&PTS[1..2])).unwrap();
    let (now, late) = gpu.unfed_keys();
    assert!(!now.is_empty(), "the dab touched a tile");
    // The older read-back arrives again: it is not newer than what the tile holds and not current.
    feed(&mut gpu, &cpu, &now, early);
    assert_eq!(gpu.unfed_keys().0, now, "a read-back as of before the touch leaves the tile unfed");
    feed(&mut gpu, &cpu, &now, late);
    assert!(gpu.unfed_keys().0.is_empty(), "a read-back as of after it makes it current");
}

/// A smoothed GPU stroke's tail is what ending it makes, as a preview: it touches no coverage tile
/// and grows no coverage bounds (so no read-back is ever asked for it), and the `redraw` rectangle
/// holds it. `finish_dabs` then makes the same dabs for real.
#[test]
fn a_smoothed_gpu_strokes_tail_is_what_finishing_makes_and_touches_nothing() {
    // The smoothing tail is there only with `catchUpOnEnd`.
    let mut p = brush(json!({"smoothing": 0.7, "brush": {"size": 22, "hardness": 0.3, "flow": 0.6, "smoothing": {"catchUpOnEnd": true}}}));
    let s = scene(8, &|_| ());
    p["points"] = json!([PTS[0]]);
    let (mut gpu, _) = LiveStroke::begin_gpu(&s, "paint.stroke", &p).unwrap();
    for c in PTS[1..].chunks(2) {
        gpu.push_dabs(&points(c)).unwrap();
    }
    let (touched, bounds) = (gpu.touched_keys(), gpu.renderer.bounds());
    let (tail, redraw) = gpu.tail_dabs();
    assert!(!tail.is_empty(), "a smoothed stroke lags its pen");
    assert_eq!((gpu.touched_keys(), gpu.renderer.bounds()), (touched, bounds), "the preview tail changes nothing of the stroke");
    assert!(tail.iter().all(|d| d.rect.intersect(&redraw) == d.rect), "the redraw rectangle holds the tail");
    // The next tail's redraw also covers this one's.
    let (_, next) = gpu.tail_dabs();
    assert!(tail.iter().all(|d| d.rect.intersect(&next) == d.rect), "the old tail is redrawn too");
    let finish = gpu.finish_dabs();
    assert_eq!(tail.iter().map(|d| d.rect).collect::<Vec<_>>(), finish.iter().map(|d| d.rect).collect::<Vec<_>>(), "finishing makes the tail's dabs");
}
