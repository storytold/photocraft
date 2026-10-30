//! Deterministic stroke/closed-fill benchmark. Includes a prefix-replay reference for the
//! incremental algorithm; it measures rasterisation, not presentation FPS or input latency.
use photocraft_algo::selection::brush::{Options, StrokeMask};
use photocraft_geom::Rect;
use photocraft_testkit::perf::{report, row, write_report};
use serde_json::json;
use std::{hint::black_box, time::Instant};

/// Measure incremental samples, prefix replay and closed fill with optional JSON output.
fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let quick = args.iter().any(|a| a == "--quick");
    let (w, h) = if quick { (1920, 1080) } else { (6000, 4000) };
    let canvas = Rect::new(0, 0, w, h);
    let n = if quick { 80 } else { 240 };
    let points: Vec<[f64; 2]> = (0..n).map(|i| [100.0 + i as f64 * 4.0, 300.0 + (i as f64 * 0.07).sin() * 100.0]).collect();
    let options = Options { fill_closed: false, ..Default::default() };
    let mut rows = Vec::new();
    let mut stroke = StrokeMask::new(options, canvas).map_err(str::to_owned)?;
    let mut incremental = Vec::new();
    for point in &points {
        let start = Instant::now();
        stroke.append(std::slice::from_ref(point)).map_err(str::to_owned)?;
        black_box(stroke.mask());
        incremental.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let mut replay = Vec::new();
    for end in 1..=points.len() {
        let start = Instant::now();
        let mut rebuilt = StrokeMask::new(options, canvas).map_err(str::to_owned)?;
        rebuilt.append(points.get(..end).ok_or("missing benchmark points")?).map_err(str::to_owned)?;
        black_box(rebuilt.mask());
        replay.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    rows.push(row("incremental sample", &incremental, None, None));
    rows.push(row("prefix replay reference", &replay, None, None));
    let outline = [[100.0, 100.0], [f64::from(w - 100), 100.0], [f64::from(w - 100), f64::from(h - 100)], [100.0, f64::from(h - 100)], [100.0, 100.0]];
    let mut closed = Vec::new();
    for _ in 0..if quick { 3 } else { 5 } {
        let start = Instant::now();
        let mut stroke = StrokeMask::new(Options::default(), canvas).map_err(str::to_owned)?;
        stroke.append(&outline).map_err(str::to_owned)?;
        black_box(stroke.finish().map_err(str::to_owned)?);
        closed.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    rows.push(row("closed selection", &closed, None, None));
    let data = report(
        "selection_brush_bench",
        json!({"quick":quick,"width":w,"height":h,"brush_size":60,"hardness":70,"opacity":100,"samples":n,"presentation_included":false}),
        rows,
        None,
    );
    if let Some(i) = args.iter().position(|a| a == "--json") {
        write_report(args.get(i + 1).ok_or("--json needs a file")?, &data)?;
    }
    println!("{}", serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?);
    Ok(())
}
