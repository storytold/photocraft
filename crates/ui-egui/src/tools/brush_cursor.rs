//! The painting cursor for brushes with a tip shape: sampled tip outlines and rotated or
//! flattened round tips. Everything else falls back to the plain circle (`tool_cursor`).

use std::sync::Arc;

use egui::{CursorIcon, Painter, Pos2, Vec2};
use photocraft_paint::brush::{BrushSettings, TipShape};

/// Below this on-screen radius an outline is unreadable: show the crosshair instead.
const MIN_RADIUS: f32 = 3.0;

#[derive(Clone)]
struct Cached {
    key: Key,
    rings: Option<Arc<Vec<Vec<[f32; 2]>>>>,
}

/// The brush fields an outline depends on.
#[derive(Clone)]
struct Key {
    tip: TipShape,
    angle: f32,
    roundness: f32,
    hardness: f32,
    flip_x: bool,
    flip_y: bool,
    full: bool,
}

impl Key {
    fn of(b: &BrushSettings, full: bool) -> Self {
        Self { tip: b.tip.clone(), angle: b.angle, roundness: b.roundness, hardness: b.hardness, flip_x: b.flip_x, flip_y: b.flip_y, full }
    }
    fn matches(&self, b: &BrushSettings, full: bool) -> bool {
        self.tip == b.tip
            && self.angle == b.angle
            && self.roundness == b.roundness
            && self.hardness == b.hardness
            && self.flip_x == b.flip_x
            && self.flip_y == b.flip_y
            && self.full == full
    }
}

/// The screen vectors of one tip radius (`size / 2` document pixels) along the document's x and
/// y axes: the view's zoom, rotation, flip and pixel aspect, which the outline follows as the
/// stroke does.
pub(crate) fn tip_axes(xf: &crate::canvas::ViewXform, size: f32) -> [Vec2; 2] {
    let r = if size.is_finite() { size.max(0.0) / 2.0 } else { 0.0 };
    [xf.map_vec(Vec2::new(r, 0.0)), xf.map_vec(Vec2::new(0.0, r))]
}

/// Apply pen orientation after the cached tip shape, in document space before the view.
/// Keeping this outside `rings` avoids retracing a sampled tip on every tablet sample.
pub(crate) fn pen_axes(xf: &crate::canvas::ViewXform, brush: &BrushSettings, pen: Option<crate::stylus::PenSample>) -> [Vec2; 2] {
    use photocraft_paint::brush::Control;
    use photocraft_paint::dynamics::{StepInput, apply_pose, control_angle};
    let axes = tip_axes(xf, brush.size);
    if !brush.shape_dynamics.enabled {
        return axes;
    }
    let Some(pen) = pen else { return axes };
    let pen = pen.sanitized();
    let point = apply_pose(
        brush,
        photocraft_paint::StrokePoint { pressure: pen.pressure, tilt_x: pen.tilt_x, tilt_y: pen.tilt_y, rotation: pen.rotation, ..Default::default() },
    );
    let sd = &brush.shape_dynamics;
    let step = StepInput { point, direction: 0.0, initial_direction: 0.0, step: 0 };
    let mut angle = match sd.angle.control {
        Control::PenTilt | Control::Rotation => control_angle(sd.angle.control, sd.angle.fade_steps, &step),
        _ => 0.0,
    };
    if sd.brush_projection {
        angle += point.rotation;
    }
    let (sn, cs) = angle.to_radians().sin_cos();
    let project = |v: Vec2| {
        if !sd.brush_projection {
            return v;
        }
        let tilt = point.tilt_x.hypot(point.tilt_y).clamp(0.0, 90.0);
        if tilt == 0.0 {
            return v;
        }
        let direction = Vec2::new(point.tilt_x, point.tilt_y).normalized();
        v + direction * v.dot(direction) * (tilt.to_radians().cos().max(0.05) - 1.0)
    };
    let x = project(Vec2::new(cs, -sn));
    let y = project(Vec2::new(sn, cs));
    [axes[0] * x.x + axes[1] * x.y, axes[0] * y.x + axes[1] * y.y]
}

/// The dab the surface-aware brush will lay down at `doc`: radius, foreshortening and the offset
/// of its centre from the pointer, from the engine. `None` when neither Follow Perspective nor
/// Follow Depth applies, the tool is not the Brush or Pencil, or nothing shapes the dab here (the
/// cursor then stays as it is).
pub(crate) fn surface_shape(app: &mut crate::PhotocraftApp, tool: crate::Tool, doc: [f64; 2]) -> Option<photocraft_engine::brush_cmds::CursorShape> {
    let o = &app.session.tools.brush.surface;
    if !matches!(tool, crate::Tool::Brush | crate::Tool::Pencil) || !(o.perspective || o.depth) {
        return None;
    }
    let base = app.session.tools.brush.size / 2.0;
    photocraft_engine::brush_cmds::brush_cursor_shape(&mut app.session, doc[0], doc[1], base)
        .filter(|c| c.radius.is_finite() && c.radius > 0.0 && c.proj_angle.is_finite() && c.proj_scale.is_finite() && c.offset.iter().all(|v| v.is_finite()))
}

/// [`surface_shape`] for the cursor under the painting-cursor preference `pref` (only the tip
/// outlines follow the surface), remembered for `ui.inspect`.
pub(crate) fn surface_for_cursor(
    app: &mut crate::PhotocraftApp,
    ctx: &egui::Context,
    tool: crate::Tool,
    doc: [f64; 2],
    pref: photocraft_engine::prefs::PaintingCursor,
) -> Option<photocraft_engine::brush_cmds::CursorShape> {
    use photocraft_engine::prefs::PaintingCursor;
    let shape = surface_shape(app, tool, doc).filter(|_| matches!(pref, PaintingCursor::NormalTip | PaintingCursor::FullSizeTip));
    remember_shape(ctx, shape);
    shape
}

/// [`tip_axes`] for a dab of `radius` document pixels foreshortened by `proj_scale` along the
/// direction `proj_angle` (radians, counter-clockwise, y up: the renderer's convention).
pub(crate) fn surface_axes(xf: &crate::canvas::ViewXform, radius: f32, proj_angle: f32, proj_scale: f32) -> [Vec2; 2] {
    let axes = tip_axes(xf, radius * 2.0);
    let scale = proj_scale.clamp(0.05, 1.0);
    if scale >= 0.999 {
        return axes;
    }
    let (sn, cs) = proj_angle.sin_cos();
    let along = Vec2::new(cs, -sn);
    let project = |v: Vec2| v + along * v.dot(along) * (scale - 1.0);
    let x = project(Vec2::new(1.0, 0.0));
    let y = project(Vec2::new(0.0, 1.0));
    [axes[0] * x.x + axes[1] * x.y, axes[0] * y.x + axes[1] * y.y]
}

/// The brush cursor for a dab shaped by the surface (`shape`): the outline at the dab's centre,
/// and, when that is off the pointer (`p`, , the crosshair at the
/// pointer joined to it by a thin line.
pub(crate) fn surface_cursor(
    painter: &Painter,
    xf: &crate::canvas::ViewXform,
    p: Pos2,
    brush: &BrushSettings,
    shape: Option<photocraft_engine::brush_cmds::CursorShape>,
    normal: bool,
    centre: bool,
) -> CursorIcon {
    let plain = photocraft_engine::brush_cmds::CursorShape { radius: brush.size / 2.0, proj_angle: 0.0, proj_scale: 1.0, offset: [0.0; 2], outline: None };
    let shape = shape.unwrap_or(plain);
    let axes = surface_axes(xf, shape.radius, shape.proj_angle, shape.proj_scale);
    let doc = xf.to_doc(p);
    let at = xf.to_screen(doc[0] as f32 + shape.offset[0], doc[1] as f32 + shape.offset[1]);
    if (at - p).length() > 1.0 {
        crate::tool_cursor::crosshair(painter, p, 4.0, 0.0);
        let t = crate::theme::Tokens::get(painter.ctx());
        painter.line_segment([p, at], egui::Stroke::new(1.0, t.accent));
    }
    // A dab bent over the depth: its own 32-point outline.
    if let Some(ring) = bent_ring(xf, brush, &shape, normal) {
        let extent = ring.iter().fold(0.0f32, |m, v| m.max(v.length()));
        if extent < MIN_RADIUS {
            return crate::tool_cursor::crosshair(painter, at, 4.0, 0.0);
        }
        return crate::tool_cursor::outline(painter, at, vec![ring], centre);
    }
    cursor(painter, at, brush, (shape.radius * xf.zoom).max(1.0), axes, normal, centre)
}

/// The outline of a dab bent over the depth, in screen vectors from its centre: for the tips the
/// renderer bends (a computed circular one; a sampled or elliptical tip keeps its own outline),
/// at the contour the Normal Tip preference draws (the 50 % coverage one; Full Size Tip keeps the
/// whole outline). `None`: draw the dab's ellipse or tip outline instead.
pub(crate) fn bent_ring(
    xf: &crate::canvas::ViewXform,
    brush: &BrushSettings,
    shape: &photocraft_engine::brush_cmds::CursorShape,
    normal: bool,
) -> Option<Vec<Vec2>> {
    if !photocraft_paint::render::footprint_tip(brush) {
        return None;
    }
    let ring = outline_ring(xf, &shape.outline?)?;
    Some(scale_ring(ring, brush, shape.radius * xf.zoom, normal))
}

/// `ring` (a surface outline of a dab of screen radius `radius`) at the contour `normal` asks for:
/// the 50 % coverage one, as [`cursor`] draws a plain circle, else the full outline.
pub(crate) fn scale_ring(ring: Vec<Vec2>, brush: &BrushSettings, radius: f32, normal: bool) -> Vec<Vec2> {
    if !normal || !radius.is_finite() || radius <= 0.0 {
        return ring;
    }
    let k = photocraft_paint::half_coverage_radius(radius, brush.hardness).max(1.0) / radius;
    ring.into_iter().map(|v| v * k.min(1.0)).collect()
}

/// The screen vectors (from the dab centre) of the closed polygon through the rays of a surface
/// outline (`k · 2π / 32`, counter-clockwise, y up in document pixels). `None` for a bad ray.
pub(crate) fn outline_ring(xf: &crate::canvas::ViewXform, rays: &[f32]) -> Option<Vec<Vec2>> {
    let n = rays.len();
    if n < 3 || !rays.iter().all(|r| r.is_finite() && *r > 0.0) {
        return None;
    }
    Some(
        rays.iter()
            .enumerate()
            .map(|(k, r)| {
                let (sn, cs) = (k as f32 * std::f32::consts::TAU / n as f32).sin_cos();
                // Document y points down.
                xf.map_vec(Vec2::new(r * cs, -r * sn))
            })
            .collect(),
    )
}

fn shape_id() -> egui::Id {
    egui::Id::unique("photocraft-brush-cursor-shape")
}

/// Remember the surface shape the cursor drew this frame (`None`: the plain outline), for `ui.inspect`.
pub(crate) fn remember_shape(ctx: &egui::Context, shape: Option<photocraft_engine::brush_cmds::CursorShape>) {
    ctx.data_mut(|d| d.insert_temp(shape_id(), shape));
}

/// The surface shape last drawn by the cursor.
pub(crate) fn last_shape(ctx: &egui::Context) -> Option<photocraft_engine::brush_cmds::CursorShape> {
    ctx.data(|d| d.get_temp::<Option<photocraft_engine::brush_cmds::CursorShape>>(shape_id())).flatten()
}

/// Outlines of the document's stored perspective planes, while Follow Perspective is on and the
/// Brush or Pencil is active: thin quads in the theme's accent, so the user sees where dabs follow.
pub(crate) fn draw_planes(app: &crate::PhotocraftApp, painter: &Painter, xf: &crate::canvas::ViewXform, tool: crate::Tool) {
    if !matches!(tool, crate::Tool::Brush | crate::Tool::Pencil) || !app.session.tools.brush.surface.perspective {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let persp = &st.doc.perspective;
    if !persp.valid_for(st.doc.size.width, st.doc.size.height) {
        return;
    }
    let t = crate::theme::Tokens::get(painter.ctx());
    let stroke = egui::Stroke::new(1.0, t.accent);
    for corners in persp.planes.iter().take(64) {
        if !corners.iter().flatten().all(|v| v.is_finite() && v.abs() < 1e7) {
            continue;
        }
        let pts: Vec<Pos2> = corners.iter().map(|c| xf.to_screen(c[0] as f32, c[1] as f32)).collect();
        painter.add(egui::Shape::closed_line(pts, stroke));
    }
}

/// Rings in radius units (document orientation) on screen, through [`tip_axes`].
fn on_screen(rings: &[Vec<[f32; 2]>], axes: [Vec2; 2]) -> Vec<Vec<Vec2>> {
    rings.iter().map(|r| r.iter().map(|p| axes[0] * p[0] + axes[1] * p[1]).collect()).collect()
}

/// The brush cursor at `at`. `radius` is the tip's screen radius (size / 2 at the current zoom),
/// for the plain circle; `axes` comes from [`tip_axes`], for a traced outline. `normal` is
/// Preferences › Cursors "Normal Brush Tip" (the 50 % contour, not the full tip).
pub(crate) fn cursor(painter: &Painter, at: Pos2, brush: &BrushSettings, radius: f32, axes: [Vec2; 2], normal: bool, centre: bool) -> CursorIcon {
    let anisotropic = (axes[0].length_sq() - axes[1].length_sq()).abs() > 0.001 || axes[0].dot(axes[1]).abs() > 0.001;
    let outline = rings(painter.ctx(), brush, !normal).or_else(|| {
        // A round tip becomes an ellipse under projection (or a non-square pixel aspect).
        anisotropic.then(|| photocraft_paint::outline::tip_outline(brush, !normal).map(Arc::new)).flatten()
    });
    if let Some(rings) = outline {
        let pts = on_screen(&rings, axes);
        let extent = pts.iter().flatten().fold(0.0f32, |m, p| m.max(p.length()));
        if extent < MIN_RADIUS {
            return crate::tool_cursor::crosshair(painter, at, 4.0, 0.0);
        }
        return crate::tool_cursor::outline(painter, at, pts, centre);
    }
    let r = if normal { photocraft_paint::half_coverage_radius(radius, brush.hardness).max(1.0) } else { radius };
    crate::tool_cursor::circle(painter, at, r, centre)
}

/// The tip outline in radius units, cached across frames; `None` when the plain circle says it all
/// (a plain round tip) or the tip has no traceable shape.
pub(crate) fn rings(ctx: &egui::Context, brush: &BrushSettings, full: bool) -> Option<Arc<Vec<Vec<[f32; 2]>>>> {
    if matches!(brush.tip, TipShape::Round) && brush.angle == 0.0 && brush.roundness >= 1.0 {
        return None;
    }
    let id = egui::Id::unique("photocraft-tip-outline");
    if let Some(c) = ctx.data(|d| d.get_temp::<Cached>(id))
        && c.key.matches(brush, full)
    {
        return c.rings;
    }
    let traced = photocraft_paint::outline::tip_outline(brush, full).map(Arc::new);
    ctx.data_mut(|d| d.insert_temp(id, Cached { key: Key::of(brush, full), rings: traced.clone() }));
    traced
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::ViewXform;
    use egui::{Rect, pos2};

    fn view(rotation: f32, flip: bool, aspect: f32) -> ViewXform {
        ViewXform { rect: Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(400.0, 300.0)), zoom: 2.0, center: [50.0, 40.0], flip, rotation, aspect }
    }

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-4
    }

    /// Component timing only: outline transformation and OS bitmap generation, not presentation
    /// or tablet latency. Run explicitly in release mode on the same runner for both cases.
    #[test]
    #[ignore]
    fn pen_cursor_cost_probe() {
        let mut brush = BrushSettings { size: 60.0, roundness: 0.25, hardness: 1.0, ..Default::default() };
        brush.shape_dynamics.enabled = true;
        brush.shape_dynamics.angle.control = photocraft_paint::brush::Control::Rotation;
        let xf = view(0.0, false, 1.0);
        for dynamic in [false, true] {
            let ctx = egui::Context::default();
            for sample in 0..5 {
                let start = std::time::Instant::now();
                let _ = ctx.run_ui(Default::default(), |ui| {
                    for i in 0..360 {
                        let axes = if dynamic {
                            pen_axes(&xf, &brush, Some(crate::stylus::PenSample { rotation: i as f32, ..Default::default() }))
                        } else {
                            tip_axes(&xf, brush.size)
                        };
                        std::hint::black_box(cursor(ui.painter(), pos2(100.0, 100.0), &brush, 60.0, axes, false, false));
                    }
                });
                println!("PEN_CURSOR_COST dynamic={dynamic} sample={sample} us_per_update={:.3}", start.elapsed().as_secs_f64() * 1e6 / 360.0);
            }
        }
    }

    #[test]
    fn pen_orientation_preserves_mouse_and_disabled_dynamics() {
        let mut brush = BrushSettings::default();
        brush.shape_dynamics.angle.control = photocraft_paint::brush::Control::Rotation;
        let pen = crate::stylus::PenSample { rotation: 90.0, ..Default::default() };
        let xf = view(30.0, true, 2.0);
        assert_eq!(pen_axes(&xf, &brush, Some(pen)), tip_axes(&xf, brush.size));
        brush.shape_dynamics.enabled = true;
        assert_eq!(pen_axes(&xf, &brush, None), tip_axes(&xf, brush.size));
    }

    #[test]
    fn pen_pose_rotation_precedes_the_view_and_reuses_the_tip_cache() {
        let mut brush = BrushSettings { size: 20.0, roundness: 0.25, ..Default::default() };
        brush.shape_dynamics.enabled = true;
        brush.shape_dynamics.angle.control = photocraft_paint::brush::Control::Rotation;
        brush.pose.enabled = true;
        brush.pose.override_rotation = true;
        brush.pose.rotation = 90.0;
        let ctx = egui::Context::default();
        let cached = rings(&ctx, &brush, true).unwrap();
        for (rotation, flip, aspect) in [(0.0, false, 1.0), (30.0, true, 2.0)] {
            let xf = view(rotation, flip, aspect);
            let axes = pen_axes(&xf, &brush, Some(crate::stylus::PenSample::default()));
            assert!(close(axes[0], xf.map_vec(Vec2::new(0.0, -10.0))));
            assert!(close(axes[1], xf.map_vec(Vec2::new(10.0, 0.0))));
            assert!(Arc::ptr_eq(&cached, &rings(&ctx, &brush, true).unwrap()));
        }
    }

    #[test]
    fn brush_projection_shortens_the_tilt_axis() {
        let mut brush = BrushSettings { size: 20.0, ..Default::default() };
        brush.shape_dynamics.enabled = true;
        brush.shape_dynamics.brush_projection = true;
        let xf = view(0.0, false, 1.0);
        let axes = pen_axes(&xf, &brush, Some(crate::stylus::PenSample { tilt_x: 60.0, ..Default::default() }));
        assert!(close(axes[0], Vec2::new(10.0, 0.0)));
        assert!(close(axes[1], Vec2::new(0.0, 20.0)));
        assert_eq!(pen_axes(&xf, &brush, Some(crate::stylus::PenSample::default())), tip_axes(&xf, brush.size));
    }

    /// The axes are the screen offsets of the document points one radius right of and below the
    /// pointer, whatever the view does.
    #[test]
    fn tip_axes_follow_the_view_like_to_screen() {
        for (rotation, flip, aspect) in [(0.0, false, 1.0), (30.0, false, 1.0), (-90.0, true, 1.0), (45.0, true, 2.0), (200.0, false, 0.5)] {
            let xf = view(rotation, flip, aspect);
            let [ex, ey] = tip_axes(&xf, 20.0);
            let o = xf.to_screen(60.0, 30.0);
            assert!(close(ex, xf.to_screen(70.0, 30.0) - o), "x axis at {rotation}°, flip {flip}, aspect {aspect}");
            assert!(close(ey, xf.to_screen(60.0, 40.0) - o), "y axis at {rotation}°, flip {flip}, aspect {aspect}");
        }
    }

    /// An unrotated, unflipped view gives what upstream's radius scaling gave.
    #[test]
    fn a_plain_view_scales_the_outline_by_the_screen_radius() {
        let rings = vec![vec![[1.0, 0.0], [0.0, 0.5], [-1.0, 0.0]]];
        let pts = on_screen(&rings, tip_axes(&view(0.0, false, 1.0), 20.0));
        let want = [Vec2::new(20.0, 0.0), Vec2::new(0.0, 10.0), Vec2::new(-20.0, 0.0)];
        assert!(pts[0].iter().zip(want).all(|(a, b)| close(*a, b)), "{pts:?}");
    }

    /// A tip pointing along the document's x axis points down the screen in a view turned 90°,
    /// and left in a flipped one.
    #[test]
    fn the_outline_turns_and_flips_with_the_view() {
        let rings = vec![vec![[1.0, 0.0]]];
        let turned = on_screen(&rings, tip_axes(&view(90.0, false, 1.0), 20.0));
        assert!(close(turned[0][0], Vec2::new(0.0, 20.0)), "{turned:?}");
        let flipped = on_screen(&rings, tip_axes(&view(0.0, true, 1.0), 20.0));
        assert!(close(flipped[0][0], Vec2::new(-20.0, 0.0)), "{flipped:?}");
    }

    #[test]
    fn hostile_sizes_give_finite_axes() {
        for size in [f32::NAN, f32::INFINITY, -5.0, 0.0] {
            let [ex, ey] = tip_axes(&view(30.0, true, 1.0), size);
            assert!(ex.x.is_finite() && ex.y.is_finite() && ey.x.is_finite() && ey.y.is_finite(), "size {size}");
        }
    }

    #[test]
    fn surface_axes_foreshorten_along_the_minor_axis() {
        let xf = view(0.0, false, 1.0);
        assert_eq!(surface_axes(&xf, 10.0, 0.0, 1.0), tip_axes(&xf, 20.0));
        let a = surface_axes(&xf, 10.0, 0.0, 0.5);
        assert!(close(a[0], Vec2::new(10.0, 0.0)), "{a:?}");
        assert!(close(a[1], Vec2::new(0.0, 20.0)), "{a:?}");
        // Angle pi/2 is up the image (y up): the y axis is the one shortened.
        let b = surface_axes(&xf, 10.0, std::f32::consts::FRAC_PI_2, 0.5);
        assert!(close(b[0], Vec2::new(20.0, 0.0)) && close(b[1], Vec2::new(0.0, 10.0)), "{b:?}");
    }

    fn bent_shape() -> photocraft_engine::brush_cmds::CursorShape {
        photocraft_engine::brush_cmds::CursorShape { radius: 10.0, proj_angle: 0.0, proj_scale: 1.0, offset: [0.0; 2], outline: Some([10.0; 32]) }
    }

    #[test]
    fn the_depth_outline_is_for_computed_circular_tips_only() {
        let xf = view(0.0, false, 1.0);
        let shape = bent_shape();
        let round = BrushSettings { size: 20.0, hardness: 0.5, ..Default::default() };
        assert!(bent_ring(&xf, &round, &shape, false).is_some());
        // The renderer keeps a sampled bitmap tip and an elliptical tip: so does the cursor.
        let sampled = BrushSettings { tip: TipShape::Sampled(photocraft_paint::tile::GrayTile::from_fn(2, 2, |_, _| 1.0)), ..round.clone() };
        assert!(bent_ring(&xf, &sampled, &shape, false).is_none());
        let flat = BrushSettings { roundness: 0.3, ..round };
        assert!(bent_ring(&xf, &flat, &shape, false).is_none());
    }

    #[test]
    fn the_depth_outline_follows_the_normal_tip_rule() {
        let xf = view(0.0, false, 1.0);
        let shape = bent_shape();
        let soft = BrushSettings { size: 20.0, hardness: 0.0, ..Default::default() };
        let extent = |r: &[Vec2]| r.iter().fold(0.0f32, |m, v| m.max(v.length()));
        let full = bent_ring(&xf, &soft, &shape, false).unwrap();
        let normal = bent_ring(&xf, &soft, &shape, true).unwrap();
        // Full Size Tip: the whole outline (10 px at zoom 2). Normal Tip: the 50 % contour, as the plain circle.
        assert!((extent(&full) - 20.0).abs() < 1e-3, "{}", extent(&full));
        let half = photocraft_paint::half_coverage_radius(20.0, 0.0);
        assert!((extent(&normal) - half).abs() < 1e-3 && extent(&normal) < extent(&full) * 0.9, "{} vs {half}", extent(&normal));
        // A hard tip has its half-coverage contour at the edge: nothing changes.
        let hard = BrushSettings { hardness: 1.0, ..soft };
        assert!((extent(&bent_ring(&xf, &hard, &shape, true).unwrap()) - 20.0).abs() < 0.5);
    }

    #[test]
    fn surface_shape_is_none_when_follow_perspective_is_off() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert!(!app.session.tools.brush.surface.perspective && !app.session.tools.brush.surface.depth);
        assert!(surface_shape(&mut app, crate::Tool::Brush, [10.0, 10.0]).is_none());
    }
}
