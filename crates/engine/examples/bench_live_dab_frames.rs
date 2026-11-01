//! Per-frame cost of a live sequential-dab stroke while it is drawn, on a 24 MP layer, in release
//! mode: each frame pushes 16 px of drag (~1000 px/s at 60 fps) while the canvas holds the shown
//! document, as it does between frames. Frames that lay no dab cost next to nothing, so the mean
//! and the worst are taken over the frames that lay one. Soft round brush; pass an `.abr` to use
//! its first tip for the 343 px Smudge at 1% spacing.
use std::time::Instant;

use photocraft_engine::{EngineError, Session, paint::StrokePoint, retouch_cmds::LiveDab};
use photocraft_geom::Rect;
use serde_json::json;

const FRAMES: usize = 60;
const STROKES: usize = 5;

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":6000,"height":4000,"depth":8,"mode":"rgb"}))?;
    s.execute("layer.new.layer", json!({}))?;
    s.edit("stripes", |doc, active| {
        let id = active.ok_or_else(|| EngineError::Other("no layer".into()))?;
        let surface = doc.layer_mut(id).and_then(|l| l.surface_mut()).ok_or_else(|| EngineError::Other("no surface".into()))?;
        for x in (0..6000).step_by(64) {
            let c = if x % 128 == 0 { [0.9, 0.3, 0.1, 1.0] } else { [0.1, 0.4, 0.8, 1.0] };
            surface.fill_rect(Rect::new(x, 0, (x + 64).min(6000), 4000), &c);
        }
        Ok(())
    })?;
    let tip = match std::env::args().nth(1) {
        Some(path) => photocraft_io::abr_map::read_abr(&std::fs::read(path)?, "bench")?.presets.into_iter().next().map(|p| p.brush),
        None => None,
    };
    let pts: Vec<[f64; 2]> = std::iter::once([1000.0, 2000.0])
        .chain((1..=FRAMES).flat_map(|f| (1..=4).map(move |k| [1000.0 + (f as f64 - 1.0) * 16.0 + k as f64 * 4.0, 2000.0])))
        .collect();
    let mut cases: Vec<_> = ["paint.blur", "paint.smudge", "paint.dodge"].iter().flat_map(|c| [50, 150, 300, 600].map(|size| (*c, size, 25))).collect();
    cases.push(("paint.smudge", 343, 1));
    for (cmd, size, spacing) in cases {
        let sampled = cmd == "paint.smudge" && spacing == 1 && tip.is_some();
        if let (true, Some(b)) = (sampled, &tip) {
            s.tools.brush = b.clone();
        }
        let p = json!({"points": [pts[0]], "size": size, "hardness": 0, "strength": 50, "spacing": spacing});
        let mut frames = Vec::new();
        for _ in 0..STROKES {
            let mut live = LiveDab::begin(&s, cmd, &p)?;
            for batch in pts[1..].chunks(4) {
                let b: Vec<_> = batch.iter().map(|q| StrokePoint::new(q[0], q[1], 1.0)).collect();
                // The canvas still shows the last frame's document while the next one is drawn.
                let shown = live.doc.clone();
                let t = Instant::now();
                if !live.push(&b)?.is_empty() {
                    frames.push(t.elapsed().as_secs_f64() * 1e3);
                }
                drop(shown);
            }
        }
        let tip = if sampled { "sampled tip" } else { "soft round" };
        println!(
            "{cmd:12} size={size:4} spacing={spacing:2}% {tip:11} | frames with a dab: {:3}  mean={:6.2} ms  max={:6.2} ms",
            frames.len(),
            frames.iter().sum::<f64>() / frames.len().max(1) as f64,
            frames.iter().copied().fold(0.0, f64::max)
        );
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
