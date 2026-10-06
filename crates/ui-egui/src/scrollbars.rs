//! Canvas scrollbars (#300), like Photoshop's in Standard Screen Mode: thin bars along the right
//! and bottom edges of the canvas viewport, shown on an axis where the document reaches past the
//! viewport. Dragging a thumb pans the view, a click on the track pages by one viewport, and the
//! wheel over a bar scrolls. They only read and move the per-document `View` (pan), so the Hand
//! tool, Space-drag, the wheel and zoom all stay in sync with them.
//!
//! The scrollable range is the document plus, with Preferences › Tools › Overscroll on, half a
//! viewport past each edge (the document's edge can reach the viewport centre). It always
//! includes the current view, so a view panned further by the Hand tool keeps a valid thumb. With
//! Overscroll off, `clamp_view` keeps the document covering the viewport (centred when smaller).

use egui::{PointerButton, Rect, Sense, Vec2, pos2, vec2};

use crate::state::View;
use crate::theme::Tokens;

/// Bar thickness in points.
pub const THICKNESS: f32 = 10.0;
/// Shortest thumb, so it stays grabbable on a huge range.
const MIN_THUMB: f32 = 20.0;
/// Sub-point overhang (rounding) does not count as overflowing.
const EPS: f32 = 0.5;

/// One axis of the scroll model, in screen points along that axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    /// Scrollable content: the document plus the overscroll margin, grown to include the view.
    pub lo: f32,
    pub hi: f32,
    /// The viewport.
    pub v0: f32,
    pub v1: f32,
}

impl Span {
    /// `img` and `view` are `[start, end]` of the document and the viewport along one screen
    /// axis. `None` when the document fits inside the viewport (no bar) or the sizes are
    /// degenerate (an empty viewport or document, non-finite numbers).
    pub fn new(img: [f32; 2], view: [f32; 2], overscroll: bool) -> Option<Span> {
        let [a, b] = img;
        let [v0, v1] = view;
        if ![a, b, v0, v1].iter().all(|v| v.is_finite()) || v1 - v0 < 1.0 || b <= a {
            return None;
        }
        if a >= v0 - EPS && b <= v1 + EPS {
            return None;
        }
        let margin = if overscroll { (v1 - v0) / 2.0 } else { 0.0 };
        Some(Span { lo: (a - margin).min(v0), hi: (b + margin).max(v1), v0, v1 })
    }

    fn len(&self) -> f32 {
        self.v1 - self.v0
    }

    /// How far the viewport start can travel through the content.
    fn travel(&self) -> f32 {
        (self.hi - self.lo - self.len()).max(0.0)
    }

    /// The thumb's `[start, end]` on a track `[t0, t1]`: its length is the visible fraction.
    pub fn thumb(&self, t0: f32, t1: f32) -> [f32; 2] {
        let track = (t1 - t0).max(0.0);
        let total = (self.hi - self.lo).max(1.0);
        let len = (self.len() / total * track).max(MIN_THUMB).min(track);
        let travel = self.travel();
        let start = if travel > 0.0 { t0 + (self.v0 - self.lo) / travel * (track - len) } else { t0 };
        [start, start + len]
    }

    /// The viewport shift (screen points, positive = towards the end) that moves the thumb's
    /// start to `s` on the track `[t0, t1]`, clamped to the scrollable range.
    pub fn shift_for_thumb(&self, t0: f32, t1: f32, s: f32) -> f32 {
        let [a, b] = self.thumb(t0, t1);
        let free = (t1 - t0) - (b - a);
        if free <= 0.0 || !s.is_finite() {
            return 0.0;
        }
        let f = ((s - t0) / free).clamp(0.0, 1.0);
        self.lo + f * self.travel() - self.v0
    }

    /// The shift that pages one viewport back or forward, clamped to the scrollable range.
    pub fn page(&self, forward: bool) -> f32 {
        let target = if forward { self.v0 + self.len() } else { self.v0 - self.len() };
        target.clamp(self.lo, self.lo + self.travel()) - self.v0
    }
}

/// One laid-out bar: its track, its thumb and the axis model behind them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    pub track: Rect,
    pub thumb: Rect,
    pub span: Span,
}

/// The canvas's bars: `h` along the bottom edge, `v` along the right edge.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bars {
    pub h: Option<Bar>,
    pub v: Option<Bar>,
}

/// Lay out the bars for a canvas viewport `rect` showing the document at `img` (screen rects).
pub fn layout(rect: Rect, img: Rect, overscroll: bool) -> Bars {
    if !(rect.width() >= 3.0 * THICKNESS && rect.height() >= 3.0 * THICKNESS) {
        return Bars::default();
    }
    let hs = Span::new([img.left(), img.right()], [rect.left(), rect.right()], overscroll);
    let vs = Span::new([img.top(), img.bottom()], [rect.top(), rect.bottom()], overscroll);
    // Both bars: they leave the bottom-right corner square to each other.
    let corner = |other: bool| if other { THICKNESS } else { 0.0 };
    let h = hs.map(|span| {
        let track = Rect::from_min_max(pos2(rect.left(), rect.bottom() - THICKNESS), pos2(rect.right() - corner(vs.is_some()), rect.bottom()));
        let [a, b] = span.thumb(track.left(), track.right());
        Bar { track, thumb: Rect::from_x_y_ranges(a..=b, track.y_range()), span }
    });
    let v = vs.map(|span| {
        let track = Rect::from_min_max(pos2(rect.right() - THICKNESS, rect.top()), pos2(rect.right(), rect.bottom() - corner(hs.is_some())));
        let [a, b] = span.thumb(track.top(), track.bottom());
        Bar { track, thumb: Rect::from_x_y_ranges(track.x_range(), a..=b), span }
    });
    Bars { h, v }
}

/// What the bars did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Outcome {
    /// Viewport shift in screen points (positive = right / down).
    pub shift: Vec2,
    /// The pointer is over a bar (or dragging one): wheel input there scrolls the canvas.
    pub hovered: bool,
    /// Per bar (`[h, v]`): hovered, dragged.
    pub hot: [(bool, bool); 2],
}

/// Handle input on the bars. Call after allocating the canvas widget so the bars sit on top of
/// it: they take presses on themselves only, and everything else reaches the canvas.
pub fn interact(ui: &egui::Ui, id: egui::Id, bars: &Bars) -> Outcome {
    let mut out = Outcome::default();
    for (k, bar) in [bars.h, bars.v].into_iter().enumerate() {
        let Some(bar) = bar else { continue };
        let horizontal = k == 0;
        let along = |p: egui::Pos2| if horizontal { p.x } else { p.y };
        let (t0, t1) = if horizontal { (bar.track.left(), bar.track.right()) } else { (bar.track.top(), bar.track.bottom()) };
        let [s0, s1] = if horizontal { [bar.thumb.left(), bar.thumb.right()] } else { [bar.thumb.top(), bar.thumb.bottom()] };
        let bar_id = id.with(k);
        let anchor_id = bar_id.with("anchor");
        let resp = ui.interact(bar.track, bar_id, Sense::click_and_drag());
        let mut shift = 0.0;
        if resp.drag_started_by(PointerButton::Primary) {
            // Grab the thumb where it was pressed; a press on the track centres it there.
            let press = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()).map(along);
            let anchor = match press {
                Some(p) if (s0..=s1).contains(&p) => p - s0,
                _ => (s1 - s0) / 2.0,
            };
            ui.ctx().data_mut(|d| d.insert_temp(anchor_id, anchor));
        }
        if resp.dragged_by(PointerButton::Primary)
            && let Some(p) = resp.interact_pointer_pos()
        {
            let anchor = ui.ctx().data(|d| d.get_temp::<f32>(anchor_id)).unwrap_or((s1 - s0) / 2.0);
            shift = bar.span.shift_for_thumb(t0, t1, along(p) - anchor);
        } else if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos().map(along)
        {
            // A click on the track pages towards it (on the thumb: nothing).
            if p < s0 {
                shift = bar.span.page(false);
            } else if p > s1 {
                shift = bar.span.page(true);
            }
        }
        if resp.drag_stopped() {
            ui.ctx().data_mut(|d| d.remove::<f32>(anchor_id));
        }
        let dragged = resp.dragged_by(PointerButton::Primary);
        out.hot[k] = (resp.hovered(), dragged);
        out.hovered |= resp.hovered() || dragged;
        if shift.is_finite() {
            if horizontal {
                out.shift.x += shift;
            } else {
                out.shift.y += shift;
            }
        }
    }
    if out.shift != Vec2::ZERO {
        ui.ctx().request_repaint();
    }
    out
}

/// Move the view so the viewport shifts by `shift` screen points (a mirrored view pans the
/// document the other way, like the Hand tool).
pub fn pan(view: &mut View, flip: bool, shift: Vec2) {
    if shift == Vec2::ZERO || !(view.zoom.is_finite() && view.zoom > 0.0) {
        return;
    }
    let (dx, dy) = (shift.x / view.zoom * if flip { -1.0 } else { 1.0 }, shift.y / view.zoom);
    if dx.is_finite() && dy.is_finite() {
        view.center[0] += dx;
        view.center[1] += dy;
    }
}

/// Preferences › Tools › Overscroll off: keep the document covering the viewport on each axis
/// where it is larger, and centred where it is smaller.
pub fn clamp_view(view: &mut View, viewport: Vec2, doc_size: [u32; 2]) {
    if !(view.zoom.is_finite() && view.zoom > 0.0) {
        return;
    }
    for (k, (len, size)) in [(viewport.x, doc_size[0]), (viewport.y, doc_size[1])].into_iter().enumerate() {
        let size = size as f32;
        let half = len / 2.0 / view.zoom;
        let Some(c) = view.center.get_mut(k) else { continue };
        if !half.is_finite() || !c.is_finite() {
            continue;
        }
        *c = if size <= 2.0 * half { size / 2.0 } else { c.clamp(half, size - half) };
    }
}

/// Paint the bars (on top of the canvas and its overlays).
pub fn paint(painter: &egui::Painter, t: &Tokens, bars: &Bars, out: &Outcome) {
    if let (Some(h), Some(v)) = (bars.h, bars.v) {
        let corner = Rect::from_min_max(pos2(h.track.right(), v.track.bottom()), pos2(v.track.right(), h.track.bottom()));
        painter.rect_filled(corner, 0.0, t.dock);
    }
    for (k, bar) in [bars.h, bars.v].into_iter().enumerate() {
        let Some(bar) = bar else { continue };
        painter.rect_filled(bar.track, 0.0, t.dock);
        let (hovered, dragged) = out.hot.get(k).copied().unwrap_or_default();
        let fill = if dragged {
            t.text_dim
        } else if hovered {
            t.text_faint
        } else {
            t.field_border
        };
        let inset = if k == 0 { vec2(1.0, 2.0) } else { vec2(2.0, 1.0) };
        painter.rect_filled(bar.thumb.shrink2(inset), t.radius_sm.min(THICKNESS / 2.0 - 2.0), fill);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_inside_the_viewport_has_no_bar() {
        assert_eq!(Span::new([10.0, 90.0], [0.0, 100.0], true), None);
        assert_eq!(Span::new([0.0, 100.0], [0.0, 100.0], true), None);
        assert_eq!(Span::new([-0.3, 100.2], [0.0, 100.0], false), None, "rounding is not overflow");
        assert!(Span::new([-50.0, 150.0], [0.0, 100.0], true).is_some());
        // A small document panned partly out of view overflows too.
        assert!(Span::new([80.0, 120.0], [0.0, 100.0], false).is_some());
    }

    #[test]
    fn overscroll_adds_half_a_viewport_and_the_range_includes_the_view() {
        let s = Span::new([-50.0, 150.0], [0.0, 100.0], true).unwrap();
        assert_eq!((s.lo, s.hi), (-100.0, 200.0));
        let s = Span::new([-50.0, 150.0], [0.0, 100.0], false).unwrap();
        assert_eq!((s.lo, s.hi), (-50.0, 150.0));
        // Panned far past the document: the range grows to the view.
        let s = Span::new([-500.0, -300.0], [0.0, 100.0], false).unwrap();
        assert_eq!((s.lo, s.hi), (-500.0, 100.0));
    }

    #[test]
    fn thumb_is_the_visible_fraction_and_round_trips() {
        let s = Span::new([-100.0, 300.0], [0.0, 100.0], false).unwrap();
        let [a, b] = s.thumb(0.0, 400.0);
        assert!((b - a - 100.0).abs() < 1e-3, "a quarter of the range is visible: {a}..{b}");
        assert!((a - 100.0).abs() < 1e-3);
        assert!(s.shift_for_thumb(0.0, 400.0, a).abs() < 1e-3, "the thumb where it is: no shift");
        assert!((s.shift_for_thumb(0.0, 400.0, a + 40.0) - 40.0).abs() < 1e-3);
        // Past the ends: clamped to the range.
        assert!((s.shift_for_thumb(0.0, 400.0, -1e6) + 100.0).abs() < 1e-3);
        assert!((s.shift_for_thumb(0.0, 400.0, 1e6) - 200.0).abs() < 1e-3);
        assert_eq!(s.shift_for_thumb(0.0, 400.0, f32::NAN), 0.0);
        assert!((s.page(true) - 100.0).abs() < 1e-3);
        assert!((s.page(false) + 100.0).abs() < 1e-3);
    }

    #[test]
    fn degenerate_sizes_do_not_panic_or_produce_nan() {
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        for img in [
            Rect::NOTHING,
            Rect::EVERYTHING,
            Rect::from_min_size(pos2(0.0, 0.0), vec2(0.0, 0.0)),
            Rect::from_min_size(pos2(-1e30, -1e30), vec2(2e30, 2e30)),
            Rect::from_min_size(pos2(f32::NAN, 0.0), vec2(10.0, 10.0)),
            Rect::from_min_size(pos2(400.0, 300.0), vec2(1e-6, 1e-6)),
        ] {
            for rect in [r, Rect::NOTHING, Rect::from_min_size(pos2(5.0, 5.0), vec2(0.0, 0.0)), Rect::from_min_size(pos2(0.0, 0.0), vec2(25.0, 1e9))] {
                for overscroll in [false, true] {
                    let bars = layout(rect, img, overscroll);
                    for bar in [bars.h, bars.v].into_iter().flatten() {
                        assert!(bar.thumb.min.is_finite() && bar.thumb.max.is_finite(), "{rect:?} {img:?}: {bar:?}");
                        assert!(bar.track.contains_rect(bar.thumb.shrink(0.01)) || bar.thumb.width() <= 0.0, "{bar:?}");
                        for s in [f32::NEG_INFINITY, 0.0, 1e9] {
                            assert!(bar.span.shift_for_thumb(0.0, 100.0, s).is_finite());
                        }
                    }
                }
            }
        }
        let mut view = View { zoom: 0.0, ..View::default() };
        pan(&mut view, false, vec2(10.0, 10.0));
        clamp_view(&mut view, vec2(100.0, 100.0), [0, 0]);
        assert!(view.center.iter().all(|c| c.is_finite()));
        let mut view = View { zoom: 64.0, center: [0.5, 0.5], ..View::default() };
        clamp_view(&mut view, vec2(0.0, 0.0), [1, 1]);
        assert_eq!(view.center, [0.5, 0.5]);
    }

    #[test]
    fn clamp_view_keeps_the_document_over_the_viewport() {
        let mut view = View { zoom: 2.0, center: [-500.0, 10.0], ..View::default() };
        // 1000×200 document at 200% in a 400×600 viewport: x covers it, y is centred.
        clamp_view(&mut view, vec2(400.0, 600.0), [1000, 200]);
        assert_eq!(view.center, [100.0, 100.0]);
        view.center = [5000.0, 0.0];
        clamp_view(&mut view, vec2(400.0, 600.0), [1000, 200]);
        assert_eq!(view.center[0], 900.0);
    }
}
