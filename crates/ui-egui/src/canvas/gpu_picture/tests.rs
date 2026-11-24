use super::*;
use craft::behavior::kurbo::Point;
use egui::{Rect, pos2};

fn picture(shows: Shows, k: u32, growth: [f32; 2]) -> GpuPicture {
    let params = ViewParams { doc: 1, doc_size: [400, 300], zoom: 1.0, center: [0.0, 0.0], rotation: 0.0, view_key: 9, display: 0, output: 0, hdr: None };
    GpuPicture { params, shows, k, growth, bounds: DRect::new(0, 0, 400, 300) }
}

#[test]
fn a_canvas_view_maps_to_the_shader_view() {
    let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 600.0));
    let half_turn = std::f32::consts::FRAC_PI_2;
    // (picture, view, zoom, centre, rotation in radians)
    let cases = [
        (picture(Shows::Document, 1, [0.0, 0.0]), CanvasView::new(rect, Point::new(100.0, 50.0), 2.0).rotation(90.0), 2.0, [100.0, 50.0], half_turn),
        // A preview reduced by 4 that grew 3 texels left and right and shrank 1 at the top and bottom.
        (picture(Shows::Preview, 4, [3.0, -1.0]), CanvasView::new(rect, Point::new(100.0, 40.0), 0.5), 2.0, [28.0, 9.0], 0.0),
        (picture(Shows::Pages, 1, [0.0, 0.0]), CanvasView::new(rect, Point::new(0.0, 0.0), 1.0).rotation(f64::NAN), 1.0, [0.0, 0.0], 0.0),
    ];
    for (picture, view, zoom, center, rotation) in cases {
        let p = picture.params(&view);
        assert_eq!((p.zoom, p.center, p.doc, p.view_key), (zoom, center, 1, 9), "{view:?}");
        assert!((p.rotation - rotation).abs() < 1e-6, "{view:?}: {}", p.rotation);
    }
}
