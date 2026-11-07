//! #11769: a count marker's dot is Marker Size physical pixels across (not 2 + 2 × Marker Size
//! points), at every display scale, with its number above right of it.

use serde_json::json;

use super::*;

/// The diameters, in physical pixels, of the dots drawn for one marker of each Marker Size.
fn drawn_diameters(ppp: f32) -> Vec<f32> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    for (i, size) in [1, 2, 10].iter().enumerate() {
        s.execute("count.newGroup", json!({"name": format!("G{size}")})).unwrap();
        s.execute("count.setGroup", json!({"markerSize": size})).unwrap();
        s.execute("count.add", json!({"x": 40 + 60 * i, "y": 50})).unwrap();
    }
    let app = PhotocraftApp::new(s, crate::Services::default());
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(ppp);
    let xf = ViewXform {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(200.0, 100.0)),
        zoom: 1.0,
        center: [100.0, 50.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    };
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| draw_overlay(&app, ui.painter(), &xf));
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Circle(circle) if circle.fill != egui::Color32::TRANSPARENT => Some(circle.radius * 2.0 * ppp),
            _ => None,
        })
        .collect()
}

#[test]
fn a_dot_is_marker_size_physical_pixels_across() {
    for ppp in [1.0, 1.25] {
        let d = drawn_diameters(ppp);
        assert_eq!(d.len(), 3, "{d:?}");
        for (got, want) in d.iter().zip([1.0, 2.0, 10.0]) {
            assert!((got - want).abs() < 1e-3, "ppp {ppp}: {d:?}");
        }
    }
}

#[test]
fn the_radius_helper_never_goes_to_zero_or_infinity() {
    assert_eq!(count_marker_radius(0, 1.0), 0.5);
    assert!(count_marker_radius(10, 0.0).is_finite());
    assert_eq!(count_marker_radius(2, 2.0), 0.5);
}
