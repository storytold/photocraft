//! Reproducible 24 MP recovery workload; synthetic solid-colour pixels compress well.
//! cargo run --release -p photocraft-format --example recovery_workload
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use photocraft_format::{Autosaver, SaveOptions};
use photocraft_ops::History;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn wait(saver: &Autosaver) -> Result<photocraft_format::SaveStats, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(completion) = saver.take_completion() {
            return Ok(completion.result?);
        }
        if Instant::now() >= deadline {
            return Err("recovery workload timed out".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let mut doc = Arc::new(Document::with_background("24 MP recovery", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U8, Color::WHITE));
    let mut history = History::default();
    for i in 1..=4 {
        let before = doc.clone();
        let mutable = Arc::make_mut(&mut doc);
        let layer = mutable.layers.first_mut().ok_or("background layer missing")?;
        layer.surface_mut().ok_or("background pixels missing")?.write_pixel(i, i, &[i as f32 / 5.0, 0.0, 0.0, 1.0]);
        history.record(format!("Paint {i}"), before, photocraft_ops::LayerTarget::default());
    }
    doc = history.try_undo(doc)?.ok_or("undo state missing")?.0;
    for (key, include_history) in [("document-only", false), ("with-history", true)] {
        let saver = Autosaver::new(root.path(), key);
        let mut samples = Vec::new();
        for revision in 1..=3 {
            let capture = Instant::now();
            let snapshot = doc.clone();
            let checkpoint = if include_history { history.checkpoint() } else { History::default().checkpoint() };
            let capture_us = capture.elapsed().as_secs_f64() * 1e6;
            let start = Instant::now();
            saver.request_checkpoint(snapshot, checkpoint, revision, None, SaveOptions::default())?;
            let enqueue_us = start.elapsed().as_secs_f64() * 1e6;
            let stats = wait(&saver)?;
            samples.push(serde_json::json!({"capture_us":capture_us,"enqueue_us":enqueue_us,"completed_ms":start.elapsed().as_secs_f64()*1000.0,"tiles_written":stats.tiles_written,"tiles_reused":stats.tiles_reused,"descriptor_bytes":stats.manifest_bytes}));
        }
        println!(
            "{}",
            serde_json::json!({"scenario":key,"pixels":24000000,"depth":"U8","undo":if include_history {history.past_len()} else {0},"redo":if include_history {history.checkpoint_signature().1} else {0},"samples":samples})
        );
    }
    Ok(())
}
