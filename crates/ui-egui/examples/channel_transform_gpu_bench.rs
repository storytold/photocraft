//! Compare CPU submission and completed offscreen frames; run the identical example on upstream.
//! cargo run --release -p photocraft-ui-egui --example channel_transform_gpu_bench
//! Add --cpu to exercise the fallback, or --save /tmp/channel-preview for synthetic screenshots.
use std::time::Instant;

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Size};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use photocraft_ui_egui::{PhotocraftApp, Services, transform_tool};
use serde_json::json;

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let cpu = args.iter().any(|a| a == "--cpu");
    let save = args.iter().position(|a| a == "--save").and_then(|i| args.get(i + 1));
    let mut doc = Document::with_background("24 MP synthetic preview", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    if let Some(s) = doc.layers.first_mut().and_then(|l| l.surface_mut()) {
        for y in (0..4000).step_by(128) {
            for x in (0..6000).step_by(128) {
                s.fill_rect(Rect::new(x, y, (x + 128).min(6000), (y + 128).min(4000)), &[(x % 251) as f32 / 250.0, (y % 241) as f32 / 240.0, 0.6, 0.75]);
            }
        }
    }
    for case in ["composite", "red-all-visible", "red-only"] {
        // The harness owns managed texture IDs; each context needs a fresh renderer.
        let rs = egui_kittest::wgpu::create_render_state(photocraft_ui_egui::gpu_canvas::wgpu_setup(), Default::default());
        eprintln!("adapter: {:?}; case={case}; CPU fallback={cpu}", rs.adapter.get_info());
        let mut session = Session::new();
        session.add_document(doc.clone(), None);
        if case != "composite" {
            session.execute("channel.target", json!({"channel":"red"}))?;
        }
        if case == "red-all-visible" {
            for channel in ["red", "green", "blue"] {
                session.execute("channel.setVisible", json!({"channel":channel,"visible":true}))?;
            }
        }
        let mut app = PhotocraftApp::new(session, Services::default());
        app.sync_views();
        if !cpu {
            app.set_wgpu(rs.clone());
        }
        let ctx = egui::Context::default();
        transform_tool::begin(&mut app, &ctx)?;
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .renderer(egui_kittest::wgpu::WgpuTestRenderer::from_render_state(rs.clone()))
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
        let mut geometry = Vec::new();
        let mut frames = Vec::new();
        for _ in 0..9 {
            if let Some(t) = &mut h.state_mut().ui.transform {
                t.quad = t.quad.map(|[x, y]| [x + 20.0, y]);
            }
            let start = Instant::now();
            h.run_steps(1);
            geometry.push(ms(start));
            let _ = h.render()?;
            frames.push(ms(start));
        }
        let ctx = h.ctx.clone();
        transform_tool::enter_warp(h.state_mut(), &ctx);
        h.run_steps(1);
        let _ = h.render()?;
        let mut warp = Vec::new();
        let mut warp_frames = Vec::new();
        for _ in 0..9 {
            if let Some(t) = &mut h.state_mut().ui.transform
                && let Some(w) = &mut t.warp
            {
                *w = photocraft_geom::warp::Warp::custom(w.to_mesh(1, 1).map_points(|p| [p[0], p[1] + p[0] / 600.0]), w.bounds);
            }
            let start = Instant::now();
            h.run_steps(1);
            warp.push(ms(start));
            let image = h.render()?;
            warp_frames.push(ms(start));
            if let Some(path) = save {
                image.save(format!("{path}-{case}-warp.png"))?;
            }
        }
        println!("{}", json!({"case":case,"cpu":cpu,"geometry_ms":geometry,"completed_frame_ms":frames,"warp_ms":warp,"completed_warp_frame_ms":warp_frames}));
    }
    Ok(())
}
