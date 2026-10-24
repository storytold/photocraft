//! Per-frame latency of the live brush stroke preview at 8K (what the UI pays per input batch).
//!
//! cargo run --release -p photocraft-ui-egui --example live_stroke_bench -- [--size 7680x4320] [--json out.json] [--only soft]
//!
//! For each brush config: begin a live stroke, feed 600 points in chunks of 2 (like a 240 Hz
//! pen at ~120 fps), timing every `push`. The UI thread runs exactly this work inside one egui
//! update, so each number is one frame's cost; the last row is the release commit.

use std::time::Instant;

use photocraft_engine::brush_cmds::LiveStroke;
use photocraft_engine::paint::StrokePoint;
use photocraft_engine::Session;
use serde_json::{json, Value};

fn arg<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn percentile(v: &mut Vec<u128>, p: f64) -> f64 {
    v.sort_unstable();
    let idx = (((v.len() as f64 - 1.0) * p).round() as usize).min(v.len().saturating_sub(1));
    v[idx] as f64 / 1e6
}

/// Phase breakdown of `LiveStroke::push` for a 1000 px soft brush: rasterize+accumulate into
/// the stroke buffer, composite it into the layer, and the smoothing-tail re-render.
fn breakdown(w: u32, h: u32) {
    use photocraft_color::PixelFormat;
    use photocraft_paint::StrokeRenderer;
    use photocraft_raster::Surface;

    let brush: photocraft_paint::BrushSettings =
        serde_json::from_value(json!({"size": 1000, "hardness": 0.0, "color": [0.75, 0.19, 0.13, 1.0]})).unwrap();
    let fmt = PixelFormat::RGBA8;
    let (mut pre, mut target) = (Surface::new(fmt), Surface::new(fmt));
    let mut r = StrokeRenderer::new(&brush, Some(fmt), 1.0);
    let (fw, fh) = (f64::from(w), f64::from(h));
    let pts: Vec<[f64; 3]> = (0..600)
        .map(|k| {
            let t = f64::from(k) / 599.0;
            [fw * (0.1 + 0.8 * t), fh * (0.5 + 0.35 * (t * 12.0).sin()), 0.6]
        })
        .collect();
    let (mut t_raster, mut t_comp, mut t_tail) = (0u128, 0u128, 0u128);
    for c in pts.chunks(2) {
        let sp: Vec<StrokePoint> = c.iter().map(|q| StrokePoint::new(q[0], q[1], q[2] as f32)).collect();
        let t = Instant::now();
        r.push(&sp);
        t_raster += t.elapsed().as_nanos();
        let t = Instant::now();
        r.composite(&pre, &mut target, None, false, false);
        t_comp += t.elapsed().as_nanos();
        let t = Instant::now();
        if let Some(mut tail) = r.tail_preview() {
            tail.composite(&pre, &mut target, None, false, false);
        }
        t_tail += t.elapsed().as_nanos();
    }
    let n = (pts.len() / 2) as f64;
    println!("\n1000 px soft breakdown (total per push, 600 pts / chunks of 2):");
    println!("  rasterize+accumulate: {:>6.2}ms", t_raster as f64 / 1e6 / n);
    println!("  composite (dirty):    {:>6.2}ms", t_comp as f64 / 1e6 / n);
    println!("  tail_preview+comp:    {:>6.2}ms", t_tail as f64 / 1e6 / n);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (w, h) = arg(&args, "--size").and_then(|s| s.split_once('x')).map_or((7680, 4320), |(a, b)| {
        (a.parse().unwrap_or(7680), b.parse().unwrap_or(4320))
    });
    let only = arg(&args, "--only");
    let configs: &[(&str, Value)] = &[
        ("60 px hard", json!({"size": 60, "hardness": 0.8, "color": "#c03020"})),
        ("120 px hard", json!({"size": 120, "hardness": 0.8, "color": "#c03020"})),
        ("200 px soft", json!({"size": 200, "hardness": 0.0, "color": "#c03020", "smoothing": 0.0})),
        ("500 px soft", json!({"size": 500, "hardness": 0.0, "color": "#c03020", "smoothing": 0.0})),
        ("1000 px soft", json!({"size": 1000, "hardness": 0.0, "color": "#c03020", "smoothing": 0.0})),
    ];

    let mut rows: Vec<(String, Vec<u128>, u128, u128)> = Vec::new();
    for (name, p0) in configs {
        if let Some(only) = only && !name.contains(only) {
            continue;
        }
        let mut p = p0.clone();
        let mut s = Session::new();
        let mut exec = |s: &mut Session, cmd: &str, p: Value| -> Result<Value, String> {
            photocraft_engine::Session::execute(s, cmd, p).map_err(|e| format!("{cmd}: {e}"))
        };
        exec(&mut s, "file.new", json!({"width": w, "height": h, "background": "white"})).unwrap();
        exec(&mut s, "layer.new.layer", json!({"name": "paint"})).unwrap();

        let (fw, fh) = (f64::from(w), f64::from(h));
        let pts: Vec<[f64; 3]> = (0..600)
            .map(|k| {
                let t = f64::from(k) / 599.0;
                [fw * (0.1 + 0.8 * t), fh * (0.5 + 0.35 * (t * 12.0).sin()), 0.6 + 0.4 * (t * 7.0).sin().abs()]
            })
            .collect();
        p["points"] = json!([pts[0]]);
        let begin = Instant::now();
        let mut live = LiveStroke::begin(&s, &p).map_err(|e| e.to_string()).unwrap();
        let begin_ms = begin.elapsed().as_nanos();

        let mut ns = Vec::new();
        for c in pts[1..].chunks(2) {
            let sp: Vec<StrokePoint> = c.iter().map(|q| StrokePoint::new(q[0], q[1], q[2] as f32)).collect();
            let t = Instant::now();
            live.push(&sp).map_err(|e| e.to_string()).unwrap();
            ns.push(t.elapsed().as_nanos());
        }
        let t = Instant::now();
        s.offer_live_stroke(live).map_err(|e| e.to_string()).unwrap();
        s.clear_live_stroke();
        rows.push(((*name).into(), ns, begin_ms, t.elapsed().as_nanos()));
    }

    println!("\ndocument {w}x{h}, 600 points, chunks of 2");
    println!("\n{:<16}{:>9}{:>9}{:>9}{:>12}{:>12}", "brush (push)", "p50", "p95", "max", "begin (once)", "release");
    for (name, ns, begin, release) in &rows {
        let mut ns = ns.clone();
        println!(
            "{:<16}{:>7.1}ms{:>7.1}ms{:>7.1}ms{:>10.1}ms{:>10.1}ms",
            name,
            percentile(&mut ns.clone(), 0.5),
            percentile(&mut ns.clone(), 0.95),
            percentile(&mut ns, 1.0),
            *begin as f64 / 1e6,
            *release as f64 / 1e6
        );
    }
    println!("\n(each push = one frame's UI-thread cost for the live preview; budget: 16.7ms / 60fps)");

    // Phase breakdown for the worst row: where a push spends its time.
    breakdown(w, h);

    if let Some(path) = arg(&args, "--json") {
        let out = json!(rows
            .iter()
            .map(|(name, ns, begin, release)| {
                let mut v = ns.clone();
                json!({
                    "row": name,
                    "p50_ms": percentile(&mut v, 0.5),
                    "p95_ms": percentile(&mut v, 0.95),
                    "max_ms": percentile(&mut v, 1.0),
                    "begin_ms": *begin as f64 / 1e6,
                    "release_ms": *release as f64 / 1e6,
                    "n": ns.len(),
                })
            })
            .collect::<Vec<_>>());
        std::fs::write(path, serde_json::to_string_pretty(&out).unwrap()).unwrap();
        println!("json: {path}");
    }
}
