//! Small vector glyphs for the LUT list, drawn with the painter so they follow the theme colours.

use egui::{Pos2, Shape, Ui, vec2};

pub(super) fn chevron(ui: &Ui, at: Pos2, open: bool, color: egui::Color32) {
    let s = 3.5;
    let pts = if open {
        vec![at + vec2(-s, -s * 0.6), at + vec2(s, -s * 0.6), at + vec2(0.0, s * 0.8)]
    } else {
        vec![at + vec2(-s * 0.6, -s), at + vec2(-s * 0.6, s), at + vec2(s * 0.8, 0.0)]
    };
    ui.painter().add(Shape::convex_polygon(pts, color, egui::Stroke::NONE));
}

pub(super) fn star(ui: &Ui, at: Pos2, on: bool, color: egui::Color32) {
    let (outer, inner) = (6.5f32, 2.8f32);
    let pts: Vec<Pos2> = (0..10)
        .map(|i| {
            let (r, a) = (if i % 2 == 0 { outer } else { inner }, std::f32::consts::PI * (i as f32 / 5.0) - std::f32::consts::FRAC_PI_2);
            at + vec2(a.cos() * r, a.sin() * r)
        })
        .collect();
    if on {
        // A star is not convex, so fill it as a fan of triangles from the centre.
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(at, color);
        for p in &pts {
            mesh.colored_vertex(*p, color);
        }
        for i in 0..pts.len() as u32 {
            mesh.add_triangle(0, 1 + i, 1 + (i + 1) % pts.len() as u32);
        }
        ui.painter().add(Shape::mesh(mesh));
    } else {
        ui.painter().add(Shape::closed_line(pts, egui::Stroke::new(1.0, color)));
    }
}
