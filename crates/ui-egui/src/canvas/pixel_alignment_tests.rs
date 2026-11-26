use super::document_display::{CanvasHost, active_view};
use super::*;
use craft::behavior::CanvasView;

/// Document points on the shown view land on device pixels.
fn assert_pixel_edges(view: &CanvasView, ppp: f32) {
    for [x, y] in [[0.0, 0.0], [1.0, 1.0], [17.0, 29.0], [1500.0, 1000.0]] {
        let p = view.to_screen(craft::behavior::kurbo::Point::new(x, y)) * ppp;
        assert!((p.x - p.x.round()).abs() < 0.001, "x edge: {p:?}, ppp {ppp}");
        assert!((p.y - p.y.round()).abs() < 0.001, "y edge: {p:?}, ppp {ppp}");
    }
}

/// Default Crop slides the view under its frame with fractional pointer distances: the shown view keeps the continuous
/// centre, as the canvas shows it; Classic Crop moves the frame and keeps the alignment.
#[test]
fn crop_image_movement_keeps_its_continuous_camera_mapping() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", serde_json::json!({"width": 3000, "height": 2000})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Crop;
    let centre = [1500.25_f32, 1000.75];
    let ctx = app.ctx().clone();
    for ppp in [1.0, 1.5] {
        for classic in [false, true] {
            app.ui.tool_options.crop.shield.classic_mode = classic;
            assert_eq!(super::document_display::view_follows_pointer(&app), !classic, "only Default Crop moves the view with the pointer");
            ctx.set_pixels_per_point(ppp);
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(700.0, 600.0));
            let rect = Rect::from_min_size(pos2(40.0, 100.0), vec2(523.0, 471.0));
            let view = View { zoom: 1.0, center: centre, doc_size: [3000, 2000], pending: Vec::new(), ..Default::default() };
            let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                canvas_view(&mut app, ui, CanvasHost::main(0), rect, view.clone(), true);
            });
            output.textures_delta.clear();
            let shown = active_view(&app).expect("the canvas showed a view");
            if classic {
                assert_pixel_edges(&shown, ppp);
            } else {
                assert!(
                    (shown.center.x - f64::from(centre[0])).abs() < 1e-6 && (shown.center.y - f64::from(centre[1])).abs() < 1e-6,
                    "ppp {ppp}: Default Crop keeps the fractional centre: {:?}",
                    shown.center
                );
            }
        }
    }
}
