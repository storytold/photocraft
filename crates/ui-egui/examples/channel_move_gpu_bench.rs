//! Completed offscreen 24 MP Move-drag frames for one targeted colour channel.
//! cargo run --release -p photocraft-ui-egui --example channel_move_gpu_bench
//! Add --cpu for the exact CPU fallback, or --save /tmp/move-preview for screenshots.
//! Add --step 21 to exercise offsets between reduced-preview samples.
//! Uses the production eframe/shared-input host and verifies the release against the engine oracle.

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
        control::Reply::Done(value) if value["ok"] == true => Ok(()),
        control::Reply::Done(value) => Err(format!("Move {kind} failed: {value}")),
        _ => Err(format!("Move {kind} did not complete synchronously")),
    }
}

/// A control event queues input on the shared host; drain it through the production app frame.
fn deliver(h: &mut egui_kittest::Harness<'static, Box<PhotocraftApp>>) -> Result<(), String> {
    for _ in 0..64 {
        h.run_steps(1);
        if craft::ui::runtime::windows::WindowHost::of(&h.ctx).injected_idle() {
            return Ok(());
        }
    }
    Err("the Move benchmark input was not delivered in 64 frames".into())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let cpu = args.iter().any(|a| a == "--cpu");
    let whole = args.iter().any(|a| a == "--whole");
    let set = args.iter().any(|a| a == "--set");
    let stride = args.iter().position(|a| a == "--step").map(|i| args.get(i + 1).ok_or("--step requires a positive integer")).transpose()?;
    let stride = stride.map(|s| s.parse::<u32>()).transpose()?.unwrap_or(20);
    if !(1..=10_000).contains(&stride) {
        return Err("--step must be between 1 and 10000 pixels".into());
    }
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
        session.execute("channel.target", if set { json!({"colors":[0,2]}) } else { json!({"channel":"red"}) })?;
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
    let ctx = app.ctx().clone();
    let mut h = egui_kittest::Harness::builder()
        .with_context(ctx)
        .with_size(egui::vec2(1000.0, 700.0))
        .with_max_steps(64)
        .renderer(egui_kittest::wgpu::WgpuTestRenderer::from_render_state(rs))
        .build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            app
        });
    h.run_steps(8);
    let _ = h.render()?;
    let ctx = h.ctx.clone();
    let source = h.state().session.active().ok_or("no benchmark document")?.doc.clone();
    let layer = h.state().session.active().and_then(|st| st.active_layer).ok_or("no benchmark layer")?;
    // Start beyond the real pointer slop; subsequent samples can be one document pixel apart.
    let cold_offset = stride.max(100);
    pointer(h.state_mut(), &ctx, "down", 3000.0, 2000.0)?;
    pointer(h.state_mut(), &ctx, "move", 3000.0 + f64::from(cold_offset), 2000.0)?;
    let cold = Instant::now();
    deliver(&mut h)?;
    let _ = h.render()?;
    let cold_ms = cold.elapsed().as_secs_f64() * 1000.0;
    let mut frames = Vec::new();
    for step in 2..=10 {
        pointer(h.state_mut(), &ctx, "move", 3000.0 + f64::from(cold_offset + (step - 1) * stride), 2000.0)?;
        let start = Instant::now();
        deliver(&mut h)?;
        let image = h.render()?;
        frames.push(start.elapsed().as_secs_f64() * 1000.0);
        if step == 10
            && let Some(path) = save
        {
            let mode = if whole {
                "whole"
            } else if set {
                "set"
            } else {
                "channel"
            };
            image.save(format!("{path}-{mode}-{}.png", if cpu { "cpu" } else { "gpu" }))?;
        }
    }
    if !std::sync::Arc::ptr_eq(&source, &h.state().session.active().ok_or("benchmark document closed")?.doc) {
        return Err("the Move preview modified its source before release".into());
    }
    let last_offset = cold_offset + 9 * stride;
    pointer(h.state_mut(), &ctx, "up", 3000.0 + f64::from(last_offset), 2000.0)?;
    deliver(&mut h)?;
    let mut oracle = Session::new();
    oracle.tools.background = h.state().session.tools.background;
    oracle.add_document((*source).clone(), None);
    let target = if whole {
        json!("pixels")
    } else if set {
        json!({"colors":[0,2]})
    } else {
        json!({"color":0})
    };
    oracle.execute("layer.translate", json!({"target":target,"dx":last_offset,"dy":0}))?;
    let actual = h.state().session.active().and_then(|st| st.doc.layer(layer)).and_then(|l| l.surface());
    let expected = oracle.active().and_then(|st| st.doc.layer(layer)).and_then(|l| l.surface());
    if actual.is_none() || actual != expected {
        return Err(format!("the delivered Move drag did not commit the expected {last_offset}-pixel offset").into());
    }
    println!(
        "{}",
        json!({"driver":"production_eframe_shared_host","cpu":cpu,"whole":whole,"set":set,"step":stride,"cold_offset":cold_offset,"cold_ms":cold_ms,"completed_frame_ms":frames,"commit_verified":true})
    );
    Ok(())
}
