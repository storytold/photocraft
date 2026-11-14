//! A small frame does not take a much larger candidate layer (#13861).
use super::*;

fn candidate(bounds: Rect) -> (PhotocraftApp, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":600,"height":400})).unwrap();
    let layer = app.session.edit("Source", |doc, active| {
        let mut layer = Layer::raster("Source", doc.pixel_format());
        layer.surface_mut().unwrap().fill_rect(bounds, &[1.0, 0.0, 0.0, 1.0]);
        let id = layer.id;
        doc.layers.push(layer);
        *active = Some(id);
        Ok(id)
    }).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Frame;
    (app, layer)
}

#[test]
fn threshold_compares_each_dimension_instead_of_area_or_overlap() {
    let (app, layer) = candidate(Rect::new(50, 50, 250, 250));
    for (width, height, taken) in [(62, 62, false), (66, 66, false), (68, 68, true), (60, 140, false), (200, 20, false), (240, 76, true), (78, 240, true)] {
        assert_eq!(content_layer(&app, [60, 60, 60 + width, 60 + height]), taken.then_some(layer.0), "{width} x {height}");
    }
}
