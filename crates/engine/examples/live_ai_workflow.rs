//! Explicit native integration probe with synthetic pixels, Undo/Redo and .pcraft persistence.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    fn require(value: bool, message: &str) -> Result<(), Box<dyn std::error::Error>> {
        if value { Ok(()) } else { Err(message.into()) }
    }
    use photocraft_engine::{EngineError, Session};
    use photocraft_geom::Rect;
    use serde_json::json;
    let mut args = std::env::args().skip(1);
    let endpoint = args.next().ok_or("usage: live_ai_workflow ENDPOINT fill|expand|segment|background|automatic OUTPUT.pcraft")?;
    let mode = args.next().ok_or("missing mode")?;
    let output = args.next().ok_or("missing output")?;
    let mut session = Session::new();
    session.execute("prefs.set", json!({"path":"integrations.comfyuiEndpoint","value":endpoint}))?;
    session.execute("file.new", json!({"width":512,"height":512,"background":"#e6d9bf","depth":16}))?;
    session.edit("Synthetic rectangle", |doc, _| {
        let surface = doc.layers.first_mut().and_then(|layer| layer.surface_mut()).ok_or_else(|| EngineError::Other("missing synthetic layer".into()))?;
        surface.write_region(Rect::from_xywh(160, 150, 192, 240), &[0.8, 0.03, 0.03, 1.0].repeat(192 * 240));
        Ok(())
    })?;
    if mode == "fill" {
        session.edit("Synthetic selection", |doc, _| {
            let mut mask = photocraft_raster::Surface::new(photocraft_color::PixelFormat::new(
                photocraft_color::ColorMode::Grayscale,
                photocraft_color::SampleType::F32,
                false,
            ));
            mask.write_region(Rect::from_xywh(160, 150, 192, 240), &vec![1.0; 192 * 240]);
            doc.selection = Some(mask);
            Ok(())
        })?;
    }
    let before = session.active().ok_or("missing document")?.doc.clone();
    let command = match mode.as_str() {
        "fill" => "edit.generativeFill",
        "expand" => "image.generativeExpand",
        "segment" => "layer.aiRemoveBackground",
        "background" => "edit.aiBackground",
        "automatic" => "edit.generateImage",
        _ => return Err("unknown mode".into()),
    };
    let prompt = match mode.as_str() {
        "expand" => "Erweitere den schlichten beigefarbenen Hintergrund in die leeren Randbereiche. Erhalte das Motiv und die vorhandene Beleuchtung.",
        "background" => "Ersetze den Hintergrund durch einen schlichten hellblauen Studiohintergrund. Erhalte das rote Motiv.",
        _ => "Eine blaue Keramiktasse vor einem schlichten beigen Hintergrund, weiches Studiolicht, kein Text.",
    };
    let parameters = if mode == "segment" {
        json!({})
    } else {
        json!({"prompt":prompt,"preparePrompt":mode=="automatic","seed":42123,"contextMargin":if mode=="fill"{256}else{0},"expansion":{"left":32,"top":32,"right":128,"bottom":0}})
    };
    let ready = session.execute(command, parameters)?;
    let id = ready.get("result").and_then(serde_json::Value::as_u64).ok_or("missing result")?;
    require(session.active().ok_or("missing document")?.doc.size == before.size, "unexpected canvas mutation")?;
    let provenance = session.ai_result(id).and_then(|result| result.prepared_prompt.as_ref()).and_then(|p| p.text_model.clone());
    session.execute("ai.result.apply", json!({"result":id,"hideOriginals":mode=="segment"}))?;
    let applied = session.active().ok_or("missing document")?.doc.clone();
    require(applied.layers.len() == before.layers.len() + 1, "unexpected layer count")?;
    if mode == "fill" {
        let old = photocraft_compose::render(&before, before.bounds());
        let new = photocraft_compose::render(&applied, applied.bounds());
        for (i, (old, new)) in old.px.iter().zip(&new.px).enumerate() {
            let x = i % 512;
            let y = i / 512;
            if !(160..352).contains(&x) || !(150..390).contains(&y) {
                require(old == new, "outside-mask pixel changed")?;
            }
        }
    }
    if mode == "expand" {
        require(applied.size == photocraft_geom::Size::new(672, 544), "wrong expanded canvas")?;
        require(
            photocraft_compose::render(&before, before.bounds()).px == photocraft_compose::render(&applied, Rect::from_xywh(32, 32, 512, 512)).px,
            "expanded original pixels changed",
        )?;
    }
    session.execute("edit.undo", json!({}))?;
    require(session.active().ok_or("missing document")?.doc.size == before.size, "unexpected canvas mutation")?;
    require(session.active().ok_or("missing document")?.doc.layers.len() == before.layers.len(), "Undo did not restore layers")?;
    session.execute("edit.redo", json!({}))?;
    let bytes = photocraft_format::save_to_bytes(&session.active().ok_or("missing document")?.doc, &Default::default())?;
    std::fs::write(&output, &bytes)?;
    let reopened = photocraft_format::load_from_bytes(&bytes)?;
    require(reopened.size == applied.size && reopened.layers.len() == applied.layers.len(), "native round-trip geometry mismatch")?;
    require(
        photocraft_compose::render(&reopened, reopened.bounds()).px == photocraft_compose::render(&applied, applied.bounds()).px,
        "native round-trip pixel mismatch",
    )?;
    println!(
        "{}",
        json!({"operation":mode,"result":id,"textModel":provenance,"undoRedo":"passed","nativeRoundTrip":"passed","layers":reopened.layers.len(),"size":[reopened.size.width,reopened.size.height]})
    );
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
