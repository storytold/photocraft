//! Shared point-curve semantics for Camera Raw, adjustment dialogs and layer Properties.
//! Hosts own typed gesture state and rendering; this module only edits their point data.
use crate::state::{PointCurveDrag, PointCurveState};
use egui::{Key, Modifiers, Pos2, Rect, Response, Ui, pos2};

#[derive(Clone, Copy, Debug, Default)]
pub struct Interaction {
    pub changed: bool,
    pub commit: bool,
}

const HIT_RADIUS: f32 = 9.0;
/// Interior points disappear outside this margin and return on re-entry during the same drag.
const DRAG_OFF: f32 = 12.0;

/// Most points a curve can hold (Photoshop's limit).
pub const MAX_POINTS: usize = 16;
/// Points closer than this (input levels) are the same point.
pub const MIN_GAP: f32 = 2.0;

/// Index of the point nearest `pos` (screen) within `radius` pixels.
pub fn hit(pts: &[[f32; 2]], to_scr: impl Fn([f32; 2]) -> Pos2, pos: Pos2, radius: f32) -> Option<usize> {
    pts.iter().enumerate().map(|(i, q)| (i, to_scr(*q).distance(pos))).filter(|(_, d)| *d <= radius).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
}

/// Adds a point at `v` (kept sorted). A point already at that input is selected instead; a
/// full curve takes no more points.
pub fn insert(pts: &mut Vec<[f32; 2]>, v: [f32; 2]) -> Option<usize> {
    if !v.iter().all(|x| x.is_finite()) {
        return None;
    }
    let v = [v[0].clamp(0.0, 255.0), v[1].clamp(0.0, 255.0)];
    if let Some(i) = pts.iter().position(|q| (q[0] - v[0]).abs() < MIN_GAP) {
        return Some(i);
    }
    if pts.len() >= MAX_POINTS {
        return None;
    }
    let i = pts.iter().position(|q| q[0] > v[0]).unwrap_or(pts.len());
    pts.insert(i, v);
    Some(i)
}

/// Endpoints stay, and a curve keeps at least two points.
pub fn can_delete(pts: &[[f32; 2]], i: usize) -> bool {
    pts.len() > 2 && i > 0 && i < pts.len().saturating_sub(1)
}

pub fn delete(pts: &mut Vec<[f32; 2]>, i: usize) -> bool {
    if !can_delete(pts, i) {
        return false;
    }
    pts.remove(i);
    true
}

/// Moves point `i` to `v`, kept strictly between its neighbours' inputs.
pub fn move_to(pts: &mut [[f32; 2]], i: usize, v: [f32; 2]) -> bool {
    let Some(old) = pts.get(i).copied() else { return false };
    if !v.iter().chain(old.iter()).all(|x| x.is_finite()) {
        return false;
    }
    let lo = i.checked_sub(1).and_then(|j| pts.get(j)).map_or(0.0, |p| p[0] + 1.0);
    let hi = i.checked_add(1).and_then(|j| pts.get(j)).map_or(255.0, |p| p[0] - 1.0);
    if !lo.is_finite() || !hi.is_finite() {
        return false;
    }
    let new = [v[0].round().clamp(lo, hi.max(lo)), v[1].round().clamp(0.0, 255.0)];
    let changed = new != old;
    if let Some(point) = pts.get_mut(i) {
        *point = new;
    }
    changed
}

/// Capture a point at button-down, preserve its grab offset, and keep it through release.
/// `commit` ends a gesture; hosts decide how that becomes document history.
pub fn interact(ui: &mut Ui, resp: &Response, graph: Rect, pts: &mut Vec<[f32; 2]>, state: &mut PointCurveState) -> Interaction {
    if !graph.is_finite() || graph.width() <= 0.0 || graph.height() <= 0.0 {
        *state = PointCurveState::default();
        return Interaction::default();
    }
    state.selected = state.selected.filter(|i| *i < pts.len());
    if state.drag.is_some_and(|drag| drag.index.is_some_and(|i| i >= pts.len())) {
        state.drag = None;
    }
    let to_scr = |q: [f32; 2]| pos2(graph.left() + q[0] / 255.0 * graph.width(), graph.bottom() - q[1] / 255.0 * graph.height());
    let to_val =
        |s: Pos2| [((s.x - graph.left()) / graph.width() * 255.0).clamp(0.0, 255.0), ((graph.bottom() - s.y) / graph.height() * 255.0).clamp(0.0, 255.0)];
    let mut e = Interaction::default();
    if resp.secondary_clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        if let Some(i) = hit(pts, to_scr, pos, HIT_RADIUS)
            && delete(pts, i)
        {
            e.changed = true;
            e.commit = true;
        }
        state.selected = None;
        state.drag = None;
    }
    // Press: hit-test where the button went down (not where a drag is first recognised).
    let press = (resp.is_pointer_button_down_on() || resp.clicked_by(egui::PointerButton::Primary)) && ui.input(|i| i.pointer.primary_pressed());
    // A press and release may arrive in the same native frame; egui then clears press_origin.
    // A recognised click still owns its interaction position, so it can select without a drag.
    let press_pos =
        ui.input(|i| i.pointer.press_origin()).or_else(|| if resp.clicked_by(egui::PointerButton::Primary) { resp.interact_pointer_pos() } else { None });
    if press && let Some(pos) = press_pos {
        resp.request_focus();
        let delete_pressed = ui.input(|i| i.modifiers.command);
        match hit(pts, to_scr, pos, HIT_RADIUS) {
            Some(i) if delete_pressed => {
                if delete(pts, i) {
                    e.changed = true;
                    e.commit = true;
                }
                state.selected = None;
                state.drag = None;
            }
            Some(i) => {
                state.selected = Some(i);
                let g = to_val(pos);
                state.drag = pts.get(i).map(|point| PointCurveDrag { index: Some(i), offset: [point[0] - g[0], point[1] - g[1]], removed: false });
            }
            None if graph.contains(pos) && !delete_pressed => {
                let before = pts.len();
                state.selected = insert(pts, to_val(pos));
                e.changed = pts.len() != before;
                state.drag = state.selected.map(|i| PointCurveDrag { index: Some(i), offset: [0.0, 0.0], removed: false });
            }
            None => state.drag = None,
        }
    }
    // Drag: follow the pointer; off the graph the point is removed (back on, it returns).
    if (resp.is_pointer_button_down_on() || ui.input(|i| i.pointer.primary_released()))
        && !press
        && let (Some(PointCurveDrag { index, offset: off, removed }), Some(pos)) = (state.drag, ui.input(|i| i.pointer.interact_pos()))
    {
        let outside = !graph.expand(DRAG_OFF).contains(pos);
        let g = to_val(pos);
        let target = [g[0] + off[0], g[1] + off[1]];
        match index {
            Some(i) if outside && can_delete(pts, i) => {
                delete(pts, i);
                state.drag = Some(PointCurveDrag { index: None, offset: off, removed: true });
                state.selected = None;
                e.changed = true;
            }
            Some(i) => e.changed |= move_to(pts, i, target),
            None if removed && !outside => {
                state.selected = insert(pts, target);
                state.drag = Some(PointCurveDrag { index: state.selected, offset: off, removed: true });
                e.changed |= state.selected.is_some();
            }
            None => {}
        }
    }
    if state.drag.is_some() && !resp.is_pointer_button_down_on() {
        state.drag = None;
        e.commit = true;
    }
    // Arrows edit this graph rather than navigating to another widget on the next frame.
    ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() }));
    // Keys while the graph has focus: Delete/Backspace removes, arrows nudge (Shift: ×10).
    if resp.has_focus()
        && let Some(i) = state.selected.filter(|i| *i < pts.len())
    {
        let del = ui.input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::Delete) || inp.consume_key(Modifiers::NONE, Key::Backspace));
        if del && delete(pts, i) {
            state.selected = None;
            e.changed = true;
            e.commit = true;
        } else {
            let step = if ui.input(|inp| inp.modifiers.shift) { 10.0 } else { 1.0 };
            let mut d = [0.0f32; 2];
            for (k, dx, dy) in [(Key::ArrowLeft, -1.0, 0.0), (Key::ArrowRight, 1.0, 0.0), (Key::ArrowUp, 0.0, 1.0), (Key::ArrowDown, 0.0, -1.0)] {
                if ui.input_mut(|inp| inp.consume_key(Modifiers::NONE, k) || inp.consume_key(Modifiers::SHIFT, k)) {
                    d = [d[0] + dx * step, d[1] + dy * step];
                }
            }
            let Some(point) = pts.get(i) else { return e };
            let to = [point[0] + d[0], point[1] + d[1]];
            if d != [0.0, 0.0] && move_to(pts, i, to) {
                e.changed = true;
                e.commit = true;
            }
        }
    }

    e
}

#[cfg(test)]
mod tests {
    use super::*;
    fn line() -> Vec<[f32; 2]> {
        vec![[0.0, 0.0], [255.0, 255.0]]
    }

    #[test]
    fn insert_keeps_order_dedupes_and_respects_the_limit() {
        let mut p = line();
        assert_eq!(insert(&mut p, [128.0, 150.0]), Some(1));
        assert_eq!(insert(&mut p, [64.0, 40.0]), Some(1));
        assert_eq!(p, vec![[0.0, 0.0], [64.0, 40.0], [128.0, 150.0], [255.0, 255.0]]);
        // A click on an existing input selects that point instead of stacking a second one.
        assert_eq!(insert(&mut p, [129.0, 10.0]), Some(2));
        assert_eq!(p.len(), 4);
        let mut full: Vec<[f32; 2]> = (0..MAX_POINTS).map(|i| [i as f32 * 10.0, 0.0]).collect();
        assert_eq!(insert(&mut full, [255.0, 0.0]), None);
        assert_eq!(insert(&mut p, [-50.0, 900.0]), Some(0), "clamped onto the black point");
    }

    #[test]
    fn endpoints_and_the_last_two_points_stay() {
        let mut p = vec![[0.0, 0.0], [100.0, 120.0], [255.0, 255.0]];
        assert!(!delete(&mut p, 0) && !delete(&mut p, 2) && !delete(&mut p, 7));
        assert!(delete(&mut p, 1));
        assert_eq!(p, line());
        assert!(!delete(&mut p, 1) && !delete(&mut p, 0));
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn moves_stay_between_neighbours() {
        let mut p = vec![[0.0, 0.0], [100.0, 120.0], [200.0, 180.0], [255.0, 255.0]];
        assert!(move_to(&mut p, 1, [250.0, 300.0]));
        assert_eq!(p[1], [199.0, 255.0]);
        move_to(&mut p, 0, [150.0, 10.0]);
        assert_eq!(p[0], [150.0, 10.0]);
        move_to(&mut p, 3, [10.0, 5.0]);
        assert_eq!(p[3], [201.0, 5.0]);
        assert!(!move_to(&mut p, 9, [0.0, 0.0]));
    }

    #[test]
    fn hit_test_picks_the_nearest_point_in_radius() {
        let p = vec![[0.0, 0.0], [100.0, 100.0], [110.0, 110.0], [255.0, 255.0]];
        let to_scr = |q: [f32; 2]| Pos2::new(q[0], 255.0 - q[1]);
        assert_eq!(hit(&p, to_scr, Pos2::new(108.0, 146.0), 9.0), Some(2));
        assert_eq!(hit(&p, to_scr, Pos2::new(50.0, 50.0), 9.0), None);
    }

    #[test]
    fn hostile_points_and_indices_cannot_panic_or_pollute_a_curve() {
        let mut p = line();
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(insert(&mut p, [bad, 128.0]), None);
            assert!(!move_to(&mut p, 0, [128.0, bad]));
        }
        assert!(!can_delete(&p, usize::MAX));
        assert!(!delete(&mut p, usize::MAX));
        assert!(!move_to(&mut p, usize::MAX, [128.0, 128.0]));
        assert_eq!(p, line());
    }
}
