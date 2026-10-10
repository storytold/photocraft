//! Reproducible 24 MP undo cache workload. No external images or filesystem servers.
//! cargo run --release -p photocraft-engine --example history_cache_bench -- --steps 12
//! Edit timing includes the actual COW mutation; overhead excludes that measured mutation.
//! Scratch=0 comparison retains fewer undo states, so this is a latency/retention report,
//! not proof that disk spill makes editing faster.
use std::time::{Duration, Instant};

use photocraft_color::PixelFormat;
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, SampleType, Size, Surface};
use photocraft_engine::{EngineError, Session};
use photocraft_geom::TileCoord;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut steps = 12usize;
    while let Some(arg) = args.next() {
        if arg != "--steps" {
            return Err(format!("unknown argument {arg}").into());
        }
        steps = args.next().ok_or("--steps requires a number")?.parse()?;
    }
    if !(3..=50).contains(&steps) {
        return Err("steps must be 3..=50".into());
    }
    for disk_mb in [0, 8192] {
        run(steps, disk_mb)?;
    }
    Ok(())
}

fn run(steps: usize, disk_mb: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut s = Session::new();
    s.edit_prefs(|p| {
        p.performance.memory_usage_mb = 128;
        p.performance.history_states = 50;
        p.scratch_disks.budget_mb = disk_mb;
    });
    let mut surface = Surface::new(PixelFormat::RGBA8);
    let mut coords = Vec::new();
    for ty in 0..16 {
        for tx in 0..24 {
            let coord = TileCoord::new(tx, ty);
            coords.push(coord);
            let bytes = surface.tile_mut(coord).bytes_mut();
            for (i, byte) in bytes.iter_mut().enumerate() {
                // Spatial texture with repeated structure, not an all-zero tile.
                *byte = ((i / 16 + tx as usize * 17 + ty as usize * 31) % 251) as u8;
            }
        }
    }
    let mut doc = Document::new("24 MP history benchmark", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::new("Image", LayerContent::Raster(surface)));
    s.add_document(doc, None);
    let mut edits = Vec::new();
    let mut overhead = Vec::new();
    let mut settles = Vec::new();
    for step in 0..steps {
        let start = Instant::now();
        let mut mutation = Duration::ZERO;
        s.edit("Full image pixel change", |doc, _| {
            let begin = Instant::now();
            let surface = doc.layers.first_mut().and_then(Layer::surface_mut).ok_or(EngineError::NoDocument)?;
            for coord in &coords {
                let bytes = surface.tile_mut(*coord).bytes_mut();
                for byte in bytes {
                    *byte = byte.wrapping_add((step % 7 + 1) as u8);
                }
            }
            mutation = begin.elapsed();
            Ok(())
        })?;
        let total = start.elapsed();
        edits.push(total.as_secs_f64() * 1000.0);
        overhead.push(total.saturating_sub(mutation).as_secs_f64() * 1000.0);
        settles.push(settle(&mut s)?.as_secs_f64() * 1000.0);
    }
    let retained = s.active().map_or(0, |st| st.history.past_len());
    let archived = s.active().map_or(0, |st| st.history.archived_states());
    let resident = s.history_resident_bytes();
    let disk = s.history_disk_bytes();
    let mut undos = Vec::new();
    let mut cold_undos = Vec::new();
    for _ in 0..retained {
        // Adjacent state is usually hot. Classify only those currently cold.
        let cold = s.active().is_some_and(|st| st.history.past_len() > 0 && st.history.resident_state(st.history.past_len() - 1).is_none());
        let start = Instant::now();
        if !s.try_undo()? {
            break;
        }
        let time = start.elapsed().as_secs_f64() * 1000.0;
        undos.push(time);
        if cold {
            cold_undos.push(time);
        }
        settle(&mut s)?;
    }
    println!(
        "disk_budget_mib={disk_mb} ram_target_mib=128 image=6000x4000 steps={steps} retained={retained} archived={archived} resident_bytes={resident} disk_bytes={disk}"
    );
    print_stats("edit_ms", &mut edits);
    print_stats("enqueue_overhead_ms", &mut overhead);
    print_stats("settle_ms", &mut settles);
    print_stats("undo_ms", &mut undos);
    print_stats("cold_undo_ms", &mut cold_undos);
    if let Some(notice) = s.take_history_cache_notice() {
        eprintln!("cache_notice={notice}");
    }
    Ok(())
}

fn settle(s: &mut Session) -> Result<Duration, Box<dyn std::error::Error>> {
    let start = Instant::now();
    loop {
        s.poll_history_cache();
        if !s.history_cache_busy() {
            return Ok(start.elapsed());
        }
        if start.elapsed() > Duration::from_secs(120) {
            return Err("history worker did not settle within 120 seconds".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn print_stats(name: &str, values: &mut [f64]) {
    values.sort_by(f64::total_cmp);
    let percentile = |p: usize| values.get(values.len().saturating_sub(1).saturating_mul(p) / 100).copied().unwrap_or(0.0);
    println!("{name}: samples={} p50={:.3} p95={:.3}", values.len(), percentile(50), percentile(95));
}
