//! Compare independent nudge descriptors with a pixel-heavy stroke on a synthetic 24 MP canvas.
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use photocraft_engine::Session;
use serde_json::json;
use std::time::Instant;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut s = Session::new();
    s.execute("prefs.set", json!({"path":"performance.historyStates","value":10000}))?;
    let mut doc = Document::with_background("24 MP synthetic undo costs", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    if let Some(layer) = doc.layers.first_mut() {
        layer.locks = Default::default();
        if let Some(surface) = layer.surface_mut() {
            for y in 20..52 {
                for x in 20..52 {
                    surface.write_pixel(x, y, &[0.2, 0.4, 0.6, 1.0]);
                }
            }
        }
    }
    s.add_document(doc, None);
    let initial = s.history_resident_bytes();
    let start = Instant::now();
    for i in 0..10000 {
        s.execute("layer.translate", json!({"dx":if i%2==0 {1} else {-1}}))?;
        if i == 999 || i == 9999 {
            println!(
                "nudges={} elapsed_ms={:.3} payload_bytes={} descriptor_bytes={} disk_bytes={} steps={}",
                i + 1,
                start.elapsed().as_secs_f64() * 1000.0,
                s.history_resident_bytes(),
                s.history_metadata_bytes(),
                s.history_disk_bytes(),
                s.active().map_or(0, |st| st.history.past_len())
            );
        }
    }
    let before = s.active().map(|st| st.doc.clone()).ok_or("missing document")?;
    let points: Vec<_> = (0..16).map(|i| json!([if i % 2 == 0 { 20 } else { 5980 }, 20 + i * 250, 1.0])).collect();
    let stroke = Instant::now();
    s.execute("paint.stroke", json!({"points":points,"size":64,"smoothing":0,"color":[0.8,0.1,0.3,1.0]}))?;
    let painted = s.active().map(|st| st.doc.clone()).ok_or("missing painted document")?;
    println!(
        "stroke_ms={:.3} payload_bytes={} initial_payload_bytes={} descriptor_bytes={}",
        stroke.elapsed().as_secs_f64() * 1000.0,
        s.history_resident_bytes(),
        initial,
        s.history_metadata_bytes()
    );
    let undo = Instant::now();
    if !s.try_undo()? || s.active().is_none_or(|st| st.doc.as_ref() != before.as_ref()) {
        return Err("stroke undo mismatch".into());
    }
    println!("stroke_undo_ms={:.3}", undo.elapsed().as_secs_f64() * 1000.0);
    if !s.try_redo()? || s.active().is_none_or(|st| st.doc.as_ref() != painted.as_ref()) {
        return Err("stroke redo mismatch".into());
    }
    Ok(())
}
