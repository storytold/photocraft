//! Visible-surface picking maps native 2D tool events into the bound texture document.
use crate::{
    PhotocraftApp,
    canvas::{self, ToolEvent},
    state::Tool,
    stylus::PenSample,
};
use egui::{Modifiers, Pos2, Response};
use photocraft_uv::{
    Scene,
    prepared::Geometry,
    preview::{Camera, Materials},
};
use serde_json::json;
use std::collections::VecDeque;

#[derive(Clone, Copy)]
struct Sample {
    triangle: usize,
    shell: u32,
    point: [f64; 2],
}
#[derive(Clone)]
struct SurfaceStrokeAnchor {
    position: [f64; 3],
    pressure: Option<PenSample>,
    doc: photocraft_doc::DocId,
    /// Keep the source scene alive so identity cannot be confused with an allocator-reused pointer.
    scene: std::sync::Arc<Scene>,
}
#[derive(Clone, Copy)]
struct MirrorSample {
    sample: Sample,
    pressure: f32,
    pen: Option<PenSample>,
    time_ms: f64,
}
#[derive(Clone, Copy)]
enum MirrorPathPoint {
    Break,
    Sample(MirrorSample),
}
const SURFACE_SAMPLE_SPACING_PX: f32 = 1.5;
const SURFACE_SAMPLES_PER_FRAME: usize = 128;
const SURFACE_PATH_BUDGET: std::time::Duration = std::time::Duration::from_millis(3);
const WRAP_STAMPS_PER_BATCH: usize = 256;
const WRAP_PREVIEW_INTERVAL_MS: f64 = 33.0;

#[derive(Clone, Copy, Debug)]
struct PathPoint {
    screen: Pos2,
    pen: Option<PenSample>,
    time_ms: f64,
}

impl From<Pos2> for PathPoint {
    fn from(screen: Pos2) -> Self {
        Self { screen, pen: None, time_ms: 0.0 }
    }
}

#[derive(Clone, Copy)]
enum PointerEvent {
    Down { point: PathPoint, modifiers: Modifiers, tool: Tool, symmetry_enabled: bool },
    Move { point: PathPoint, modifiers: Modifiers },
    Up { point: PathPoint, modifiers: Modifiers },
    Gone,
}

fn collect_pointer_events(
    app: &mut PhotocraftApp,
    ui: &egui::Ui,
    response: &Response,
    events: &[egui::Event],
    tool: Tool,
    now_ms: f64,
    frame_time_base: f64,
) -> VecDeque<PointerEvent> {
    let move_count = events.iter().filter(|event| matches!(event, egui::Event::PointerMoved(_))).count();
    let mut move_index = 0usize;
    let mut queued = VecDeque::new();
    for event in events {
        match event {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers }
                if !modifiers.alt && response.contains_pointer() =>
            {
                queued.push_back(PointerEvent::Down {
                    point: PathPoint { screen: *pos, pen: app.stylus.raw_selection(), time_ms: now_ms },
                    modifiers: *modifiers,
                    tool,
                    symmetry_enabled: crate::uv_symmetry::get(ui.ctx()).enabled,
                });
            }
            egui::Event::PointerMoved(pos) => {
                let index = move_index;
                move_index += 1;
                app.stylus.select(index, move_count);
                let pen = app.stylus.raw_selection();
                app.stylus.clear_selection();
                let fraction = (index + 1) as f64 / move_count.max(1) as f64;
                queued.push_back(PointerEvent::Move {
                    point: PathPoint { screen: *pos, pen, time_ms: frame_time_base + (now_ms - frame_time_base).max(0.0) * fraction },
                    modifiers: ui.input(|input| input.modifiers),
                });
            }
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers } => {
                queued
                    .push_back(PointerEvent::Up { point: PathPoint { screen: *pos, pen: app.stylus.raw_selection(), time_ms: now_ms }, modifiers: *modifiers });
            }
            egui::Event::PointerGone => queued.push_back(PointerEvent::Gone),
            _ => {}
        }
    }
    app.stylus.clear_selection();
    queued
}

#[derive(Default)]
pub(crate) struct State {
    held: bool,
    wrapped: bool,
    action: Action,
    modifiers: Modifiers,
    selection_split_pending: bool,
    contour_sample: Option<Sample>,
    contour_tool: Option<Tool>,
    contour_doc: Option<photocraft_doc::DocId>,
    material: Option<usize>,
    doc: Option<photocraft_doc::DocId>,
    tool: Option<Tool>,
    last: Option<Sample>,
    output_last: Option<Sample>,
    path_last: Option<PathPoint>,
    path_pending: VecDeque<PathPoint>,
    deferred_pointer_events: VecDeque<PointerEvent>,
    path_release_pending: bool,
    wrap_pending: Vec<serde_json::Value>,
    wrap_options: Option<serde_json::Value>,
    wrap_last_flush_ms: Option<f64>,
    /// A native mask Brush gesture stays active across disconnected UV island renderers.
    mask_segmented_live: bool,
    mask_segment_start_pending: bool,
    /// A GPU-backed mask stroke waits one render turn for its single release readback before Up.
    native_gpu_release_waiting: bool,
    symmetry_snap_consumed: bool,
    /// Suppress the texture document's independent 2D symmetry for this 3D gesture.
    suppress_document_symmetry: bool,
    mirror_path_failed: bool,
    mirror_path: Vec<MirrorPathPoint>,
    live_mirrored_path: bool,
    /// Last committed surface endpoint for Shift-click connected strokes.
    line_anchor: Option<SurfaceStrokeAnchor>,
    /// Latest visible surface point acquired by the current painting stroke.
    stroke_end: Option<SurfaceStrokeAnchor>,
    surface_line_active: bool,
}

fn defer_pointer_event(state: &mut State, event: PointerEvent) {
    state.deferred_pointer_events.push_back(event);
}

fn defer_new_gesture_behind_path(state: &mut State, event: PointerEvent) -> bool {
    if state.path_pending.is_empty() {
        return false;
    }
    state.held = false;
    state.path_release_pending = true;
    defer_pointer_event(state, event);
    true
}

fn push_mirror_break(app: &mut PhotocraftApp, state: &mut State) {
    if state.mirror_path_failed || matches!(state.mirror_path.last(), Some(MirrorPathPoint::Break)) {
        return;
    }
    if state.mirror_path.try_reserve(1).is_err() {
        state.mirror_path.clear();
        state.mirror_path_failed = true;
        app.ui.status = tl!("Could not buffer the mirrored brush path").into();
        app.ui.status_error = true;
        return;
    }
    state.mirror_path.push(MirrorPathPoint::Break);
}

fn push_mirror_sample(state: &mut State, sample: MirrorSample) -> bool {
    if state.mirror_path_failed || state.mirror_path.try_reserve(1).is_err() {
        state.mirror_path.clear();
        state.mirror_path_failed = true;
        return false;
    }
    state.mirror_path.push(MirrorPathPoint::Sample(sample));
    true
}

/// A UV 3D symmetry gesture has already built its mirrored surface path. Temporarily remove the
/// document's 2D symmetry while creating, previewing and committing these mapped texture points.
/// Otherwise the engine would reflect them again in unrelated texture space.
fn tool_event(app: &mut PhotocraftApp, state: &State, event: ToolEvent, modifiers: Modifiers) {
    // In 3D, Shift belongs to the surface line gesture even if there is no usable anchor yet.
    // Never let the canvas reinterpret it as a stale 2D UV connection or angle constraint.
    let surface_shift = app.ui.workspace == "3D" && state.action == Action::Paint && state.tool.is_some_and(crate::stroke_constraint::connects);
    let modifiers = if surface_shift { Modifiers { shift: false, ..modifiers } } else { modifiers };
    let native_uv_mask_gpu_disabled = std::mem::replace(&mut app.native_uv_mask_gpu_disabled, state.suppress_document_symmetry);
    let saved = if state.suppress_document_symmetry {
        app.session.active_mut().and_then(|document| document.symmetry.take().map(|symmetry| (document.doc.id, symmetry)))
    } else {
        None
    };
    canvas::tool_event(app, event, modifiers);
    if let Some((id, symmetry)) = saved
        && let Some(document) = app.session.active_mut().filter(|document| document.doc.id == id)
    {
        document.symmetry = Some(symmetry);
    }
    app.native_uv_mask_gpu_disabled = native_uv_mask_gpu_disabled;
}

fn flush_wrap_samples(app: &mut PhotocraftApp, state: &mut State) {
    let now_ms = app.stylus.clock_ms;
    flush_wrap_samples_at(app, state, now_ms);
}

fn flush_wrap_samples_at(app: &mut PhotocraftApp, state: &mut State, now_ms: f64) {
    if state.wrap_pending.is_empty() {
        return;
    }
    let mut params = state.wrap_options.take().unwrap_or_default();
    if let Some(object) = params.as_object_mut() {
        object.insert("stamps".into(), serde_json::Value::Array(std::mem::take(&mut state.wrap_pending)));
    }
    if let Err(error) = app.run("paint.uvWrap", params) {
        app.ui.status = error;
        app.ui.status_error = true;
    }
    state.wrapped = true;
    state.wrap_last_flush_ms = now_ms.is_finite().then_some(now_ms);
}

fn wrap_preview_due(mask_target: bool, last_flush_ms: Option<f64>, now_ms: f64) -> bool {
    !mask_target || last_flush_ms.is_none_or(|last| now_ms - last >= WRAP_PREVIEW_INTERVAL_MS)
}

fn interpolate_pen(a: Option<PenSample>, b: Option<PenSample>, t: f32) -> Option<PenSample> {
    let (Some(a), Some(b)) = (a, b) else {
        return if t >= 1.0 { b } else { a };
    };
    let angle = (b.rotation - a.rotation + 540.0).rem_euclid(360.0) - 180.0;
    Some(
        PenSample {
            pressure: a.pressure + (b.pressure - a.pressure) * t,
            tilt_x: a.tilt_x + (b.tilt_x - a.tilt_x) * t,
            tilt_y: a.tilt_y + (b.tilt_y - a.tilt_y) * t,
            rotation: (a.rotation + angle * t).rem_euclid(360.0),
            eraser: if t < 1.0 { a.eraser } else { b.eraser },
        }
        .sanitized(),
    )
}

/// Advance one continuous screen-space sample without truncating long OS pointer segments.
fn next_path_point(last: &mut PathPoint, pending: &mut VecDeque<PathPoint>) -> Option<PathPoint> {
    loop {
        let target = *pending.front()?;
        let distance = last.screen.distance(target.screen);
        if !distance.is_finite() || distance <= f32::EPSILON {
            pending.pop_front();
            if distance.is_finite() {
                *last = target;
                continue;
            }
            continue;
        }
        let t = (SURFACE_SAMPLE_SPACING_PX / distance).min(1.0);
        let point = if t >= 1.0 {
            pending.pop_front();
            target
        } else {
            PathPoint {
                screen: last.screen.lerp(target.screen, t),
                pen: interpolate_pen(last.pen, target.pen, t),
                time_ms: last.time_ms + (target.time_ms - last.time_ms) * f64::from(t),
            }
        };
        *last = point;
        return Some(point);
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Action {
    #[default]
    Paint,
    CloneSource,
    Eyedropper,
}
fn action_for_down(modifiers: Modifiers, tool: Tool) -> Action {
    if modifiers.ctrl && matches!(tool, Tool::CloneStamp | Tool::Healing) {
        Action::CloneSource
    } else if modifiers.ctrl && tool == Tool::Brush {
        Action::Eyedropper
    } else {
        Action::Paint
    }
}
pub(crate) fn pending(state: &State) -> bool {
    state.doc.is_some() && (state.last.is_some() || state.mask_segmented_live) && state.tool.is_some_and(canvas::strokes_live)
}
pub(crate) fn active_document(state: &State) -> Option<photocraft_doc::DocId> {
    pending(state).then_some(state.doc).flatten()
}
fn close_selection_contour(app: &mut PhotocraftApp, state: &mut State) {
    match state.tool {
        Some(Tool::PolygonLasso) if app.ui.polygon.len() >= 3 => canvas::commit_polygon(app),
        Some(Tool::PolygonLasso) => {
            app.ui.polygon.clear();
            app.ui.polygon_mode.clear();
        }
        Some(Tool::MagneticLasso) if app.ui.magnetic.path.len() + app.ui.magnetic.live.len() >= 3 => {
            crate::magnetic_lasso_ui::close(app, true);
        }
        Some(Tool::MagneticLasso) => crate::magnetic_lasso_ui::cancel(app),
        _ => {}
    }
    state.contour_sample = None;
    state.selection_split_pending = true;
}

fn end_segment(app: &mut PhotocraftApp, state: &mut State, close_contour: bool, finish_gesture: bool) {
    let source_last = state.last.take();
    let had_last = source_last.is_some();
    if had_last || finish_gesture && (state.mask_segmented_live || app.drag.is_some()) {
        let last = state
            .output_last
            .take()
            .or(source_last)
            .map(|last| last.point)
            .or_else(|| app.drag.as_ref().and_then(|drag| drag.points.last().map(|point| [point[0], point[1]])));
        if app.drag.is_some() {
            let deferred = std::mem::replace(&mut app.defer_live_stroke, false);
            canvas::feed_live_stroke(app);
            app.defer_live_stroke = deferred;
        }
        if state.tool == Some(Tool::Brush)
            && state.action == Action::Paint
            && !finish_gesture
            && app.native_uv_surface_input
            && app.ui.workspace == "3D"
            && canvas::break_native_uv_mask_segment(app)
        {
            state.mask_segmented_live = true;
            state.mask_segment_start_pending = true;
            if close_contour && matches!(state.tool, Some(Tool::PolygonLasso | Tool::MagneticLasso)) {
                close_selection_contour(app, state);
            }
            return;
        }
        if app.session.active().is_none_or(|st| Some(st.doc.id) != state.doc) {
            app.drag = None;
            app.live_stroke = None;
            app.session.clear_live_stroke();
            state.mask_segmented_live = false;
            return;
        }
        if let Some([x, y]) = last {
            tool_event(app, state, ToolEvent::Up { x, y }, state.modifiers);
        }
        state.mask_segmented_live = false;
    }
    if had_last && state.tool.is_some_and(crate::tool_feedback::is_selection_tool) && !matches!(state.tool, Some(Tool::PolygonLasso | Tool::MagneticLasso)) {
        // A surface stroke can cross UV seams. Commit this island, then add subsequent
        // islands to it so a default Replace gesture doesn't discard the earlier segment.
        state.selection_split_pending = true;
    }
    if close_contour && matches!(state.tool, Some(Tool::PolygonLasso | Tool::MagneticLasso)) {
        close_selection_contour(app, state);
    }
}
pub(crate) fn finish(app: &mut PhotocraftApp, state: &mut State) {
    finish_inner(app, state, false);
}
fn finish_contour(app: &mut PhotocraftApp, state: &mut State) {
    finish_inner(app, state, true);
}
fn finish_inner(app: &mut PhotocraftApp, state: &mut State, preserve_contour: bool) {
    flush_wrap_samples(app, state);
    replay_mirror_path(app, state);
    end_segment(app, state, false, true);
    if state.wrapped {
        let _ = app.run("paint.uvWrap", json!({"end":true}));
    }
    let keep = preserve_contour
        && state.action == Action::Paint
        && matches!(state.tool, Some(Tool::PolygonLasso | Tool::MagneticLasso))
        && state.contour_sample.is_some();
    let line_anchor = state.stroke_end.clone().or_else(|| state.line_anchor.clone());
    if keep {
        let contour_sample = state.contour_sample;
        let contour_tool = state.tool;
        let contour_doc = state.doc;
        let selection_split_pending = state.selection_split_pending;
        *state = State { contour_sample, contour_tool, contour_doc, selection_split_pending, line_anchor, ..Default::default() };
    } else {
        *state = State { line_anchor, ..Default::default() };
    }
    app.uv_paint_key = None;
}
fn replay_mirror_path(app: &mut PhotocraftApp, state: &mut State) {
    let points = std::mem::take(&mut state.mirror_path);
    if points.iter().all(|point| matches!(point, MirrorPathPoint::Break)) {
        return;
    }
    let Some(scene) = app.session.uv.scene.clone() else { return };
    let previous_override = app.tool_override;
    if let Some(tool) = state.tool {
        app.tool_override = Some(tool);
    }
    if app.drag.is_some() {
        mark_path_break(app);
    }
    // The primary source path has ended for this rendered batch. Keep its own continuity state,
    // but split the live renderer path before appending the corresponding mirrored batch.
    let doc_size = app.session.active().filter(|st| Some(st.doc.id) == state.doc).map(|st| st.doc.size);
    if let Some(doc_size) = doc_size {
        let mut prior: Option<Sample> = None;
        let mut pending_break = false;
        for point in points {
            let MirrorPathPoint::Sample(mirrored) = point else {
                pending_break |= prior.take().is_some();
                continue;
            };
            let discontinuous = prior.is_some_and(|last| {
                last.shell != mirrored.sample.shell
                    || (!adjacent(&scene, last.triangle, mirrored.sample.triangle)
                        && (last.point[0] - mirrored.sample.point[0]).hypot(last.point[1] - mirrored.sample.point[1])
                            > f64::from(doc_size.width.max(doc_size.height)) * 0.02)
            });
            if pending_break || discontinuous {
                mark_path_break(app);
            }
            pending_break = false;
            let event = if app.drag.is_some() {
                ToolEvent::Move { x: mirrored.sample.point[0], y: mirrored.sample.point[1], pressure: mirrored.pressure }
            } else {
                ToolEvent::Down { x: mirrored.sample.point[0], y: mirrored.sample.point[1], pressure: mirrored.pressure }
            };
            app.stylus.clock_ms = mirrored.time_ms;
            app.stylus.select_raw_sample(mirrored.pen);
            tool_event(app, state, event, state.modifiers);
            app.stylus.clear_selection();
            state.output_last = Some(mirrored.sample);
            prior = Some(mirrored.sample);
        }
        if prior.is_some() {
            state.live_mirrored_path = true;
        }
    }
    app.tool_override = previous_override;
}
fn adjacent(scene: &Scene, a: usize, b: usize) -> bool {
    if a == b {
        return true;
    }
    let (Some(a), Some(b)) = (scene.triangles.get(a), scene.triangles.get(b)) else { return false };
    a.corners.iter().filter(|a| b.corners.iter().any(|b| a.vertex == b.vertex && a.uv.iter().zip(b.uv).all(|(a, b)| (a - b).abs() < 1e-6))).count() >= 2
}

fn symmetry_hit(ctx: &egui::Context, view: &View<'_>, hit: &photocraft_uv::prepared::Hit, tool: Tool) -> Option<photocraft_uv::prepared::Hit> {
    if !matches!(tool, Tool::Brush | Tool::Pencil | Tool::Eraser) {
        return None;
    }
    let settings = crate::uv_symmetry::get(ctx);
    if !settings.enabled {
        return None;
    }
    let axis = settings.axis.index();
    let plane = view.geometry.center[axis] + settings.offset * view.geometry.diameter;
    let source_distance = hit.position[axis] - plane;
    if !source_distance.is_finite() || source_distance.abs() <= view.geometry.diameter * 1e-7 {
        return None;
    }
    let mirrored = view.geometry.mirror_hit(view.scene, view.camera, view.maps, hit, axis, plane, view.size, view.geometry.diameter * 0.02)?;
    let target_distance = mirrored.position[axis] - plane;
    (target_distance.is_finite() && source_distance * target_distance < 0.0 && mirrored.uv.iter().all(|value| (0.0..=1.0).contains(value))).then_some(mirrored)
}

fn symmetry_document_matches(app: &PhotocraftApp, state: &State, target_material: usize) -> bool {
    let Some(document) = state.doc else { return false };
    let channel = app.session.uv.current_channel();
    app.session.uv.map_bindings(channel).get(target_material).copied().flatten() == Some(document)
        && (!app.ui.mask_target || crate::uv_mask::has_mask_binding(app, target_material, document))
}

fn mark_path_break(app: &mut PhotocraftApp) {
    if let Some(drag) = app.drag.as_mut() {
        let index = drag.points.len();
        if index > 0 && drag.breaks.last() != Some(&index) {
            drag.breaks.push(index);
        }
    }
    let _ = canvas::break_live_stroke_path(app);
}

fn begin_source_path(app: &mut PhotocraftApp, state: &mut State) {
    if state.live_mirrored_path {
        // Source and reflected passes share one drag for Undo, but each renderer path needs an
        // explicit boundary when the next ordered source batch follows a mirrored batch.
        mark_path_break(app);
        state.live_mirrored_path = false;
    }
}

struct View<'a> {
    wrap: bool,
    geometry: &'a Geometry,
    scene: &'a Scene,
    camera: Camera,
    maps: &'a Materials<'a>,
    size: [u32; 2],
    rect: egui::Rect,
}

fn document_at_surface_point(app: &PhotocraftApp, view: &View<'_>, point: PathPoint) -> Option<photocraft_doc::DocId> {
    let at = (point.screen - view.rect.min) / view.rect.size();
    if !view.rect.contains(point.screen) {
        return None;
    }
    let hit = view.geometry.hit(view.scene, view.camera, view.maps, [f64::from(at.x), f64::from(at.y)], view.size)?;
    app.session.uv.map_bindings(app.session.uv.current_channel()).get(view.geometry.triangles.get(hit.triangle)?.material).copied().flatten()
}

/// Reproject a prior stroke endpoint into this camera and ensure the same bound document is
/// visible there. This lets Shift-click remain attached to the model if the camera moved between
/// strokes, while refusing to start a line from an occluded or newly rebound surface point.
fn surface_line_start(app: &PhotocraftApp, state: &State, view: &View<'_>, endpoint: PathPoint, tool: Tool) -> Option<PathPoint> {
    let anchor = state.line_anchor.as_ref()?;
    if !app.session.uv.scene.as_ref().is_some_and(|scene| std::sync::Arc::ptr_eq(scene, &anchor.scene)) || !crate::stroke_constraint::connects(tool) {
        return None;
    }
    if document_at_surface_point(app, view, endpoint) != Some(anchor.doc) {
        return None;
    }
    let normalized = view.geometry.project_normalized(view.camera, anchor.position, view.size)?;
    let screen = view.rect.min + egui::vec2(normalized[0] as f32 * view.rect.width(), normalized[1] as f32 * view.rect.height());
    let start = PathPoint { screen, pen: anchor.pressure, time_ms: endpoint.time_ms };
    let at = (screen - view.rect.min) / view.rect.size();
    let hit = view.geometry.hit(view.scene, view.camera, view.maps, [f64::from(at.x), f64::from(at.y)], view.size)?;
    let position = std::array::from_fn::<_, 3, _>(|index| (hit.position[index] - view.geometry.center[index]) / view.geometry.diameter);
    let distance = position.iter().zip(anchor.position).map(|(a, b)| (a - b) * (a - b)).sum::<f64>().sqrt();
    (distance <= 0.01 && document_at_surface_point(app, view, start) == Some(anchor.doc)).then_some(start)
}

fn draw_surface_line_preview(app: &PhotocraftApp, ui: &egui::Ui, state: &State, view: &View<'_>, tool: Tool) {
    if !ui.input(|input| input.modifiers.shift && !input.modifiers.alt) || state.held || !crate::stroke_constraint::connects(tool) {
        return;
    }
    let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) else { return };
    let endpoint = PathPoint { screen: pointer, pen: None, time_ms: ui.input(|input| input.time * 1000.0) };
    let Some(start) = surface_line_start(app, state, view, endpoint, tool) else { return };
    let tokens = crate::theme::Tokens::get(ui.ctx());
    let painter = ui.painter_at(view.rect);
    painter.line_segment([start.screen, pointer], egui::Stroke::new(2.5, tokens.canvas));
    painter.line_segment([start.screen, pointer], egui::Stroke::new(1.0, tokens.accent));
}

fn sample(app: &mut PhotocraftApp, ctx: &egui::Context, state: &mut State, view: &View<'_>, path_point: impl Into<PathPoint>, action: Action) {
    let path_point = path_point.into();
    if state.symmetry_snap_consumed {
        return;
    }
    let p = path_point.screen;
    if action == Action::Paint && state.suppress_document_symmetry {
        begin_source_path(app, state);
    }
    let at = (p - view.rect.min) / view.rect.size();
    let hit = view.rect.contains(p).then(|| view.geometry.hit(view.scene, view.camera, view.maps, [f64::from(at.x), f64::from(at.y)], view.size)).flatten();
    if crate::uv_symmetry::get(ctx).snap_pending {
        if let Some(hit) = hit {
            state.symmetry_snap_consumed = true;
            crate::uv_symmetry::snap_hit(
                ctx,
                view.scene,
                view.geometry,
                view.camera,
                view.size,
                hit,
                [f64::from(at.x), f64::from(at.y)],
                [f64::from(view.rect.width()), f64::from(view.rect.height())],
            );
        }
        return;
    }
    let Some(hit) = hit else {
        if state.suppress_document_symmetry {
            push_mirror_break(app, state);
        }
        end_segment(app, state, true, false);
        return;
    };
    let Some(triangle) = view.geometry.triangles.get(hit.triangle) else { return };
    if hit.uv.iter().any(|v| !(0.0..=1.0).contains(v)) {
        if state.suppress_document_symmetry {
            push_mirror_break(app, state);
        }
        end_segment(app, state, true, false);
        return;
    }
    let material = triangle.material;
    if crate::uv_mask_paint::levels_picker_active(app, ctx) {
        if state.last.is_none() {
            crate::uv_mask_paint::sample_levels_at_uv(app, ctx, material, hit.uv);
        }
        finish(app, state);
        return;
    }
    // In grayscale mask view only surfaces whose color texture is the displayed document are
    // editable. A checker on another material is a visual placeholder, never a paint target.
    if let Some(mask_doc) = app.session.active().and_then(|st| photocraft_engine::mask_view_cmds::current(st).map(|_| st.doc.id))
        && (!crate::uv_mask::has_mask_binding(app, material, mask_doc)
            || app.session.uv.map_bindings(app.session.uv.current_channel()).get(material).copied().flatten() != Some(mask_doc))
    {
        if state.suppress_document_symmetry {
            push_mirror_break(app, state);
        }
        end_segment(app, state, true, false);
        return;
    }
    // One physical gesture targets one material/document, so it has one predictable Undo.
    if state.material.is_some_and(|m| m != material) {
        if state.suppress_document_symmetry {
            push_mirror_break(app, state);
        }
        end_segment(app, state, true, false);
        return;
    }
    let bindings = app.session.uv.map_bindings(app.session.uv.current_channel());
    let Some(id) = bindings.get(material).copied().flatten() else {
        app.ui.status = tl!("Assign a texture to this material before painting").into();
        if state.suppress_document_symmetry {
            push_mirror_break(app, state);
        }
        end_segment(app, state, true, false);
        return;
    };
    if state.material.is_none() {
        let changed = app.session.active().is_none_or(|st| st.doc.id != id);
        if app.run("uv.material.select", json!({"material":material})).is_err() {
            return;
        }
        if changed {
            use photocraft_engine::channel_cmds::ChannelTarget;
            app.ui.mask_target = app.session.active().is_some_and(|st| st.channel_view.target == ChannelTarget::LayerMask);
            app.ui.vector_mask_target = app.session.active().is_some_and(|st| st.channel_view.target == ChannelTarget::VectorMask);
            app.sync_mask_targets();
        }
        state.material = Some(material);
        state.doc = Some(id);
    }
    if app.session.active().is_none_or(|st| Some(st.doc.id) != state.doc) {
        if state.suppress_document_symmetry {
            push_mirror_break(app, state);
        }
        finish(app, state);
        return;
    }
    let (doc_id, doc_size, point) = {
        let Some(st) = app.session.active() else { return };
        let point = [
            (hit.uv[0] * f64::from(st.doc.size.width)).min(f64::from(st.doc.size.width) - 1e-6),
            ((1.0 - hit.uv[1]) * f64::from(st.doc.size.height)).min(f64::from(st.doc.size.height) - 1e-6),
        ];
        (st.doc.id, st.doc.size, point)
    };
    let tool = app.active_tool();
    if action == Action::Paint && crate::stroke_constraint::connects(tool) {
        state.stroke_end = app.session.uv.scene.clone().map(|scene| SurfaceStrokeAnchor {
            position: std::array::from_fn(|index| (hit.position[index] - view.geometry.center[index]) / view.geometry.diameter),
            pressure: path_point.pen,
            doc: doc_id,
            scene,
        });
    }
    let mirrored_hit = (action == Action::Paint && state.suppress_document_symmetry)
        .then(|| symmetry_hit(ctx, view, &hit, tool))
        .flatten()
        .filter(|mirrored| view.geometry.triangles.get(mirrored.triangle).is_some_and(|target| symmetry_document_matches(app, state, target.material)));
    let contour_tool = state.tool.unwrap_or_else(|| app.active_tool());
    if state.last.is_none()
        && matches!(contour_tool, Tool::PolygonLasso | Tool::MagneticLasso)
        && state.contour_tool == Some(contour_tool)
        && state.contour_doc.is_some_and(|doc| doc == doc_id)
        && state.contour_sample.is_some_and(|sample| sample.shell != triangle.shell)
    {
        close_selection_contour(app, state);
    } else if state.contour_doc.is_some_and(|doc| doc != doc_id) {
        // Pending lasso contours belong to one document; never commit them into a newly
        // selected material document.
        canvas::polygon_cancel_stale(app);
        crate::magnetic_lasso_ui::cancel(app);
        state.contour_sample = None;
        state.contour_tool = None;
        state.contour_doc = None;
        state.selection_split_pending = false;
    }
    if view.wrap && matches!(tool, Tool::Brush | Tool::Pencil | Tool::Eraser) && action == Action::Paint {
        end_segment(app, state, false, false);
        let tool = app.active_tool();
        let target = canvas::paint_target(app);
        let options = json!({
            "erase":tool == Tool::Eraser,
            "pencil":tool == Tool::Pencil || crate::eraser_ui::pencil_mode(app, tool),
            "target":target,
            "coalesce":app.uv_paint_key.clone().unwrap_or_else(|| "uv3d-wrap".into()),
        });
        if state.wrap_options.as_ref().is_some_and(|pending| pending != &options) {
            flush_wrap_samples(app, state);
        }
        if state.wrap_pending.len() >= WRAP_STAMPS_PER_BATCH {
            flush_wrap_samples(app, state);
        }
        state.wrap_options.get_or_insert(options);
        state.wrap_pending.push(json!({"triangle":hit.triangle,"position":hit.position,"pressure":app.stylus.pressure()}));
        if let Some(mirrored) = mirrored_hit {
            state.wrap_pending.push(json!({"triangle":mirrored.triangle,"position":mirrored.position,"pressure":app.stylus.pressure()}));
        }
        return;
    }
    if state.last.is_some_and(|last| {
        last.shell != triangle.shell
            || (!adjacent(view.scene, last.triangle, hit.triangle)
                && (last.point[0] - point[0]).hypot(last.point[1] - point[1]) > f64::from(doc_size.width.max(doc_size.height)) * 0.02)
    }) {
        let shell_changed = state.last.is_some_and(|last| last.shell != triangle.shell);
        if state.suppress_document_symmetry {
            push_mirror_break(app, state);
        }
        end_segment(app, state, shell_changed, false);
    }
    let pressure = app.stylus.pressure();
    if action == Action::CloneSource {
        crate::retouch_ui::set_source(app, point[0], point[1]);
        return;
    }
    if action == Action::Eyedropper {
        if let Some([r, g, b]) = canvas::eyedropper_color(app, point[0], point[1]) {
            let _ = app.run("tools.setColors", json!({"foreground":[r,g,b,1.0]}));
        }
        return;
    }
    let event = if state.last.is_some() || state.mask_segmented_live {
        ToolEvent::Move { x: point[0], y: point[1], pressure }
    } else {
        if state.selection_split_pending
            && state.tool.is_some_and(crate::tool_feedback::is_selection_tool)
            && crate::canvas::selection_mode(app, state.modifiers) == "replace"
        {
            state.modifiers.shift = true;
        }
        state.selection_split_pending = false;
        ToolEvent::Down { x: point[0], y: point[1], pressure }
    };
    let source_sample = Sample { triangle: hit.triangle, shell: triangle.shell, point };
    let segment_start = state.mask_segment_start_pending.then(|| app.drag.as_ref().map_or(0, |drag| drag.points.len()));
    tool_event(app, state, event, state.modifiers);
    state.output_last = Some(source_sample);
    if let Some(mirrored) = mirrored_hit
        && let Some(mirror_triangle) = view.geometry.triangles.get(mirrored.triangle)
    {
        let mirror_point = [
            (mirrored.uv[0] * f64::from(doc_size.width)).min(f64::from(doc_size.width) - 1e-6),
            ((1.0 - mirrored.uv[1]) * f64::from(doc_size.height)).min(f64::from(doc_size.height) - 1e-6),
        ];
        if !push_mirror_sample(
            state,
            MirrorSample {
                sample: Sample { triangle: mirrored.triangle, shell: mirror_triangle.shell, point: mirror_point },
                pressure,
                pen: path_point.pen,
                time_ms: path_point.time_ms,
            },
        ) {
            app.ui.status = tl!("Could not buffer the mirrored brush path").into();
            app.ui.status_error = true;
        }
    } else if state.suppress_document_symmetry {
        push_mirror_break(app, state);
    }
    if state.mirror_path_failed {
        app.ui.status = tl!("Could not buffer the mirrored brush path").into();
        app.ui.status_error = true;
    }
    if let Some(start) = segment_start
        && let Some(drag) = app.drag.as_mut()
        && drag.points.len() > start
    {
        drag.breaks.push(start);
        state.mask_segment_start_pending = false;
    }
    state.last = Some(source_sample);
    if state.tool == Some(Tool::PolygonLasso) && app.ui.polygon.is_empty() || state.tool == Some(Tool::MagneticLasso) && !app.ui.magnetic.active() {
        // The user explicitly closed the contour in the viewport. Do not carry its UV island
        // into the next independent lasso selection.
        state.contour_sample = None;
        state.contour_tool = None;
        state.contour_doc = None;
        state.selection_split_pending = false;
    } else if matches!(state.tool, Some(Tool::PolygonLasso | Tool::MagneticLasso)) && action == Action::Paint {
        state.contour_sample = Some(source_sample);
        state.contour_tool = state.tool;
        state.contour_doc = Some(doc_id);
    }
}

fn path_batch_exhausted(processed: usize, limit: usize, elapsed: std::time::Duration) -> bool {
    processed >= limit || elapsed >= SURFACE_PATH_BUDGET
}

fn process_path(app: &mut PhotocraftApp, ctx: &egui::Context, state: &mut State, view: &View<'_>, limit: usize) -> usize {
    let Some(mut last) = state.path_last else { return 0 };
    let started = std::time::Instant::now();
    let mut processed = 0;
    while !path_batch_exhausted(processed, limit, started.elapsed()) {
        let Some(point) = next_path_point(&mut last, &mut state.path_pending) else { break };
        app.stylus.clock_ms = point.time_ms;
        app.stylus.select_raw_sample(point.pen);
        let action = state.action;
        sample(app, ctx, state, view, point, action);
        app.stylus.clear_selection();
        state.path_last = Some(last);
        processed += 1;
    }
    app.stylus.clear_selection();
    processed
}

fn finish_after_pending_path(app: &mut PhotocraftApp, ctx: &egui::Context, state: &mut State, view: &View<'_>) {
    let previous_override = app.tool_override;
    if let Some(tool) = state.tool {
        // A mode/tool change ends the previous gesture; replay its already-acquired points with
        // the tool that owned them, then send Up before restoring the new tool.
        app.tool_override = Some(tool);
    }
    let deferred = std::mem::replace(&mut app.defer_live_stroke, true);
    if !state.path_pending.is_empty() {
        process_path(app, ctx, state, view, SURFACE_SAMPLES_PER_FRAME);
    }
    if state.suppress_document_symmetry {
        replay_mirror_path(app, state);
    }
    let more = !state.path_pending.is_empty();
    app.defer_live_stroke = deferred;
    if more {
        app.tool_override = previous_override;
        if !deferred {
            canvas::feed_live_stroke(app);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    } else {
        if canvas::request_native_mask_gpu_release(app) {
            state.native_gpu_release_waiting = true;
            app.tool_override = previous_override;
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            return;
        }
        let deferred_pointer_events = std::mem::take(&mut state.deferred_pointer_events);
        finish(app, state);
        state.deferred_pointer_events = deferred_pointer_events;
        app.tool_override = previous_override;
        if !state.deferred_pointer_events.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}

fn defer_native_mask_path(app: &PhotocraftApp, tool: Tool) -> bool {
    use_bounded_mask_input(
        app.native_uv_surface_input,
        app.ui.workspace == "3D",
        tool == Tool::Brush,
        canvas::paint_target(app) == json!("mask"),
        app.uv_gpu_active(),
    )
}

fn use_bounded_mask_input(native_surface_input: bool, workspace_3d: bool, brush: bool, mask_target: bool, gpu: bool) -> bool {
    native_surface_input && workspace_3d && brush && mask_target && gpu
}

fn defer_symmetry_path(native_surface_3d: bool, symmetry_active: bool, tool: Tool) -> bool {
    native_surface_3d && symmetry_active && matches!(tool, Tool::Brush | Tool::Pencil | Tool::Eraser)
}

fn surface_tool(tool: Tool) -> bool {
    canvas::strokes_live(tool)
        || matches!(
            tool,
            Tool::Eyedropper
                | Tool::RectMarquee
                | Tool::EllipseMarquee
                | Tool::SingleRowMarquee
                | Tool::SingleColumnMarquee
                | Tool::Lasso
                | Tool::PolygonLasso
                | Tool::MagneticLasso
                | Tool::ObjectSelection
                | Tool::QuickSelection
                | Tool::MagicWand
                | Tool::Type
                | Tool::VerticalType
                | Tool::HorizontalTypeMask
                | Tool::VerticalTypeMask
                | Tool::Pen
                | Tool::FreeformPen
                | Tool::CurvaturePen
        )
}
/// Ctrl+RMB owns its drag until release, including after Ctrl is released.
pub(crate) fn resize_brush(app: &mut PhotocraftApp, ui: &egui::Ui, response: &Response, enabled: bool) -> bool {
    let mut handled = app.brush_resize.is_some_and(|r| r.screen);
    if !enabled
        || app.ui.brush_picker.is_some()
        || !crate::brush_resize::applies(app.active_tool())
        || !ui.input(|i| i.focused)
        || !crate::menus::modal_allows(app, "paint.stroke")
        || app.ui.transform.is_some()
    {
        if handled {
            app.brush_resize = None;
        }
        return handled;
    }
    crate::paint_mouse::sync_tool_brush(app);
    for event in ui.input(|i| i.events.clone()) {
        let active = app.brush_resize.is_some_and(|r| r.screen);
        match event {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed: true, modifiers }
                if modifiers.ctrl
                    && !modifiers.alt
                    && response.rect.contains(pos)
                    && ui.ctx().layer_id_at(pos).is_none_or(|layer| layer == ui.layer_id())
                    && app.drag.is_none() =>
            {
                handled |=
                    crate::brush_resize::pointer_screen(app, ToolEvent::Down { x: f64::from(pos.x), y: f64::from(pos.y), pressure: 1.0 }, modifiers, true);
            }
            egui::Event::PointerMoved(pos) if active => {
                crate::brush_resize::pointer_screen(app, ToolEvent::Move { x: f64::from(pos.x), y: f64::from(pos.y), pressure: 1.0 }, Modifiers::NONE, false);
                handled = true;
            }
            egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed: false, .. } if active => {
                crate::brush_resize::pointer_screen(app, ToolEvent::Up { x: f64::from(pos.x), y: f64::from(pos.y) }, Modifiers::NONE, false);
                handled = true;
            }
            egui::Event::PointerGone if active => {
                app.brush_resize = None;
                handled = true;
            }
            _ => {}
        }
    }
    if app.brush_resize.is_some_and(|r| r.screen) && !ui.input(|i| i.pointer.secondary_down()) {
        app.brush_resize = None;
    }
    handled
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn input(
    app: &mut PhotocraftApp,
    ui: &egui::Ui,
    response: &Response,
    state: &mut State,
    geometry: &Geometry,
    scene: &Scene,
    camera: Camera,
    maps: &Materials<'_>,
    size: [u32; 2],
    enabled: bool,
    wrap: bool,
) {
    app.stylus.use_pressure = app.session.prefs().tools.use_tablet_pressure;
    app.stylus.set_pressure_curve(&app.session.prefs().tools.pressure_curve.clone());
    app.stylus.update_for_frame(ui.ctx().cumulative_frame_nr(), &ui.input(|i| i.events.clone()));
    crate::stylus::Stylus::sync_eraser_tool(app);
    crate::uv_mask_paint::prune_levels_picker(app, ui.ctx());
    let tool = if ui.input(|i| i.modifiers.ctrl) && matches!(app.ui.tool, Tool::CloneStamp | Tool::Healing) { app.ui.tool } else { app.active_tool() };
    let native_surface_3d = app.native_uv_surface_input && app.ui.workspace == "3D";
    let native_mask_brush = native_surface_3d && state.tool.unwrap_or(tool) == Tool::Brush && canvas::paint_target(app) == json!("mask");
    let levels_picker = crate::uv_mask_paint::levels_picker_active(app, ui.ctx());
    let view = View { wrap, geometry, scene, camera, maps, size, rect: response.rect };
    if response.hovered() {
        draw_surface_line_preview(app, ui, state, &view, tool);
    }
    let events = ui.input(|input| input.events.clone());
    let now_ms = ui.input(|input| input.time) * 1000.0;
    let frame_time_base = state.path_pending.back().or(state.path_last.as_ref()).map_or(now_ms, |point| point.time_ms).min(now_ms);
    let mut incoming_pointer_events = collect_pointer_events(app, ui, response, &events, tool, now_ms, frame_time_base);
    if state.native_gpu_release_waiting {
        if canvas::complete_native_mask_gpu_release(app) {
            state.native_gpu_release_waiting = false;
            let deferred_pointer_events = std::mem::take(&mut state.deferred_pointer_events);
            finish_contour(app, state);
            state.deferred_pointer_events = deferred_pointer_events;
        } else {
            state.deferred_pointer_events.append(&mut incoming_pointer_events);
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
            return;
        }
    }
    let has_queued_gesture = state.path_release_pending || !state.path_pending.is_empty() || !state.deferred_pointer_events.is_empty();
    if app.brush_resize.is_some_and(|r| r.screen) {
        let finish_mask_stroke = native_mask_brush && (state.held || state.path_release_pending || !state.path_pending.is_empty());
        if response.hovered() && !state.path_pending.is_empty() && canvas::strokes_live(state.tool.unwrap_or(tool)) {
            cursor(app, ui, &view);
        }
        if state.path_release_pending || !state.path_pending.is_empty() || !state.deferred_pointer_events.is_empty() {
            state.deferred_pointer_events.append(&mut incoming_pointer_events);
        }
        finish_after_pending_path(app, ui.ctx(), state, &view);
        if finish_mask_stroke {
            ui.ctx().request_repaint();
        }
        cursor(app, ui, &view);
        if let Some(at) = ui.input(|i| i.pointer.latest_pos()) {
            canvas::draw_readout(ui.ctx(), "uv-brush-size", at, [tl!("Size:")], [format!("{} px", app.session.tools.brush.size.round())]);
        }
        return;
    }
    if (!enabled
        || app.ui.brush_picker.is_some()
        || !crate::menus::modal_allows(app, "paint.stroke")
        || app.ui.transform.is_some()
        || (!surface_tool(tool) && !levels_picker)
        || state.tool.is_some_and(|old| old != tool)
        || ui.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused))
        && !has_queued_gesture
    {
        let finish_mask_stroke = native_mask_brush && (state.held || state.path_release_pending || !state.path_pending.is_empty());
        if response.hovered() && !state.path_pending.is_empty() && canvas::strokes_live(state.tool.unwrap_or(tool)) {
            cursor(app, ui, &view);
        }
        if state.path_release_pending || !state.path_pending.is_empty() || !state.deferred_pointer_events.is_empty() {
            state.deferred_pointer_events.append(&mut incoming_pointer_events);
        }
        finish_after_pending_path(app, ui.ctx(), state, &view);
        if finish_mask_stroke {
            ui.ctx().request_repaint();
        }
        return;
    }
    crate::paint_mouse::sync_tool_brush(app);
    let bounded_mask_path = defer_native_mask_path(app, state.tool.unwrap_or(tool));
    let draw_brush_cursor_before_path = response.hovered()
        && (state.suppress_document_symmetry || bounded_mask_path)
        && (canvas::strokes_live(tool) || tool == Tool::QuickSelection)
        && !(tool == Tool::Brush && ui.input(|i| i.modifiers.ctrl));
    if draw_brush_cursor_before_path {
        cursor(app, ui, &view);
    }
    let mut pointer_events = std::mem::take(&mut state.deferred_pointer_events);
    pointer_events.append(&mut incoming_pointer_events);
    let mut mask_stroke_finished = false;
    if bounded_mask_path {
        app.stylus.clock_ms = now_ms;
    }
    let deferred = app.defer_live_stroke;
    let previous_override = app.tool_override;
    app.defer_live_stroke = true;
    let mut defer_remaining_pointer_events = false;
    while let Some(event) = pointer_events.pop_front() {
        if defer_remaining_pointer_events {
            defer_pointer_event(state, event);
            continue;
        }
        match event {
            PointerEvent::Down { point, modifiers, tool: down_tool, symmetry_enabled } => {
                if defer_new_gesture_behind_path(state, event) {
                    defer_remaining_pointer_events = true;
                    continue;
                }
                if state.path_release_pending || state.held {
                    if let Some(previous_tool) = state.tool {
                        app.tool_override = Some(previous_tool);
                    }
                    flush_wrap_samples_at(app, state, now_ms);
                }
                finish_contour(app, state);
                static GESTURES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                app.uv_paint_key = Some(format!("uv3d-{}", GESTURES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
                app.tool_override = Some(down_tool);
                let action = action_for_down(modifiers, down_tool);
                let line_start = (action == Action::Paint && modifiers.shift).then(|| surface_line_start(app, state, &view, point, down_tool)).flatten();
                state.held = true;
                state.path_release_pending = false;
                state.path_pending.clear();
                state.tool = Some(down_tool);
                state.path_last = Some(line_start.unwrap_or(point));
                state.modifiers = modifiers;
                state.action = action;
                state.stroke_end = None;
                state.surface_line_active = line_start.is_some();
                state.suppress_document_symmetry =
                    action == Action::Paint && symmetry_enabled && matches!(down_tool, Tool::Brush | Tool::Pencil | Tool::Eraser);
                app.stylus.select_raw_sample(point.pen);
                if let Some(start) = line_start {
                    app.stylus.select_raw_sample(start.pen);
                    sample(app, ui.ctx(), state, &view, start, action);
                    app.stylus.select_raw_sample(point.pen);
                    state.path_pending.push_back(point);
                } else {
                    sample(app, ui.ctx(), state, &view, point, action);
                }
                app.stylus.clear_selection();
                if action != Action::Paint || down_tool == Tool::MagicWand {
                    finish(app, state);
                }
            }
            PointerEvent::Move { point, modifiers } => {
                if state.held && state.tool != Some(Tool::MagicWand) {
                    state.modifiers = modifiers;
                    app.tool_override = state.tool;
                    let gesture_tool = state.tool.unwrap_or(tool);
                    if bounded_mask_path || state.surface_line_active || defer_symmetry_path(native_surface_3d, state.suppress_document_symmetry, gesture_tool)
                    {
                        state.path_pending.push_back(point);
                    } else {
                        let previous = state.path_last.unwrap_or(point);
                        let steps = ((point.screen - previous.screen).length() / SURFACE_SAMPLE_SPACING_PX).ceil().clamp(1.0, 128.0) as usize;
                        for i in 1..=steps {
                            let t = i as f32 / steps as f32;
                            let sample_point = PathPoint {
                                screen: previous.screen.lerp(point.screen, t),
                                pen: interpolate_pen(previous.pen, point.pen, t),
                                time_ms: previous.time_ms + (point.time_ms - previous.time_ms) * f64::from(t),
                            };
                            app.stylus.select_raw_sample(sample_point.pen);
                            sample(app, ui.ctx(), state, &view, sample_point, state.action);
                            app.stylus.clear_selection();
                        }
                        state.path_last = Some(point);
                    }
                }
            }
            PointerEvent::Up { point, modifiers } => {
                state.modifiers = modifiers;
                app.tool_override = state.tool;
                mask_stroke_finished |= native_mask_brush && state.held;
                let bounded_path = bounded_mask_path
                    || state.surface_line_active
                    || defer_symmetry_path(native_surface_3d, state.suppress_document_symmetry, state.tool.unwrap_or(tool));
                if bounded_path
                    && state.held
                    && state.tool != Some(Tool::MagicWand)
                    && state.path_last.is_some_and(|last| last.screen.distance(point.screen) > f32::EPSILON)
                {
                    state.path_pending.push_back(point);
                }
                state.held = false;
                if bounded_path {
                    state.path_release_pending = true;
                } else {
                    finish_contour(app, state);
                }
            }
            PointerEvent::Gone => {
                app.tool_override = state.tool;
                mask_stroke_finished |= native_mask_brush && state.held;
                state.held = false;
                if bounded_mask_path
                    || state.surface_line_active
                    || defer_symmetry_path(native_surface_3d, state.suppress_document_symmetry, state.tool.unwrap_or(tool))
                {
                    state.path_release_pending = true;
                } else {
                    finish_contour(app, state);
                }
            }
        }
    }
    if state.held && !ui.input(|i| i.pointer.primary_down()) {
        mask_stroke_finished |= native_mask_brush;
        state.held = false;
        if bounded_mask_path
            || state.surface_line_active
            || defer_symmetry_path(native_surface_3d, state.suppress_document_symmetry, state.tool.unwrap_or(tool))
        {
            state.path_release_pending = true;
        } else {
            finish_contour(app, state);
        }
    }
    if let Some(active_tool) = state.tool {
        app.tool_override = Some(active_tool);
    }
    let bounded_path =
        bounded_mask_path || state.surface_line_active || defer_symmetry_path(native_surface_3d, state.suppress_document_symmetry, state.tool.unwrap_or(tool));
    let processed = if bounded_path { process_path(app, ui.ctx(), state, &view, SURFACE_SAMPLES_PER_FRAME) } else { 0 };
    if state.suppress_document_symmetry {
        replay_mirror_path(app, state);
    }
    // Mask preview is throttled to the same 30 Hz cadence as native live strokes. The cursor
    // still follows the latest pointer every frame; ordered stamps wait in this bounded queue.
    let wrap_preview_due = wrap_preview_due(canvas::paint_target(app) == json!("mask"), state.wrap_last_flush_ms, now_ms);
    if wrap_preview_due || (state.path_release_pending && state.path_pending.is_empty()) {
        flush_wrap_samples_at(app, state, now_ms);
    }
    if bounded_mask_path {
        app.stylus.clock_ms = now_ms;
    }
    if state.path_release_pending && state.path_pending.is_empty() {
        flush_wrap_samples_at(app, state, now_ms);
        if canvas::request_native_mask_gpu_release(app) {
            state.native_gpu_release_waiting = true;
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
        } else {
            let deferred_pointer_events = std::mem::take(&mut state.deferred_pointer_events);
            finish_contour(app, state);
            state.deferred_pointer_events = deferred_pointer_events;
        }
    }
    if state.held
        && processed == 0
        && state.path_pending.is_empty()
        && !state.wrapped
        && !state.suppress_document_symmetry
        && let Some(last) = state.last
    {
        tool_event(app, state, ToolEvent::Move { x: last.point[0], y: last.point[1], pressure: app.stylus.pressure() }, state.modifiers);
    }
    app.defer_live_stroke = deferred;
    let worker_needs_repaint = if !deferred { canvas::feed_live_stroke(app) } else { false };
    if worker_needs_repaint {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
    if bounded_path && !state.path_pending.is_empty() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
    if !state.deferred_pointer_events.is_empty() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
    if mask_stroke_finished {
        ui.ctx().request_repaint();
    }
    app.tool_override = previous_override;
    if response.hovered() {
        if levels_picker {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        } else if tool.is_type() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        } else if tool == Tool::Brush && ui.input(|i| i.modifiers.ctrl) {
            if let Some(point) = ui.input(|i| i.pointer.hover_pos()) {
                ui.ctx().set_cursor_icon(crate::canvas::pipette_cursor(ui.ctx(), point));
            }
        } else if canvas::strokes_live(tool) || tool == Tool::QuickSelection {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            if !draw_brush_cursor_before_path {
                cursor(app, ui, &view);
            }
            if tool == Tool::CloneStamp {
                crate::uv_clone::preview(app, ui, geometry, scene, camera, maps, size, response.rect);
            }
        } else if surface_tool(tool) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
    }
    if state.held || state.path_release_pending || !state.path_pending.is_empty() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
}

fn cursor(app: &PhotocraftApp, ui: &egui::Ui, view: &View<'_>) {
    let Some(p) = cursor_screen_pos(app, ui) else {
        return;
    };
    let at = (p - view.rect.min) / view.rect.size();
    let Some(hit) = view.geometry.hit(view.scene, view.camera, view.maps, [f64::from(at.x), f64::from(at.y)], view.size) else { return };
    let Some(t) = view.geometry.triangles.get(hit.triangle) else { return };
    let bindings = app.session.uv.map_bindings(app.session.uv.current_channel());
    let Some(st) = bindings.get(t.material).copied().flatten().and_then(|id| app.session.documents().iter().find(|st| st.doc.id == id)) else { return };
    let tex = t.uv.map(|uv| [f64::from(uv[0]) * f64::from(st.doc.size.width), -f64::from(uv[1]) * f64::from(st.doc.size.height)]);
    let [u, v] = [0, 1].map(|axis| [tex[1][axis] - tex[0][axis], tex[2][axis] - tex[0][axis]]);
    let det = u[0] * v[1] - u[1] * v[0];
    if det.abs() < 1e-12 {
        return;
    }
    if view.wrap && matches!(app.active_tool(), Tool::Brush | Tool::Pencil | Tool::Eraser) {
        let sub = |a: [f64; 3], b: [f64; 3]| std::array::from_fn::<_, 3, _>(|i| a[i] - b[i]);
        let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
        let norm = |a: [f64; 3]| {
            let n = a.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-20);
            a.map(|v| v / n)
        };
        let positions = t.positions.map(|p| p.map(f64::from));
        let area = cross(sub(positions[1], positions[0]), sub(positions[2], positions[0])).iter().map(|v| v * v).sum::<f64>().sqrt();
        let scale = app.session.uv.wrap_pixels_per_unit().map(|unit| unit * view.geometry.diameter).unwrap_or_else(|| (det.abs() / area.max(1e-24)).sqrt());
        let radius = f64::from(app.session.tools.brush.size) * 0.5 / scale.max(1e-20);
        let u = norm(t.tangent.map(f64::from));
        let v = cross(t.face.map(f64::from), u).map(|v| v * det.signum());
        let center = std::array::from_fn::<_, 3, _>(|i| (hit.position[i] - view.geometry.center[i]) / view.geometry.diameter);
        let points: Option<Vec<_>> = (0..48)
            .map(|i| {
                let angle = f64::from(i) * std::f64::consts::TAU / 48.0;
                let p = std::array::from_fn(|j| center[j] + radius * (u[j] * angle.cos() + v[j] * angle.sin()));
                let q = view.geometry.project_normalized(view.camera, p, view.size)?;
                Some(view.rect.min + egui::vec2(q[0] as f32 * view.rect.width(), q[1] as f32 * view.rect.height()))
            })
            .collect();
        if let Some(points) = points {
            paint_cursor_ring(ui, view, p, points);
        }
        return;
    }
    let screen = t.positions.map(|p| view.geometry.project_normalized(view.camera, p.map(f64::from), view.size));
    let [Some(a), Some(b), Some(c)] = screen else {
        return;
    };
    let screen = [a, b, c].map(|p| [p[0] * f64::from(view.rect.width()), p[1] * f64::from(view.rect.height())]);
    let radius = f64::from(app.session.tools.brush.size) * 0.5;
    let points: Vec<_> = (0..48)
        .map(|i| {
            let a = f64::from(i) * std::f64::consts::TAU / 48.0;
            let (x, y) = (a.cos() * radius, a.sin() * radius);
            let (a, b) = ((x * v[1] - y * u[1]) / det, (y * u[0] - x * v[0]) / det);
            p + egui::vec2(
                (a * (screen[1][0] - screen[0][0]) + b * (screen[2][0] - screen[0][0])) as f32,
                (a * (screen[1][1] - screen[0][1]) + b * (screen[2][1] - screen[0][1])) as f32,
            )
        })
        .collect();
    paint_cursor_ring(ui, view, p, points);
}
fn cursor_screen_pos(app: &PhotocraftApp, ui: &egui::Ui) -> Option<Pos2> {
    app.brush_resize.filter(|r| r.screen).map(|r| egui::pos2(r.anchor[0] as f32, r.anchor[1] as f32)).or_else(|| ui.input(|i| i.pointer.hover_pos()))
}
fn paint_cursor_ring(ui: &egui::Ui, view: &View<'_>, p: Pos2, points: Vec<Pos2>) {
    if points.iter().any(|q| !q.is_finite() || q.distance(p) > 2048.0) {
        return;
    }
    let painter = ui.painter().with_clip_rect(view.rect);
    painter.add(egui::Shape::closed_line(points.clone(), egui::Stroke::new(2.5, egui::Color32::BLACK)));
    painter.add(egui::Shape::closed_line(points, egui::Stroke::new(1.0, egui::Color32::WHITE)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_uv::prepared::rotate;
    fn setup(tool: Tool) -> (PhotocraftApp, std::sync::Arc<Scene>, Geometry) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("uv.model.import", json!({"path":path})).unwrap();
        app.run("uv.texture.create", json!({"width":128,"height":128})).unwrap();
        app.run("tools.setColors", json!({"foreground":[1.0,0.0,0.0,1.0]})).unwrap();
        app.run("tools.setBrush", json!({"brush":{"size":6.0,"hardness":1.0}})).unwrap();
        app.ui.tool = tool;
        let scene = app.session.uv.scene.clone().unwrap();
        let geometry = Geometry::new(&scene, app.session.uv.topology.as_ref().unwrap()).unwrap();
        (app, scene, geometry)
    }
    fn screen(view: &View<'_>, tri: usize) -> Pos2 {
        let t = &view.geometry.triangles[tri];
        let p: [f64; 3] = std::array::from_fn(|i| t.positions.iter().map(|p| f64::from(p[i])).sum::<f64>() / 3.0);
        let p = rotate(p.into(), view.camera);
        let c = view.geometry.camera_values(view.camera, view.size);
        view.rect.min
            + egui::vec2(
                ((p.x + f64::from(c[6])) * f64::from(c[4]) + 1.0) as f32 * 0.5 * view.rect.width(),
                (1.0 - (p.y + f64::from(c[7])) * f64::from(c[5])) as f32 * 0.5 * view.rect.height(),
            )
    }
    fn screen_vertex(view: &View<'_>, tri: usize, corner: usize) -> Pos2 {
        let p = view.geometry.triangles[tri].positions[corner].map(f64::from);
        let normalized = std::array::from_fn(|i| (p[i] - view.geometry.center[i]) / view.geometry.diameter);
        let p = view.geometry.project_normalized(view.camera, normalized, view.size).unwrap();
        view.rect.min + egui::vec2(p[0] as f32 * view.rect.width(), p[1] as f32 * view.rect.height())
    }
    fn make_view<'a>(scene: &'a Scene, g: &'a Geometry, maps: &'a Materials<'a>) -> View<'a> {
        View {
            wrap: false,
            geometry: g,
            scene,
            camera: Camera { yaw: 0.0, pitch: 0.0, ..Default::default() },
            maps,
            size: [600, 400],
            rect: egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 400.0)),
        }
    }

    #[test]
    fn shift_line_anchor_reprojects_only_to_its_visible_bound_surface() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut app, scene, geometry) = setup(Tool::Brush);
        let maps = Materials::default();
        let view = make_view(&scene, &geometry, &maps);
        let endpoint = screen(&view, 0);
        let at = (endpoint - view.rect.min) / view.rect.size();
        let hit = geometry.hit(&scene, view.camera, &maps, [f64::from(at.x), f64::from(at.y)], view.size).unwrap();
        let material = geometry.triangles[hit.triangle].material;
        let doc = app.session.uv.map_bindings(app.session.uv.current_channel())[material].unwrap();
        let position = std::array::from_fn(|index| (hit.position[index] - geometry.center[index]) / geometry.diameter);
        let anchor = SurfaceStrokeAnchor { position, pressure: None, doc, scene: scene.clone() };
        let state = State { tool: Some(Tool::Brush), line_anchor: Some(anchor.clone()), ..Default::default() };
        let line_start = surface_line_start(&app, &state, &view, PathPoint::from(endpoint), Tool::Brush).unwrap();
        assert!(line_start.screen.distance(endpoint) < 1.0);

        app.session.uv.generation = app.session.uv.generation.wrapping_add(1);
        assert!(
            surface_line_start(&app, &state, &view, PathPoint::from(endpoint), Tool::Brush).is_some(),
            "non-geometric UV view changes must not invalidate the surface anchor"
        );

        let stale_scene = std::sync::Arc::new((*scene).clone());
        let stale = State { line_anchor: Some(SurfaceStrokeAnchor { scene: stale_scene, ..anchor }), ..state };
        assert!(
            surface_line_start(&app, &stale, &view, PathPoint::from(endpoint), Tool::Brush).is_none(),
            "a replaced model cannot reuse the old surface point"
        );
    }

    #[test]
    fn shift_connects_clone_and_healing_instead_of_selecting_the_clone_source() {
        let shift = Modifiers { shift: true, ..Default::default() };
        let ctrl = Modifiers { ctrl: true, ..Default::default() };
        assert_eq!(action_for_down(shift, Tool::CloneStamp), Action::Paint);
        assert_eq!(action_for_down(shift, Tool::Healing), Action::Paint);
        assert_eq!(action_for_down(ctrl, Tool::CloneStamp), Action::CloneSource);
        assert_eq!(action_for_down(ctrl, Tool::Healing), Action::CloneSource);
    }

    #[test]
    fn long_pointer_segments_are_consumed_without_a_sampling_gap() {
        let mut last = PathPoint { screen: Pos2::ZERO, pen: Some(PenSample { pressure: 0.2, ..Default::default() }), time_ms: 0.0 };
        let mut pending = VecDeque::from([
            PathPoint { screen: egui::pos2(260.0, 0.0), pen: Some(PenSample { pressure: 0.8, ..Default::default() }), time_ms: 10.0 },
            PathPoint { screen: egui::pos2(420.0, 80.0), pen: Some(PenSample { pressure: 0.4, ..Default::default() }), time_ms: 20.0 },
        ]);
        let mut previous = last.screen;
        let mut samples = 0;
        let mut frames = 0;
        while !pending.is_empty() {
            let mut frame_samples = 0;
            while frame_samples < 17 {
                let Some(point) = next_path_point(&mut last, &mut pending) else { break };
                assert!(previous.distance(point.screen) <= SURFACE_SAMPLE_SPACING_PX + 1e-4);
                previous = point.screen;
                samples += 1;
                frame_samples += 1;
            }
            frames += 1;
        }
        assert!(samples > 128, "long segments must not be truncated at the old interpolation cap");
        assert!(frames > 1, "backlog is processed in bounded frame-sized batches");
        assert_eq!(last.screen, egui::pos2(420.0, 80.0), "the final OS pointer endpoint is preserved");
        assert!((last.pen.unwrap().pressure - 0.4).abs() < 1e-6, "endpoint pressure is preserved");
    }

    #[test]
    fn surface_path_batch_yields_on_sample_or_time_budget() {
        assert!(SURFACE_SAMPLES_PER_FRAME > 0);
        assert!(SURFACE_PATH_BUDGET >= std::time::Duration::from_millis(2));
        assert!(SURFACE_PATH_BUDGET <= std::time::Duration::from_millis(4));
        assert!(!path_batch_exhausted(127, 128, std::time::Duration::from_millis(2)));
        assert!(path_batch_exhausted(128, 128, std::time::Duration::ZERO));
        assert!(path_batch_exhausted(1, 128, SURFACE_PATH_BUDGET));
    }

    #[test]
    fn bounded_input_is_opt_in_only_for_native_3d_gpu_mask_brush() {
        assert!(use_bounded_mask_input(true, true, true, true, true));
        assert!(!use_bounded_mask_input(false, true, true, true, true), "split-view 2D input stays eager");
        assert!(!use_bounded_mask_input(true, false, true, true, true), "non-3D workspaces stay eager");
        assert!(!use_bounded_mask_input(true, true, true, false, true), "pixel strokes stay immediate");
        assert!(!use_bounded_mask_input(true, true, false, true, true), "non-Brush tools stay immediate");
        assert!(!use_bounded_mask_input(true, true, true, true, false), "CPU fallback stays eager");
    }

    #[test]
    fn bounded_symmetry_input_is_limited_to_native_3d_brush_family() {
        assert!(defer_symmetry_path(true, true, Tool::Brush));
        assert!(defer_symmetry_path(true, true, Tool::Pencil));
        assert!(defer_symmetry_path(true, true, Tool::Eraser));
        assert!(!defer_symmetry_path(false, true, Tool::Brush), "non-native surface input stays eager");
        assert!(!defer_symmetry_path(true, false, Tool::Brush), "ordinary strokes stay eager");
        assert!(!defer_symmetry_path(true, true, Tool::CloneStamp), "the feature does not broaden to other tools");
    }

    #[test]
    fn new_pointer_gesture_waits_behind_backlog_without_losing_pen_samples() {
        let old_point = PathPoint { screen: egui::pos2(10.0, 10.0), pen: None, time_ms: 1.0 };
        let mut state = State { held: true, path_pending: VecDeque::from([old_point]), ..Default::default() };
        let down = PointerEvent::Down {
            point: PathPoint { screen: egui::pos2(20.0, 20.0), pen: Some(PenSample { pressure: 0.4, ..Default::default() }), time_ms: 2.0 },
            modifiers: Modifiers::NONE,
            tool: Tool::Brush,
            symmetry_enabled: true,
        };
        assert!(defer_new_gesture_behind_path(&mut state, down));
        defer_pointer_event(
            &mut state,
            PointerEvent::Move {
                point: PathPoint { screen: egui::pos2(25.0, 24.0), pen: Some(PenSample { pressure: 0.75, ..Default::default() }), time_ms: 3.0 },
                modifiers: Modifiers { shift: true, ..Modifiers::NONE },
            },
        );
        defer_pointer_event(
            &mut state,
            PointerEvent::Up {
                point: PathPoint { screen: egui::pos2(30.0, 28.0), pen: Some(PenSample { pressure: 0.2, ..Default::default() }), time_ms: 4.0 },
                modifiers: Modifiers::NONE,
            },
        );

        assert!(!state.held && state.path_release_pending);
        assert_eq!(state.path_pending.len(), 1, "the previous ordered stroke backlog remains untouched");
        assert_eq!(state.deferred_pointer_events.len(), 3);
        let mut events = state.deferred_pointer_events.iter();
        let PointerEvent::Down { point, tool, symmetry_enabled, .. } = events.next().unwrap() else { panic!("new press stays first") };
        assert_eq!((*tool, *symmetry_enabled), (Tool::Brush, true));
        assert_eq!((point.pen.unwrap().pressure, point.time_ms), (0.4, 2.0));
        let PointerEvent::Move { point, modifiers } = events.next().unwrap() else { panic!("move follows the press") };
        assert_eq!((point.pen.unwrap().pressure, point.time_ms, *modifiers), (0.75, 3.0, Modifiers { shift: true, ..Modifiers::NONE }));
        let PointerEvent::Up { point, .. } = events.next().unwrap() else { panic!("release follows all moves") };
        assert_eq!((point.pen.unwrap().pressure, point.time_ms), (0.2, 4.0));
    }

    #[test]
    fn wrapped_mask_preview_is_throttled_but_pixel_wrap_stays_immediate() {
        assert!(wrap_preview_due(false, Some(100.0), 101.0), "pixel Wrap keeps its per-frame preview");
        assert!(wrap_preview_due(true, None, 100.0), "the first mask preview is immediate");
        assert!(!wrap_preview_due(true, Some(100.0), 132.9), "mask recomposition waits for the 33 ms cadence");
        assert!(wrap_preview_due(true, Some(100.0), 133.0), "mask preview refreshes at 30 Hz");
    }

    #[test]
    fn deferred_surface_mask_preview_keeps_the_eager_final_path_and_cursor_tracks_latest_pointer() {
        let _gpu = photocraft_testkit::gpu_lock();
        fn paint(limit: usize, defer_preview: bool, finish_on_guard: bool) -> photocraft_doc::LayerMask {
            let (mut app, scene, geometry) = setup(Tool::Brush);
            app.run("layer.layerMask.revealAll", json!({})).unwrap();
            app.ui.mask_target = true;
            app.sync_mask_targets();
            assert_eq!(canvas::paint_target(&app), json!("mask"));
            let maps = Materials::default();
            let view = make_view(&scene, &geometry, &maps);
            let start = screen(&view, 0);
            let target = start.lerp(screen_vertex(&view, 0, 0), 0.72);
            let mut state = State {
                held: true,
                tool: Some(Tool::Brush),
                action: Action::Paint,
                path_last: Some(PathPoint { screen: start, pen: None, time_ms: 0.0 }),
                ..Default::default()
            };
            app.uv_paint_key = Some("queued-mask-preview-test".into());
            app.defer_live_stroke = defer_preview;
            sample(&mut app, &egui::Context::default(), &mut state, &view, start, Action::Paint);
            for i in 1..=12 {
                let t = i as f32 / 12.0;
                state.path_pending.push_back(PathPoint { screen: start.lerp(target, t), pen: None, time_ms: f64::from(i) });
            }
            state.held = false;
            state.path_release_pending = true;
            let mut samples = 0;
            if !finish_on_guard {
                while !state.path_pending.is_empty() {
                    samples += process_path(&mut app, &egui::Context::default(), &mut state, &view, limit);
                }
                assert!(samples >= 12, "all queued pointer endpoints are consumed");
            } else {
                assert_eq!(state.path_pending.len(), 12, "early guard starts with the entire acquired path queued");
            }
            if finish_on_guard {
                app.tool_override = Some(Tool::Move);
                let mut guard_frames = 0;
                while !state.path_pending.is_empty() {
                    finish_after_pending_path(&mut app, &egui::Context::default(), &mut state, &view);
                    guard_frames += 1;
                    assert!(guard_frames < 32, "guard drains the complete ordered backlog over bounded frames");
                }
                assert_eq!(app.tool_override, Some(Tool::Move), "guard drain restores the newly selected tool");
                assert!(guard_frames > 0);
            } else {
                app.defer_live_stroke = false;
                if state.path_release_pending && state.path_pending.is_empty() {
                    finish_contour(&mut app, &mut state);
                }
            }
            assert!(app.live_stroke.is_none(), "release flushes the final queued points and commits the live stroke");
            let active = app.session.active().unwrap();
            let layer = active.doc.layer(active.active_layer.unwrap()).unwrap();
            layer.mask.as_ref().expect("stroke target is a layer mask").clone()
        }
        let eager = paint(usize::MAX, false, false);
        let deferred = paint(7, true, false);
        let guard_drained = paint(usize::MAX, true, true);
        assert_eq!(deferred, eager, "preview deferral and multi-frame input batching must preserve the committed mask pixels");
        assert_eq!(guard_drained, eager, "an early focus/tool guard drains acquired points with the original tool before finishing");
        let (mut baseline, _, _) = setup(Tool::Brush);
        baseline.run("layer.layerMask.revealAll", json!({})).unwrap();
        let active = baseline.session.active().unwrap();
        let baseline = active.doc.layer(active.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().clone();
        assert_ne!(eager, baseline, "the exercised mask path must contain a committed paint change");

        let latest = egui::pos2(411.0, 177.0);
        let ctx = egui::Context::default();
        let mut input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(500.0, 300.0))), ..Default::default() };
        input.events.push(egui::Event::PointerMoved(latest));
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let mut output = ctx.run_ui(input, |ui| assert_eq!(cursor_screen_pos(&app, ui), Some(latest)));
        output.textures_delta.clear();
    }

    #[test]
    fn clone_takes_source_from_3d_despite_temporary_ctrl_move_and_generates_native_overlay() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut app, scene, g) = setup(Tool::CloneStamp);
        let maps = Materials::default();
        let view = make_view(&scene, &g, &maps);
        let at = screen(&view, 0);
        app.tool_override = Some(Tool::Move);
        sample(&mut app, &egui::Context::default(), &mut State::default(), &view, at, Action::CloneSource);
        assert!(app.session.presets.clone.active().source.is_some());
        assert!(app.drag.is_none());
        app.tool_override = None;
        let source = app.session.presets.clone.active().source.unwrap();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            assert!(crate::canvas::clone_preview_texture(&mut app, ui.painter(), source, None).is_some());
        });
        output.textures_delta.clear();
        let before = app.session.active().unwrap().history.past_len();
        let mut paint = State::default();
        sample(&mut app, &egui::Context::default(), &mut paint, &view, at, Action::Paint);
        finish(&mut app, &mut paint);
        assert!(app.session.active().unwrap().history.past_len() <= before + 1);
    }

    #[test]
    fn uv_clone_preview_projects_to_visible_model_and_ctrl_click_samples() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut app, scene, g) = setup(Tool::CloneStamp);
        app.ui.workspace = "3D".into();
        app.run("tools.setBrush", json!({"brush":{"size":36.0,"hardness":1.0}})).unwrap();
        app.run("select.rect", json!({"x":0,"y":0,"width":32,"height":32})).unwrap();
        app.run("edit.fill", json!({"color":"#00ff00"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        crate::retouch_ui::set_source(&mut app, 16.0, 16.0);
        app.ui.uv_workbench.split_view = true;
        app.ui.uv_workbench.view_3d = true;
        app.ui.uv_workbench.channel = photocraft_uv::preview::Channel::Color;
        app.ui.uv_workbench.yaw = 0.0;
        app.ui.uv_workbench.pitch = 0.0;
        let maps = Materials::default();
        let view = make_view(&scene, &g, &maps);
        let local = screen(&view, 0);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            app.set_wgpu(cc.wgpu_render_state.clone().unwrap());
            app
        });
        h.step();
        h.step();
        let rect = h.state().uv_cache.viewport.unwrap();
        let p = rect.min + egui::vec2(local.x / 600.0 * rect.width(), local.y / 400.0 * rect.height());
        h.event(egui::Event::PointerMoved(p));
        h.step();
        // Native source preview is generated while hovering over the visible mesh.
        assert!(h.state().clone_preview.is_some());
        let ctrl = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
        h.event(egui::Event::ModifiersChanged(ctrl));
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: ctrl });
        h.step();
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: ctrl });
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.step();
        assert_ne!(h.state().session.presets.clone.active().source.unwrap(), [16.0, 16.0]);
        assert!(h.state().drag.is_none());
        crate::retouch_ui::set_source(h.state_mut(), 16.0, 16.0);
        h.step();
        if let Ok(path) = std::env::var("PHOTOCRAFT_UV_CLONE_SCREENSHOT") {
            // Documentation captures must not expose the checkout's absolute user path.
            let st = h.state_mut().session.active_mut().unwrap();
            st.path = st.path.as_deref().and_then(|p| std::path::Path::new(p).file_name()).map(|name| name.to_string_lossy().into_owned());
            h.run_steps(2);
            h.render().unwrap().save(path).unwrap();
        }
    }

    #[test]
    #[ignore = "explicit seam-crossing 4K native paint benchmark"]
    fn benchmark_uv_seam_4k_refresh() {
        let _gpu = photocraft_testkit::gpu_lock();
        for (wrap, mask_target, with_blur) in
            [(false, false, false), (true, false, false), (false, true, false), (true, true, false), (false, true, true), (true, true, true)]
        {
            let (mut app, scene, g) = setup(Tool::Brush);
            app.run("uv.texture.create", json!({"width":4096,"height":4096,"depth":16})).unwrap();
            app.run("tools.setBrush", json!({"brush":{"size":32.0,"hardness":0.5}})).unwrap();
            if mask_target {
                // Exercise the same native 3D mask preview cadence as the actual viewport.
                app.native_uv_surface_input = true;
                app.ui.workspace = "3D".into();
            }
            let mut mask_layer = None;
            if mask_target {
                app.run("layer.layerMask.revealAll", json!({})).unwrap();
                app.ui.mask_target = true;
                app.sync_mask_targets();
                let layer = app.session.active().and_then(|state| state.active_layer).unwrap();
                app.run("uv.maskPaint.add", json!({"layer":layer.0,"kind":"paint","name":"Perf Paint"})).unwrap();
                if with_blur {
                    app.run("uv.maskPaint.add", json!({"layer":layer.0,"kind":"blur","radius":3})).unwrap();
                }
                app.run("uv.maskPaint.select", json!({"layer":layer.0,"index":0})).unwrap();
                app.run("view.layerMask", json!({"mode":"gray"})).unwrap();
                mask_layer = Some(layer);
            }
            let maps = Materials::default();
            let mut view = make_view(&scene, &g, &maps);
            view.wrap = wrap;
            let mut cache = crate::uv_lighting::SurfaceMaps::default();
            if !mask_target {
                crate::uv_lighting::refresh(&mut app, &mut cache).unwrap();
            }
            let material = app.session.uv.material;
            let mut paint = State { tool: Some(Tool::Brush), action: Action::Paint, ..Default::default() };
            app.uv_paint_key = Some("seam-benchmark".into());
            let mut elapsed = Vec::new();
            let mut cold_frame_ms = 0.0;
            let mut last_input_ms = 0.0;
            let mut last_feed_ms = 0.0;
            let mut largest = 0;
            for frame in 0..8 {
                let begin = std::time::Instant::now();
                let input_begin = std::time::Instant::now();
                let now_ms = frame as f64 * 16.67;
                app.stylus.clock_ms = now_ms;
                app.defer_live_stroke = true;
                for i in 0..4 {
                    let x = if i % 2 == 0 { -0.003 } else { 0.003 };
                    let world = [x, frame as f64 * 0.002, if x > 0.0 { x * 0.6 } else { 0.0 }];
                    let normalized = std::array::from_fn(|j| (world[j] - g.center[j]) / g.diameter);
                    let at = g.project_normalized(view.camera, normalized, view.size).unwrap();
                    let p = view.rect.min + egui::vec2(at[0] as f32 * view.rect.width(), at[1] as f32 * view.rect.height());
                    sample(&mut app, &egui::Context::default(), &mut paint, &view, p, Action::Paint);
                }
                last_input_ms = input_begin.elapsed().as_secs_f64() * 1000.0;
                if wrap_preview_due(mask_target, paint.wrap_last_flush_ms, now_ms) {
                    flush_wrap_samples_at(&mut app, &mut paint, now_ms);
                }
                app.defer_live_stroke = false;
                let feed_begin = std::time::Instant::now();
                canvas::feed_live_stroke(&mut app);
                last_feed_ms = feed_begin.elapsed().as_secs_f64() * 1000.0;
                let frame_ms = begin.elapsed().as_secs_f64() * 1000.0;
                if frame == 0 {
                    cold_frame_ms = frame_ms;
                } else {
                    elapsed.push(frame_ms);
                }
                // Measure the synchronous input + engine/cache work consistently. Keep map
                // preparation outside the timer, then still verify the sparse pixel damage.
                let pixels = if mask_target {
                    0
                } else {
                    crate::uv_lighting::refresh(&mut app, &mut cache).unwrap();
                    cache.regions[0][material].as_ref().map_or(0, |(_, r)| r.iter().map(|r| u64::from(r[2]) * u64::from(r[3])).sum::<u64>())
                };
                largest = largest.max(pixels);
                // The stroke cadence uses wall-clock time. Leave the timer out of the synthetic
                // 60 Hz frame wait, but let every second frame exercise a real 33 ms refresh.
                if mask_target {
                    std::thread::sleep(std::time::Duration::from_millis(17));
                }
            }
            let release_started = std::time::Instant::now();
            finish(&mut app, &mut paint);
            let release_ms = release_started.elapsed().as_secs_f64() * 1000.0;
            if let Some(layer) = mask_layer {
                let stack =
                    app.session.active().and_then(|state| state.doc.layer(layer)).and_then(|layer| layer.mask.as_ref()).and_then(|mask| mask.paint.as_ref());
                assert!(
                    stack.is_some_and(|stack| {
                        stack.layers[0].surface.read_region(photocraft_geom::Rect::new(0, 0, 4096, 4096)).iter().any(|value| *value > 0.01)
                    }),
                    "3D mask samples must alter the mask surface"
                );
            } else {
                assert!(largest > 0 && largest < 1_048_576, "seam must edit both islands without full-map refresh: {largest}");
            }
            println!(
                "4K {} {} seam crossing, 4 samples/frame: {:.2} ms warm average, {:.2} ms warm max, {:.2} ms cold first frame, {:.2} ms release flush; max {} recomposited texels",
                if mask_target && with_blur {
                    "mask + Blur"
                } else if mask_target {
                    "mask"
                } else {
                    "pixel"
                },
                if wrap { "Paint Wrap" } else { "Paint UV" },
                elapsed.iter().sum::<f64>() / elapsed.len() as f64,
                elapsed.iter().copied().fold(0.0, f64::max),
                cold_frame_ms,
                release_ms,
                largest
            );
            if mask_target && with_blur && !wrap {
                println!("  last frame stages: surface input {:.2} ms, stroke feed {:.2} ms", last_input_ms, last_feed_ms);
            }
        }
    }
    #[test]
    fn eyedropper_samples_bound_surface_and_first_undo_finishes_live_stroke() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut app, scene, g) = setup(Tool::Brush);
        let maps = Materials::default();
        let view = make_view(&scene, &g, &maps);
        let before = app.session.active().unwrap().doc.clone();
        app.uv_paint_key = Some("first-undo".into());
        let first = screen(&view, 0);
        let mut paint = State::default();
        sample(&mut app, &egui::Context::default(), &mut paint, &view, first, Action::Paint);
        assert!(app.live_stroke.is_some());
        app.uv_cache.paint_for_test(paint);
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.live_stroke.is_none());
        assert_eq!(app.session.active().unwrap().doc.layers[0].surface(), before.layers[0].surface());
        app.ui.tool = Tool::Eyedropper;
        app.run("tools.setColors", json!({"foreground":"#000000"})).unwrap();
        sample(&mut app, &egui::Context::default(), &mut State::default(), &view, first, Action::Paint);
        assert!(app.session.tools.foreground[..3].iter().all(|v| *v > 0.99));
    }
    #[test]
    fn uv_brush_previews_live_splits_shells_and_coalesces_one_undo() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut app, scene, g) = setup(Tool::Brush);
        let maps = Materials::default();
        let view = make_view(&scene, &g, &maps);
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        app.uv_paint_key = Some("one-uv-gesture".into());
        let mut state = State::default();
        let first = screen(&view, 0);
        sample(&mut app, &egui::Context::default(), &mut state, &view, first, Action::Paint);
        assert!(app.live_stroke.is_some());
        let index = app.session.active_index().unwrap();
        let (preview, key) = canvas::display_doc(&mut app, index);
        assert_ne!(key, 0);
        assert_ne!(preview.layers[0].surface().unwrap(), before.layers[0].surface().unwrap());
        sample(&mut app, &egui::Context::default(), &mut state, &view, screen(&view, 2), Action::Paint);
        finish(&mut app, &mut state);
        let after = app.session.active().unwrap().doc.clone();
        assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
        assert_eq!(after.layers[0].surface().unwrap().rgba(64, 64), [1.0; 4], "UV gap must not acquire a connecting stroke");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers[0].surface().unwrap(), before.layers[0].surface().unwrap());
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers[0].surface().unwrap(), after.layers[0].surface().unwrap());
    }

    #[test]
    fn mirrored_surface_paths_keep_missed_hit_breaks_and_one_undo_without_2d_symmetry() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut app, scene, geometry) = setup(Tool::Brush);
        app.run("paint.setSymmetry", json!({"mode":"horizontal","center":[64.0,64.0]})).unwrap();
        let doc = app.session.active().unwrap().doc.id;
        let before = app.session.active().unwrap().doc.layers[0].surface().unwrap().clone();
        let history = app.session.active().unwrap().history.past_len();
        let mut state =
            State { tool: Some(Tool::Brush), material: Some(app.session.uv.material), doc: Some(doc), suppress_document_symmetry: true, ..Default::default() };
        app.uv_paint_key = Some("uv3d-symmetry-test".into());
        let maps = Materials::default();
        let view = make_view(&scene, &geometry, &maps);

        tool_event(&mut app, &state, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
        tool_event(&mut app, &state, ToolEvent::Move { x: 30.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
        state.mirror_path.extend([
            MirrorPathPoint::Sample(MirrorSample { sample: Sample { triangle: 0, shell: 0, point: [100.0, 20.0] }, pressure: 1.0, pen: None, time_ms: 1.0 }),
            MirrorPathPoint::Sample(MirrorSample { sample: Sample { triangle: 0, shell: 0, point: [100.0, 30.0] }, pressure: 1.0, pen: None, time_ms: 2.0 }),
        ]);
        sample(
            &mut app,
            &egui::Context::default(),
            &mut state,
            &view,
            PathPoint { screen: view.rect.max + egui::vec2(2.0, 2.0), pen: None, time_ms: 2.5 },
            Action::Paint,
        );
        state.mirror_path.extend([
            MirrorPathPoint::Sample(MirrorSample { sample: Sample { triangle: 1, shell: 1, point: [100.0, 70.0] }, pressure: 1.0, pen: None, time_ms: 3.0 }),
            MirrorPathPoint::Sample(MirrorSample { sample: Sample { triangle: 1, shell: 1, point: [100.0, 80.0] }, pressure: 1.0, pen: None, time_ms: 4.0 }),
        ]);

        replay_mirror_path(&mut app, &mut state);
        assert_eq!(app.drag.as_ref().unwrap().breaks, [2, 4], "the gap becomes an identical preview/commit path break");
        let active_index = app.session.active_index().unwrap();
        let (preview, _) = canvas::display_doc(&mut app, active_index);
        let preview_surface = preview.layers[0].surface().unwrap().clone();
        assert_ne!(preview_surface.rgba(100, 25), [1.0; 4], "the first mirrored segment is previewed");
        assert_ne!(preview_surface.rgba(100, 75), [1.0; 4], "the second mirrored segment is previewed");
        assert_eq!(preview_surface.rgba(100, 55), [1.0; 4], "a skipped-hit gap is not connected in live preview");
        assert_eq!(preview_surface.rgba(25, 108), [1.0; 4], "document-space 2D symmetry is suppressed during a 3D symmetric stroke");

        finish(&mut app, &mut state);
        let active = app.session.active().unwrap();
        let committed = active.doc.layers[0].surface().unwrap();
        assert_eq!(active.history.past_len(), history + 1, "source and mirror paths share one Undo");
        assert_eq!(committed, &preview_surface, "the committed result matches the broken live preview");
        assert_eq!(active.symmetry.as_ref().and_then(|symmetry| symmetry.preset()).unwrap().mode, photocraft_paint::symmetry::SymmetryMode::Horizontal);
        assert_ne!(committed.rgba(25, 20), [1.0; 4], "the source side is painted");
        assert_ne!(committed.rgba(100, 25), [1.0; 4], "the mirrored side is painted");
        assert_eq!(committed.rgba(100, 55), [1.0; 4], "the skipped-hit gap remains untouched after commit");
        assert_eq!(committed.rgba(25, 108), [1.0; 4], "the retained 2D symmetry setting was not also applied");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers[0].surface().unwrap(), &before, "one Undo restores both sides");
    }

    #[test]
    fn batched_mirror_preview_keeps_source_order_pressure_breaks_and_one_undo() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut app, _, _) = setup(Tool::Brush);
        app.run("paint.setSymmetry", json!({"mode":"horizontal","center":[64.0,64.0]})).unwrap();
        let history = app.session.active().unwrap().history.past_len();
        let mut state = State {
            tool: Some(Tool::Brush),
            material: Some(app.session.uv.material),
            doc: app.session.active().map(|document| document.doc.id),
            suppress_document_symmetry: true,
            ..Default::default()
        };
        app.uv_paint_key = Some("uv3d-symmetry-batches-test".into());

        tool_event(&mut app, &state, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
        tool_event(&mut app, &state, ToolEvent::Move { x: 30.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
        state.last = Some(Sample { triangle: 0, shell: 0, point: [30.0, 20.0] });
        state.output_last = state.last;
        state.mirror_path.extend([
            MirrorPathPoint::Sample(MirrorSample { sample: Sample { triangle: 0, shell: 0, point: [100.0, 20.0] }, pressure: 0.25, pen: None, time_ms: 1.0 }),
            MirrorPathPoint::Sample(MirrorSample { sample: Sample { triangle: 0, shell: 0, point: [100.0, 30.0] }, pressure: 0.5, pen: None, time_ms: 2.0 }),
        ]);
        replay_mirror_path(&mut app, &mut state);
        assert_eq!(app.drag.as_ref().unwrap().breaks, [2], "the first rendered batch starts a separate mirrored path");
        assert_eq!(app.drag.as_ref().unwrap().points[2..4].iter().map(|point| point[2]).collect::<Vec<_>>(), [0.25, 0.5]);

        begin_source_path(&mut app, &mut state);
        tool_event(&mut app, &state, ToolEvent::Move { x: 40.0, y: 20.0, pressure: 0.75 }, Modifiers::NONE);
        state.last = Some(Sample { triangle: 0, shell: 0, point: [40.0, 20.0] });
        state.output_last = state.last;
        state.mirror_path.extend([
            MirrorPathPoint::Break,
            MirrorPathPoint::Sample(MirrorSample { sample: Sample { triangle: 1, shell: 1, point: [100.0, 70.0] }, pressure: 0.75, pen: None, time_ms: 3.0 }),
            MirrorPathPoint::Sample(MirrorSample { sample: Sample { triangle: 1, shell: 1, point: [100.0, 80.0] }, pressure: 1.0, pen: None, time_ms: 4.0 }),
        ]);
        replay_mirror_path(&mut app, &mut state);
        let drag = app.drag.as_ref().unwrap();
        assert_eq!(drag.breaks, [2, 4, 5], "each bounded render batch remains separated from the other ordered path");
        assert_eq!(drag.points[5..7].iter().map(|point| point[2]).collect::<Vec<_>>(), [0.75, 1.0], "recorded pen pressure order is preserved");

        finish(&mut app, &mut state);
        assert_eq!(app.session.active().unwrap().history.past_len(), history + 1, "all source and mirrored batches share one Undo");
    }

    #[test]
    fn native_mask_blur_seam_keeps_one_live_drag_and_one_undo() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut app, scene, g) = setup(Tool::Brush);
        app.ui.workspace = "3D".into();
        app.native_uv_surface_input = true;
        app.run("tools.setColors", json!({"foreground":"#000000"})).unwrap();
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        app.ui.mask_target = true;
        app.sync_mask_targets();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        app.run("uv.maskPaint.add", json!({"layer":layer.0,"kind":"paint","name":"Paint"})).unwrap();
        app.run("uv.maskPaint.add", json!({"layer":layer.0,"kind":"blur","name":"Blur","radius":3})).unwrap();
        app.run("uv.maskPaint.select", json!({"layer":layer.0,"index":0})).unwrap();
        app.run("view.layerMask", json!({"mode":"gray"})).unwrap();
        let maps = Materials::default();
        let view = make_view(&scene, &g, &maps);
        let before_surface = app.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].surface.clone();
        let history = app.session.active().unwrap().history.past_len();
        let mut state = State { tool: Some(Tool::Brush), action: Action::Paint, ..Default::default() };
        sample(&mut app, &egui::Context::default(), &mut state, &view, screen(&view, 0), Action::Paint);
        assert!(app.live_stroke.is_some());
        sample(&mut app, &egui::Context::default(), &mut state, &view, screen(&view, 2), Action::Paint);
        assert!(app.drag.is_some(), "an island boundary must not send Up and discard the current drag");
        assert!(app.live_stroke.is_some(), "the same engine stroke remains alive across islands");
        assert_eq!(app.drag.as_ref().unwrap().breaks, [1], "the next island begins at an explicit renderer boundary");
        assert_eq!(app.session.active().unwrap().history.past_len(), history, "no history entry is created at a seam");

        finish(&mut app, &mut state);
        assert!(app.drag.is_none());
        assert!(app.live_stroke.is_none());
        assert_eq!(app.session.active().unwrap().history.past_len(), history + 1, "the entire physical gesture commits once");
        let after = app.session.active().unwrap().doc.clone();
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].surface, before_surface);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(
            app.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].surface,
            after.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].surface
        );
    }

    #[test]
    fn native_mask_blur_release_off_mesh_after_seam_does_not_leave_empty_segment() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut app, scene, g) = setup(Tool::Brush);
        app.ui.workspace = "3D".into();
        app.native_uv_surface_input = true;
        app.run("tools.setColors", json!({"foreground":"#000000"})).unwrap();
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        app.ui.mask_target = true;
        app.sync_mask_targets();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        app.run("uv.maskPaint.add", json!({"layer":layer.0,"kind":"paint","name":"Paint"})).unwrap();
        app.run("uv.maskPaint.add", json!({"layer":layer.0,"kind":"blur","name":"Blur","radius":3})).unwrap();
        app.run("uv.maskPaint.select", json!({"layer":layer.0,"index":0})).unwrap();
        let maps = Materials::default();
        let view = make_view(&scene, &g, &maps);
        let history = app.session.active().unwrap().history.past_len();
        let mut state = State { tool: Some(Tool::Brush), action: Action::Paint, ..Default::default() };
        sample(&mut app, &egui::Context::default(), &mut state, &view, screen(&view, 0), Action::Paint);
        sample(&mut app, &egui::Context::default(), &mut state, &view, view.rect.max + egui::vec2(10.0, 10.0), Action::Paint);
        assert!(state.mask_segmented_live && state.mask_segment_start_pending);
        assert!(app.drag.is_some() && app.live_stroke.is_some());
        assert!(app.drag.as_ref().unwrap().breaks.is_empty(), "a boundary is recorded only if another island receives a point");

        finish(&mut app, &mut state);
        assert!(app.drag.is_none() && app.live_stroke.is_none());
        assert_eq!(app.session.active().unwrap().history.past_len(), history + 1, "off-mesh release still commits the painted segment once");
        let active = app.session.active().unwrap();
        let mask = active.doc.layer(layer).unwrap().mask.as_ref().unwrap();
        assert!(mask.paint.as_ref().unwrap().layers[0].surface.read_region(photocraft_geom::Rect::new(0, 0, 128, 128)).iter().any(|value| *value > 0.0));
    }

    #[test]
    fn wrapped_surface_samples_are_dispatched_as_one_ordered_batch() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut app, scene, g) = setup(Tool::Brush);
        app.run("tools.setBrush", json!({"brush":{"size":10.0,"hardness":1.0,"spacing":0.1}})).unwrap();
        let maps = Materials::default();
        let mut view = make_view(&scene, &g, &maps);
        view.wrap = true;
        app.uv_paint_key = Some("wrap-batch-test".into());
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        let mut state = State { tool: Some(Tool::Brush), ..Default::default() };
        let start = screen(&view, 0);
        let end = screen_vertex(&view, 0, 0);
        for i in 0..6 {
            sample(&mut app, &egui::Context::default(), &mut state, &view, start.lerp(end, i as f32 / 5.0), Action::Paint);
        }
        assert_eq!(state.wrap_pending.len(), 6, "all accepted path hits are retained until frame flush");
        assert!(!state.wrapped, "the command has not run before the frame batch flush");
        flush_wrap_samples(&mut app, &mut state);
        assert!(state.wrap_pending.is_empty());
        assert!(state.wrapped);
        finish(&mut app, &mut state);
        let after = app.session.active().unwrap().doc.clone();
        assert_ne!(after.layers[0].surface().unwrap(), before.layers[0].surface().unwrap());
        assert_eq!(app.session.active().unwrap().history.past_len(), history + 1, "the entire physical stroke remains one undo step");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers[0].surface().unwrap(), before.layers[0].surface().unwrap());
    }
    #[test]
    fn uv_paint_respects_selection_and_hidden_materials_and_eraser() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut app, scene, g) = setup(Tool::Brush);
        let maps = Materials::default();
        let view = make_view(&scene, &g, &maps);
        let before = app.session.active().unwrap().doc.clone();
        app.run("uv.mask.apply", json!({})).unwrap();
        let history = app.session.active().unwrap().history.past_len();
        let mut state = State::default();
        sample(&mut app, &egui::Context::default(), &mut state, &view, screen(&view, 0), Action::Paint);
        finish(&mut app, &mut state);
        assert_eq!(before.layers[0].surface().unwrap(), app.session.active().unwrap().doc.layers[0].surface().unwrap());
        assert_eq!(app.session.active().unwrap().history.past_len(), history, "masked-out live brush must not add an empty Undo step");
        app.run("select.deselect", json!({})).unwrap();
        app.ui.tool = Tool::Eraser;
        sample(&mut app, &egui::Context::default(), &mut state, &view, screen(&view, 0), Action::Paint);
        finish(&mut app, &mut state);
        assert!(app.session.active().unwrap().doc.layers[0].surface().unwrap().read_region(before.bounds()).iter().any(|v| *v < 0.9));
        let hidden = Materials { hidden: std::collections::BTreeSet::from([scene.triangles[0].material]), ..Default::default() };
        let view = make_view(&scene, &g, &hidden);
        let revision = app.session.active().unwrap().revision;
        sample(&mut app, &egui::Context::default(), &mut state, &view, screen(&view, 0), Action::Paint);
        finish(&mut app, &mut state);
        assert_eq!(app.session.active().unwrap().revision, revision);
    }
}
