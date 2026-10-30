//! Camera Raw › Geometry › Upright › Guided (Shift+T): guides drawn on the preview along lines
//! that should be plumb or level. Drag on the image to draw one (up to four; two or more apply
//! the correction), drag an endpoint to move it, click a guide to select it and press Delete to
//! remove it. A loupe magnifies the pointer's surroundings while placing an endpoint.
//!
//! Guides are stored in the frame before any Geometry ([`CameraRaw::upright_guides`]), so they
//! stay on the picture's lines as the correction changes; they are drawn through the same
//! [`GeometryMap`] the develop uses. The map is frozen while a guide is dragged, so the guide
//! follows the pointer and the correction lands when it is let go.

use egui::{Color32, Pos2, Rect, Response, Stroke, Ui, pos2, vec2};
use photocraft_algo::camera_raw::{CameraRaw, GeometryMap, upright};

/// At most this many guides, as Camera Raw allows.
pub(crate) const MAX_GUIDES: usize = 4;
/// Screen distance (points) within which an endpoint or a guide is picked.
const PICK: f32 = 9.0;

enum Drag {
    New { start: Pos2 },
    End { guide: usize, end: usize },
}

/// Dialog-local state of the Guided Upright tool.
#[derive(Default)]
pub(crate) struct GuideTool {
    /// The tool is chosen (Shift+T, or the Guided button).
    pub active: bool,
    pub selected: Option<usize>,
    /// Scroll the panels to Geometry on the next frame (Shift+T opened it).
    pub reveal: bool,
    drag: Option<(Drag, Pos2, GeometryMap)>,
}

impl GuideTool {
    /// Drops a drag in progress; `true` if there was one.
    pub(crate) fn cancel_drag(&mut self) -> bool {
        self.drag.take().is_some()
    }
    /// Esc: drops a drag in progress, else puts the tool down; `false` when neither was up (Esc
    /// then cancels the dialog).
    pub(crate) fn escape(&mut self) -> bool {
        if self.cancel_drag() {
            return true;
        }
        std::mem::take(&mut self.active)
    }
}

/// The preview's map for `p` over a `w × h` frame: Guided Upright from its guides only (the
/// automatic modes aren't shown with guides).
pub(crate) fn map_for(p: &CameraRaw, w: usize, h: usize) -> Option<GeometryMap> {
    let up = if p.upright == "guided" && p.upright_active() { upright::homography("guided", &p.upright_guides, &[], w, h) } else { None };
    GeometryMap::new(p, w, h, up.as_ref())
}

/// Selects Guided Upright and its tool (Shift+T); a second press puts the tool down.
pub(crate) fn toggle(tool: &mut GuideTool, p: &mut CameraRaw) -> bool {
    if tool.active && p.upright == "guided" {
        tool.active = false;
        tool.drag = None;
        return false;
    }
    tool.active = true;
    if p.upright != "guided" {
        p.upright = "guided".into();
        return true;
    }
    false
}

/// Removes the selected guide (Delete / Backspace); `true` if one went.
pub(crate) fn delete_selected(tool: &mut GuideTool, p: &mut CameraRaw) -> bool {
    let Some(i) = tool.selected.take() else { return false };
    if i < p.upright_guides.len() {
        p.upright_guides.remove(i);
        return true;
    }
    false
}

struct Frame {
    rect: Rect,
    w: f64,
    h: f64,
}

impl Frame {
    /// Screen → output pixel.
    fn px(&self, s: Pos2) -> [f64; 2] {
        [f64::from((s.x - self.rect.left()) / self.rect.width().max(1e-3)) * self.w, f64::from((s.y - self.rect.top()) / self.rect.height().max(1e-3)) * self.h]
    }
    /// Output pixel → screen.
    fn screen(&self, p: [f64; 2]) -> Pos2 {
        pos2(self.rect.left() + (p[0] / self.w) as f32 * self.rect.width(), self.rect.top() + (p[1] / self.h) as f32 * self.rect.height())
    }
    /// Screen → guide coordinates (fractions of the source frame).
    fn guide_point(&self, map: &GeometryMap, s: Pos2) -> Option<[f32; 2]> {
        let [x, y] = self.px(s);
        let q = map.source_of(x, y)?;
        Some([(q[0] / self.w).clamp(0.0, 1.0) as f32, (q[1] / self.h).clamp(0.0, 1.0) as f32])
    }
    /// Guide coordinates → screen.
    fn guide_screen(&self, map: &GeometryMap, g: [f32; 2]) -> Option<Pos2> {
        map.output_of(f64::from(g[0]) * self.w, f64::from(g[1]) * self.h).map(|p| self.screen(p))
    }
}

fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Handles the tool on the preview image `rect` (the whole `w × h` frame on screen) and paints
/// the guides. `params` are the settings the preview shows. Returns `true` when the guides
/// changed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn preview(
    ui: &mut Ui,
    tool: &mut GuideTool,
    params: &mut CameraRaw,
    shown: &CameraRaw,
    rect: Rect,
    w: usize,
    h: usize,
    response: &Response,
    panning: bool,
    texture: Option<&egui::TextureHandle>,
) -> bool {
    let frame = Frame { rect, w: w as f64, h: h as f64 };
    let Some(map) = map_for(shown, w, h) else { return false };
    let mut changed = false;
    if !panning {
        response.clone().on_hover_cursor(egui::CursorIcon::Crosshair);
        let ends: Vec<[Option<Pos2>; 2]> = params.upright_guides.iter().map(|g| [frame.guide_screen(&map, g[0]), frame.guide_screen(&map, g[1])]).collect();
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(start) = ui.input(|i| i.pointer.press_origin())
        {
            let near = ends
                .iter()
                .enumerate()
                .flat_map(|(i, e)| e.iter().enumerate().filter_map(move |(k, p)| p.map(|p| (i, k, p.distance(start)))))
                .filter(|(_, _, d)| *d < PICK)
                .min_by(|a, b| a.2.total_cmp(&b.2));
            let drag = match near {
                Some((guide, end, _)) => Some(Drag::End { guide, end }),
                None if params.upright_guides.len() < MAX_GUIDES && rect.contains(start) => Some(Drag::New { start }),
                None => None,
            };
            if let Some(drag) = drag {
                tool.selected = match drag {
                    Drag::End { guide, .. } => Some(guide),
                    Drag::New { .. } => None,
                };
                if let Some(frozen) = map_for(shown, w, h) {
                    tool.drag = Some((drag, start, frozen));
                }
            }
        }
        if let Some((_, cur, _)) = tool.drag.as_mut()
            && let Some(p) = response.interact_pointer_pos()
        {
            *cur = p;
        }
        if response.drag_stopped()
            && let Some((drag, cur, frozen)) = tool.drag.take()
        {
            match drag {
                Drag::New { start } if start.distance(cur) > 2.0 * PICK => {
                    if let (Some(a), Some(b)) = (frame.guide_point(&frozen, start), frame.guide_point(&frozen, cur)) {
                        params.upright_guides.push([a, b]);
                        tool.selected = Some(params.upright_guides.len() - 1);
                        changed = true;
                    }
                }
                Drag::End { guide, end } => {
                    if let (Some(p), Some(g)) = (frame.guide_point(&frozen, cur), params.upright_guides.get_mut(guide)) {
                        g[end] = p;
                        changed = true;
                    }
                }
                Drag::New { .. } => {}
            }
        }
        if response.clicked()
            && let Some(p) = response.interact_pointer_pos()
        {
            tool.selected = ends.iter().enumerate().find_map(|(i, e)| match e {
                [Some(a), Some(b)] if distance_to_segment(p, *a, *b) < PICK => Some(i),
                _ => None,
            });
        }
    }
    // Paint: stored guides through the current map (the frozen one mid-drag), then the drag.
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let draw_map = tool.drag.as_ref().map_or(&map, |d| &d.2);
    let line = |a: Pos2, b: Pos2, selected: bool, axis: upright::Axis| {
        let colour = guide_colour(axis);
        painter.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(140)));
        painter.line_segment([a, b], Stroke::new(if selected { 2.0 } else { 1.2 }, colour));
        for p in [a, b] {
            painter.circle(p, 4.0, if selected { colour } else { Color32::from_black_alpha(160) }, Stroke::new(1.2, colour));
        }
    };
    for (i, g) in params.upright_guides.iter().enumerate() {
        let (mut a, mut b) = (frame.guide_screen(draw_map, g[0]), frame.guide_screen(draw_map, g[1]));
        if let Some((Drag::End { guide, end }, cur, _)) = &tool.drag
            && *guide == i
        {
            if *end == 0 {
                a = Some(*cur);
            } else {
                b = Some(*cur);
            }
        }
        if let (Some(a), Some(b)) = (a, b) {
            line(a, b, tool.selected == Some(i), guide_axis(g, w, h));
        }
    }
    if let Some((drag, cur, _)) = &tool.drag {
        if let Drag::New { start } = drag {
            // On screen, near enough to how the uncorrected frame will see it.
            let d = *cur - *start;
            line(*start, *cur, true, if d.y.abs() >= d.x.abs() { upright::Axis::Vertical } else { upright::Axis::Horizontal });
        }
        if let Some(tex) = texture {
            loupe(ui, tex, rect, *cur);
        }
    }
    changed
}

/// Which way Upright straightens guide `g` (fractions of the uncorrected `w`×`h` frame), as
/// [`upright::homography`] decides it.
fn guide_axis(g: &[[f32; 2]; 2], w: usize, h: usize) -> upright::Axis {
    let s = g.map(|p| upright::to_normalised([f64::from(p[0]) * w as f64, f64::from(p[1]) * h as f64], w, h));
    upright::axis_of(&s)
}

/// Guides made vertical are yellow, guides made level cyan.
fn guide_colour(axis: upright::Axis) -> Color32 {
    match axis {
        upright::Axis::Vertical => Color32::from_rgb(255, 214, 10),
        upright::Axis::Horizontal => Color32::from_rgb(40, 200, 255),
    }
}

/// A 4× magnifier of the preview around `at`, beside the pointer.
fn loupe(ui: &Ui, tex: &egui::TextureHandle, image: Rect, at: Pos2) {
    const SIZE: f32 = 128.0;
    const ZOOM: f32 = 4.0;
    let view = ui.clip_rect();
    let mut min = at + vec2(24.0, -SIZE - 24.0);
    if min.x + SIZE > view.right() {
        min.x = at.x - 24.0 - SIZE;
    }
    if min.y < view.top() {
        min.y = at.y + 24.0;
    }
    let lens = Rect::from_min_size(min, vec2(SIZE, SIZE));
    let half = vec2(SIZE / ZOOM / 2.0 / image.width().max(1.0), SIZE / ZOOM / 2.0 / image.height().max(1.0));
    let c = vec2((at.x - image.left()) / image.width().max(1.0), (at.y - image.top()) / image.height().max(1.0));
    let uv = Rect::from_min_max((c - half).to_pos2(), (c + half).to_pos2());
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, ui.scope_id().with("cr-guide-loupe")));
    painter.rect_filled(lens, 0.0, Color32::from_gray(40));
    painter.image(tex.id(), lens, uv, Color32::WHITE);
    let s = Stroke::new(1.0, Color32::from_rgb(255, 214, 10));
    painter.line_segment([lens.center() - vec2(10.0, 0.0), lens.center() + vec2(10.0, 0.0)], s);
    painter.line_segment([lens.center() - vec2(0.0, 10.0), lens.center() + vec2(0.0, 10.0)], s);
    painter.rect_stroke(lens, 0.0, Stroke::new(1.5, Color32::from_gray(220)), egui::StrokeKind::Outside);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A guide is coloured by the axis Upright gives it in the uncorrected frame: a wide frame's
    /// 45° in fractions is closer to vertical in pixels.
    #[test]
    fn guides_are_coloured_by_the_axis_they_are_straightened_to() {
        let (post, ledge) = ([[0.3f32, 0.1], [0.32, 0.9]], [[0.1f32, 0.5], [0.9, 0.55]]);
        assert_eq!(guide_axis(&post, 600, 400), upright::Axis::Vertical);
        assert_eq!(guide_axis(&ledge, 600, 400), upright::Axis::Horizontal);
        assert_eq!(guide_axis(&[[0.2, 0.2], [0.4, 0.6]], 1200, 400), upright::Axis::Horizontal);
        assert_ne!(guide_colour(upright::Axis::Vertical), guide_colour(upright::Axis::Horizontal));
    }

    #[test]
    fn shift_t_selects_guided_and_a_second_press_puts_the_tool_down() {
        let (mut tool, mut p) = (GuideTool::default(), CameraRaw::default());
        assert!(toggle(&mut tool, &mut p), "switching to Guided is an edit");
        assert!(tool.active && p.upright == "guided");
        assert!(!toggle(&mut tool, &mut p));
        assert!(!tool.active && p.upright == "guided", "the mode stays");
        assert!(!toggle(&mut tool, &mut p), "already Guided: only the tool");
        assert!(tool.active);
    }

    #[test]
    fn delete_removes_the_selected_guide() {
        let mut tool = GuideTool { selected: Some(0), ..Default::default() };
        let mut p = CameraRaw { upright: "guided".into(), upright_guides: vec![[[0.1, 0.1], [0.1, 0.9]], [[0.9, 0.1], [0.9, 0.9]]], ..Default::default() };
        assert!(delete_selected(&mut tool, &mut p));
        assert_eq!(p.upright_guides, vec![[[0.9, 0.1], [0.9, 0.9]]]);
        assert!(!delete_selected(&mut tool, &mut p), "nothing selected now");
    }

    #[test]
    fn guides_map_to_the_screen_and_back() {
        let p = CameraRaw { upright: "guided".into(), upright_guides: vec![[[0.22, 0.1], [0.18, 0.9]], [[0.78, 0.1], [0.82, 0.9]]], ..Default::default() };
        let map = map_for(&p, 600, 400).unwrap();
        let frame = Frame { rect: Rect::from_min_size(pos2(10.0, 20.0), vec2(300.0, 200.0)), w: 600.0, h: 400.0 };
        let [a, b] = p.upright_guides[0].map(|g| frame.guide_screen(&map, g).unwrap());
        assert!((a.x - b.x).abs() < 0.5, "the guide shows plumb: {a:?} {b:?}");
        let back = frame.guide_point(&map, a).unwrap();
        assert!((back[0] - 0.22).abs() < 1e-3 && (back[1] - 0.1).abs() < 1e-3, "{back:?}");
    }
}
