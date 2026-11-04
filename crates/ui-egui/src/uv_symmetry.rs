//! 3D-workbench brush symmetry preferences and vertex-plane snapping.
use egui::{Context, Ui};
use photocraft_uv::{
    Scene,
    prepared::{Geometry, Hit},
    preview::Camera,
};

const MAX_PROJECTED_COORD: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Axis {
    #[default]
    X,
    Y,
    Z,
}

impl Axis {
    pub(crate) fn index(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
            Self::Z => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Settings {
    pub enabled: bool,
    pub axis: Axis,
    /// Plane offset in normalized prepared-geometry coordinates; zero is model center.
    pub offset: f64,
    pub snap_pending: bool,
}

fn id(_ctx: &Context) -> egui::Id {
    egui::Id::unique("uv-3d-symmetry-settings")
}

pub(crate) fn get(ctx: &Context) -> Settings {
    let id = id(ctx);
    ctx.data(|data| data.get_temp(id)).unwrap_or_default()
}

pub(crate) fn put(ctx: &Context, settings: Settings) {
    let id = id(ctx);
    ctx.data_mut(|data| data.insert_temp(id, settings));
}

/// A symmetry snap is a one-shot 3D surface action. Cancel it when the view switches to a mask
/// image, where brush input targets the mask rather than a model vertex.
pub(crate) fn cancel_pending_snap(ctx: &Context) {
    let mut settings = get(ctx);
    if settings.snap_pending {
        settings.snap_pending = false;
        put(ctx, settings);
        ctx.request_repaint();
    }
}

/// Prepared-space corners for the depth-tested GPU symmetry-plane pass.
pub(crate) fn plane_corners(geometry: &Geometry, settings: Settings) -> Option<[[f32; 3]; 4]> {
    plane_corners_in_bounds(geometry.bounds, settings)
}

fn plane_corners_in_bounds(bounds: [[f64; 2]; 3], settings: Settings) -> Option<[[f32; 3]; 4]> {
    if !settings.enabled || !settings.offset.is_finite() {
        return None;
    }
    let axis = settings.axis.index();
    if settings.offset < bounds[axis][0] || settings.offset > bounds[axis][1] {
        return None;
    }
    let (u_axis, v_axis) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let corners = std::array::from_fn(|index| {
        let mut point = [0.0; 3];
        point[axis] = settings.offset;
        point[u_axis] = bounds[u_axis][usize::from(index & 1 != 0)];
        point[v_axis] = bounds[v_axis][usize::from(index & 2 != 0)];
        point.map(|value| value as f32)
    });
    corners.iter().flatten().all(|value| value.is_finite()).then_some(corners)
}

pub(crate) fn menu(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let mut settings = get(&ctx);
    ui.checkbox(&mut settings.enabled, tl!("Enable 3D symmetry"));
    ui.horizontal(|ui| {
        ui.label(tl!("Mirror axis"));
        ui.selectable_value(&mut settings.axis, Axis::X, "X");
        ui.selectable_value(&mut settings.axis, Axis::Y, "Y");
        ui.selectable_value(&mut settings.axis, Axis::Z, "Z");
    });
    ui.add(egui::DragValue::new(&mut settings.offset).speed(0.005).range(-2.0..=2.0).prefix(tl!("Offset ")));
    if ui.button(if settings.snap_pending { tl!("Click a model vertex…") } else { tl!("Snap plane to vertex") }).clicked() {
        settings.enabled = true;
        settings.snap_pending = !settings.snap_pending;
    }
    if settings.snap_pending {
        ui.weak(tl!("Click a visible model vertex to place the mirror plane."));
    }
    put(&ctx, settings);
}

pub(crate) fn paint_plane(ui: &Ui, geometry: &Geometry, camera: Camera, size: [u32; 2], rect: egui::Rect) {
    let settings = get(ui.ctx());
    let rect_size = rect.size();
    if !settings.enabled
        || !settings.offset.is_finite()
        || !rect.min.x.is_finite()
        || !rect.min.y.is_finite()
        || !rect_size.x.is_finite()
        || !rect_size.y.is_finite()
        || rect_size.x <= 0.0
        || rect_size.y <= 0.0
    {
        return;
    }
    let Some(plane_corners) = plane_corners(geometry, settings) else { return };
    let corners: [Option<[f64; 2]>; 4] = std::array::from_fn(|index| geometry.project_normalized(camera, plane_corners[index].map(f64::from), size));
    let painter = ui.painter_at(rect);
    let color = egui::Color32::from_rgba_unmultiplied(70, 165, 255, 180);
    for [a, b] in [[0, 1], [1, 3], [3, 2], [2, 0]] {
        if let (Some(a), Some(b)) = (corners[a], corners[b])
            && let Some([a, b]) = clip_projected_segment(a, b)
        {
            let to_screen = |point: [f64; 2]| rect.min + egui::vec2((point[0] as f32) * rect_size.x, (point[1] as f32) * rect_size.y);
            painter.line_segment([to_screen(a), to_screen(b)], egui::Stroke::new(1.0, color));
        }
    }
}

/// Clip normalized projection coordinates to the view before converting to egui's f32 points.
/// Very large finite values can still overflow f32 (notably near the perspective near plane), so
/// reject them before doing Liang–Barsky arithmetic.
fn clip_projected_segment(a: [f64; 2], b: [f64; 2]) -> Option<[[f64; 2]; 2]> {
    if a.iter().chain(&b).any(|value| !value.is_finite() || value.abs() > MAX_PROJECTED_COORD) {
        return None;
    }
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let (mut t_min, mut t_max): (f64, f64) = (0.0, 1.0);
    for (p, q) in [(-dx, a[0]), (dx, 1.0 - a[0]), (-dy, a[1]), (dy, 1.0 - a[1])] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let t = q / p;
        if p < 0.0 {
            t_min = t_min.max(t);
        } else {
            t_max = t_max.min(t);
        }
        if t_min > t_max {
            return None;
        }
    }
    Some([[a[0] + t_min * dx, a[1] + t_min * dy], [a[0] + t_max * dx, a[1] + t_max * dy]])
}

/// Consume the click armed by the popup and align the plane to the nearest projected
/// vertex of the picked triangle. Returns true while consuming this gesture.
pub(crate) fn snap_hit(
    ctx: &Context,
    scene: &Scene,
    geometry: &Geometry,
    camera: Camera,
    size: [u32; 2],
    hit: Hit,
    pointer_uv: [f64; 2],
    viewport_px: [f64; 2],
) -> bool {
    let mut settings = get(ctx);
    if !settings.snap_pending {
        return false;
    }
    let axis = settings.axis.index();
    let Some(triangle) = scene.triangles.get(hit.triangle) else { return true };
    let Some((vertex, distance)) = triangle
        .corners
        .iter()
        .filter_map(|corner| {
            let position = *scene.positions.get(corner.vertex)?;
            let normalized = std::array::from_fn(|i| (position[i] - geometry.center[i]) / geometry.diameter);
            let screen = geometry.project_normalized(camera, normalized, size)?;
            let dx = (screen[0] - pointer_uv[0]) * viewport_px[0];
            let dy = (screen[1] - pointer_uv[1]) * viewport_px[1];
            Some((position, (dx * dx + dy * dy).sqrt()))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
    else {
        return true;
    };
    if distance <= 14.0 {
        settings.offset = (vertex[axis] - geometry.center[axis]) / geometry.diameter;
        settings.snap_pending = false;
        put(ctx, settings);
        ctx.request_repaint();
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn axis_indexes_are_stable() {
        assert_eq!(Axis::X.index(), 0);
        assert_eq!(Axis::Y.index(), 1);
        assert_eq!(Axis::Z.index(), 2);
    }

    #[test]
    fn plane_segments_clip_to_view_and_reject_near_plane_extremes() {
        let clipped = clip_projected_segment([-0.5, 0.5], [1.5, 0.5]);
        assert_eq!(clipped, Some([[0.0, 0.5], [1.0, 0.5]]));
        assert!(clip_projected_segment([f64::MAX, 0.5], [f64::MAX / 2.0, 0.5]).is_none());
        assert!(clip_projected_segment([f64::INFINITY, 0.5], [0.5, 0.5]).is_none());
    }

    #[test]
    fn opening_menu_and_toggling_symmetry_do_not_reenter_context_data() {
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(360.0, 240.0)).build_ui(menu);
        // Opening and leaving the popup open exercises get/put repeatedly, as production does
        // while the menu is visible. Clicking the real checkbox then exercises the enabled path.
        harness.run_steps(4);
        assert!(!get(&harness.ctx).enabled);
        harness.get_by_label(tl!("Enable 3D symmetry")).click();
        harness.run_steps(4);
        assert!(get(&harness.ctx).enabled);
    }

    #[test]
    fn normalized_plane_corners_are_bounded_by_prepared_geometry() {
        let corners =
            plane_corners_in_bounds([[-0.5, 0.5], [-0.25, 0.25], [-0.1, 0.1]], Settings { enabled: true, axis: Axis::X, offset: 0.0, snap_pending: false });
        assert_eq!(corners, Some([[0.0, -0.25, -0.1], [0.0, 0.25, -0.1], [0.0, -0.25, 0.1], [0.0, 0.25, 0.1]]));
    }
}
