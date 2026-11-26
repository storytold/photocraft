//! Render synthetic color-picker evidence without capturing the user's desktop.
//! cargo run -p photocraft-ui-egui --example color_picker_demo -- /tmp/color-picker
use craft::color_ui::ui::{CaptureSource, ScreenPickGesture, StillCapture};
use craft::platform::screen::{Capture, ScreenImage};
use craft::theme::Srgba;
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{PhotocraftApp, Services, screen_picker};
use serde_json::json;
use std::sync::Arc;

/// An 800 × 600 monitor at the desktop's origin with a red-green ramp, so the loupe shows a
/// readable gradient.
fn ramp() -> ScreenImage {
    let mut rgba = Vec::with_capacity(800 * 600 * 4);
    for y in 0..600 {
        for x in 0..800 {
            rgba.extend_from_slice(&[(x * 255 / 799) as u8, (y * 255 / 599) as u8, 128, 255]);
        }
    }
    ScreenImage { origin: [0, 0], size: [800, 600], scale: 1.0, rgba: Arc::new(rgba), icc_profile: None, name: "Demo".into() }
}

fn main() {
    let output = std::env::args().nth(1).unwrap_or_else(|| "/tmp/photocraft-color-picker".into());
    std::fs::create_dir_all(&output).expect("output directory");
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width":800,"height":500,"background":"white"})).unwrap();
    session.execute("type.create", json!({"text":"Color from anywhere","color":"#2070c0","size":48,"x":90,"y":200})).unwrap();
    let mut text = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_pixels_per_point(1.0).with_max_steps(64).wgpu().build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let services = Services {
            screen_capture: Some(Arc::new(|| -> Box<dyn CaptureSource> { Box::new(StillCapture::color(Srgba::rgb(1.0, 136.0 / 255.0, 0.0))) })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), session, services);
        app.ui.tool = photocraft_ui_egui::state::Tool::Type;
        app
    });
    text.run_steps(8);
    text.get_all_by_role_and_label(egui::accesskit::Role::ColorWell, "Text color").next().expect("the text swatch").click();
    text.run_steps(6);
    text.render().unwrap().save(format!("{output}/text-color-picker.png")).unwrap();

    // PLAN: the loupe renders through the app's own frame, which calls `screen_picker::tick` and
    // `show` once per pass (frame.rs, hub-w1); calling them again here would run the pick twice a pass.
    let mut loupe = Harness::builder().with_size(egui::vec2(800.0, 600.0)).with_pixels_per_point(1.0).with_max_steps(64).wgpu().build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let services = Services {
            screen_capture: Some(Arc::new(|| -> Box<dyn CaptureSource> { Box::new(StillCapture::new(Ok(Capture::Images(vec![ramp()])))) })),
            ..Default::default()
        };
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width":400,"height":300,"background":"white"})).unwrap();
        PhotocraftApp::with_context(cc.egui_ctx.clone(), session, services)
    });
    loupe.run_steps(8);
    // The Color Picker dialog has no screen button (#17017); the compact popovers start the same pick.
    let ctx = loupe.ctx.clone();
    screen_picker::hold(loupe.state_mut(), &ctx, "color-picker-demo", ScreenPickGesture::Clicked);
    loupe.run_steps(3);
    loupe.input_mut().events.push(egui::Event::PointerMoved(egui::pos2(360.0, 220.0)));
    loupe.run_steps(2);
    loupe.render().unwrap().save(format!("{output}/screen-color-loupe.png")).unwrap();
    screen_picker::cancel(loupe.state_mut());
    loupe.run_steps(1);
}
