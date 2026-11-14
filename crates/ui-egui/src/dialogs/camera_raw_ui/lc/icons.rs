//! The develop view's icons, drawn in code on a 20×20 design grid (1.5 px strokes): LightCraft's
//! own vector icons (`crates/ui-egui/src/icons.rs` in storytold/lightcraft, original work, MIT OR
//! Apache-2.0), the subset the RAW Develop dialog uses, and PhotoCraft's own Upright mode icons
//! (`Upright*`, original work in the same style; recorded in `ATTRIBUTION.md`).

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Icon {
    /// Edit (three sliders).
    Sliders,
    /// Heal / remove (an eraser).
    Eraser,
    /// Masking (a dotted disc).
    Mask,
    Eye,
    EyeOff,
    ChevronDown,
    ChevronRight,
    ChevronLeft,
    Brush,
    Linear,
    Radial,
    /// The colour-range picker.
    Picker,
    Trash,
    Curve,
    /// Colour grading (three circles).
    GradeAll,
    /// Red eye: an eye with a filled pupil.
    RedEye,
    /// Upright Off: a circle struck through.
    UprightOff,
    /// Upright Auto: an "A" in a frame.
    UprightAuto,
    /// Upright Level: a level line with a bubble at its middle and marks at its ends.
    UprightLevel,
    /// Upright Vertical: converging verticals (faint) made upright.
    UprightVertical,
    /// Upright Full: a square frame with its grid (level and both perspectives).
    UprightFull,
    /// Upright Guided: two guide lines with handles at their ends.
    UprightGuided,
}

struct Pen<'a> {
    p: &'a Painter,
    r: Rect,
    s: Stroke,
    k: f32,
}

impl Pen<'_> {
    /// The same pen in a faded colour (the "before" of a correction).
    fn faint(&self) -> Pen<'_> {
        Pen { p: self.p, r: self.r, s: Stroke::new(self.s.width, self.s.color.gamma_multiply(0.55)), k: self.k }
    }
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.r.min.x + x * self.k, self.r.min.y + y * self.k)
    }
    fn line(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|(x, y)| self.pt(*x, *y)).collect();
        self.p.add(Shape::line(v, self.s));
    }
    fn closed(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|(x, y)| self.pt(*x, *y)).collect();
        self.p.add(Shape::closed_line(v, self.s));
    }
    fn circle(&self, x: f32, y: f32, r: f32) {
        self.p.circle_stroke(self.pt(x, y), r * self.k, self.s);
    }
    fn dot(&self, x: f32, y: f32, r: f32) {
        self.p.circle_filled(self.pt(x, y), r * self.k, self.s.color);
    }
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32, round: f32) {
        self.p.rect_stroke(Rect::from_min_max(self.pt(x0, y0), self.pt(x1, y1)), round * self.k, self.s, egui::StrokeKind::Middle);
    }
    fn arc(&self, cx: f32, cy: f32, r: f32, a0: f32, a1: f32) {
        let n = 24;
        let v: Vec<(f32, f32)> = (0..=n)
            .map(|i| {
                let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        self.line(&v);
    }
}

/// Paints `icon` centred in `rect` (its square part) in `color`.
pub(crate) fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let side = rect.width().min(rect.height());
    // disclosure chevrons share one size and stroke
    let side = if matches!(icon, Icon::ChevronDown | Icon::ChevronLeft | Icon::ChevronRight) { side.min(16.0) } else { side };
    let r = Rect::from_center_size(rect.center(), vec2(side, side));
    let k = side / 20.0;
    let pen = Pen { p, r, s: Stroke::new((1.5 * k).max(1.0), color), k };
    match icon {
        Icon::Sliders => {
            for (y, x) in [(5.0, 12.0), (10.0, 7.0), (15.0, 13.0)] {
                pen.line(&[(3.0, y), (x - 2.0, y)]);
                pen.line(&[(x + 2.0, y), (17.0, y)]);
                pen.circle(x, y, 2.0);
            }
        }
        Icon::Eraser => {
            pen.closed(&[(3.0, 12.0), (10.0, 5.0), (16.0, 11.0), (11.0, 16.0), (7.0, 16.0)]);
            pen.line(&[(6.5, 8.5), (12.5, 14.5)]);
            pen.line(&[(7.0, 17.5), (17.0, 17.5)]);
        }
        Icon::Mask => {
            pen.circle(10.0, 10.0, 7.0);
            for i in 0..5 {
                for j in 0..5 {
                    let (x, y) = (5.0 + i as f32 * 2.5, 5.0 + j as f32 * 2.5);
                    if (x - 10.0).hypot(y - 10.0) < 5.8 {
                        pen.dot(x, y, 0.6);
                    }
                }
            }
        }
        Icon::Eye => {
            pen.arc(10.0, 16.0, 9.0, 212.0, 328.0);
            pen.arc(10.0, 4.0, 9.0, 32.0, 148.0);
            pen.circle(10.0, 10.0, 2.5);
        }
        Icon::EyeOff => {
            pen.arc(10.0, 16.0, 9.0, 212.0, 328.0);
            pen.arc(10.0, 4.0, 9.0, 32.0, 148.0);
            pen.circle(10.0, 10.0, 2.5);
            pen.line(&[(3.5, 3.5), (16.5, 16.5)]);
        }
        Icon::RedEye => {
            pen.arc(10.0, 16.0, 9.0, 212.0, 328.0);
            pen.arc(10.0, 4.0, 9.0, 32.0, 148.0);
            pen.dot(10.0, 10.0, 2.8);
        }
        Icon::ChevronDown => pen.line(&[(5.0, 7.5), (10.0, 12.5), (15.0, 7.5)]),
        Icon::ChevronRight => pen.line(&[(7.5, 5.0), (12.5, 10.0), (7.5, 15.0)]),
        Icon::ChevronLeft => pen.line(&[(12.5, 5.0), (7.5, 10.0), (12.5, 15.0)]),
        Icon::Brush => {
            pen.line(&[(16.5, 3.5), (9.0, 11.0)]);
            pen.closed(&[(9.0, 11.0), (6.0, 11.5), (4.0, 14.5), (3.5, 17.0), (6.0, 16.5), (8.5, 14.0)]);
        }
        Icon::Linear => {
            for (i, x) in [4.0, 7.5, 11.0, 14.5].iter().enumerate() {
                let a = 1.0 - i as f32 * 0.22;
                pen.p.line_segment([pen.pt(*x, 3.0), pen.pt(*x, 17.0)], Stroke::new(pen.s.width, color.gamma_multiply(a)));
            }
        }
        Icon::Radial => {
            pen.circle(10.0, 10.0, 7.0);
            pen.p.circle_stroke(pen.pt(10.0, 10.0), 3.5 * k, Stroke::new(pen.s.width, color.gamma_multiply(0.5)));
        }
        Icon::Picker => {
            pen.line(&[(4.0, 16.0), (11.5, 8.5)]);
            pen.closed(&[(11.0, 5.0), (15.0, 9.0), (17.0, 7.0), (13.0, 3.0)]);
        }
        Icon::Trash => {
            pen.line(&[(3.0, 5.5), (17.0, 5.5)]);
            pen.line(&[(8.0, 5.5), (8.0, 3.0), (12.0, 3.0), (12.0, 5.5)]);
            pen.closed(&[(5.0, 5.5), (15.0, 5.5), (14.0, 17.0), (6.0, 17.0)]);
        }
        Icon::Curve => {
            pen.rect(3.0, 3.0, 17.0, 17.0, 1.5);
            let pts: Vec<(f32, f32)> = (0..=12)
                .map(|i| {
                    let t = i as f32 / 12.0;
                    let y = t * t * (3.0 - 2.0 * t);
                    (4.5 + 11.0 * t, 15.5 - 11.0 * y)
                })
                .collect();
            pen.line(&pts);
        }
        Icon::GradeAll => {
            pen.circle(10.0, 6.0, 3.2);
            pen.circle(5.5, 13.5, 3.2);
            pen.circle(14.5, 13.5, 3.2);
        }
        Icon::UprightOff => {
            pen.circle(10.0, 10.0, 7.0);
            pen.line(&[(5.05, 14.95), (14.95, 5.05)]);
        }
        Icon::UprightAuto => {
            pen.rect(2.5, 2.5, 17.5, 17.5, 2.0);
            pen.line(&[(6.2, 14.5), (10.0, 5.5), (13.8, 14.5)]);
            pen.line(&[(7.6, 11.2), (12.4, 11.2)]);
        }
        Icon::UprightLevel => {
            pen.line(&[(2.0, 10.0), (7.5, 10.0)]);
            pen.line(&[(12.5, 10.0), (18.0, 10.0)]);
            pen.circle(10.0, 10.0, 2.5);
            pen.line(&[(2.5, 5.5), (2.5, 14.5)]);
            pen.line(&[(17.5, 5.5), (17.5, 14.5)]);
        }
        Icon::UprightVertical => {
            let f = pen.faint();
            f.line(&[(4.0, 17.0), (8.0, 3.0)]);
            f.line(&[(16.0, 17.0), (12.0, 3.0)]);
            pen.line(&[(4.0, 3.0), (4.0, 17.0)]);
            pen.line(&[(16.0, 3.0), (16.0, 17.0)]);
        }
        Icon::UprightFull => {
            pen.rect(3.0, 3.0, 17.0, 17.0, 0.5);
            pen.line(&[(10.0, 3.0), (10.0, 17.0)]);
            pen.line(&[(3.0, 10.0), (17.0, 10.0)]);
        }
        Icon::UprightGuided => {
            for (top, bottom) in [((6.0, 4.0), (4.0, 16.0)), ((14.0, 4.0), (16.0, 16.0))] {
                pen.line(&[top, bottom]);
                pen.dot(top.0, top.1, 1.9);
                pen.dot(bottom.0, bottom.1, 1.9);
            }
        }
    }
}
