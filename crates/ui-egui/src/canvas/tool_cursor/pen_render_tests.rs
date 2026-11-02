//! Headless WGPU render test verifying visual consistency of native Pen cursor bitmaps
//! against the painted cursor baseline (#9122).

use egui::{Color32, Pos2, Rect, Vec2, pos2, vec2};
use photocraft_testkit::gpu_lock;

use super::PenVariant;

#[test]
fn pen_wgpu_rendered_reference_comparison() {
    let _gpu = gpu_lock();

    let scale = 1.0;
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        egui_kittest::Harness::builder().with_size(vec2(300.0, 200.0)).with_pixels_per_point(scale).wgpu().build_ui(|ui| {
            // Render dark, midgrey, and light bands
            let bands = [
                (Rect::from_min_size(pos2(10.0, 10.0), vec2(280.0, 50.0)), Color32::from_rgb(240, 240, 240)),
                (Rect::from_min_size(pos2(10.0, 70.0), vec2(280.0, 50.0)), Color32::from_rgb(128, 128, 128)),
                (Rect::from_min_size(pos2(10.0, 130.0), vec2(280.0, 50.0)), Color32::from_rgb(15, 15, 15)),
            ];
            for (rect, col) in &bands {
                ui.painter().rect_filled(*rect, 0.0, *col);
            }

            // Render standard variant on light band: painted on left, native on right
            let variant = PenVariant::Standard;
            let p_painted = pos2(50.0, 35.0);
            crate::icons::cursor(ui.ctx(), variant.icon_name(), p_painted, variant.hotspot_in_box() / 24.0, 22.0);

            if let Some(native_img) = super::pen::rasterize(variant, scale) {
                let [w, h] = native_img.size;
                let [hx, hy] = native_img.hotspot;
                let color_img = egui::ColorImage::from_rgba_unmultiplied([usize::from(w), usize::from(h)], &native_img.rgba);
                let tex = ui.ctx().load_texture("pen-test-native", color_img, egui::TextureOptions::NEAREST);
                let p_native = pos2(120.0, 35.0);
                let native_rect = Rect::from_min_size(p_native - vec2(hx as f32 / scale, hy as f32 / scale), vec2(w as f32 / scale, h as f32 / scale));
                ui.painter().image(tex.id(), native_rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
            }
        })
    }));

    let Ok(mut harness) = built else {
        eprintln!("skipping: no GPU adapter");
        return;
    };

    harness.run_steps(3);
    let img = harness.render().expect("render pen comparison");
    assert!(img.width() >= 300);
    assert!(img.height() >= 200);

    // Verify ink was rendered on light background for both painted (near x=50) and native (near x=120)
    let has_dark_ink = |x0: u32, x1: u32, y0: u32, y1: u32| {
        (y0..y1).any(|y| {
            (x0..x1).any(|x| {
                let px = img.get_pixel(x, y).0;
                // Contrast against light background (240): ink has significantly lower luminance
                px[0] < 100 && px[1] < 100 && px[2] < 100
            })
        })
    };
    assert!(has_dark_ink(35, 65, 20, 50), "missing painted cursor ink on light background");
    assert!(has_dark_ink(105, 135, 20, 50), "missing native cursor ink on light background");
}
