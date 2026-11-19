//! Real Camera Raw proxy-update timing on a 36 MP document (CPU preparation, not GPU presentation).
use photocraft_ui_egui::{PhotocraftApp, camera_raw_ui};
use serde_json::{Value, json};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let quick = args.iter().any(|a| a == "--quick");
    let (width, height, reps) = if quick { (6000, 4000, 12) } else { (7360, 4912, 40) };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = app.ctx().clone();
    let _ = ctx.run_ui(Default::default(), |_| {});
    // A full-resolution synthetic image with dense hue/saturation coverage and exact clipped
    // endpoints. Setup/import is outside the interactive measurements.
    let mut bytes = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            bytes.extend([(x % 256) as u8, (y % 256) as u8, ((x ^ y) % 256) as u8]);
        }
    }
    let image =
        photocraft_codecs::Image::from_raw(width as u32, height as u32, photocraft_codecs::ChannelLayout::Rgb, photocraft_codecs::SampleType::U8, bytes)?;
    let encoded = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default())?;
    app.session.open_document(photocraft_io::import("scope.png", &encoded)?.document, None);
    camera_raw_ui::menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).ok_or("no dialog entry point")??;
    let mut samples = Vec::new();
    let mut histogram_samples = Vec::new();
    for i in 0..reps + 3 {
        let r = camera_raw_ui::menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": if i % 2 == 0 { 0.25 } else { 0.75 }}}}))
            .ok_or("no dialog entry point")??;
        if i >= 3 {
            samples.push(r.get("renderMs").and_then(Value::as_f64).ok_or("no render timing")?);
            if let Some(ms) = r.get("histogramMs").and_then(Value::as_f64) {
                histogram_samples.push(ms);
            }
        }
        // Drain texture deltas just as a real frame does, rather than retaining every upload.
        let _ = ctx.run_ui(Default::default(), |_| {});
    }
    // Measure the complete native dialog CPU path with every SDR scope feature enabled:
    // filter preparation + ICC vectorscope + clipping texture + nine Lab probes + pixel-hover overlays + UI mesh.
    // A full-coverage selection exercises the mask blend and selected-region scope without
    // reducing the amount of analysed colour data. Selection setup is outside the timer.
    app.run("select.rect", json!({"x":0,"y":0,"width":width,"height":height}))?;
    camera_raw_ui::menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).ok_or("no dialog entry point")??;
    camera_raw_ui::menu(
        &mut app,
        &ctx,
        "filter.cameraRaw",
        &json!({"ui":{"scope":{
            "vectorscope":true,"selectedRegion":true,"lab":true,"shadows":true,"highlights":true,
            "samplers":(0..9).map(|i| [i as f32/9.0,0.5]).collect::<Vec<_>>()
        }}}),
    )
    .ok_or("no dialog entry point")??;
    let mut interactive_samples = Vec::new();
    let mut scope_samples = Vec::new();
    let mut idle_samples = Vec::new();
    let input = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
        events: vec![egui::Event::PointerMoved(egui::pos2(430.0, 400.0))],
        ..Default::default()
    };
    for i in 0..reps + 3 {
        let start = std::time::Instant::now();
        let r = camera_raw_ui::menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"set":{"exposure":if i%2==0 {0.25}else{0.75}},"scope":{}}}))
            .ok_or("no dialog entry point")??;
        // PLAN: assumed (dialogs-w19): `dialogs::run(app, ui)` runs the dialog host's passes for one frame.
        let output = ctx.run_ui(input(), |ui| photocraft_ui_egui::dialogs::run(&mut app, ui));
        std::hint::black_box(ctx.tessellate(output.shapes, output.pixels_per_point));
        if i >= 3 {
            interactive_samples.push(start.elapsed().as_secs_f64() * 1000.0);
            scope_samples.push(r.get("scopeMs").and_then(Value::as_f64).ok_or("no scope timing")?);
        }
        let start = std::time::Instant::now();
        let output = ctx.run_ui(input(), |ui| photocraft_ui_egui::dialogs::run(&mut app, ui));
        std::hint::black_box(ctx.tessellate(output.shapes, output.pixels_per_point));
        if i >= 3 {
            idle_samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let mut sorted = samples.clone();
    sorted.sort_by(f64::total_cmp);
    let p50 = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    let p95 = sorted.get(sorted.len().saturating_sub(1) * 95 / 100).copied().unwrap_or(0.0);
    let max = sorted.last().copied().unwrap_or(0.0);
    let report = json!({"width": width, "height": height, "rows": [{"name": "exposure proxy update", "median_ms": p50, "p95_ms": p95, "max_ms": max, "samples_ms": samples}, {"name": "RGB histogram", "samples_ms": histogram_samples},{"name":"interactive scope update", "samples_ms":interactive_samples}, {"name":"vectorscope ICC analysis", "samples_ms":scope_samples}, {"name":"cached scope frame", "samples_ms":idle_samples}]});
    println!("{}", serde_json::to_string_pretty(&report)?);
    if let Some(path) = args.iter().position(|a| a == "--json").and_then(|i| args.get(i + 1)) {
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    Ok(())
}
