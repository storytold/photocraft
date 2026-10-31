//! Rectangle shell picking, evaluated once on release, independent of texture resolution.
use crate::{PhotocraftApp, theme::Tokens};
use egui::{Pos2, Rect};

#[derive(Clone, Debug)]
pub(crate) struct Drag {
    start: Pos2,
    modifiers: egui::Modifiers,
    lasso: bool,
    points: Vec<Pos2>,
}
#[derive(Clone, Debug)]
pub(crate) enum SelectionArea {
    Rect(Rect),
    Lasso(Vec<Pos2>),
}

const LASSO_MIN_BOUNDS_DIAGONAL: f32 = 4.0;
const LASSO_MIN_AREA: f32 = 4.0;
const LASSO_CLOSE_DISTANCE: f32 = 12.0;
const LASSO_MAX_POINTS: usize = 4096;

fn lasso_bounds(points: &[Pos2]) -> Option<Rect> {
    if points.len() < 3 {
        return None;
    }
    let bounds = Rect::from_points(points);
    if bounds.size().length() <= LASSO_MIN_BOUNDS_DIAGONAL {
        return None;
    }
    let origin = points[0];
    let twice_area = points
        .iter()
        .map(|p| *p - origin)
        .zip(points.iter().cycle().skip(1).take(points.len()).map(|p| *p - origin))
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f32>()
        .abs();
    (twice_area * 0.5 > LASSO_MIN_AREA).then_some(bounds)
}

fn finish_lasso(points: &mut Vec<Pos2>, release: Pos2) -> Option<Rect> {
    let start = *points.first()?;
    if points.last().is_none_or(|last| last.distance_sq(release) > f32::EPSILON) {
        if points.len() < LASSO_MAX_POINTS {
            points.push(release);
        } else if let Some(last) = points.last_mut() {
            *last = release;
        }
    }
    if points.len() >= 3 && points.last().is_some_and(|last| last.distance(start) <= LASSO_CLOSE_DISTANCE) {
        if let Some(last) = points.last_mut() {
            *last = start;
        }
    }
    lasso_bounds(points)
}

pub(crate) fn input(ui: &egui::Ui, response: &egui::Response, drag: &mut Option<Drag>, lasso: bool) -> Option<(SelectionArea, egui::Modifiers)> {
    if ui.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused) {
        *drag = None;
        return None;
    }
    if response.contains_pointer()
        && ui.input(|i| i.pointer.primary_pressed() && !i.modifiers.alt)
        && let Some(start) = ui.input(|i| i.pointer.interact_pos())
    {
        *drag = Some(Drag { start, modifiers: ui.input(|i| i.modifiers), lasso, points: vec![start] });
    }
    if let (Some(drag), Some(pointer)) = (drag.as_mut(), ui.input(|i| i.pointer.interact_pos()))
        && drag.lasso
        && drag.points.last().is_none_or(|last| last.distance_sq(pointer) >= 9.0)
        && drag.points.len() < LASSO_MAX_POINTS
    {
        drag.points.push(pointer);
    }
    let d = drag.as_ref()?;
    let end = ui.input(|i| i.pointer.interact_pos()).unwrap_or(d.start);
    let rect = Rect::from_two_pos(d.start, end);
    if d.lasso && d.points.len() >= 2 {
        let mut points = d.points.clone();
        points.push(d.start);
        let t = Tokens::get(ui.ctx());
        ui.painter_at(response.rect).add(egui::Shape::line(points, egui::Stroke::new(1.0, t.accent)));
    } else if rect.size().length() > 4.0 {
        let t = Tokens::get(ui.ctx());
        ui.painter_at(response.rect).rect(rect, 0, t.accent.gamma_multiply(0.12), egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }
    if ui.input(|i| i.pointer.primary_released()) {
        let d = drag.take()?;
        if d.lasso {
            let release = ui.input(|i| i.pointer.interact_pos()).unwrap_or(d.start);
            let mut points = d.points;
            finish_lasso(&mut points, release)?;
            return Some((SelectionArea::Lasso(points), d.modifiers));
        }
        return (rect.size().length() > 4.0).then_some((SelectionArea::Rect(rect), d.modifiers));
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        *drag = None;
    }
    None
}

fn triangle_touches(rect: Rect, tri: [Pos2; 3]) -> bool {
    if tri.iter().any(|p| rect.contains(*p)) {
        return true;
    }
    if !Rect::from_points(&tri).intersects(rect) {
        return false;
    }
    // Clip against the rectangle: includes edges crossing it and a rectangle inside a triangle.
    let mut poly = tri.to_vec();
    for (axis, edge, sign) in [(0, rect.left(), -1.0), (0, rect.right(), 1.0), (1, rect.top(), -1.0), (1, rect.bottom(), 1.0)] {
        let mut out = Vec::new();
        for (a, b) in poly.iter().zip(poly.iter().cycle().skip(1)).take(poly.len()) {
            let value = |p: Pos2| ((if axis == 0 { p.x } else { p.y }) - edge) * sign;
            let (va, vb) = (value(*a), value(*b));
            if va <= 0.0 {
                out.push(*a);
            }
            if (va <= 0.0) != (vb <= 0.0) {
                out.push(a.lerp(*b, va / (va - vb)));
            }
        }
        poly = out;
    }
    !poly.is_empty()
}
pub(crate) fn select(
    app: &mut PhotocraftApp,
    state: &mut crate::uv_ui::State,
    rect: Rect,
    modifiers: egui::Modifiers,
    material: usize,
    visible_only: bool,
    project: impl Fn(usize) -> Option<[Pos2; 3]>,
    visible: impl Fn(usize, [Pos2; 3], Rect) -> bool,
) {
    let Some(topology) = app.session.uv.topology.clone() else { return };
    let shells: Vec<_> = topology
        .shells
        .iter()
        .enumerate()
        .filter(|(_, sh)| sh.material == material && !app.session.uv.hidden_materials.contains(&sh.material))
        .filter_map(|(id, sh)| {
            let projected: Vec<_> = sh
                .triangles
                .iter()
                .filter_map(|triangle_id| project(*triangle_id).map(|tri| (*triangle_id, tri)))
                .filter(|(triangle_id, tri)| !visible_only || visible(*triangle_id, *tri, rect))
                .collect();
            let selected = !projected.is_empty()
                && if state.shell_enclosed {
                    projected.iter().all(|(_, tri)| tri.iter().all(|p| rect.contains(*p)))
                } else {
                    projected.iter().any(|(_, tri)| triangle_touches(rect, *tri))
                };
            selected.then_some(id)
        })
        .collect();
    select_ids(app, state, material, &shells, modifiers);
}

pub(crate) fn select_ids(app: &mut PhotocraftApp, state: &mut crate::uv_ui::State, material: usize, shells: &[usize], modifiers: egui::Modifiers) {
    let mode = if modifiers.ctrl {
        "subtract"
    } else if modifiers.shift {
        "add"
    } else {
        "replace"
    };
    match app.run("uv.shell.select", serde_json::json!({"material":material,"shells":shells,"mode":mode})) {
        Ok(_) => {
            if let Err(e) = app.run("uv.mask.apply", serde_json::json!({"guard":state.guard,"horizontal":state.horizontal,"vertical":state.vertical,"feather":state.feather,"openBorders":state.open_borders,"padding":state.padding})) {
                state.status = e;
            }
        }
        Err(e) => state.status = e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lasso_bounds_include_the_pointer_release_point_even_inside_sampling_spacing() {
        let mut points = vec![egui::pos2(0.0, 0.0), egui::pos2(20.0, 0.0), egui::pos2(20.0, 20.0)];
        let release = egui::pos2(22.0, 20.0);
        let bounds = finish_lasso(&mut points, release).unwrap();
        assert_eq!(points.last(), Some(&release));
        assert_eq!(bounds.max.x, release.x);
    }

    #[test]
    fn lasso_closes_near_its_start_and_rejects_tiny_contours() {
        let start = egui::pos2(0.0, 0.0);
        let mut closed = vec![start, egui::pos2(20.0, 0.0), egui::pos2(20.0, 20.0), egui::pos2(1.0, 1.0)];
        assert!(finish_lasso(&mut closed, egui::pos2(1.0, 1.0)).is_some());
        assert_eq!(closed.last(), Some(&start));

        let mut tiny = vec![start, egui::pos2(2.0, 0.0), egui::pos2(2.0, 1.0)];
        assert!(finish_lasso(&mut tiny, egui::pos2(0.0, 0.0)).is_none());
    }

    #[test]
    fn rectangle_selects_touched_or_enclosed_shells_and_supports_subtraction() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
        app.run("uv.model.import", serde_json::json!({"path":path})).unwrap();
        let topology = app.session.uv.topology.clone().unwrap();
        let material = app.session.uv.material;
        let project = |id| {
            if topology.triangle_shell[id] == 0 {
                Some([egui::pos2(-2.0, 2.0), egui::pos2(2.0, 2.0), egui::pos2(2.0, 8.0)])
            } else {
                Some([egui::pos2(4.0, 4.0), egui::pos2(6.0, 4.0), egui::pos2(6.0, 8.0)])
            }
        };
        let rect = Rect::from_min_max(Pos2::ZERO, egui::pos2(10.0, 10.0));
        let mut state = crate::uv_ui::State::default();
        select(&mut app, &mut state, rect, egui::Modifiers::NONE, material, false, project, |_, _, _| true);
        assert_eq!(app.session.uv.selected.len(), 2);
        state.shell_enclosed = true;
        select(&mut app, &mut state, rect, egui::Modifiers::NONE, material, false, project, |_, _, _| true);
        assert_eq!(app.session.uv.selected, std::collections::BTreeSet::from([1]));
        select(&mut app, &mut state, rect, egui::Modifiers { ctrl: true, ..Default::default() }, material, false, project, |_, _, _| true);
        assert!(app.session.uv.selected.is_empty());
    }

    #[test]
    fn visible_shell_selection_filters_occluded_projected_faces_but_through_selects_them() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
        app.run("uv.model.import", serde_json::json!({"path":path})).unwrap();
        let topology = app.session.uv.topology.clone().unwrap();
        let material = app.session.uv.material;
        let project = |_id| Some([egui::pos2(1.0, 1.0), egui::pos2(8.0, 1.0), egui::pos2(1.0, 8.0)]);
        let rect = Rect::from_min_max(Pos2::ZERO, egui::pos2(10.0, 10.0));
        let mut state = crate::uv_ui::State::default();
        select(&mut app, &mut state, rect, egui::Modifiers::NONE, material, true, project, |triangle, _, _| triangle == 0);
        assert_eq!(app.session.uv.selected.len(), 1, "visible-only keeps the projected front shell");
        select(&mut app, &mut state, rect, egui::Modifiers::NONE, material, false, project, |_, _, _| false);
        assert_eq!(
            app.session.uv.selected.len(),
            topology.shells.iter().filter(|shell| shell.material == material).count(),
            "through ignores depth but still selects only this material"
        );
    }
    #[test]
    fn touches_includes_crossing_edges_and_rect_inside_but_rejects_bounding_box_false_positive() {
        let _gpu = photocraft_testkit::gpu_lock();
        let tri = [egui::pos2(0.0, 0.0), egui::pos2(100.0, 0.0), egui::pos2(0.0, 100.0)];
        let r = |a, b, c, d| Rect::from_min_max(egui::pos2(a, b), egui::pos2(c, d));
        assert!(triangle_touches(r(10.0, 10.0, 20.0, 20.0), tri));
        assert!(triangle_touches(r(40.0, -1.0, 60.0, 1.0), tri));
        assert!(!triangle_touches(r(80.0, 80.0, 90.0, 90.0), tri));
    }
}
