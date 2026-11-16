//! #12513: the Cylinder's handles as measured in the reference application, on a 400 × 300 box at
//! (200, 150): a fresh cylinder stores wrap 0.9; the bottom-centre handle changes only the lower
//! arc's depth; a corner keeps both arcs' depth in pixels; all five handles are squares.

use super::*;

const BOX: [f64; 4] = [200.0, 150.0, 600.0, 450.0];

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn a_fresh_cylinder_stores_wrap_0_9_with_its_handle_20_px_right_of_centre() {
    let v = cylinder_values(&cylinder_default(BOX).unwrap()).unwrap();
    assert_eq!(&v[..6], &[200.0, 450.0, 600.0, 150.0, 0.5, 0.5]);
    assert!(close(v[6], 0.9, 1e-9), "{v:?}");
    let wrap = cylinder_handles(&v)[4].0;
    assert!(close(wrap[0], 420.0, 1e-9), "measured at x 421: {wrap:?}");
}

#[test]
fn the_bottom_handle_changes_only_the_lower_arcs_depth() {
    let v0 = cylinder_values(&cylinder_default(BOX).unwrap()).unwrap();
    let v = cylinder_dragged(v0, CylinderHandle::Bottom, [0.0, 40.0]);
    assert_eq!([v[0], v[1], v[2], v[3], v[4], v[6]], [200.0, 450.0, 600.0, 150.0, 0.5, v0[6]], "edges, corners and the top arc stay");
    assert!(close(v[5], 0.6975, 0.005), "measured 0.6975: {}", v[5]);
    let front = cylinder_handles(&v)[3].0;
    assert!(close(front[1], 590.0, 1.0), "the arc's front moves to y 590: {front:?}");
}

#[test]
fn a_corner_keeps_both_arcs_depth_in_pixels() {
    // From the measured depths 0.5 / 0.6975, the top-right corner dragged (-20, -20).
    let v0 = [200.0, 450.0, 600.0, 150.0, 0.5, 0.6975, 0.9];
    let v = cylinder_dragged(v0, CylinderHandle::Corner(1), [-20.0, -20.0]);
    assert_eq!([v[0], v[1], v[2], v[3]], [200.0, 450.0, 580.0, 130.0]);
    assert!(close(v[4], 0.527, 0.005) && close(v[5], 0.735, 0.005), "measured 0.527 / 0.735: {v:?}");
    // In pixels the arcs are as deep as before.
    let (a0, a1) = (200.0, 190.0);
    assert!(close(v[4] * a1, v0[4] * a0, 1e-9) && close(v[5] * a1, v0[5] * a0, 1e-9));
    assert_eq!(v[6], 0.9, "the wrap stays");
}

/// CodeRabbit on #15020: pixels are kept only while the depth fits the half axis. A corner dragged
/// 200 px in on a 400 px cylinder leaves a 100 px half axis; the lower arc (0.6975 of 200 px =
/// 139.5 px) would need 1.395 and stops at the half axis, 100 px, as the Bend and Bottom handles
/// do. The reference was measured only with arcs inside the half axis.
#[test]
fn a_corner_keeps_pixel_depth_only_up_to_the_half_axis() {
    let v0 = [200.0, 450.0, 600.0, 150.0, 0.5, 0.6975, 0.9];
    let v = cylinder_dragged(v0, CylinderHandle::Corner(1), [-200.0, 0.0]);
    assert_eq!([v[0], v[2]], [200.0, 400.0], "half axis 100 px");
    assert_eq!(v[5], 1.0, "the lower arc stops at the half axis (100 px), not 1.395: {v:?}");
    assert!(close(v[4], 1.0, 1e-9), "the upper arc (100 px of 200) reaches exactly the half axis: {v:?}");
    // A shrink that still fits keeps the pixels: 60 px in leaves a 170 px half axis.
    let w = cylinder_dragged(v0, CylinderHandle::Corner(1), [-60.0, 0.0]);
    assert!(close(w[5] * 170.0, 0.6975 * 200.0, 1e-9) && close(w[4] * 170.0, 0.5 * 200.0, 1e-9), "{w:?}");
    // A depth a file put past the half axis is kept as it was, not pulled in.
    let past = cylinder_dragged([200.0, 450.0, 600.0, 150.0, 1.4, 0.5, 0.9], CylinderHandle::Corner(1), [-20.0, 0.0]);
    assert!(close(past[4], 1.4, 1e-9), "its own reach: {past:?}");
}

#[test]
fn a_drag_inside_moves_the_whole_cylinder_with_its_values() {
    let v0 = cylinder_values(&cylinder_default(BOX).unwrap()).unwrap();
    let v = cylinder_dragged(v0, CylinderHandle::Surface, [15.0, -10.0]);
    assert_eq!(v, [215.0, 440.0, 615.0, 140.0, v0[4], v0[5], v0[6]]);
}

#[test]
fn all_five_handles_are_squares() {
    let ctx = egui::Context::default();
    // A real preview: a layer in Warp mode.
    let mut app = super::super::tests::app_with_square(800, photocraft_geom::Rect::new(200, 150, 600, 450));
    super::super::begin_warp(&mut app, &ctx).unwrap();
    let pv = app.transform_preview.as_ref().expect("the preview");
    let w = cylinder_default(BOX).unwrap();
    let line = Stroke::new(1.0, Color32::from_rgb(40, 120, 220));
    let mut o = ctx.run_ui(Default::default(), |ui| {
        draw_cylinder(ui.painter(), &w, |_, _| Vec::new(), |q| egui::pos2(q[0] as f32 / 4.0, q[1] as f32 / 4.0), pv, line);
    });
    o.textures_delta.clear();
    let shapes: Vec<&egui::Shape> = o.shapes.iter().map(|c| &c.shape).collect();
    let rects = shapes.iter().filter(|s| matches!(s, egui::Shape::Rect(_))).count();
    assert!(rects >= 5, "a filled and stroked square per handle: {rects}");
    assert!(!shapes.iter().any(|s| matches!(s, egui::Shape::Circle(_))), "no circles");
    assert!(!shapes.iter().any(|s| matches!(s, egui::Shape::Path(p) if p.points.len() == 4 && p.closed)), "no diamond");
}
