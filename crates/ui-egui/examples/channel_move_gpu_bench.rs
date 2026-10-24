//! Completed offscreen 24 MP Move-drag frames for one targeted colour channel.
//! cargo run --release -p photocraft-ui-egui --example channel_move_gpu_bench
//! Add --cpu for the exact CPU fallback, or --save /tmp/move-preview for screenshots.

use std::time::Instant;

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, Size};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use photocraft_ui_egui::{PhotocraftApp, Services, control};
use serde_json::json;

fn pointer(app: &mut PhotocraftApp, ctx: &egui::Context, kind: &str, x: f64, y: f64) -> Result<(), String> {
    let (request, _) = control::ControlRequest::new("ui.pointer", json!({"tool":"Move", "events":[{"kind":kind,"x":x,"y":y}]}));
    match control::handle(app, ctx, &request) {
        control::Outcome::Done(value) if value["ok"] == true => Ok(()),
        control::Outcome::Done(value) => Err(format!("Move {kind} failed: {value}")),
        _ => Err(format!("Move {kind} did not complete synchronously")),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let cpu = args.iter().any(|a| a == "--cpu");
    let whole = args.iter().any(|a| a == "--whole");
    let save = args.iter().position(|a| a == "--save").and_then(|i| args.get(i + 1));
    let mut doc = Document::new("24 MP channel Move", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::raster("Unlocked image", doc.pixel_format()));
    if let Some(surface) = doc.layers.first_mut().and_then(|l| l.surface_mut()) {
        for y in (0..4000).step_by(256) {
            for x in (0..6000).step_by(256) {
                surface.fill_rect(Rect::new(x, y, (x + 256).min(6000), (y + 256).min(4000)), &[(x % 251) as f32 / 250.0, (y % 241) as f32 / 240.0, 0.6, 1.0]);
            }
        }
    }
    let mut session = Session::new();
    session.add_document(doc, None);
    if !whole {
        session.execute("channel.target", json!({"channel":"red"}))?;
        for channel in ["red", "green", "blue"] {
            session.execute("channel.setVisible", json!({"channel":channel,"visible":true}))?;
        }
    }
    let rs = egui_kittest::wgpu::create_render_state(photocraft_ui_egui::gpu_canvas::wgpu_setup(), Default::default());
    eprintln!("adapter: {:?}; CPU fallback={cpu}; whole layer={whole}", rs.adapter.get_info());
    let mut app = PhotocraftApp::new(session, Services::default());
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    if !app.session.is_enabled("layer.translate") {
        return Err("the fixture cannot start a Move drag".into());
    }
    if !cpu {
        app.set_wgpu(rs.clone());
    }
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .renderer(egui_kittest::wgpu::WgpuTestRenderer::from_render_state(rs))
        .build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| photocraft_ui_egui::canvas::document_area(app, ui));
            },
            app,
        );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(2);
    let _ = h.render()?;
    let ctx = h.ctx.clone();
    pointer(h.state_mut(), &ctx, "down", 3000.0, 2000.0)?;
    pointer(h.state_mut(), &ctx, "move", 3020.0, 2000.0)?;
    let cold = Instant::now();
    h.run_steps(1);
    let _ = h.render()?;
    let cold_ms = cold.elapsed().as_secs_f64() * 1000.0;
    let mut frames = Vec::new();
    for step in 2..=10 {
        pointer(h.state_mut(), &ctx, "move", 3000.0 + f64::from(step * 20), 2000.0)?;
        let start = Instant::now();
        h.run_steps(1);
        let image = h.render()?;
        frames.push(start.elapsed().as_secs_f64() * 1000.0);
        if step == 10
            && let Some(path) = save
        {
            image.save(format!("{path}-{}-{}.png", if whole { "whole" } else { "channel" }, if cpu { "cpu" } else { "gpu" }))?;
        }
    }
    println!("{}", json!({"cpu":cpu,"whole":whole,"cold_ms":cold_ms,"completed_frame_ms":frames}));
    Ok(())
}
