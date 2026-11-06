//! Placement box for Affinity Patch and Content-Aware Move’s Transform On Drop.
//! Its handles are the Free Transform box's
//! ([`crate::transform_tool::hit`], [`crate::transform_tool::apply_drag`]), limited to moving,
//! scaling and rotating. Enter or the tool’s click gesture applies the result in one history
//! step. Escape discards the placement.

use egui::{CursorIcon, Modifiers, Pos2};
use photocraft_algo::transform::Homography;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::state::{CamTransform, PatchTarget, Tool, TransformSession};
use crate::transform_tool::{Gesture, Hit};

/// Where the patch lands: the drag offset (px) and the scale (%, per axis) and clockwise rotation
/// (degrees) about its centre, as `paint.contentAwareMove` takes them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    pub offset: [i64; 2],
    pub scale: [f64; 2],
    pub rotate: f64,
}

impl Placement {
    /// The patch is only moved (or not even that).
    pub fn is_plain_move(&self) -> bool {
        self.scale == [100.0, 100.0] && self.rotate == 0.0
    }

    pub(crate) fn quad(&self, [x0, y0, x1, y1]: [f64; 4]) -> [[f64; 2]; 4] {
        let [cx, cy] = [(x0 + x1) / 2.0, (y0 + y1) / 2.0];
        let [sx, sy] = self.scale.map(|s| s / 100.0);
        let (sin, cos) = self.rotate.to_radians().sin_cos();
        [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(|[x, y]| {
            let (x, y) = ((x - cx) * sx, (y - cy) * sy);
            [cx + self.offset[0] as f64 + x * cos - y * sin, cy + self.offset[1] as f64 + x * sin + y * cos]
        })
    }
}

pub(crate) fn active(app: &PhotocraftApp) -> bool {
    app.ui.cam_transform.is_some()
}

/// Put the box on the selection dropped `offset` away. False when there is no selection to box.
pub(crate) fn begin(app: &mut PhotocraftApp, offset: [i32; 2]) -> bool {
    begin_box(app, offset, None)
}

pub(crate) fn move_active(app: &PhotocraftApp) -> bool {
    app.ui.cam_transform.as_ref().is_some_and(|t| t.patch.is_none())
}

pub(crate) fn patch_placement(app: &PhotocraftApp, offset: [i32; 2]) -> Placement {
    let o = &app.ui.tool_options;
    let scale = o.patch_scale.map(|s| if (1.0..=10_000.0).contains(&s.abs()) { s } else { 100.0 });
    let rotate = if o.patch_rotation.is_finite() { o.patch_rotation % 360.0 } else { 0.0 };
    Placement { offset: offset.map(i64::from), scale, rotate }
}

/// Apply typed values about the current box centre, without moving the patch.
pub(crate) fn set_patch_transform(app: &mut PhotocraftApp, scale: [f64; 2], rotate: f64) {
    app.ui.tool_options.patch_scale = scale;
    app.ui.tool_options.patch_rotation = rotate;
    let Some(t) = app.ui.cam_transform.as_mut().filter(|t| t.patch.is_some()) else { return };
    let q = t.frame.quad;
    let center = [(q[0][0] + q[2][0]) / 2.0, (q[0][1] + q[2][1]) / 2.0];
    let [x0, y0, x1, y1] = t.frame.rect;
    let (w, h) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
    let [x, y] = center;
    t.frame.quad = Placement { offset: [0, 0], scale, rotate }.quad([x - w, y - h, x + w, y + h]);
    t.frame.pivot = center;
    t.gesture = None;
}

pub(crate) fn begin_patch(app: &mut PhotocraftApp, offset: [i32; 2]) {
    let target = crate::retouch_ui::patch_target(app);
    let place = patch_placement(app, offset);
    if begin_box(app, offset, Some(target)) {
        set_patch_transform(app, place.scale, place.rotate);
    }
}

fn begin_box(app: &mut PhotocraftApp, offset: [i32; 2], patch: Option<PatchTarget>) -> bool {
    let Some((document, revision)) = app.session.active().map(|st| (st.doc.id.0, st.revision)) else { return false };
    let Some(b) = crate::retouch_ui::patch_bounds(app) else { return false };
    let r = [b.x0.saturating_add(offset[0]), b.y0.saturating_add(offset[1]), b.x1.saturating_add(offset[0]), b.y1.saturating_add(offset[1])].map(f64::from);
    let frame = TransformSession {
        session: app.ui.alloc_id(),
        layer: 0,
        rect: r,
        quad: [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]],
        pivot: [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0],
        interpolation: crate::transform_tool::default_interpolation(app).into(),
        warp: None,
        selection: false,
        target: None,
        path: None,
        made: None,
        mode: Default::default(),
        content_aware: None,
        skew_v: 0.0,
        layers: Vec::new(),
    };
    app.ui.status = if patch.is_some() {
        tl!("Adjust the patch, then click outside the box to apply").into()
    } else {
        crate::i18n::fmt(tl!("Scale or rotate the moved area, then press {key}"), &[("key", &crate::shortcuts::pretty("Enter"))])
    };
    app.ui.cam_transform = Some(CamTransform { document, revision, patch, offset, frame, gesture: None });
    app.ui.status_error = false;
    true
}

/// The box as `paint.contentAwareMove`'s params: its rect → quad map split into a scale along the
/// box's own axes, a rotation and the move of its centre. `None` for a collapsed or absurd box.
pub(crate) fn placement(t: &CamTransform) -> Option<Placement> {
    let [x0, y0, x1, y1] = t.frame.rect;
    let (w, h) = (x1 - x0, y1 - y0);
    if !(w > 0.0 && h > 0.0) {
        return None;
    }
    let q = t.frame.quad;
    // The box's x axis (scaled by sx, turned) and y axis (scaled by sy, a negative sy flips).
    let u = [(q[1][0] - q[0][0]) / w, (q[1][1] - q[0][1]) / w];
    let v = [(q[3][0] - q[0][0]) / h, (q[3][1] - q[0][1]) / h];
    let angle = u[1].atan2(u[0]);
    let (sn, cs) = angle.sin_cos();
    let (sx, sy) = (u[0].hypot(u[1]), v[1] * cs - v[0] * sn);
    // Scale and rotation are about the selection's centre, then the offset moves it.
    let centre = [(q[0][0] + q[2][0]) / 2.0, (q[0][1] + q[2][1]) / 2.0];
    let origin = [(x0 + x1) / 2.0 - f64::from(t.offset[0]), (y0 + y1) / 2.0 - f64::from(t.offset[1])];
    let off = [(centre[0] - origin[0]).round(), (centre[1] - origin[1]).round()];
    let round = |v: f64| {
        let r = (v * 1e4).round() / 1e4;
        if r == 0.0 { 0.0 } else { r }
    };
    let (sx, sy, rotate) = (round(sx * 100.0), round(sy * 100.0), round(angle.to_degrees()));
    let sane = |s: f64| (1.0..=10_000.0).contains(&s.abs());
    let ok = rotate.is_finite() && sane(sx) && sane(sy) && off.iter().all(|v| v.is_finite() && v.abs() < 1e7);
    ok.then_some(Placement { offset: [off[0] as i64, off[1] as i64], scale: [sx, sy], rotate })
}

pub(crate) fn cancel(app: &mut PhotocraftApp) {
    if app.ui.cam_transform.take().is_some() && !app.ui.status_error {
        // The box's hint is done with.
        app.ui.status.clear();
    }
}

/// Drop the box when what it was made for changed: another tool, document or edit.
pub(crate) fn cancel_stale(app: &mut PhotocraftApp) {
    let Some(t) = &app.ui.cam_transform else { return };
    let tool_matches = match &t.patch {
        Some(target) => crate::retouch_ui::affinity_patch(app) && app.ui.transform.is_none() && *target == crate::retouch_ui::patch_target(app),
        None => app.ui.tool == Tool::ContentAwareMove,
    };
    let valid = tool_matches && app.session.active().is_some_and(|st| st.doc.id.0 == t.document && st.revision == t.revision && st.doc.selection.is_some());
    if !valid {
        cancel(app);
    }
}

/// `paint.contentAwareMove`'s params for the box's placement and the options bar.
pub(crate) fn params(app: &PhotocraftApp, place: &Placement) -> serde_json::Value {
    let mut p = crate::retouch_ui::content_aware_move_params(app, place.offset);
    transform_params(&mut p, place);
    p
}

pub(crate) fn transform_params(p: &mut serde_json::Value, place: &Placement) {
    if !place.is_plain_move() {
        p["scaleX"] = place.scale[0].into();
        p["scaleY"] = place.scale[1].into();
        p["rotate"] = place.rotate.into();
    }
}

/// Apply the current placement in one history step.
pub(crate) fn commit(app: &mut PhotocraftApp) {
    cancel_stale(app);
    let Some(t) = app.ui.cam_transform.clone() else { return };
    let Some(place) = placement(&t) else { return };
    if t.patch.is_some() {
        if place.offset == [0, 0] && place.is_plain_move() {
            cancel(app);
            return;
        }
        let mut p = crate::retouch_ui::patch_params(app, place.offset);
        transform_params(&mut p, &place);
        let repeat = t.patch.as_ref().is_some_and(|target| target.mode == "destination");
        if !repeat {
            p["deselect"] = true.into();
        }
        match app.run("paint.patch", p) {
            Ok(_) => {
                cancel(app);
                app.ui.tool_options.patch_scale = place.scale;
                app.ui.tool_options.patch_rotation = place.rotate;
                if repeat {
                    crate::retouch_ui::patch_hint(app);
                }
            }
            Err(e) => {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
        return;
    }
    cancel(app);
    if place.offset == [0, 0] && place.is_plain_move() {
        crate::retouch_ui::patch_hint(app);
        return;
    }
    let p = params(app, &place);
    if let Err(e) = app.run("paint.contentAwareMove", p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Pointer input while the box is up: every press goes to it (the canvas is otherwise inert, as
/// during Free Transform). Returns false when there is no box.
pub(crate) fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: Modifiers) -> bool {
    cancel_stale(app);
    let tol = crate::transform_tool::handle_tolerance(app);
    let zoom = f64::from(app.point_zoom());
    let legacy = app.session.prefs().general.use_legacy_free_transform;
    let Some(t) = app.ui.cam_transform.as_mut() else { return false };
    match ev {
        ToolEvent::Down { x, y, .. } => {
            let p = [x, y];
            if !p.iter().all(|v| v.is_finite()) {
                return true;
            }
            let mut h = frame_hit(app.ui.tool_options.transform_reference_point, t, p, tol);
            // ⌥-click away from the handles moves the reference point there (and drags it).
            if mods.alt && (t.patch.is_none() || app.ui.tool_options.transform_reference_point) && matches!(h, Hit::Inside | Hit::Outside) {
                t.frame.pivot = p;
                h = Hit::Pivot;
            }
            t.gesture = Some(Gesture { hit: h, start: p, quad0: t.frame.quad, pivot0: t.frame.pivot });
        }
        ToolEvent::Move { x, y, .. } | ToolEvent::Up { x, y } => {
            if let Some(g) = t.gesture
                && [x, y].iter().all(|v| v.is_finite() && v.abs() <= 1e7)
            {
                // Outside the box a click applies the patch, while a drag rotates it.
                if t.patch.is_some() && g.hit == Hit::Outside && t.frame.quad == g.quad0 && (x - g.start[0]).hypot(y - g.start[1]) * zoom < 3.0 {
                    if matches!(ev, ToolEvent::Up { .. }) {
                        t.gesture = None;
                        app.patch_hover = Some([x, y]);
                        commit(app);
                    }
                    return true;
                }
                let before = (t.frame.quad, t.frame.pivot);
                // Move, scale and rotate only: ⌘ would skew or distort, which the blend can't take.
                let plain = Modifiers { command: false, ctrl: false, mac_cmd: false, ..mods };
                crate::transform_tool::apply_drag(&mut t.frame, g, [x, y], crate::transform_tool::legacy_mods(legacy, g.hit, plain));
                if placement(t).is_none() {
                    (t.frame.quad, t.frame.pivot) = before;
                }
            }
            if matches!(ev, ToolEvent::Up { .. }) {
                t.gesture = None;
            }
        }
    }
    true
}

/// The selection's outline where the box puts it, and the box.
pub(crate) fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(t) = &app.ui.cam_transform else { return };
    if let Some((doc, _, segs)) = &app.outline_cache
        && doc.0 == t.document
        && let Some(h) = Homography::rect_to_quad(t.frame.rect, t.frame.quad)
    {
        let [ox, oy] = t.offset.map(f64::from);
        let time = painter.ctx().input(|i| i.time);
        crate::canvas::marching_ants_mapped(painter, segs, time, |p| {
            let (x, y) = h.apply(f64::from(p[0]) + ox, f64::from(p[1]) + oy);
            xf.to_screen(x as f32, y as f32)
        });
        painter.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    crate::type_transform::draw_box(painter, xf, &t.frame, t.patch.is_none() || app.ui.tool_options.transform_reference_point);
}

fn frame_hit(reference_point: bool, t: &CamTransform, p: [f64; 2], tol: f64) -> Hit {
    if t.patch.is_some() { crate::transform_tool::hit_with(&t.frame, p, tol, reference_point) } else { crate::transform_tool::hit(&t.frame, p, tol) }
}

/// The Free Transform box's cursors over the box's handles.
pub(crate) fn cursor(app: &PhotocraftApp, ctx: &egui::Context, xf: &ViewXform, p: Pos2) -> Option<CursorIcon> {
    let t = app.ui.cam_transform.as_ref()?;
    let q = t.frame.quad;
    let h = t
        .gesture
        .map_or_else(|| frame_hit(app.ui.tool_options.transform_reference_point, t, xf.to_doc(p), crate::transform_tool::handle_tolerance(app)), |g| g.hit);
    let mid = |i: usize| -> Option<[f64; 2]> {
        let (a, b) = (q.get(i % 4)?, q.get((i + 1) % 4)?);
        Some([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0])
    };
    Some(match h {
        Hit::Corner(i) => crate::tool_cursor::corner_resize(&q, i, xf),
        Hit::Edge(i) => {
            let (a, b) = (mid(i)?, mid(i + 2)?);
            crate::tool_cursor::resize([a[0] - b[0], a[1] - b[1]], xf)
        }
        Hit::Outside => {
            crate::type_transform::rotation_cursor(ctx, p);
            CursorIcon::None
        }
        Hit::Pivot => CursorIcon::Crosshair,
        Hit::Inside => CursorIcon::Move,
    })
}
