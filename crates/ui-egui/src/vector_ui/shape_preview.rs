//! Shape appearance is stroked in document space before the camera transform.

use crate::canvas::ViewXform;
use egui::{Color32, Painter, Pos2, Stroke};

/// Tessellate the existing convex preview in document pixels, including its stroke. Stroking
/// after mapping the contour would give vertical and horizontal edges the same screen width
/// even when the view stretches document pixels horizontally.
pub(super) fn paint(painter: &Painter, xf: &ViewXform, points: &[Pos2], fill: Color32, stroke: Stroke) {
    let pixels_per_native_pixel = painter.ctx().pixels_per_point() * xf.zoom * xf.aspect.max(1.0);
    if !pixels_per_native_pixel.is_finite() || pixels_per_native_pixel <= 0.0 {
        return;
    }
    // Use the largest camera scale so the transformed antialias feather is at most one display
    // pixel on either axis. Its narrower-axis feather scales with the native stroke geometry.
    let options = painter.ctx().tessellation_options(|options| *options);
    let mut tessellator = egui::epaint::Tessellator::new(pixels_per_native_pixel, options, [1, 1], Vec::new());
    let mut mesh = egui::Mesh::default();
    tessellator.tessellate_shape(egui::Shape::convex_polygon(points.to_vec(), fill, stroke), &mut mesh);
    for vertex in &mut mesh.vertices {
        vertex.pos = xf.to_screen(vertex.pos.x, vertex.pos.y);
    }
    painter.add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use crate::PhotocraftApp;
    use crate::canvas::ViewXform;
    use crate::state::Tool;
    use egui::{Color32, Modifiers, Pos2, Rect, pos2, vec2};

    #[test]
    fn rendered_preview_stroke_width_follows_native_pixel_aspect() {
        let mut failures = Vec::new();
        for ppp in [1.0, 2.0] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.ui.tool_options.stroke_width = 20.0;
            app.ui.tool_options.shape_fill = false;
            app.session.tools.background = [1.0, 0.0, 0.0, 1.0];
            app.session.tools.foreground = [0.0, 1.0, 0.0, 1.0];
            let xf = ViewXform {
                rect: Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0)),
                zoom: 1.0 / ppp,
                aspect: 1.0,
                center: [0.0, 0.0],
                flip: false,
                rotation: 0.0,
            };
            // Render the production overlay through wgpu; both canvas compositors use this same
            // draw_shape_preview function. A missing adapter must fail rather than skip the proof.
            let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 400.0)).with_pixels_per_point(ppp).wgpu().build_ui_state(
                |ui, (app, xf)| {
                    ui.painter().rect_filled(ui.max_rect(), 0.0, Color32::WHITE);
                    super::super::draw_shape_preview(app, ui.painter(), xf, Tool::Rectangle, [-60.0, -60.0], [60.0, 60.0], Modifiers::NONE);
                },
                (app, xf),
            );
            for aspect in [1.0, 2.0, 0.91] {
                for rotation in [0.0, 90.0] {
                    for flip in [false, true] {
                        let xf = &mut h.state_mut().1;
                        xf.aspect = aspect;
                        xf.rotation = rotation;
                        xf.flip = flip;
                        let xf = *xf;
                        h.run_steps(2);
                        let image = h.render().expect("wgpu preview render");
                        if let Ok(dir) = std::env::var("PHOTOCRAFT_PREVIEW_ARTIFACT_DIR") {
                            std::fs::create_dir_all(&dir).expect("artifact directory");
                            image.save(format!("{dir}/preview-ppp{ppp}-par{aspect}-rot{rotation}-flip{flip}.png")).expect("save preview");
                        }
                        for (edge, outward, expected) in [(pos2(-60.0, 0.0), vec2(-1.0, 0.0), 20.0 * aspect), (pos2(0.0, -60.0), vec2(0.0, -1.0), 20.0)] {
                            let center = xf.to_screen(edge.x, edge.y) * ppp;
                            let normal = xf.map_vec(outward).normalized();
                            let red: Vec<_> = (-30..=30)
                                .filter(|offset| {
                                    let p = center + normal * *offset as f32;
                                    let c = image.get_pixel(p.x.floor() as u32, p.y.floor() as u32).0;
                                    c[0] > 200 && c[1] < 128 && c[2] < 128
                                })
                                .collect();
                            let width = red.last().zip(red.first()).map(|(last, first)| last - first + 1).unwrap_or(0) as f32;
                            eprintln!("ppp={ppp} par={aspect} rotation={rotation} flip={flip} edge={edge:?}: rendered={width} expected={expected}");
                            if (width - expected).abs() > 1.1 {
                                failures.push(format!("ppp={ppp} par={aspect} rotation={rotation} flip={flip} edge={edge:?}: {width} != {expected}"));
                            }
                        }
                        let fill = image.get_pixel((200.0 * ppp) as u32, (200.0 * ppp) as u32).0;
                        assert!(fill[0] > 240 && fill[1] > 240 && fill[2] > 240, "unfilled center preserved");
                    }
                }
            }
            h.state_mut().0.ui.tool_options.shape_fill = true;
            h.run_steps(2);
            let filled = h.render().expect("filled preview render");
            let center = filled.get_pixel((200.0 * ppp) as u32, (200.0 * ppp) as u32).0;
            assert!(center[1] > 240 && center[0] < 10 && center[2] < 10, "foreground fill preserved");
        }
        assert!(failures.is_empty(), "native stroke width mismatch:\n{}", failures.join("\n"));
    }
}
