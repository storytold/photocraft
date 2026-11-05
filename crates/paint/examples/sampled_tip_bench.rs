//! Where a live stroke with a large sampled (custom) tip spends its CPU time (#8950): a recorded
//! stroke of 500 dabs replayed in frame-sized chunks, as the canvas feeds a live stroke, on a
//! materialised photo layer. Each frame is split into the stages of the brush engine:
//!
//! - `generate`: smoothing, spacing and dynamics (points → dabs, `DabGenerator`);
//! - `rasterize`: each dab's tip stamped into a dense coverage buffer (`BrushContext::rasterize_into`);
//! - `accumulate`: that buffer built up into the stroke's sparse coverage tiles (`CoverageMap::accumulate`);
//! - `composite`: the touched coverage tiles composited onto the layer from the pre-stroke pixels
//!   (`StrokeRenderer::composite`).
//!
//! The first three stages are timed on their own copy of the stroke (the same dabs), the
//! composite on a `StrokeRenderer` fed the same points, so the stages add up to one frame of a
//! live stroke without timer calls inside the engine. `setup` is the per-stroke `BrushContext`
//! (the tip's mip pyramid). The tip is a synthetic grain bitmap (no third-party brush).
//!
//! ```sh
//! cargo run --release -p photocraft-paint --example sampled_tip_bench -- [--canvas 4000x3000] [--brush 300] [--tip 1024] [--dabs 500] [--reps 3] [--angle-jitter 1] [--json out.json]
//! ```

use std::time::Instant;

use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_paint::dynamics::DabGenerator;
use photocraft_paint::render::CoverageMap;
use photocraft_paint::{BrushContext, BrushSettings, Dynamic, GrayTile, ShapeDynamics, StrokePoint, StrokeRenderer, TipShape};
use photocraft_raster::{Surface, from_rgba};
use serde_json::{Value, json};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

/// A deterministic grain tip `side` px square: a soft disc broken up by two octaves of hashed
/// value noise, like a scanned chalk or sponge stamp (zero outside the disc).
pub fn grain_tip(side: u32) -> GrayTile {
    let hash = |x: i64, y: i64| -> f32 {
        let mut h = (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (y as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
        h ^= h >> 29;
        h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        h ^= h >> 32;
        (h >> 40) as f32 / (1u64 << 24) as f32
    };
    let noise = |x: f32, y: f32, cell: f32| {
        let (gx, gy) = (x / cell, y / cell);
        let (x0, y0) = (gx.floor(), gy.floor());
        let (tx, ty) = (gx - x0, gy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let a = hash(x0, y0) + (hash(x0 + 1, y0) - hash(x0, y0)) * tx;
        let b = hash(x0, y0 + 1) + (hash(x0 + 1, y0 + 1) - hash(x0, y0 + 1)) * tx;
        a + (b - a) * ty
    };
    let c = side as f32 / 2.0;
    GrayTile::from_fn(side, side, |x, y| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let d = ((fx - c).powi(2) + (fy - c).powi(2)).sqrt() / c;
        if d >= 1.0 {
            return 0.0;
        }
        let disc = (1.0 - d * d).powf(0.7);
        let n = 0.6 * noise(fx, fy, side as f32 / 24.0) + 0.4 * noise(fx, fy, side as f32 / 96.0);
        (disc * (n * 1.8 - 0.35)).clamp(0.0, 1.0)
    })
}

/// The recorded stroke: a pen scribble across the canvas (a Lissajous path with a pressure
/// swell), sampled every `step` px of arc length until it is `length` px long, with an 8 ms
/// pen report interval.
pub fn recorded_stroke(w: u32, h: u32, length: f64, step: f64) -> Vec<StrokePoint> {
    let (fw, fh) = (f64::from(w), f64::from(h));
    let at = |t: f64| (fw * (0.5 + 0.4 * (t * 1.0).sin()), fh * (0.5 + 0.4 * (t * 1.7 + 0.5).sin()));
    let mut pts = vec![];
    let (mut t, mut walked, mut since) = (0.0f64, 0.0f64, step);
    let mut prev = at(0.0);
    while walked < length && pts.len() < 1_000_000 {
        let p = at(t);
        let d = ((p.0 - prev.0).powi(2) + (p.1 - prev.1).powi(2)).sqrt();
        walked += d;
        since += d;
        if since >= step {
            since = 0.0;
            let i = pts.len();
            pts.push(StrokePoint { time: i as f64 * 8.0, ..StrokePoint::new(p.0, p.1, 0.7 + 0.3 * (t * 3.0).sin().abs() as f32) });
        }
        prev = p;
        t += 0.0005;
    }
    pts
}

#[derive(Default, Clone, Copy)]
struct Frame {
    generate: f64,
    rasterize: f64,
    accumulate: f64,
    composite: f64,
    dabs: usize,
    px: u64,
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn pct(v: &mut [f64], q: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    let k = ((q * v.len() as f64).ceil() as usize).clamp(1, v.len().max(1));
    v.get(k - 1).copied().unwrap_or(f64::NAN)
}

fn median(mut v: Vec<f64>) -> f64 {
    pct(&mut v, 0.5)
}

/// One replay of the stroke; returns (setup ms, frames).
fn replay(brush: &BrushSettings, layer: &Surface, pts: &[StrokePoint], per_frame: usize) -> (f64, Vec<Frame>) {
    let t = Instant::now();
    let ctx = BrushContext::new(brush);
    let setup = ms(t);
    let mut generator = DabGenerator::new(brush, 1.0);
    let mut cov = CoverageMap::new(0);
    let mut renderer = StrokeRenderer::new(brush, Some(layer.format()), 1.0);
    let pre = layer.clone();
    let mut target = layer.clone();
    let (mut dabs, mut dual, mut buf) = (Vec::new(), Vec::new(), Vec::new());
    let mut frames = Vec::new();
    let mut chunks: Vec<&[StrokePoint]> = pts.chunks(per_frame.max(1)).collect();
    chunks.push(&[]);
    let last = chunks.len() - 1;
    for (i, ch) in chunks.into_iter().enumerate() {
        let mut f = Frame::default();
        dabs.clear();
        dual.clear();
        let t = Instant::now();
        if i == last {
            generator.finish(&mut dabs, &mut dual);
        } else {
            generator.push(ch, &mut dabs, &mut dual);
        }
        f.generate = ms(t);
        for d in &dabs {
            let rect = ctx.dab_rect(d, false);
            let t = Instant::now();
            ctx.rasterize_into(d, false, rect, &mut buf, None);
            f.rasterize += ms(t);
            let t = Instant::now();
            cov.accumulate(rect, &buf, d.opacity, None, None);
            f.accumulate += ms(t);
            f.px += rect.width() as u64 * rect.height() as u64;
        }
        f.dabs = dabs.len();
        if i == last {
            renderer.finish();
        } else {
            renderer.push(ch);
        }
        let t = Instant::now();
        renderer.composite(&pre, &mut target, None, false, false);
        f.composite = ms(t);
        frames.push(f);
    }
    std::hint::black_box(&target);
    (setup, frames)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let canvases: Vec<(u32, u32)> = match arg(&args, "--canvas") {
        Some(c) => c.split(',').filter_map(|s| s.split_once('x')).filter_map(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))).collect(),
        None => vec![(4000, 3000), (8000, 6000)],
    };
    let size: f32 = arg(&args, "--brush").and_then(|v| v.parse().ok()).unwrap_or(300.0);
    let tip_side: u32 = arg(&args, "--tip").and_then(|v| v.parse().ok()).unwrap_or(1024);
    let want_dabs: usize = arg(&args, "--dabs").and_then(|v| v.parse().ok()).unwrap_or(500);
    let reps: usize = arg(&args, "--reps").and_then(|v| v.parse().ok()).unwrap_or(3).max(1);
    let per_frame: usize = arg(&args, "--points-per-frame").and_then(|v| v.parse().ok()).unwrap_or(4);
    let angle_jitter: f32 = arg(&args, "--angle-jitter").and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let spacing = 0.1f32;
    let brush = BrushSettings {
        size,
        spacing,
        pressure_size: false,
        seed: 7,
        tip: TipShape::Sampled(grain_tip(tip_side)),
        shape_dynamics: ShapeDynamics { enabled: angle_jitter > 0.0, angle: Dynamic::jitter(angle_jitter), ..Default::default() },
        ..Default::default()
    };
    let fmt = PixelFormat::RGBA8;
    let mut rows = Vec::new();
    println!("sampled_tip_bench: {size} px brush, {tip_side}² sampled tip, spacing {:.0} %, {want_dabs} dabs, {reps} reps", spacing * 100.0);
    for (w, h) in canvases {
        let mut layer = Surface::with_default(fmt, &from_rgba(&fmt, [1.0; 4]));
        layer.fill_rect(Rect::new(0, 0, w as i32, h as i32), &from_rgba(&fmt, [0.8, 0.7, 0.6, 1.0]));
        // 500 dabs at the brush spacing (the generator's first dab is at the first point).
        let length = f64::from(size * spacing) * (want_dabs as f64 - 0.5);
        let pts = recorded_stroke(w, h, length, 15.0);
        let mut setups = Vec::new();
        let mut totals: Vec<[f64; 4]> = Vec::new();
        let mut frame_ms = Vec::new();
        let mut dab_count = 0;
        let mut px = 0;
        for _ in 0..reps {
            let (setup, frames) = replay(&brush, &layer, &pts, per_frame);
            setups.push(setup);
            let sum = |f: fn(&Frame) -> f64| frames.iter().map(f).sum::<f64>();
            totals.push([sum(|f| f.generate), sum(|f| f.rasterize), sum(|f| f.accumulate), sum(|f| f.composite)]);
            frame_ms.extend(frames.iter().map(|f| f.generate + f.rasterize + f.accumulate + f.composite));
            dab_count = frames.iter().map(|f| f.dabs).sum();
            px = frames.iter().map(|f| f.px).sum();
        }
        let stage = |i: usize| median(totals.iter().map(|t| t[i]).collect());
        let (g, r, a, c) = (stage(0), stage(1), stage(2), stage(3));
        let setup = median(setups.clone());
        let stroke = setup + g + r + a + c;
        let n = dab_count.max(1) as f64;
        let (p50, p95) = (pct(&mut frame_ms.clone(), 0.5), pct(&mut frame_ms.clone(), 0.95));
        println!(
            "{w}x{h}: {dab_count} dabs, {} frames | stroke {stroke:.1} ms = setup {setup:.1} + generate {g:.1} + rasterize {r:.1} ({:.0} %) + accumulate {a:.1} ({:.0} %) + composite {c:.1} ({:.0} %) | per dab: rasterize {:.3} ms, accumulate {:.3} ms, {:.2} ns/px | frame p50 {p50:.2} p95 {p95:.2} ms",
            frame_ms.len() / reps,
            100.0 * r / stroke,
            100.0 * a / stroke,
            100.0 * c / stroke,
            r / n,
            a / n,
            r * 1e6 / px.max(1) as f64,
        );
        let name = format!("sampled tip {size} px, {dab_count} dabs, {w}x{h}");
        rows.push(json!({
            "name": name,
            "canvas": [w, h],
            "dabs": dab_count,
            "dab_pixels": px,
            "stroke_ms": stroke,
            "setup_ms": setup,
            "generate_ms": g,
            "rasterize_ms": r,
            "accumulate_ms": a,
            "composite_ms": c,
            "rasterize_per_dab_ms": r / n,
            "accumulate_per_dab_ms": a / n,
            "frame_p50_ms": p50,
            "frame_p95_ms": p95,
            "stage_totals_per_rep_ms": totals,
        }));
    }
    if let Some(out) = arg(&args, "--json") {
        let report: Value = json!({
            "bench": "sampled_tip_bench",
            "context": {"brush_px": size, "tip_px": tip_side, "spacing": spacing, "reps": reps, "points_per_frame": per_frame, "angle_jitter": angle_jitter},
            "rows": rows,
        });
        if let Err(e) = std::fs::write(&out, serde_json::to_string_pretty(&report).unwrap_or_default()) {
            eprintln!("write {out}: {e}");
            std::process::exit(1);
        }
    }
}
