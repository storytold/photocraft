//! #13872: an empty frame's placeholder is its box with an X over it, whatever its shape, drawn
//! over the canvas only (the composite pixels stay as they are).
use super::*;
use crate::canvas::ViewXform;

/// A 600 × 400 white document with one empty frame of `shape` at 100,80 sized 200 × 120.
fn app(shape: &str) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400, "background": "white"})).unwrap();
    app.run("layer.new.frame", json!({"x": 100, "y": 80, "width": 200, "height": 120, "shape": shape})).unwrap();
    app
}

/// What the Frame tool's overlay paints for `app`, at 100 % with the document's origin at (0,0).
fn overlay(app: &PhotocraftApp) -> Vec<egui::Shape> {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    ctx.begin_pass(egui::RawInput::default());
    {
        let painter = ctx.layer_painter(egui::LayerId::background());
        let xf = ViewXform {
            rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0)),
            zoom: 1.0,
            center: [300.0, 200.0],
            flip: false,
            rotation: 0.0,
            aspect: 1.0,
        };
        draw_overlay(app, &painter, &xf);
    }
    let mut out = ctx.end_pass();
    out.textures_delta.clear();
    fn flat(s: egui::Shape, out: &mut Vec<egui::Shape>) {
        match s {
            egui::Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
            s => out.push(s),
        }
    }
    let mut shapes = Vec::new();
    out.shapes.into_iter().for_each(|c| flat(c.shape, &mut shapes));
    shapes
}

/// The polylines and segments painted, as point lists. A closed path ends on its first point again, so
/// its closing side is a segment like any other; an open path does not get one (`PathShape::closed`).
fn lines(shapes: &[egui::Shape]) -> Vec<Vec<[f32; 2]>> {
    shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Path(p) => {
                let mut points: Vec<[f32; 2]> = p.points.iter().map(|p| [p.x, p.y]).collect();
                if p.closed
                    && let Some(first) = points.first().copied()
                {
                    points.push(first);
                }
                Some(points)
            }
            egui::Shape::LineSegment { points, .. } => Some(points.iter().map(|p| [p.x, p.y]).collect()),
            _ => None,
        })
        .collect()
}

const BOX: [[f32; 2]; 4] = [[100.0, 80.0], [300.0, 80.0], [300.0, 200.0], [100.0, 200.0]];

fn on_box(p: [f32; 2]) -> bool {
    BOX.iter().any(|q| (q[0] - p[0]).abs() < 0.01 && (q[1] - p[1]).abs() < 0.01)
}

#[test]
fn every_shape_shows_its_box_and_an_x() {
    for shape in ["rectangle", "ellipse", "triangle", "polygon"] {
        let drawn = lines(&overlay(&app(shape)));
        // Only corner-to-corner lines: the box's four sides and the two diagonals.
        assert!(drawn.iter().all(|l| l.iter().all(|p| on_box(*p))), "{shape}: no shape outline, {drawn:?}");
        let has = |a: [f32; 2], b: [f32; 2]| drawn.iter().any(|l| l.windows(2).any(|w| (w[0] == a && w[1] == b) || (w[0] == b && w[1] == a)));
        for (a, b) in [(0, 1), (1, 2), (2, 3)] {
            assert!(has(BOX[a], BOX[b]), "{shape}: side {a}-{b}");
        }
        assert!(has(BOX[3], BOX[0]), "{shape}: the box is closed (its fourth side, 3-0, is drawn)");
        assert!(has(BOX[0], BOX[2]) && has(BOX[1], BOX[3]), "{shape}: both diagonals");
    }
}

#[test]
fn the_placeholder_is_not_in_the_pixels() {
    for shape in ["rectangle", "ellipse"] {
        let app = app(shape);
        let doc = &app.session.active().unwrap().doc;
        // On the X's lines and the box's edge.
        for (x, y) in [(200, 140), (150, 110), (100, 140), (250, 80)] {
            let px = photocraft_compose::render(doc, photocraft_geom::Rect::new(x, y, x + 1, y + 1)).px[0];
            assert_eq!(px, [1.0, 1.0, 1.0, 1.0], "{shape} at ({x},{y})");
        }
    }
}

/// CodeRabbit on #15149: an open path with the box's four corners has no fourth side, and the closure check
/// says so (the first three sides and both diagonals alone do not make the box).
#[test]
fn an_open_four_corner_path_does_not_pass_for_a_closed_box() {
    let open = egui::Shape::Path(egui::epaint::PathShape::line(BOX.iter().map(|p| egui::pos2(p[0], p[1])).collect(), egui::Stroke::NONE));
    let drawn = lines(&[open]);
    let has = |a: [f32; 2], b: [f32; 2]| drawn.iter().any(|l| l.windows(2).any(|w| (w[0] == a && w[1] == b) || (w[0] == b && w[1] == a)));
    assert!(has(BOX[0], BOX[1]) && has(BOX[1], BOX[2]) && has(BOX[2], BOX[3]), "its three sides");
    assert!(!has(BOX[3], BOX[0]), "no fourth side for an open path");
    let closed = egui::Shape::Path(egui::epaint::PathShape::closed_line(BOX.iter().map(|p| egui::pos2(p[0], p[1])).collect(), egui::Stroke::NONE));
    let drawn = lines(&[closed]);
    assert!(drawn.iter().any(|l| l.windows(2).any(|w| w[0] == BOX[3] && w[1] == BOX[0])), "a closed one has it");
}
