//! Reproducible 24 MP channel-transform measurements; synthetic pixels, no external assets.
//! cargo run --release -p photocraft-ui-egui --example channel_transform_bench
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Size};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use photocraft_ui_egui::{PhotocraftApp, Services, transform_tool};
use serde_json::json;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::with_background("24 MP synthetic channels", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    if let Some(surface) = doc.layers.first_mut().and_then(|l| l.surface_mut()) {
        for y in (0..4000).step_by(128) {
            for x in (0..6000).step_by(128) {
                surface.fill_rect(Rect::new(x, y, (x + 128).min(6000), (y + 128).min(4000)), &[(x % 251) as f32 / 250.0, (y % 241) as f32 / 240.0, 0.6, 0.75]);
            }
        }
    }
    for isolated in [false, true] {
        for rep in 0..3 {
            let mut session = Session::new();
            session.add_document(doc.clone(), None);
            let target = if isolated { json!({"color":0}) } else { json!("pixels") };
            let start = Instant::now();
            session.execute("edit.transform", json!({"target":target,"matrix":[0.9,0,0,0.9,73,41],"interpolation":"bicubic"}))?;
            println!("24MP isolated={isolated} rep={rep} commit_ms={:.2}", start.elapsed().as_secs_f64() * 1000.0);
            std::hint::black_box(session.active());
        }
    }
    for rect in [[0.0, 0.0, 6000.0, 4000.0], [1500.0, 1000.0, 4500.0, 3000.0]] {
        let mut session = Session::new();
        session.add_document(doc.clone(), None);
        let warp =
            photocraft_geom::warp::Warp::custom(photocraft_geom::warp::BezierMesh::identity(rect, 1, 1).map_points(|p| [p[0] + 73.0, p[1] + 41.0]), rect);
        let start = Instant::now();
        session.execute("edit.transform.warp", json!({"target":{"color":0},"rect":rect,"warp":warp,"interpolation":"bicubic"}))?;
        println!("24MP isolated_warp rect={rect:?} commit_ms={:.2}", start.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(session.active());
    }
    let mut session = Session::new();
    session.add_document(doc, None);
    session.execute("channel.target", json!({"channel":"red"}))?;
    let mut app = PhotocraftApp::new(session, Services::default());
    app.sync_views();
    let ctx = egui::Context::default();
    let start = Instant::now();
    transform_tool::begin(&mut app, &ctx)?;
    println!("24MP isolated_begin_ms={:.2}", start.elapsed().as_secs_f64() * 1000.0);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| photocraft_ui_egui::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    let start = Instant::now();
    h.run_steps(2);
    println!("24MP isolated_first_preview_ms={:.2}", start.elapsed().as_secs_f64() * 1000.0);
    if let Some(t) = &mut h.state_mut().ui.transform {
        t.quad = t.quad.map(|[x, y]| [x + 20.0, y]);
    }
    let start = Instant::now();
    h.run_steps(1);
    println!("24MP isolated_geometry_preview_ms={:.2}", start.elapsed().as_secs_f64() * 1000.0);
    h.state_mut().run("channel.setVisible", json!({"channel":"green","visible":true}))?;
    let start = Instant::now();
    h.run_steps(1);
    println!("24MP isolated_eye_preview_ms={:.2}", start.elapsed().as_secs_f64() * 1000.0);
    let start = Instant::now();
    h.run_steps(2);
    println!("24MP isolated_cached_frames_ms={:.2}", start.elapsed().as_secs_f64() * 1000.0);
    let ctx = h.ctx.clone();
    transform_tool::enter_warp(h.state_mut(), &ctx);
    let start = Instant::now();
    h.run_steps(1);
    println!("24MP isolated_warp_preview_ms={:.2}", start.elapsed().as_secs_f64() * 1000.0);
    Ok(())
}
