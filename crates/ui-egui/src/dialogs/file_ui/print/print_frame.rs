//! Preview-only canvas frame. Persistent geometry belongs exclusively to PrintLayout fields.
use egui::{CursorIcon, Pos2, Rect, Vec2};
use photocraft_engine::print_cmds::{PrintLayout, layout};
use serde_json::{Map, Value, json};

use super::{drag_print_position, params, print_preview_image};

const HANDLE_HALF: f32 = 3.0;
const HIT_RADIUS: f32 = 7.0;

/// Screen-space directions; zero denotes a side midpoint, never a stretch axis.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Move,
    Resize(i8, i8),
}
impl Target {
    fn cursor(self) -> CursorIcon {
        match self {
            Self::Move => CursorIcon::Grab,
            Self::Resize(0, _) => CursorIcon::ResizeVertical,
            Self::Resize(_, 0) => CursorIcon::ResizeHorizontal,
            Self::Resize(x, y) if x == y => CursorIcon::ResizeNwSe,
            Self::Resize(_, _) => CursorIcon::ResizeNeSw,
        }
    }
}

fn handles(rect: Rect) -> [(Target, Pos2); 8] {
    [
        (Target::Resize(-1, -1), rect.left_top()),
        (Target::Resize(1, -1), rect.right_top()),
        (Target::Resize(1, 1), rect.right_bottom()),
        (Target::Resize(-1, 1), rect.left_bottom()),
        (Target::Resize(0, -1), rect.center_top()),
        (Target::Resize(1, 0), rect.right_center()),
        (Target::Resize(0, 1), rect.center_bottom()),
        (Target::Resize(-1, 0), rect.left_center()),
    ]
}

fn hit(rect: Rect, pointer: Pos2) -> Option<Target> {
    // Nearest handle wins even when a small frame's hit areas overlap.
    handles(rect)
        .into_iter()
        .map(|(target, p)| (target, p.distance_sq(pointer)))
        .filter(|(_, d)| *d <= HIT_RADIUS * HIT_RADIUS)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(target, _)| target)
        .or_else(|| rect.contains(pointer).then_some(Target::Move))
}

/// Total displacement from press, with the opposite corner/side midpoint fixed. Projection
/// onto the corner diagonal gives one uniform scale; side drags use their normal axis.
/// A handle cannot cross its anchor and reflect the image. No document data is touched.
fn resize(f: &mut Map<String, Value>, start: PrintLayout, delta: Vec2, k: f32, target: Target) -> bool {
    let Target::Resize(sx, sy) = target else { return false };
    if !k.is_finite() || k <= 0.0 || !delta.is_finite() {
        return false;
    }
    let (x, y, w, h) = start.rect;
    if ![x, y, w, h, start.scale].into_iter().all(f64::is_finite) || w <= 0.0 || h <= 0.0 || start.scale <= 0.0 {
        return false;
    }
    let (dx, dy) = (f64::from(delta.x) / f64::from(k), f64::from(delta.y) / f64::from(k));
    let (vx, vy) = (f64::from(sx) * w, f64::from(sy) * h);
    let factor = 1.0 + (dx * vx + dy * vy) / (vx * vx + vy * vy);
    // Same range as the existing percentage control, without per-frame rounding.
    scale_about(f, start, start.scale * factor, [(1.0 - f64::from(sx)) / 2.0, (1.0 - f64::from(sy)) / 2.0])
}

/// Both wheel and handle resizing write the same print fields. Fractions are measured in
/// the displayed (possibly mirrored) canvas, keeping that point stationary on the paper.
fn scale_about(f: &mut Map<String, Value>, start: PrintLayout, scale: f64, anchor: [f64; 2]) -> bool {
    let (x, y, w, h) = start.rect;
    if ![scale, start.scale, x, y, w, h, anchor[0], anchor[1]].into_iter().all(f64::is_finite) || start.scale <= 0.0 || w <= 0.0 || h <= 0.0 {
        return false;
    }
    let scale = scale.clamp(0.01, 10.0);
    let ratio = scale / start.scale;
    let (nw, nh) = (w * ratio, h * ratio);
    let visual_left = if start.emulsion_down { start.paper.0 - x - w } else { x };
    let left = visual_left + (w - nw) * anchor[0];
    let top = start.paper.1 - y - h + (h - nh) * anchor[1];
    let left = if start.emulsion_down { start.paper.0 - left - nw } else { left };
    let limit = photocraft_engine::print_cmds::MAX_POSITION_INCHES;
    if ![scale, left, top, nw, nh].into_iter().all(f64::is_finite) || left.abs() / 72.0 > limit || top.abs() / 72.0 > limit || nw.max(nh) > 1.0e9 {
        return false;
    }
    f.insert("scale".into(), json!(scale * 100.0));
    f.insert("scaleToFit".into(), json!(false));
    f.insert("center".into(), json!(false));
    f.insert("left".into(), json!(left / 72.0));
    f.insert("top".into(), json!(top / 72.0));
    true
}

/// egui wheel points: one ordinary 40-point line is about a 10% size change. Use actual
/// events once, rather than accumulating egui's smoothed tail into the physical scale.
fn wheel_scale(f: &mut Map<String, Value>, start: PrintLayout, image: Rect, at: Pos2, points: f64) -> bool {
    if !points.is_finite() || points == 0.0 || !at.is_finite() || !image.is_finite() || image.width() <= 0.0 || image.height() <= 0.0 {
        return false;
    }
    let anchor = [f64::from((at.x - image.left()) / image.width()), f64::from((at.y - image.top()) / image.height())];
    let scale = start.scale * (points * (std::f64::consts::LN_2 / 300.0)).clamp(-32.0, 32.0).exp();
    // Even at a limit the intentional wheel event is consumed, without changing placement.
    if scale.clamp(0.01, 10.0) == start.scale {
        return true;
    }
    scale_about(f, start, scale, anchor)
}

fn wheel_points(event: &egui::Event, line_points: f32, page_points: f32) -> Option<f64> {
    if let egui::Event::MouseWheel { unit, delta, modifiers, .. } = event
        && *modifiers == egui::Modifiers::NONE
        && delta.is_finite()
        && delta.y != 0.0
    {
        let unit = match unit {
            egui::MouseWheelUnit::Point => 1.0,
            egui::MouseWheelUnit::Line => f64::from(line_points),
            egui::MouseWheelUnit::Page => f64::from(page_points),
        };
        let points = f64::from(delta.y) * unit;
        return (points.is_finite() && points != 0.0).then_some(points);
    }
    None
}

fn wheel_input(ui: &mut egui::Ui, response: &egui::Response, image: Rect, captured: bool) -> Option<(Pos2, f64)> {
    let tail_id = response.id.with("wheel-tail");
    let active = ui.is_enabled()
        && response.hovered()
        && !captured
        && !ui.input(|i| i.pointer.any_down())
        && response.hover_pos().is_some_and(|p| hit(image, p).is_some());
    let line_points = ui.ctx().options(|o| o.input_options.line_scroll_speed);
    let page_points = ui.input(|i| i.content_rect().height());
    let eligible = |e: &egui::Event| wheel_points(e, line_points, page_points).is_some();
    // Focused fields normally own wheel input globally; only this hovered canvas overrides it.
    let mut pending = if active { crate::widgets::field_wheel::take_focused_events(ui.ctx(), eligible) } else { Vec::new() };
    let fresh = !pending.is_empty() || ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. })));
    if active {
        ui.input_mut(|i| {
            i.events.retain(|event| {
                if eligible(event) {
                    pending.push(event.clone());
                    false
                } else {
                    true
                }
            });
        });
    }
    let taken = !pending.is_empty();
    let tail = if fresh { taken } else { ui.data(|d| d.get_temp::<bool>(tail_id)).unwrap_or(false) };
    if tail {
        // Suppress only the remainder of consumed input; a fresh wheel elsewhere stays intact.
        ui.input_mut(|i| i.smooth_scroll_delta.y = 0.0);
    }
    let scrolling = ui.input(|i| i.is_scrolling());
    ui.data_mut(|d| d.insert_temp(tail_id, tail && scrolling));
    let points = pending.iter().filter_map(|e| wheel_points(e, line_points, page_points)).sum();
    response.hover_pos().filter(|_| taken).map(|at| (at, points))
}

#[derive(Clone)]
struct Drag {
    start: PrintLayout,
    expected: PrintLayout,
    origin: Pos2,
    page: Rect,
    k: f32,
    target: Target,
}

pub(super) fn interact(
    ui: &mut egui::Ui,
    f: &mut Map<String, Value>,
    doc: &photocraft_doc::Document,
    mut l: PrintLayout,
    page: Rect,
    area: Rect,
    id: u64,
) -> PrintLayout {
    let k = page.width() / l.paper.0 as f32;
    let img = print_preview_image(l, page, k);
    let drag_id = ui.scope_id().with(("print-position", id));
    let visible = img.expand(HIT_RADIUS).intersect(area).intersect(ui.clip_rect());
    let response = ui.interact(visible, drag_id, egui::Sense::drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, tl!("Print preview image")));
    if response.hovered()
        && let Some(target) = response.hover_pos().and_then(|p| hit(img, p))
    {
        ui.ctx().set_cursor_icon(target.cursor());
    }
    if response.drag_started_by(egui::PointerButton::Primary)
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        && let Some(target) = hit(img, origin)
    {
        ui.data_mut(|d| d.insert_temp(drag_id, Drag { start: l, expected: l, origin, page, k, target }));
    }
    if let Some(mut drag) = ui.data(|d| d.get_temp::<Drag>(drag_id)) {
        // A paper/numeric edit or preview reflow during capture ends the old gesture.
        if drag.expected != l || drag.page != page || drag.k != k {
            ui.data_mut(|d| d.remove::<Drag>(drag_id));
        } else if (response.dragged_by(egui::PointerButton::Primary) || response.drag_stopped_by(egui::PointerButton::Primary))
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let delta = pointer - drag.origin;
            let mut candidate = f.clone();
            let changed = match drag.target {
                Target::Move => drag_print_position(&mut candidate, drag.start, delta, drag.k),
                target => resize(&mut candidate, drag.start, delta, drag.k, target),
            };
            if changed && let Ok(updated) = layout(doc, &params(&candidate), "file.print") {
                *f = candidate;
                l = updated;
                drag.expected = updated;
                ui.ctx().request_repaint();
                ui.data_mut(|d| d.insert_temp(drag_id, drag.clone()));
            }
            ui.ctx().set_cursor_icon(if drag.target == Target::Move { CursorIcon::Grabbing } else { drag.target.cursor() });
        }
    }
    let captured = ui.data(|d| d.get_temp::<Drag>(drag_id)).is_some();
    if let Some((at, points)) = wheel_input(ui, &response, print_preview_image(l, page, k), captured) {
        let mut candidate = f.clone();
        if wheel_scale(&mut candidate, l, print_preview_image(l, page, k), at, points)
            && let Ok(updated) = layout(doc, &params(&candidate), "file.print")
        {
            *f = candidate;
            l = updated;
            ui.ctx().request_repaint();
        }
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        ui.data_mut(|d| d.remove::<Drag>(drag_id));
    }
    l
}

/// Called only by the egui preview after the artwork/marks. Never part of PrintPage/PDF.
pub(super) fn paint(painter: &egui::Painter, rect: Rect, t: &crate::theme::Tokens) {
    // Contrasting inner/outer strokes remain legible over white margins and dark artwork.
    painter.rect_stroke(rect, 0.0, egui::Stroke::new(3.0, t.accent_text), egui::StrokeKind::Middle);
    painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Middle);
    for (_, p) in handles(rect) {
        let handle = Rect::from_center_size(p, Vec2::splat(HANDLE_HALF * 2.0));
        painter.rect_filled(handle, 0.0, t.accent_text);
        painter.rect_stroke(handle, 0.0, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Middle);
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod paper_fit_tests;
