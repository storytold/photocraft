//! Bounded 24 MP selected-texture adjustment benchmark. No files or user documents are opened.
//! cargo run --release -p photocraft-engine --example uv_selected_adjustment_bench
use photocraft_engine::Session;
use serde_json::json;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for depth in [8, 16, 32] {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":6000,"height":4000,"depth":depth}))?;
        session.execute("edit.fill", json!({"color":"#6486a2"}))?;
        session.execute("select.rect", json!({"x":20,"y":20,"width":32,"height":32}))?;
        let mut times = Vec::new();
        for _ in 0..6 {
            let start = Instant::now();
            session.execute("image.adjustments.levels", json!({"inBlack":20,"inWhite":230,"gamma":1.2}))?;
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            session.execute("edit.undo", json!({}))?;
        }
        let cold = times.first().copied().unwrap_or_default();
        let mut warm: Vec<_> = times.into_iter().skip(1).collect();
        warm.sort_by(f64::total_cmp);
        println!(
            "24 MP RGB/{depth}, Levels through 32x32 selection: cold {cold:.3} ms, p50 {:.3} ms, max {:.3} ms (command + history, excludes canvas)",
            warm.get(2).copied().unwrap_or_default(),
            warm.last().copied().unwrap_or_default()
        );
    }
    Ok(())
}
