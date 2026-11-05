//! Live-stroke vs upload vs composite (#8950): a recorded stroke of 500 dabs with a large sampled
//! (custom) tip, replayed through the engine's live stroke (`brush_cmds::LiveStroke`, a few
//! pointer samples per frame as `canvas::feed_live_stroke` feeds it) on a new layer of a
//! 4000×3000 and an 8000×6000 document, with the app's GPU canvas refreshed after every frame
//! on a device created the way the app creates it. Per frame:
//!
//! - `live`: `LiveStroke::push`, the CPU side: dabs generated, rasterised and accumulated, and
//!   the touched tiles composited into the layer copy the canvas shows (with the smoothing tail);
//! - `refresh`: `GpuCanvas::refresh` of that frame's damage, waited for: the touched 256² layer
//!   tiles uploaded, the wgpu composite of the damage, the display encode and mips;
//! - `composite`: the same refresh again with nothing new to upload (measurement only), so
//!   `refresh − composite` is the upload;
//! - `gpu_ms`: the GPU's own execution time of the composite passes (timestamp queries, when
//!   the adapter has them), which excludes the uploads.
//!
//! Then the release (the commit taking the live pixels, P32's path). The paint-level split of
//! `live` into rasterise / accumulate / composite is `photocraft-paint`'s `sampled_tip_bench`.
//!
//! ```sh
//! cargo run --release -p photocraft-ui-egui --example brush_attribution -- [--canvas 4000x3000,8000x6000] [--brush 300] [--tip 1024] [--dabs 500] [--reps 3] [--cpu] [--json out.json]
//! ```

use std::time::Instant;

use eframe::egui_wgpu::RenderState;
use photocraft_engine::Session;
use photocraft_engine::paint::{Dynamic, GrayTile, ShapeDynamics, StrokePoint, TipShape};
use photocraft_testkit::perf::{RssSampler, report, row, write_report};
use photocraft_ui_egui::gpu_canvas::GpuCanvas;
use serde_json::{Value, json};

type Res<T> = Result<T, String>;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn exec(s: &mut Session, id: &str, p: Value) -> Res<Value> {
    s.execute(id, p).map_err(|e| format!("{id}: {e}"))
}

/// A deterministic grain tip `side` px square (as `sampled_tip_bench`): a soft disc broken up by
/// two octaves of hashed value noise, like a scanned chalk or sponge stamp.
fn grain_tip(side: u32) -> GrayTile {
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

/// The recorded stroke (as `sampled_tip_bench`): a pen scribble across the canvas sampled every
/// `step` px of arc length until it is `length` px long, 8 ms between reports.
fn recorded_stroke(w: u32, h: u32, length: f64, step: f64) -> Vec<StrokePoint> {
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

/// A headless GPU canvas on a device created like the app's.
fn canvas() -> Option<(GpuCanvas, RenderState)> {
    let setup = photocraft_ui_egui::gpu_canvas::wgpu_setup();
    let rs = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| egui_kittest::wgpu::create_render_state(setup, Default::default()))).ok()?;
    eprintln!("adapter: {}", rs.adapter.get_info().name);
    Some((GpuCanvas::new(&rs), rs))
}

/// The refresh of `damage` of `doc` the canvas does after a frame, waited for.
fn refresh(
    gpu: &Option<(GpuCanvas, RenderState)>,
    doc: &photocraft_doc::Document,
    damage: Option<photocraft_geom::Rect>,
) -> (f64, u64, Option<f64>, &'static str) {
    let t = Instant::now();
    match gpu {
        Some((g, rs)) => {
            let r = g.refresh(doc.id.0, doc, damage, None);
            let _ = rs.device.poll(eframe::wgpu::PollType::Wait { submission_index: None, timeout: None });
            (ms(t), r.uploads, r.gpu.map(|g| g.ms), r.kind)
        }
        None => {
            let r = damage.unwrap_or(doc.bounds()).intersect(&doc.bounds());
            if !r.is_empty() {
                std::hint::black_box(photocraft_compose::render(doc, r));
            }
            (ms(t), 0, None, "cpu")
        }
    }
}

struct Run {
    live: Vec<f64>,
    refresh: Vec<f64>,
    composite: Vec<f64>,
    gpu: Vec<f64>,
    uploads: u64,
    release: f64,
    begin: f64,
    kinds: Vec<&'static str>,
}

fn one_stroke(gpu: &Option<(GpuCanvas, RenderState)>, w: u32, h: u32, size: f32, tip: &GrayTile, dabs: usize, per_frame: usize) -> Res<Run> {
    let mut s = Session::new();
    exec(&mut s, "file.new", json!({"width": w, "height": h, "background": "white"}))?;
    exec(&mut s, "layer.new.layer", json!({"name": "paint"}))?;
    let spacing = 0.1f32;
    {
        let b = &mut s.tools.brush;
        b.tip = TipShape::Sampled(tip.clone());
        b.size = size;
        b.spacing = spacing;
        b.pressure_size = false;
        b.shape_dynamics = ShapeDynamics { enabled: true, angle: Dynamic::jitter(1.0), ..Default::default() };
    }
    // The canvas shows the document before the stroke starts.
    let doc = s.active().ok_or("no document")?.doc.clone();
    refresh(gpu, &doc, None);
    let pts = recorded_stroke(w, h, f64::from(size * spacing) * (dabs as f64 - 0.5), 15.0);
    // Every field the live stroke saw, so the release takes its pixels (`LiveCommit::matches`).
    let as_json = |p: &[StrokePoint]| {
        json!(p.iter().map(|q| [q.x, q.y, f64::from(q.pressure), f64::from(q.tilt_x), f64::from(q.tilt_y), f64::from(q.rotation), q.time]).collect::<Vec<_>>())
    };
    let mut p = json!({"points": as_json(&pts[..1]), "color": "#c03020"});
    let t = Instant::now();
    let mut live = photocraft_engine::brush_cmds::LiveStroke::begin(&s, &p).map_err(|e| e.to_string())?;
    let begin = ms(t);
    let mut run = Run { live: vec![], refresh: vec![], composite: vec![], gpu: vec![], uploads: 0, release: 0.0, begin, kinds: vec![] };
    let first = live.bounds();
    let (r, ..) = refresh(gpu, &live.doc, Some(first));
    run.refresh.push(r);
    for c in pts[1..].chunks(per_frame.max(1)) {
        let t = Instant::now();
        let dmg = live.push(c).map_err(|e| e.to_string())?;
        run.live.push(ms(t));
        let (r, uploads, g, kind) = refresh(gpu, &live.doc, Some(dmg));
        let (again, ..) = refresh(gpu, &live.doc, Some(dmg));
        run.refresh.push(r);
        run.composite.push(again);
        run.uploads += uploads;
        run.gpu.extend(g);
        run.kinds.push(kind);
    }
    p["points"] = as_json(&pts);
    p["seed"] = json!(live.seed);
    let t = Instant::now();
    s.offer_live_stroke(live).map_err(|e| e.to_string())?;
    let r = exec(&mut s, "paint.stroke", p);
    s.clear_live_stroke();
    run.release = ms(t);
    if r?.get("fromLive").and_then(Value::as_bool) != Some(true) {
        return Err("the release rendered the stroke again".into());
    }
    Ok(run)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(f64::NAN)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let canvases: Vec<(u32, u32)> = match arg(&args, "--canvas") {
        Some(c) => c.split(',').filter_map(|s| s.split_once('x')).filter_map(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))).collect(),
        None => vec![(4000, 3000), (8000, 6000)],
    };
    let size: f32 = arg(&args, "--brush").and_then(|v| v.parse().ok()).unwrap_or(300.0);
    let tip_side: u32 = arg(&args, "--tip").and_then(|v| v.parse().ok()).unwrap_or(1024);
    let dabs: usize = arg(&args, "--dabs").and_then(|v| v.parse().ok()).unwrap_or(500);
    let reps: usize = arg(&args, "--reps").and_then(|v| v.parse().ok()).unwrap_or(3).max(1);
    let per_frame: usize = arg(&args, "--points-per-frame").and_then(|v| v.parse().ok()).unwrap_or(4);
    let gpu = if args.iter().any(|a| a == "--cpu") { None } else { canvas() };
    let adapter = gpu.as_ref().map(|(_, rs)| rs.adapter.get_info().name);
    let sampler = RssSampler::start(std::time::Duration::from_millis(5));
    let tip = grain_tip(tip_side);
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    println!("brush_attribution: {size} px sampled tip ({tip_side}²), {dabs} dabs, {reps} reps, {}", adapter.as_deref().unwrap_or("CPU canvas"));
    for (w, h) in canvases {
        let mut per: Vec<[f64; 6]> = Vec::new();
        let (mut live_all, mut refresh_all, mut composite_all) = (vec![], vec![], vec![]);
        for _ in 0..reps {
            sampler.reset();
            match one_stroke(&gpu, w, h, size, &tip, dabs, per_frame) {
                Ok(r) => {
                    let (live, refresh, composite): (f64, f64, f64) = (r.live.iter().sum(), r.refresh.iter().sum(), r.composite.iter().sum());
                    let gpu_ms = if r.gpu.is_empty() { f64::NAN } else { r.gpu.iter().sum::<f64>() * r.composite.len() as f64 / r.gpu.len() as f64 };
                    per.push([r.begin + live, refresh, composite, gpu_ms, r.release, r.uploads as f64]);
                    // A GPU canvas that fell back (to `rect` uploads, say) is worth a note; the CPU
                    // canvas (`--cpu`, or no adapter) always refreshes as `cpu`, which is not.
                    if gpu.is_some() && r.kinds.iter().any(|k| !k.starts_with("gpu")) {
                        errors
                            .push(json!({"canvas": [w, h], "note": format!("refresh kinds {:?}", r.kinds.iter().collect::<std::collections::BTreeSet<_>>())}));
                    }
                    live_all.extend(r.live);
                    refresh_all.extend(r.refresh);
                    composite_all.extend(r.composite);
                }
                Err(e) => {
                    println!("{w}x{h}: ERROR {e}");
                    errors.push(json!({"canvas": [w, h], "error": e}));
                }
            }
        }
        if per.is_empty() {
            continue;
        }
        let m = |i: usize| median(per.iter().map(|p| p[i]).collect());
        let (live, refresh, composite, gpu_ms, release, uploads) = (m(0), m(1), m(2), m(3), m(4), m(5));
        let upload = (refresh - composite).max(0.0);
        let total = live + refresh;
        println!(
            "{w}x{h}: stroke {total:.1} ms = live (CPU) {live:.1} ({:.0} %) + refresh {refresh:.1} ({:.0} %: upload ≈ {upload:.1}, composite {composite:.1}, GPU exec {gpu_ms:.1}) | {uploads:.0} tile uploads | release {release:.2} ms",
            100.0 * live / total,
            100.0 * refresh / total,
        );
        let base = format!("{size} px sampled tip, {dabs} dabs, {w}x{h}");
        rows.push(row(&format!("live stroke frame: {base}"), &live_all, sampler.peak(), None));
        rows.push(row(&format!("canvas refresh frame: {base}"), &refresh_all, None, None));
        rows.push(row(&format!("composite-only refresh frame: {base}"), &composite_all, None, None));
        rows.push(json!({
            "name": format!("stroke split: {base}"),
            "live_ms": live, "refresh_ms": refresh, "upload_ms": upload, "composite_ms": composite, "gpu_exec_ms": gpu_ms,
            "release_ms": release, "tile_uploads": uploads, "per_rep": per,
        }));
    }
    if let Some(out) = arg(&args, "--json") {
        let context =
            json!({"gpu_adapter": adapter, "brush_px": size, "tip_px": tip_side, "dabs": dabs, "reps": reps, "points_per_frame": per_frame, "errors": errors});
        if let Err(e) = write_report(&out, &report("brush_attribution", context, rows, Some(&sampler))) {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
