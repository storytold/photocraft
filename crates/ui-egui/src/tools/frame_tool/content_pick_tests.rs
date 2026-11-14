//! Frame drags pick content under the drag independently of the active layer (#13860).
use super::*;

fn raster(name: &str, bounds: Rect, format: photocraft_color::PixelFormat) -> Layer {
    let mut layer = Layer::raster(name, format);
    layer.surface_mut().unwrap().fill_rect(bounds, &[1., 0., 0., 1.]);
    layer
}

fn scene() -> (PhotocraftApp, LayerId, LayerId, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":600,"height":400})).unwrap();
    let ids = app
        .session
        .edit("Sources", |doc, active| {
            let red = raster("Red", Rect::new(50, 50, 150, 130), doc.pixel_format());
            let blue = raster("Blue", Rect::new(300, 200, 420, 260), doc.pixel_format());
            let empty = Layer::raster("Empty", doc.pixel_format());
            let ids = (red.id, blue.id, empty.id);
            doc.layers.extend([red, blue, empty]);
            *active = Some(ids.2);
            Ok(ids)
        })
        .unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Frame;
    (app, ids.0, ids.1, ids.2)
}

#[test]
fn topmost_overlapping_content_is_independent_of_active_layer() {
    let (mut app, red, blue, empty) = scene();
    for active in [empty, blue] {
        app.run("layer.select", json!({"layer":active.0})).unwrap();
        assert_eq!(content_layer(&app, [80, 60, 200, 160]), Some(red.0));
        assert_eq!(content_layer(&app, [40, 40, 450, 300]), Some(blue.0));
        assert_eq!(content_layer(&app, [450, 300, 550, 350]), None);
    }
}
