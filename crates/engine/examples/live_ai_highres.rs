//! Opt-in high-resolution live probe; synthetic instruction, no personal images.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use serde_json::json;
    fn require(value: bool, message: &str) -> Result<(), Box<dyn std::error::Error>> {
        if value { Ok(()) } else { Err(message.into()) }
    }
    let mut args = std::env::args().skip(1);
    let endpoint = args.next().ok_or("usage: live_ai_highres ENDPOINT WIDTH HEIGHT OUTPUT.pcraft [detail]")?;
    let width = args.next().ok_or("missing width")?.parse::<u32>()?;
    let height = args.next().ok_or("missing height")?.parse::<u32>()?;
    let output = args.next().ok_or("missing output path")?;
    let detail = args.next().as_deref() == Some("detail");
    let mut session = photocraft_engine::Session::new();
    session.execute("prefs.set", json!({"path":"integrations.comfyuiEndpoint","value":endpoint}))?;
    session.execute("file.new", json!({"width":128,"height":128,"background":"white","depth":16}))?;
    let before = session.active().ok_or("missing source document")?.doc.clone();
    let start = std::time::Instant::now();
    let ready = session.execute(
        "edit.generateImage",
        json!({
            "prompt":"Eine blaue Keramiktasse vor einem schlichten beigen Hintergrund, weiches Studiolicht, kein Text.",
            "seed":42123,"outputSize":{"width":width,"height":height},"detailUpscale":detail,
            "resolutionMode":if detail { "reduced" } else { "native" },
        }),
    )?;
    let id = ready.get("result").and_then(serde_json::Value::as_u64).ok_or("missing preview")?;
    require(session.active().ok_or("missing document")?.doc.size == before.size, "canvas changed before confirmation")?;
    let preview = session.ai_preview(id, 0).ok_or("missing display preview")?;
    let float = photocraft_ai::RgbaImage::new(preview.size, preview.rgba.iter().map(|v| f32::from(*v) / 255.0).collect())?;
    std::fs::write(format!("{output}.png"), photocraft_ai::native::png(&float)?)?;
    let info = session.execute("ai.result.list", json!({}))?;
    if !detail {
        let row = info.get("results").and_then(serde_json::Value::as_array).and_then(|rows| rows.first()).ok_or("missing result metadata")?;
        require(row.get("workingSize") == Some(&json!({"width":width,"height":height})), "native generation was reduced")?;
        require(row.get("detailUpscaler").is_none_or(serde_json::Value::is_null), "native generation unexpectedly upscaled")?;
    }
    session.execute("ai.result.apply", json!({"result":id}))?;
    let applied = session.active().ok_or("missing applied document")?.doc.clone();
    require(applied.size == photocraft_geom::Size::new(width, height), "wrong output canvas")?;
    require(applied.layers.first() == before.layers.first(), "original layer changed")?;
    require(session.undo(), "Undo failed")?;
    require(*session.active().ok_or("missing Undo document")?.doc == *before, "Undo mismatch")?;
    require(session.redo(), "Redo failed")?;
    let bytes = photocraft_format::save_to_bytes(&applied, &Default::default())?;
    std::fs::write(&output, &bytes)?;
    let reopened = photocraft_format::load_from_bytes(&bytes)?;
    require(reopened.size == applied.size && reopened.layers.len() == applied.layers.len(), "native roundtrip geometry mismatch")?;
    for (a, b) in reopened.layers.iter().zip(&applied.layers) {
        require(a.surface() == b.surface(), "native roundtrip tile mismatch")?;
        require(a.mask == b.mask, "native roundtrip mask mismatch")?;
    }
    println!(
        "{}",
        json!({"outputSize":[width,height],"detailUpscale":detail,"seconds":start.elapsed().as_secs_f64(),
        "undoRedo":"passed","nativeRoundTrip":"passed","results":info})
    );
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
