//! #5633: the sequential-dab tools (Dodge, Burn, Sponge, Blur, Sharpen, Smudge) copy and change
//! the canvas, not the whole dab. A soft 5000 px dab reaches about 8,600 × 8,600 px, and its
//! working copy, footprint and write-back used to span all of it, mostly off the canvas: gigabytes
//! at once, whatever the image size. And Smudge never picks up the layer's hidden pixels past the
//! canvas (the review of #5804).

use super::*;
use photocraft_paint::retouch::apply_dab_stroke;

/// `base` with the keys of `extra` added.
fn with_params(mut base: Value, extra: &Value) -> Value {
    if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
        b.extend(e.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    base
}

fn active_surface(s: &Session) -> &Surface {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap()
}

fn damage_rect(r: &Value) -> Rect {
    let v: Vec<i32> = serde_json::from_value(r["damage"].clone()).unwrap();
    Rect::new(v[0], v[1], v[0] + v[2], v[1] + v[3])
}

/// The stroke as paint's unclipped `apply_dab_stroke` paints it, every dab tool's path before
/// #5633: the reference the canvas-clipped commands are held to. The halo is wider than any of
/// these tools reads (Smudge's pickup square for these brushes included), and a wider working copy
/// changes nothing where the dabs and what they read stay on the canvas.
fn unclipped_dab_stroke(s: &Session, cmd: &str, p: &Value) -> Vec<f32> {
    let (stroke, id) = parse_brush(s, p, cmd).unwrap();
    let (tool, _) = DabTool::parse(s, cmd, p, &stroke).unwrap();
    // The surface, selection and transparency lock exactly as `run_stroke` takes them (a new
    // document's Background layer has its transparency locked).
    let mut doc = (*s.active().unwrap().doc).clone();
    let (canvas, sel) = (doc.bounds(), doc.selection.clone());
    let (surf, lock) = crate::channel_cmds::target_surface(&mut doc, id, p).unwrap();
    let mut effect = tool.effect(surf.format(), stroke.brush.spacing);
    apply_dab_stroke(surf, &stroke, sel.as_ref(), lock, 64, |w, fp| effect(w, fp)).unwrap();
    surf.read_region(canvas)
}

#[test]
fn huge_brush_on_a_small_canvas_stays_inside_the_canvas() {
    for (cmd, extra) in dab_cases() {
        let mut s = session(32, 24, 8, "rgb");
        paint_layer(&mut s, texture);
        let canvas = s.active().unwrap().doc.bounds();
        let tiles = active_surface(&s).tile_bounds();
        let r = s.execute(cmd, with_params(json!({"points": [[4, 6], [28, 18]], "size": 300, "hardness": 0}), &extra)).unwrap();
        let dmg = damage_rect(&r);
        assert!(!dmg.is_empty() && canvas.contains_rect(&dmg), "{cmd}: damage {dmg:?} reaches past the canvas {canvas:?}");
        assert_eq!(active_surface(&s).tile_bounds(), tiles, "{cmd}: pixels were written off the canvas");
    }
}

#[test]
fn max_size_soft_brush_on_a_small_image_works_on_the_canvas_only() {
    // #5633's repro: a soft 5000 px brush on a small image. Unclipped, one dab's working copy is
    // about 74 megapixels, far over the 2²⁰ px budget tests run with, so the stroke and its
    // preview would be refused. On the canvas they fit, change only the canvas, and the commit
    // takes the preview's pixels. Smudge's working copy is the canvas plus its whole pickup square,
    // so it gets 900 px: about 6 megapixels unclipped, 968 × 952 px on the canvas.
    for (cmd, extra) in dab_cases() {
        let size = if cmd == "paint.smudge" { 900 } else { 5000 };
        let mut s = session(64, 48, 8, "rgb");
        paint_layer(&mut s, texture);
        let (canvas, tiles) = (s.active().unwrap().doc.bounds(), active_surface(&s).tile_bounds());
        let p = |pts: &[[f64; 2]]| with_params(json!({"points": pts, "size": size, "hardness": 0}), &extra);
        let pts = [[10.0, 10.0], [54.0, 38.0]];
        let r = s.execute(cmd, p(&pts)).unwrap();
        let want = (damage_rect(&r), active_surface(&s).read_region(canvas));
        assert!(canvas.contains_rect(&want.0) && active_surface(&s).tile_bounds() == tiles, "{cmd}: {:?}", want.0);
        s.execute("edit.undo", json!({})).unwrap();
        let mut live = LiveDab::begin(&s, cmd, &p(&pts[..1])).unwrap();
        let step = live.push(&[StrokePoint::new(pts[1][0], pts[1][1], 1.0)]).unwrap();
        assert!(canvas.contains_rect(&live.bounds()) && canvas.contains_rect(&step), "{cmd}: {:?}, step {step:?}", live.bounds());
        let shown = live.doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap();
        assert_eq!(shown.tile_bounds(), tiles, "{cmd}: the preview wrote off the canvas");
        s.offer_live_dab(live).unwrap();
        let taken = s.execute(cmd, p(&pts)).unwrap();
        assert_eq!(taken["fromLive"], json!(true), "{cmd}");
        assert!((damage_rect(&taken), active_surface(&s).read_region(canvas)) == want, "{cmd}: the preview isn't the commit");
    }
}

/// What a clipped live stroke hands its effect at each dab: the working copy's rectangle, the
/// footprint's and the footprint's clip.
type Seen = std::sync::Arc<std::sync::Mutex<Vec<(Rect, Rect, Rect)>>>;

/// Start `cmd` live with a probe effect that records what each dab gets; with the tool's halo.
fn probe_live(s: &Session, cmd: &str, p: &Value) -> (LiveDab, i32, Seen) {
    let (stroke, _) = parse_brush(s, p, cmd).unwrap();
    let halo = DabTool::parse(s, cmd, p, &stroke).unwrap().0.halo();
    let seen = Seen::default();
    let probe = seen.clone();
    let record = move |work: &mut Region, fp: &Footprint| probe.lock().unwrap().push((work.rect, fp.rect, fp.clip));
    (LiveDab::begin_with_effect(s, cmd, p, Box::new(record)).unwrap(), halo, seen)
}

#[test]
fn live_dab_regions_stay_within_the_canvas_plus_halo() {
    // A 300 px soft tip on a 32 × 24 canvas reaches about 260 px past every edge. Each dab's
    // working copy is its part of the canvas plus what the tool reads around it (Blur's halo,
    // Smudge's pickup square), and its footprint stops inside that, at most a halo past the canvas.
    let mut s = session(32, 24, 8, "rgb");
    paint_layer(&mut s, texture);
    let canvas = s.active().unwrap().doc.bounds();
    let p = json!({"points": [[4, 6], [28, 18]], "size": 300, "hardness": 0});
    for (cmd, extra) in dab_cases() {
        let p = with_params(p.clone(), &extra);
        let (mut live, halo, seen) = probe_live(&s, cmd, &p);
        live.push(&[StrokePoint::new(30.0, 4.0, 1.0)]).unwrap();
        assert!(!seen.lock().unwrap().is_empty(), "{cmd}: the effect ran");
        for (work, fp, clip) in seen.lock().unwrap().iter() {
            assert!(canvas.inflate(halo).contains_rect(work), "{cmd}: working copy {work:?} past the canvas plus halo {halo}");
            assert!(work.contains_rect(fp) && *clip == canvas, "{cmd}: footprint {fp:?} (clip {clip:?}), working copy {work:?}");
        }
        // The tool's own preview shows, and changes, the canvas only.
        let mut live = LiveDab::begin(&s, cmd, &p).unwrap();
        let step = live.push(&[StrokePoint::new(30.0, 4.0, 1.0)]).unwrap();
        assert!(canvas.contains_rect(&live.bounds()) && canvas.contains_rect(&step), "{cmd}: {:?}, step {step:?}", live.bounds());
        let shown = live.doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap();
        assert_eq!(shown.tile_bounds(), active_surface(&s).tile_bounds(), "{cmd}: the preview wrote off the canvas");
    }
}

#[test]
fn max_size_soft_blur_on_a_small_image_works_on_the_canvas_only() {
    // #5633's repro at its real size: a soft 5000 px Blur on a small image. Every working copy
    // the preview hands the effect is the canvas plus Blur's halo, and the commit damages only the
    // canvas.
    let mut s = session(64, 48, 8, "rgb");
    paint_layer(&mut s, texture);
    let canvas = s.active().unwrap().doc.bounds();
    let p = json!({"points": [[10, 10], [54, 38]], "size": 5000, "hardness": 0});
    let (mut live, halo, seen) = probe_live(&s, "paint.blur", &p);
    live.push(&[StrokePoint::new(60.0, 4.0, 1.0)]).unwrap();
    assert!(!seen.lock().unwrap().is_empty() && halo > 0, "halo {halo}");
    for (work, fp, _) in seen.lock().unwrap().iter() {
        assert!(canvas.inflate(halo).contains_rect(work) && work.contains_rect(fp), "{work:?} {fp:?}");
    }
    let r = s.execute("paint.blur", p).unwrap();
    assert!(canvas.contains_rect(&damage_rect(&r)));
}

#[test]
fn smudge_paints_as_before_up_to_the_canvas_edge() {
    // Smudge carries paint from dab to dab. A hard 20 px tip covers 13 px either side of its centre
    // (plus a pixel of slack), so this stroke's dabs reach up to the canvas edge, the last one
    // exactly onto it, and lose nothing to the clip.
    for depth in DEPTHS {
        let mut s = session(64, 40, depth, "rgb");
        paint_layer(&mut s, texture);
        let p = json!({"points": [[14, 14], [40, 20], [50, 26]], "size": 20, "hardness": 100, "strength": 80, "spacing": 10});
        let want = unclipped_dab_stroke(&s, "paint.smudge", &p);
        s.execute("paint.smudge", p).unwrap();
        assert!(active_surface(&s).read_region(s.active().unwrap().doc.bounds()) == want, "depth {depth}");
    }
}

#[test]
fn smudge_from_off_the_canvas_drags_in_the_edge_colour() {
    // Smudge's pickup stops at the canvas, so a stroke started past the edge carries the canvas's
    // edge colour in. Before #5633 it carried the layer's off-canvas pixels, which wore the edge
    // away.
    let mut s = session(64, 32, 8, "rgb");
    paint_layer(&mut s, |_, _| [0.9, 0.2, 0.1, 1.0]);
    s.execute("paint.smudge", json!({"points": [[-30, 16], [40, 16]], "size": 20, "hardness": 100, "strength": 90})).unwrap();
    for x in [0, 4, 12] {
        let p = rgba(&s, x, 16);
        assert!(p[3] > 0.99 && (p[0] - 0.9).abs() < 0.02 && (p[1] - 0.2).abs() < 0.02, "x {x}: {p:?}");
    }
}

#[test]
fn live_smudge_from_off_the_canvas_matches_the_commit() {
    // The preview and the commit both skip the dabs that don't reach the canvas, so the preview's
    // carry starts where the commit's does. 32-bit, so the live working copy isn't quantised
    // between dabs.
    let mut s = session(64, 32, 32, "rgb");
    paint_layer(&mut s, texture);
    let all = [[-30.0, 16.0], [10.0, 12.0], [40.0, 20.0]];
    let opts = |p: &[[f64; 2]]| json!({"points": p, "size": 20, "hardness": 100, "strength": 80});
    let mut live = LiveDab::begin(&s, "paint.smudge", &opts(&all[..1])).unwrap();
    live.push(&all[1..].iter().map(|q| StrokePoint::new(q[0], q[1], 1.0)).collect::<Vec<_>>()).unwrap();
    let shown = live.doc.clone();
    // The commit also lays the stroke's finishing dabs (brush smoothing catching up with the last
    // point), which the preview shows only on release: leave out what they cover.
    let (stroke, _) = parse_brush(&s, &opts(&all), "paint.smudge").unwrap();
    let ctx = photocraft_paint::BrushContext::new(&stroke.brush);
    let release_only: Vec<Rect> = photocraft_paint::dabs(&stroke)[live.dabs_done()..].iter().map(|d| ctx.dab_rect(d, false)).collect();
    s.execute("paint.smudge", opts(&all)).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let (a, b) = (shown.layer(id).unwrap().surface().unwrap(), active_surface(&s));
    let (mut changed, mut worst) = (false, 0.0f32);
    for y in 0..32 {
        for x in 0..64 {
            let (got, want) = (a.rgba(x, y), b.rgba(x, y));
            changed |= (want[0] - texture(x, y)[0]).abs() > 0.02;
            if !release_only.iter().any(|r| r.contains(x, y)) {
                worst = (0..4).map(|c| (got[c] - want[c]).abs()).fold(worst, f32::max);
            }
        }
    }
    assert!(changed && worst < 2e-4, "changed {changed}, worst {worst}");
}

#[test]
fn dab_stroke_over_the_pixel_budget_is_refused_and_leaves_the_document() {
    // Tests use a 2²⁰ px budget (the real one is 2³⁰). A stroke across this canvas needs a working
    // copy of the whole 1500 × 1000 canvas; the preview's first step, which brush smoothing holds a
    // few dabs short of the end, still shows well over the budget.
    let mut s = session(1500, 1000, 8, "rgb");
    let p = json!({"points": [[0, 0], [1500, 1000]], "size": 100, "hardness": 100, "exposure": 80});
    let before = (s.active().unwrap().revision, rgba(&s, 750, 500));
    let err = s.execute("paint.dodge", p.clone()).unwrap_err().to_string();
    assert!(err.contains("Dodge Tool") && err.contains("the document was not changed"), "{err}");
    assert_eq!((s.active().unwrap().revision, rgba(&s, 750, 500)), before);
    assert!(LiveDab::begin(&s, "paint.dodge", &p).is_err(), "the live preview is refused too");
}

#[test]
fn dab_stroke_allocation_failure_is_an_error_not_an_abort() {
    let mut s = session(40, 30, 8, "rgb");
    paint_layer(&mut s, texture);
    let p = json!({"points": [[5, 15], [35, 15]], "size": 12});
    let before = (s.active().unwrap().revision, rgba(&s, 20, 15));
    crate::allocation::fail_next_for_test();
    let err = s.execute("paint.blur", p.clone()).unwrap_err().to_string();
    assert!(err.contains("not enough memory"), "{err}");
    assert_eq!((s.active().unwrap().revision, rgba(&s, 20, 15)), before);
    crate::allocation::fail_next_for_test();
    assert!(LiveDab::begin(&s, "paint.blur", &p).is_err(), "the live preview fails the same way");
}

#[test]
fn a_live_batch_that_fails_part_way_is_not_taken_as_the_commit() {
    // The first dabs of the batch are on the working copy when a later copy fails: the commit
    // must render the stroke itself rather than take those pixels.
    let mut s = session(64, 40, 32, "rgb");
    paint_layer(&mut s, texture);
    let p = |pts: &[[f64; 2]]| json!({"points": pts, "size": 12, "hardness": 100, "strength": 80, "spacing": 25});
    let pts = [[5.0, 20.0], [30.0, 20.0], [58.0, 20.0]];
    let mut live = LiveDab::begin(&s, "paint.smudge", &p(&pts[..1])).unwrap();
    crate::allocation::fail_after_for_test(2);
    let batch: Vec<_> = pts[1..].iter().map(|q| StrokePoint::new(q[0], q[1], 1.0)).collect();
    assert!(live.push(&batch).is_err());
    let want = {
        let mut replay = session(64, 40, 32, "rgb");
        paint_layer(&mut replay, texture);
        replay.execute("paint.smudge", p(&pts)).unwrap();
        layer_pixels(&replay)
    };
    let _ = s.offer_live_dab(live);
    let r = s.execute("paint.smudge", p(&pts)).unwrap();
    assert!(r.get("fromLive").is_none() && layer_pixels(&s) == want, "{r}");
}

#[test]
fn dab_tools_paint_as_before_away_from_the_canvas_edge() {
    // The dabs, Blur's halo and Smudge's pickup square stay inside the canvas, so clipping must
    // change nothing at all, at any depth.
    for depth in DEPTHS {
        for (cmd, extra) in dab_cases() {
            let mut s = session(96, 64, depth, "rgb");
            paint_layer(&mut s, texture);
            let p = with_params(json!({"points": [[30, 24], [50, 40], [66, 30]], "size": 16, "hardness": 40, "spacing": 10}), &extra);
            let want = unclipped_dab_stroke(&s, cmd, &p);
            s.execute(cmd, p).unwrap();
            assert!(active_surface(&s).read_region(s.active().unwrap().doc.bounds()) == want, "depth {depth}: {cmd} changed");
        }
    }
}

#[test]
fn per_pixel_dab_tools_paint_as_before_across_the_canvas_edge() {
    // Dodge, Burn and Sponge change each pixel on its own, so even a stroke that runs off the
    // canvas paints the canvas exactly as before.
    for depth in DEPTHS {
        for (cmd, extra) in dab_cases().into_iter().filter(|(c, _)| matches!(*c, "paint.dodge" | "paint.burn" | "paint.sponge")) {
            let mut s = session(96, 64, depth, "rgb");
            paint_layer(&mut s, texture);
            let p = with_params(json!({"points": [[-20, 10], [40, 30], [110, 54]], "size": 40, "hardness": 20, "spacing": 10}), &extra);
            let want = unclipped_dab_stroke(&s, cmd, &p);
            s.execute(cmd, p).unwrap();
            assert!(active_surface(&s).read_region(s.active().unwrap().doc.bounds()) == want, "depth {depth}: {cmd} changed");
        }
    }
}

#[test]
fn blur_and_sharpen_stay_close_to_before_across_the_canvas_edge() {
    // Across the edge the working copy now ends one halo past the canvas, so a later dab reads halo
    // pixels an earlier dab blurred against that boundary, not against the off-canvas pixels beyond.
    // The commit and the live preview do the same (`live_dab_batches_match_the_accumulating_renderer`).
    // Measured on this stroke: Sharpen is identical; Blur is identical at 8-bit and moves by at most
    // 0.000046 at 16-bit (within 4 px of the edge) and 0.000042 at 32-bit (within 8 px), about 0.01
    // of an 8-bit level. The bounds leave room for other platforms' rounding: one 8-bit level at
    // 8-bit (the smallest change it can show), 1e-4 deeper, 16 px from the edge.
    for depth in DEPTHS {
        for cmd in ["paint.blur", "paint.sharpen"] {
            let mut s = session(96, 64, depth, "rgb");
            paint_layer(&mut s, texture);
            let p = json!({"points": [[-20, 10], [40, 30], [110, 54], [60, 70]], "size": 40, "hardness": 20, "spacing": 10, "strength": 100});
            let want = unclipped_dab_stroke(&s, cmd, &p);
            s.execute(cmd, p).unwrap();
            let canvas = s.active().unwrap().doc.bounds();
            let got = active_surface(&s).read_region(canvas);
            let (w, n) = (canvas.width() as usize, active_surface(&s).channels());
            let (mut worst, mut reach) = (0.0f32, 0i32);
            for (i, (a, b)) in got.iter().zip(&want).enumerate() {
                let d = (a - b).abs();
                if d > 0.0 {
                    let (x, y) = (((i / n) % w) as i32, ((i / n) / w) as i32);
                    reach = reach.max(x.min(y).min(canvas.x1 - 1 - x).min(canvas.y1 - 1 - y));
                }
                worst = worst.max(d);
            }
            let tol = if depth == 8 { 1.0 / 255.0 + 1e-6 } else { 1e-4 };
            assert!(worst <= tol, "{cmd} depth {depth}: off by {worst} ({reach} px in from the edge)");
            assert!(reach <= 16, "{cmd} depth {depth}: changed {reach} px in from the edge");
        }
    }
}

/// A uniform canvas over hidden pixels of `sentinel` (a 40 px ring past the canvas, in the layer).
fn over_hidden_pixels(depth: u64, sentinel: f32, all: bool) -> Session {
    let mut s = session(64, 32, depth, "rgb");
    paint_layer(&mut s, |_, _| [0.9, 0.2, 0.1, 1.0]);
    s.edit("hidden pixels", |doc, active| {
        let canvas = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let c = photocraft_raster::from_rgba(&fmt, [sentinel, 1.0 - sentinel, sentinel, 1.0]);
        let o = canvas.inflate(40);
        for r in
            [Rect::new(o.x0, o.y0, o.x1, 0), Rect::new(o.x0, canvas.y1, o.x1, o.y1), Rect::new(o.x0, 0, 0, canvas.y1), Rect::new(canvas.x1, 0, o.x1, canvas.y1)]
        {
            surf.fill_rect(r, &c);
        }
        Ok(())
    })
    .unwrap();
    if all {
        s.execute("layer.new.layer", json!({})).unwrap();
    }
    s
}

/// Every pixel of every layer, the hidden ring included.
fn all_layer_pixels(s: &Session) -> Vec<Vec<f32>> {
    let doc = &s.active().unwrap().doc;
    let around = doc.bounds().inflate(48);
    doc.walk().into_iter().filter_map(|(_, _, l)| l.surface().map(|surf| surf.read_region(around))).collect()
}

#[test]
fn smudge_never_picks_up_hidden_pixels_past_the_canvas() {
    // The review of #5804: Smudge's pickup square reaches past the canvas, where a layer can hold
    // pixels nobody sees. Starting outside a uniform canvas, or entirely outside it, the stroke
    // must carry the canvas's own colour in, whatever is hidden there; the hidden pixels stay as
    // they were, and undo and redo restore exactly. On the layer itself and, with Sample All
    // Layers, onto an empty layer above.
    let uniform = [0.9, 0.2, 0.1, 1.0];
    for depth in DEPTHS {
        for all in [false, true] {
            for (case, pts) in [("entering", [[-30.0, 16.0], [40.0, 16.0]]), ("outside", [[-60.0, 16.0], [-14.0, 16.0]])] {
                let p = json!({"points": pts, "size": 20, "hardness": 100, "strength": 90, "sampleAllLayers": all});
                let mut results = Vec::new();
                for sentinel in [0.0, 1.0] {
                    let what = format!("depth {depth} all {all} {case} sentinel {sentinel}");
                    let mut s = over_hidden_pixels(depth, sentinel, all);
                    let before = all_layer_pixels(&s);
                    let r = s.execute("paint.smudge", p.clone()).unwrap();
                    let canvas = s.active().unwrap().doc.bounds();
                    let after = all_layer_pixels(&s);
                    // The hidden ring is untouched on every layer.
                    let ring = |px: &[f32]| -> Vec<f32> {
                        let full = Region { rect: canvas.inflate(48), ch: 4, data: px.to_vec() };
                        (full.rect.y0..full.rect.y1)
                            .flat_map(|y| (full.rect.x0..full.rect.x1).map(move |x| (x, y)))
                            .filter(|&(x, y)| !canvas.contains(x, y))
                            .flat_map(|(x, y)| full.px(x, y).to_vec())
                            .collect()
                    };
                    for (b, a) in before.iter().zip(&after) {
                        assert!(ring(b) == ring(a), "{what}: hidden pixels changed");
                    }
                    // What the stroke laid on the canvas is the canvas's colour.
                    for y in 0..canvas.y1 {
                        for x in 0..canvas.x1 {
                            let px = rgba(&s, x, y);
                            if all && px[3] == 0.0 {
                                continue;
                            }
                            assert!((0..4).all(|c| (px[c] - uniform[c]).abs() <= tol(depth)), "{what}: ({x},{y}) {px:?}");
                        }
                    }
                    if case == "entering" && all {
                        assert!(rgba(&s, 10, 16)[3] > 0.99, "{what}: the stroke smudged onto the empty layer");
                    }
                    if case == "outside" {
                        assert!(damage_rect(&r).is_empty() && after == before, "{what}: {r}");
                    }
                    s.execute("edit.undo", json!({})).unwrap();
                    assert!(all_layer_pixels(&s) == before, "{what}: undo");
                    s.execute("edit.redo", json!({})).unwrap();
                    assert!(all_layer_pixels(&s) == after, "{what}: redo");
                    results.push(after);
                }
                assert!(results[0] != results[1], "depth {depth} all {all} {case}: the sentinels differ");
                let canvas_only = |layers: &[Vec<f32>]| -> Vec<Vec<f32>> {
                    layers
                        .iter()
                        .map(|px| Region { rect: Rect::new(0, 0, 64, 32).inflate(48), ch: 4, data: px.clone() }.crop(Rect::new(0, 0, 64, 32)).data)
                        .collect()
                };
                assert!(canvas_only(&results[0]) == canvas_only(&results[1]), "depth {depth} all {all} {case}: the hidden pixels showed through");
            }
        }
    }
}

#[test]
fn live_smudge_over_hidden_pixels_is_the_commit() {
    // The preview and the commit skip the same off-canvas dabs and bound the same pickups, so the
    // commit takes the preview's pixels and they are what the command paints on its own.
    for depth in DEPTHS {
        for sentinel in [0.0, 1.0] {
            let mut s = over_hidden_pixels(depth, sentinel, false);
            s.edit("texture", |doc, active| {
                let b = doc.bounds();
                let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                let fmt = surf.format();
                let data: Vec<f32> = (b.y0..b.y1).flat_map(|y| (b.x0..b.x1).flat_map(move |x| photocraft_raster::from_rgba(&fmt, texture(x, y)))).collect();
                surf.write_region(b, &data);
                Ok(())
            })
            .unwrap();
            let pts = [[-30.0, 16.0], [10.0, 12.0], [40.0, 20.0], [70.0, 6.0]];
            let p = |pts: &[[f64; 2]]| json!({"points": pts, "size": 20, "hardness": 70, "strength": 80, "spacing": 25});
            let rendered = s.execute("paint.smudge", p(&pts)).unwrap();
            let want = all_layer_pixels(&s);
            s.execute("edit.undo", json!({})).unwrap();
            let mut live = LiveDab::begin(&s, "paint.smudge", &p(&pts[..1])).unwrap();
            live.push(&pts[1..].iter().map(|q| StrokePoint::new(q[0], q[1], 1.0)).collect::<Vec<_>>()).unwrap();
            s.offer_live_dab(live).unwrap();
            let taken = s.execute("paint.smudge", p(&pts)).unwrap();
            assert_eq!(taken["fromLive"], json!(true), "depth {depth}");
            assert_eq!(taken["damage"], rendered["damage"], "depth {depth}");
            assert!(all_layer_pixels(&s) == want, "depth {depth} sentinel {sentinel}: the preview isn't the commit");
        }
    }
}
