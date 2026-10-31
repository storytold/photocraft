//! Project the native Clone Source overlay onto visible UV triangles, without GPU readback.
use crate::PhotocraftApp;
use egui::{Pos2, Rect};
use photocraft_uv::{
    Scene,
    prepared::Geometry,
    preview::{Camera, Materials},
};

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
        let center = poly.iter().fold(egui::Vec2::ZERO, |a, p| a + p.to_vec2()) / poly.len() as f32;
        let Some(center_weights) = weights([f64::from(center.x), f64::from(center.y)], uv) else { continue };
        let position = std::array::from_fn(|j| t.positions.iter().zip(center_weights).map(|(p, w)| f64::from(p[j]) * w).sum());
        let Some(q) = g.project_normalized(camera, position, size) else { continue };
        // Discard occluded triangles; the source must not shine through another part of the mesh.
        if id != hit.triangle && g.hit(scene, camera, maps, q, size).is_none_or(|h| h.triangle != id) {
            continue;
        }
        let mut vertices = Vec::new();
        for p in poly {
            let Some(w) = weights([f64::from(p.x), f64::from(p.y)], uv) else { continue };
            let pos = std::array::from_fn(|j| t.positions.iter().zip(w).map(|(p, w)| f64::from(p[j]) * w).sum());
            let Some(q) = g.project_normalized(camera, pos, size) else { continue };
            vertices.push(egui::epaint::Vertex {
                pos: viewport.min + egui::vec2(q[0] as f32 * viewport.width(), q[1] as f32 * viewport.height()),
                uv: egui::pos2((p.x - patch.x0 as f32) / patch.width() as f32, (p.y - patch.y0 as f32) / patch.height() as f32),
                color: tint,
            });
        }
        let base = mesh.vertices.len() as u32;
        for i in 1..vertices.len().saturating_sub(1) {
            mesh.indices.extend_from_slice(&[base, base + i as u32, base + i as u32 + 1]);
        }
        mesh.vertices.extend(vertices);
    }
    painter.add(egui::Shape::mesh(mesh));
}
