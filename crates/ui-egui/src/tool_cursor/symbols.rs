//! Original tool pointer geometry, shared by bitmap rasterization and painter fallbacks.

use egui::{Painter, Pos2, Stroke, vec2};

const LENS: f32 = 6.5;
const HANDLE: [[f32; 2]; 2] = [[5.0, 5.0], [12.5, 12.5]];
const MINUS: [[f32; 2]; 2] = [[-3.0, 0.0], [3.0, 0.0]];
const PLUS: [[f32; 2]; 2] = [[0.0, -3.0], [0.0, 3.0]];
const TUBE: [[f32; 2]; 6] = [[0.0, 0.0], [1.0, -4.0], [12.0, -15.0], [16.0, -11.0], [5.0, 0.0], [0.0, 0.0]];
const COLLAR: [[f32; 2]; 2] = [[10.0, -17.0], [18.0, -9.0]];
const GRIP: [[f32; 2]; 5] = [[12.0, -17.0], [16.0, -21.0], [22.0, -15.0], [18.0, -11.0], [12.0, -17.0]];

fn paint_path(painter: &Painter, at: Pos2, points: &[[f32; 2]], stroke: Stroke) {
    for pair in points.windows(2) {
        if let [a, b] = pair {
            painter.line_segment([at + vec2(a[0], a[1]), at + vec2(b[0], b[1])], stroke);
        }
    }
}

pub(super) fn paint_zoom(painter: &Painter, at: Pos2, out: bool, stroke: Stroke) {
    painter.circle_stroke(at, LENS, stroke);
    paint_path(painter, at, &HANDLE, stroke);
    paint_path(painter, at, &MINUS, stroke);
    if !out {
        paint_path(painter, at, &PLUS, stroke);
    }
}

pub(super) fn paint_pipette(painter: &Painter, at: Pos2, stroke: Stroke) {
    for points in [&TUBE[..], &COLLAR[..], &GRIP[..]] {
        paint_path(painter, at, points, stroke);
    }
}

fn path_distance(x: f32, y: f32, points: &[[f32; 2]]) -> f32 {
    let mut distance = f32::INFINITY;
    for pair in points.windows(2) {
        if let [a, b] = pair {
            let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
            let length = vx * vx + vy * vy;
            let t = if length > 0.0 { ((x - a[0]) * vx + (y - a[1]) * vy) / length } else { 0.0 };
            let t = t.clamp(0.0, 1.0);
            distance = distance.min((x - a[0] - t * vx).hypot(y - a[1] - t * vy));
        }
    }
    distance
}

pub(super) fn zoom_distance(x: f32, y: f32, out: bool) -> f32 {
    let d = (x.hypot(y) - LENS).abs().min(path_distance(x, y, &HANDLE)).min(path_distance(x, y, &MINUS));
    if out { d } else { d.min(path_distance(x, y, &PLUS)) }
}

pub(super) fn pipette_distance(x: f32, y: f32) -> f32 {
    path_distance(x, y, &TUBE).min(path_distance(x, y, &COLLAR)).min(path_distance(x, y, &GRIP))
}
