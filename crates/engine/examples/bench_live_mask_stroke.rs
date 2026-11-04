//! Brush engine LiveStroke begin/push/commit with a 4096² filled second raster layer.
//!
//! Compares an ordinary pixel-target stroke with and without an enabled reveal-all mask.
//! The fixture is synthesized in memory and the run does not write a document.
use std::{error::Error, time::Instant};

use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerMask};
use photocraft_engine::{Session, brush_cmds::LiveStroke};
use photocraft_geom::{Rect, Size};
use photocraft_paint::StrokePoint;
use photocraft_raster::Surface;
use serde_json::{Value, json};

const SIDE: u32 = 4096;
const FRAMES: usize = 6;
const PUSHES: usize = 8;

fn fixture(masked: bool) -> Document {
    let size = Size::new(SIDE, SIDE);
    let mut doc = Document::with_background("LiveStroke diagnostic", size, ColorMode::Rgb, SampleType::U8, Color::rgb(0.18, 0.2, 0.22));
    let mut surface = Surface::with_default(PixelFormat::RGBA8, &[0.0, 0.0, 0.0, 0.0]);
    surface.fill_rect(doc.bounds(), &[0.36, 0.4, 0.44, 1.0]);
    let mut layer = Layer::new("Background copy", photocraft_doc::LayerContent::Raster(surface));
    if masked {
        layer.mask = Some(LayerMask::reveal_all());
    }
    doc.layers.push(layer);
    doc
}

fn stroke_points(frame: usize) -> Vec<StrokePoint> {
    (0..=PUSHES)
        .map(|step| {
            let x = 850.0 + frame as f64 * 300.0 + step as f64 * 120.0;
            let y = 1400.0 + (step as f64 * 0.73).sin() * 240.0;
            StrokePoint::new(x, y, 0.85 + (step % 3) as f32 * 0.05)
        })
        .collect()
}

fn percentile(values: &[f64], p: f64) -> f64 {
    let rank = ((values.len() as f64 * p).ceil() as usize).saturating_sub(1).min(values.len().saturating_sub(1));
    values.get(rank).copied().unwrap_or_default()
}

fn run(masked: bool) -> Result<(), Box<dyn Error>> {
    let mut begin_ms = Vec::with_capacity(FRAMES);
    let mut push_ms = Vec::with_capacity(FRAMES);
    let mut commit_ms = Vec::with_capacity(FRAMES);
    let mut damage_pixels = 0u64;
    let mut prefix_nonempty = false;
    for frame in 0..FRAMES {
        let mut session = Session::new();
        session.add_document(fixture(masked), None);
        let points = stroke_points(frame);
        let mut params = json!({
            "points": [[points[0].x, points[0].y, points[0].pressure]],
            "target": "pixels",
            "size": 36,
            "hardness": 0.72,
            "opacity": 1.0,
            "flow": 1.0,
            "smoothing": 0.0,
            "color": "#d87342",
            "seed": frame as u64 + 7,
        });
        let start = Instant::now();
        let mut live = LiveStroke::begin(&session, &params)?;
        begin_ms.push(start.elapsed().as_secs_f64() * 1000.0);

        let start = Instant::now();
        for point in points[1..].chunks(1) {
            let damage = live.push(point)?;
            damage_pixels = damage_pixels.saturating_add(u64::from(damage.width()) * u64::from(damage.height()));
        }
        push_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        prefix_nonempty |= !live.bounds().is_empty();

        params["points"] = Value::Array(points.iter().map(|point| json!([point.x, point.y, point.pressure])).collect());
        params["seed"] = json!(live.seed);
        let start = Instant::now();
        session.offer_live_stroke(live)?;
        let result = session.execute("paint.stroke", params)?;
        commit_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        if result.get("fromLive") != Some(&Value::Bool(true)) {
            return Err("stroke commit did not consume the live preview".into());
        }
    }
    begin_ms.sort_by(f64::total_cmp);
    push_ms.sort_by(f64::total_cmp);
    commit_ms.sort_by(f64::total_cmp);
    println!(
        "mask={} target=pixels begin_p50_ms={:.3} begin_p95_ms={:.3} push8_p50_ms={:.3} push8_p95_ms={:.3} commit_p50_ms={:.3} commit_p95_ms={:.3} damage_pixels_total={} live_bounds_nonempty={}",
        if masked { "reveal_all" } else { "none" },
        percentile(&begin_ms, 0.5),
        percentile(&begin_ms, 0.95),
        percentile(&push_ms, 0.5),
        percentile(&push_ms, 0.95),
        percentile(&commit_ms, 0.5),
        percentile(&commit_ms, 0.95),
        damage_pixels,
        prefix_nonempty,
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let bounds = Rect::from_xywh(0, 0, SIDE, SIDE);
    println!("fixture={}x{} layers=2 pushes_per_stroke={} measured_strokes={FRAMES} (engine LiveStroke only)", bounds.width(), bounds.height(), PUSHES);
    run(false)?;
    run(true)?;
    Ok(())
}
