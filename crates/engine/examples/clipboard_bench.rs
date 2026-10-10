//! Synthetic 24 MP clipboard timings; bitmap-only Copy is the previous workflow's comparison.
//! `cargo run --release -p photocraft-engine --example clipboard_bench`
use photocraft_doc::LayerMask;
use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::json;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 6000, "height": 4000, "background": "transparent"}))?;
    s.edit("Synthetic mask fixture", |doc, active| {
        let id = active.ok_or_else(|| photocraft_engine::EngineError::Other("no layer".into()))?;
        let layer = doc.layer_mut(id).ok_or(photocraft_engine::EngineError::NoLayer(id))?;
        let surface = layer.surface_mut().ok_or_else(|| photocraft_engine::EngineError::Other("no pixels".into()))?;
        surface.fill_rect(Rect::new(0, 0, 6000, 4000), &[0.8, 0.2, 0.4, 1.0]);
        let mut mask = LayerMask::reveal_all();
        mask.surface.fill_rect(Rect::new(0, 0, 2000, 4000), &[0.0]);
        mask.surface.fill_rect(Rect::new(2000, 0, 4000, 4000), &[0.5]);
        layer.mask = Some(mask);
        Ok(())
    })?;
    for pixels in [true, false] {
        for repetition in 0..3 {
            let start = Instant::now();
            s.execute("edit.copy", json!({"pixels": pixels}))?;
            let copy_ms = start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            s.execute("edit.pasteSpecial.pasteInPlace", json!({}))?;
            let paste_ms = start.elapsed().as_secs_f64() * 1000.0;
            s.undo();
            println!("{} {repetition}: copy {copy_ms:.2} ms, paste {paste_ms:.2} ms", if pixels { "bitmap" } else { "layer+mask" });
        }
    }
    Ok(())
}
