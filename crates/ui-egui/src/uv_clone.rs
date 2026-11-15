//! Project the native Clone Source overlay onto visible UV triangles, without GPU readback.
use crate::PhotocraftApp;
use egui::{Pos2, Rect};
use photocraft_uv::{
    Scene,
    prepared::Geometry,
    preview::{Camera, Materials},
};

const MAX_OCCLUSION_SAMPLES: usize = 200_000;
const MAX_SUBDIVISION_DEPTH: u8 = 8;
const OCCLUSION_CELL_PIXELS: f32 = 3.0;

// Clipping is in screen f32 coordinates. Its boundary vertices can land a few
// ulps outside the original f64 triangle: evaluate its affine map without an
// inside-triangle rejection, or neighboring preview triangles acquire holes.
fn weights(p: [f64; 2], t: [[f64; 2]; 3]) -> Option<[f64; 3]> {
    let [a, b, c] = t;
    let cross = |a: [f64; 2], b: [f64; 2]| a[0] * b[1] - a[1] * b[0];
    let e = [b[0] - a[0], b[1] - a[1]];
    let f = [c[0] - a[0], c[1] - a[1]];
    let q = [p[0] - a[0], p[1] - a[1]];
    let area = cross(e, f);
    if !area.is_finite() || area.abs() < 1e-15 {
        return None;
    }
    let (v, w) = (cross(q, f) / area, cross(e, q) / area);
    Some([1.0 - v - w, v, w])
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn preview(
    app: &mut PhotocraftApp,
    ui: &egui::Ui,
    g: &Geometry,
    scene: &Scene,
    camera: Camera,
    maps: &Materials<'_>,
    size: [u32; 2],
    viewport: Rect,
) {
    let Some(p) = ui.input(|i| i.pointer.hover_pos()).filter(|p| viewport.contains(*p)) else { return };
    let at = (p - viewport.min) / viewport.size();
    let Some(hit) = g.hit(scene, camera, maps, [f64::from(at.x), f64::from(at.y)], size) else { return };
    let Some(tri) = g.triangles.get(hit.triangle) else { return };
    let Some(st) = app.session.active() else { return };
    if app.session.uv.map_bindings(app.session.uv.current_channel()).get(tri.material).copied().flatten() != Some(st.doc.id) {
        return;
    }
    let dimensions = [f64::from(st.doc.size.width), f64::from(st.doc.size.height)];
    let point = [hit.uv[0] * dimensions[0], (1.0 - hit.uv[1]) * dimensions[1]];
    let painter = ui.painter_at(viewport);
    let Some((texture, patch, tint)) = crate::canvas::clone_preview_texture(app, &painter, point, None) else { return };
    if patch.is_empty() {
        return;
    }
    let rect = Rect::from_min_max(egui::pos2(patch.x0 as f32, patch.y0 as f32), egui::pos2(patch.x1 as f32, patch.y1 as f32));
    let mut mesh = egui::Mesh::with_texture(texture);
    let mut sample_budget = MAX_OCCLUSION_SAMPLES;
    for (id, t) in g.triangles.iter().enumerate().filter(|(_, t)| t.material == tri.material) {
        let uv = t.uv.map(|q| [f64::from(q[0]) * dimensions[0], (1.0 - f64::from(q[1])) * dimensions[1]]);
        let points = uv.map(|q| egui::pos2(q[0] as f32, q[1] as f32));
        if !Rect::from_points(&points).intersects(rect) {
            continue;
        }
        let mut poly = points.to_vec();
        for (axis, edge, sign) in [(0, rect.left(), -1.0), (0, rect.right(), 1.0), (1, rect.top(), -1.0), (1, rect.bottom(), 1.0)] {
            let mut out = Vec::new();
            for (a, b) in poly.iter().zip(poly.iter().cycle().skip(1)).take(poly.len()) {
                let val = |p: Pos2| ((if axis == 0 { p.x } else { p.y }) - edge) * sign;
                let (a0, b0) = (val(*a), val(*b));
                if a0 <= 0.0 {
                    out.push(*a);
                }
                if (a0 <= 0.0) != (b0 <= 0.0) {
                    out.push(a.lerp(*b, a0 / (a0 - b0)));
                }
            }
            poly = out;
        }
        if poly.len() < 3 {
            continue;
        }
        for i in 1..poly.len() - 1 {
            let triangle = [poly[0], poly[i], poly[i + 1]].map(|p| [f64::from(p.x), f64::from(p.y)]);
            emit_visible_triangle(
                triangle,
                uv,
                t.positions,
                id,
                g,
                scene,
                camera,
                maps,
                size,
                viewport,
                patch,
                tint,
                &mut sample_budget,
                0,
                &mut mesh,
            );
            if sample_budget == 0 {
                break;
            }
        }
        if sample_budget == 0 {
            break;
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

#[allow(clippy::too_many_arguments)]
fn emit_visible_triangle(
    points: [[f64; 2]; 3],
    uv: [[f64; 2]; 3],
    positions: [[f32; 3]; 3],
    triangle_id: usize,
    geometry: &Geometry,
    scene: &Scene,
    camera: Camera,
    maps: &Materials<'_>,
    size: [u32; 2],
    viewport: Rect,
    patch: photocraft_geom::Rect,
    tint: egui::Color32,
    sample_budget: &mut usize,
    depth: u8,
    mesh: &mut egui::Mesh,
) {
    if *sample_budget == 0 {
        return;
    }
    let Some(projected) = points.map(|point| project_uv_point(point, uv, positions, geometry, camera, size)).into_iter().collect::<Option<Vec<_>>>()
    else {
        return;
    };
    let screen: Vec<_> = projected.iter().map(|point| viewport.min + egui::vec2(point[0] as f32 * viewport.width(), point[1] as f32 * viewport.height())).collect();
    let longest_edge = (0..3)
        .map(|index| screen[index].distance(screen[(index + 1) % 3]))
        .fold(0.0f32, f32::max);
    if longest_edge > OCCLUSION_CELL_PIXELS && depth < MAX_SUBDIVISION_DEPTH {
        let midpoint = |a: [f64; 2], b: [f64; 2]| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
        let [a, b, c] = points;
        let (ab, bc, ca) = (midpoint(a, b), midpoint(b, c), midpoint(c, a));
        for child in [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]] {
            emit_visible_triangle(
                child,
                uv,
                positions,
                triangle_id,
                geometry,
                scene,
                camera,
                maps,
                size,
                viewport,
                patch,
                tint,
                sample_budget,
                depth + 1,
                mesh,
            );
            if *sample_budget == 0 {
                return;
            }
        }
        return;
    }

    *sample_budget -= 1;
    let center = [points.iter().map(|point| point[0]).sum::<f64>() / 3.0, points.iter().map(|point| point[1]).sum::<f64>() / 3.0];
    let Some(center_screen) = project_uv_point(center, uv, positions, geometry, camera, size) else { return };
    // Check every small screen-space cell, including the cursor triangle. A triangle can be
    // visible at its centroid while much of its UV area is behind another surface.
    if geometry.hit(scene, camera, maps, center_screen, size).is_none_or(|hit| hit.triangle != triangle_id) {
        return;
    }
    let base = mesh.vertices.len() as u32;
    for (point, position) in points.into_iter().zip(projected) {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: viewport.min + egui::vec2(position[0] as f32 * viewport.width(), position[1] as f32 * viewport.height()),
            uv: egui::pos2(((point[0] as f32) - patch.x0 as f32) / patch.width() as f32, ((point[1] as f32) - patch.y0 as f32) / patch.height() as f32),
            color: tint,
        });
    }
    mesh.indices.extend_from_slice(&[base, base + 1, base + 2]);
}

fn project_uv_point(point: [f64; 2], uv: [[f64; 2]; 3], positions: [[f32; 3]; 3], geometry: &Geometry, camera: Camera, size: [u32; 2]) -> Option<[f64; 2]> {
    let barycentric = weights(point, uv)?;
    let position = std::array::from_fn(|axis| positions.iter().zip(barycentric).map(|(p, weight)| f64::from(p[axis]) * weight).sum());
    geometry.project_normalized(camera, position, size)
}
