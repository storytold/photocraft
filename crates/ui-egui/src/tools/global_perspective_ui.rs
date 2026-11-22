//! Edit › Global Perspective Warp on the canvas: independent source guides, one global matrix.
//!
//! The guides are quads drawn on door frames, windows and other lines that should be parallel; the WHOLE layer follows
//! one homography the solver derives from them (`photocraft_algo::global_perspective`). Editing the guides and
//! inspecting the result are separate: there are no destination handles. ↩ / Commit runs the one engine command
//! (`edit.perspectiveWarpGlobal`) with the guides, so the result is a single history step and exactly replayable.
//!
//! The session lives in `Distort::global_perspective`. While it is open and belongs to the active document, the canvas
//! lends it to the tool host as a [`CanvasTool`](craft::ui::canvas::CanvasTool) (`canvas::tools::global_warp`): that tool
//! owns the primary button whichever tool is selected, captures the drag until its release, and takes Enter, Escape and
//! Delete. A session whose document, layer or colour display no longer matches is never lent, so it cannot swallow a click;
//! it keeps its Cancel on the options bar.
//!
//! Modifier keys steer a corner drag: Ctrl (⌘ on a Mac) locks it to the image axes, Shift to the guide's own two edge
//! directions as they were when the drag began. Ctrl wins when both are held.

use std::sync::Arc;

use craft::behavior::CanvasView;
use craft::behavior::kurbo::Point;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _};
use craft::ui::display::Text;
use craft::ui::form::SliderField;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use egui::{Color32, Pos2, Shape, Stroke, TextureHandle, pos2};
use photocraft_algo::global_perspective::{Guide, MAX_GUIDES, Options, Solution, solve, validate_guides, validate_options};
use photocraft_doc::{DocId, Document, LayerId};
use photocraft_engine::global_perspective_cmds::{COMMAND, validate_target};
use photocraft_geom::Rect;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::channel_view::Screen;

mod local_axes;
#[cfg(test)]
mod tests;
use local_axes::LocalAxes;

/// The menu row's label: the engine command's own, so the menu and the command list name the feature alike.
pub const MENU_LABEL: &str = "Global Perspective Warp";

/// Cells per side of the preview mesh at the start, and the most the faithfulness check may ask for.
const GRID: usize = 128;
const MAX_PREVIEW_GRID: usize = 512;
const MAX_MESH_ERROR_PX: f64 = 0.5;
/// Screen points within which a handle or edge is hit.
pub(crate) const HIT_POINTS: f64 = 8.0;
/// Screen points a Ctrl or Shift drag must move before its axis is chosen.
const AXIS_ACTIVATION_POINTS: f64 = 2.0;
/// Preview meshes kept for the canvases showing the document (the main one, a New Window copy, …).
const MESH_CANVASES: usize = 4;

/// Reject any mesh whose affine triangles visibly diverge from the exact
/// homography. Test their edges and interiors, not just the projected corners.
fn preview_mesh_exceeds_error(bounds: Rect, h: &photocraft_algo::transform::Homography, steps: usize) -> bool {
    let dx = f64::from(bounds.width()) / steps as f64;
    let dy = f64::from(bounds.height()) / steps as f64;
    let check = |source: [f64; 2], approx: [f64; 2]| {
        let exact = h.apply(source[0], source[1]);
        !exact.0.is_finite()
            || !exact.1.is_finite()
            || !approx.iter().all(|v| v.is_finite() && v.abs() <= f32::MAX as f64)
            || (exact.0 - approx[0]).hypot(exact.1 - approx[1]) > MAX_MESH_ERROR_PX
    };
    for j in 0..steps {
        for i in 0..steps {
            let x = f64::from(bounds.x0) + i as f64 * dx;
            let y = f64::from(bounds.y0) + j as f64 * dy;
            let corners = [[x, y], [x + dx, y], [x + dx, y + dy], [x, y + dy]];
            let projected = corners.map(|p| {
                let (a, b) = h.apply(p[0], p[1]);
                [a, b]
            });
            if projected.iter().flatten().any(|v| !v.is_finite() || v.abs() > f32::MAX as f64) {
                return true;
            }
            // Both triangles use the diagonal from corner 0 to corner 2.
            for (a, b) in [(0, 1), (1, 2), (2, 3), (3, 0), (0, 2)] {
                let mid = [(corners[a][0] + corners[b][0]) * 0.5, (corners[a][1] + corners[b][1]) * 0.5];
                let linear = [(projected[a][0] + projected[b][0]) * 0.5, (projected[a][1] + projected[b][1]) * 0.5];
                if check(mid, linear) {
                    return true;
                }
            }
            for (a, b, c) in [(0, 1, 2), (0, 2, 3)] {
                let third = [(corners[a][0] + corners[b][0] + corners[c][0]) / 3.0, (corners[a][1] + corners[b][1] + corners[c][1]) / 3.0];
                let linear = [(projected[a][0] + projected[b][0] + projected[c][0]) / 3.0, (projected[a][1] + projected[b][1] + projected[c][1]) / 3.0];
                if check(third, linear) {
                    return true;
                }
            }
        }
    }
    false
}

fn preview_grid_size(bounds: Rect, h: &photocraft_algo::transform::Homography) -> Result<usize, String> {
    for size in [GRID, GRID * 2, MAX_PREVIEW_GRID] {
        if !preview_mesh_exceeds_error(bounds, h, size) {
            return Ok(size);
        }
    }
    Err(tl!("Perspective is too strong for a faithful live preview within the mesh budget; adjust the guides or use the headless global warp").into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    Corner { id: u64, corner: usize },
    Edge { id: u64, edge: usize },
}

/// The modifier keys that steer a drag, as the canvas tool reads them from the pointer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Keys {
    /// Ctrl (⌘ on a Mac): lock to the image's horizontal or vertical axis.
    pub ctrl: bool,
    /// Shift: lock to the guide's own edge directions.
    pub shift: bool,
}

/// What the pointer is over, for the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hover {
    /// Nothing to draw or grab (the result is shown, or the pointer is over the pasteboard).
    Idle,
    /// A drag is in progress.
    Grabbing,
    /// A corner or an edge of a guide.
    Handle,
    /// Empty photo: a press starts a new guide.
    Draw,
}

#[derive(Clone)]
enum Drag {
    New {
        start: [f64; 2],
        current: [f64; 2],
    },
    Corner {
        id: u64,
        corner: usize,
        offset: [f64; 2],
        start: [f64; 2],
        axis: Option<Axis>,
        local_axes: Option<LocalAxes>,
        local_axis: Option<usize>,
        activation_distance: f64,
    },
    Edge {
        id: u64,
        edge: usize,
        mouse_start: [f64; 2],
        original: [[f64; 2]; 4],
        axis: Option<Axis>,
        activation_distance: f64,
    },
}

/// Ctrl constrains to the dominant drag axis, fixed at the first deliberate movement.
/// Releasing Ctrl restores free movement; pressing it again determines a new axis.
fn axis_restrict(delta: [f64; 2], ctrl: bool, locked: Option<Axis>, activation_distance: f64) -> ([f64; 2], Option<Axis>) {
    if !ctrl {
        return (delta, None);
    }
    if !activation_distance.is_finite() || activation_distance < 0.0 || delta.iter().any(|n| !n.is_finite()) {
        return ([0.0, 0.0], None);
    }
    let axis = locked.or_else(|| {
        if delta[0].abs().max(delta[1].abs()) < activation_distance.max(f64::EPSILON) {
            None
        } else if delta[0].abs() >= delta[1].abs() {
            Some(Axis::Horizontal)
        } else {
            Some(Axis::Vertical)
        }
    });
    match axis {
        Some(Axis::Horizontal) => ([delta[0], 0.0], axis),
        Some(Axis::Vertical) => ([0.0, delta[1]], axis),
        None => ([0.0, 0.0], None),
    }
}

fn distance_to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let v = [b[0] - a[0], b[1] - a[1]];
    let len2 = v[0] * v[0] + v[1] * v[1];
    if len2 <= f64::EPSILON {
        return f64::INFINITY;
    }
    let t = ((p[0] - a[0]) * v[0] + (p[1] - a[1]) * v[1]) / len2;
    if !(0.0..=1.0).contains(&t) {
        return f64::INFINITY;
    }
    let closest = [a[0] + t * v[0], a[1] + t * v[1]];
    (p[0] - closest[0]).hypot(p[1] - closest[1])
}

/// Test corners first so an endpoint never accidentally selects its adjoining edge.
fn hit_handle(s: &GlobalSession, p: [f64; 2], tol: f64) -> Option<Handle> {
    let mut nearest: Option<(f64, bool, Handle)> = None;
    for g in s.guides.iter().rev() {
        let Some(corners) = s.display_corners(g) else { continue };
        for (corner, q) in corners.iter().enumerate() {
            let distance = (q[0] - p[0]).hypot(q[1] - p[1]);
            let selected = s.selected == Some(g.id);
            // Equal distances prefer the selected guide, then the topmost drawn guide.
            if distance <= tol && nearest.is_none_or(|(best, was_selected, _)| distance < best || (distance == best && selected && !was_selected)) {
                nearest = Some((distance, selected, Handle::Corner { id: g.id, corner }));
            }
        }
    }
    if let Some((_, _, handle)) = nearest {
        return Some(handle);
    }
    for g in s.guides.iter().rev() {
        let Some(corners) = s.display_corners(g) else { continue };
        for edge in 0..4 {
            let a = corners[edge];
            let b = corners[(edge + 1) % 4];
            if distance_to_segment(p, a, b) <= tol {
                return Some(Handle::Edge { id: g.id, edge });
            }
        }
    }
    None
}

fn inside(p: [f64; 2], q: &[[f64; 2]; 4]) -> bool {
    (0..4).all(|i| {
        let a = q[i];
        let b = q[(i + 1) % 4];
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) >= 0.0
    })
}

/// An open Global Perspective Warp.
pub struct GlobalSession {
    /// Names the session's textures.
    pub key: u64,
    /// The document this session belongs to.
    pub document: DocId,
    /// The layer being rectified.
    pub layer: LayerId,
    /// The document without the layer: what the canvas shows under the preview.
    pub preview_doc: Arc<Document>,
    /// The document as it was when the session opened. A commit is refused once the session's document is another snapshot.
    source_doc: Arc<Document>,
    /// The layer's content bounds: the guides' frame and the preview's source.
    bounds: Rect,
    guides: Vec<Guide>,
    options: Options,
    selected: Option<u64>,
    solution: Option<Solution>,
    show_result: bool,
    drag: Option<Drag>,
    texture: TextureHandle,
    opacity: f32,
    /// [`Screen::key`] of the document when the session opened: a changed monitor transform invalidates the raw preview.
    display_key: u64,
    grid_size: usize,
    /// The screen meshes by the view they were built for, one per canvas showing the document.
    meshes: Vec<(CanvasView, Arc<egui::Mesh>)>,
}

impl GlobalSession {
    /// The session's state for `ui.inspect` and the control channel.
    pub fn describe(&self) -> Value {
        json!({"version":"V3","document":self.document.0,"layer":self.layer.0,
            "mode":if self.show_result {"result"}else{"layout"},"guides":self.guides,
            "selectedGuide":self.selected,"options":self.options,"solved":self.solution.is_some(),
            "dirty":self.solution.is_none(),"solution":self.solution,"scope":"wholeRasterLayer",
            "interactionRevision":"3.2", "localAxisConstraint":self.active_local_axis().map(|(anchor, direction)|
                json!({"anchor":anchor,"direction":direction,"space":"guide","frozen":true}))})
    }

    fn active_local_axis(&self) -> Option<([f64; 2], [f64; 2])> {
        match &self.drag {
            Some(Drag::Corner { start, local_axes: Some(axes), local_axis: Some(axis), .. }) => axes.direction(*axis).map(|direction| (*start, direction)),
            _ => None,
        }
    }

    fn map(&self, p: [f64; 2]) -> [f64; 2] {
        if self.show_result
            && let Some(solution) = &self.solution
        {
            let (x, y) = solution.homography().apply(p[0], p[1]);
            [x, y]
        } else {
            p
        }
    }

    /// In result mode only solver-validated used guides may be projected.
    /// Excluded/disabled guides stay in source coordinates, including near the horizon.
    fn display_corners(&self, guide: &Guide) -> Option<[[f64; 2]; 4]> {
        let corners = if self.show_result {
            let report = self.solution.as_ref()?.guides.iter().find(|r| r.id == guide.id)?;
            if report.used { report.transformed_corners? } else { guide.corners }
        } else {
            guide.corners
        };
        corners.iter().flatten().all(|v| v.is_finite() && v.abs() <= f32::MAX as f64).then_some(corners)
    }

    fn invalidate_mesh(&mut self) {
        // Mapping or tessellation changes invalidate every view's mesh.
        self.meshes.clear();
    }

    fn preview_mesh(&mut self, view: &CanvasView) -> Arc<egui::Mesh> {
        if let Some((_, mesh)) = self.meshes.iter().find(|(shown, _)| shown == view) {
            return Arc::clone(mesh);
        }
        let mut mesh = egui::Mesh::with_texture(self.texture.id());
        let tint = Color32::from_white_alpha((self.opacity.clamp(0.0, 1.0) * 255.0) as u8);
        let n = self.grid_size;
        mesh.vertices.reserve((n + 1) * (n + 1));
        mesh.indices.reserve(n * n * 6);
        let b = self.bounds;
        for j in 0..=n {
            for i in 0..=n {
                let p = self.map([f64::from(b.x0) + f64::from(b.width()) * i as f64 / n as f64, f64::from(b.y0) + f64::from(b.height()) * j as f64 / n as f64]);
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: view.to_screen(Point::new(p[0], p[1])),
                    uv: pos2(i as f32 / n as f32, j as f32 / n as f32),
                    color: tint,
                });
            }
        }
        for j in 0..n {
            for i in 0..n {
                let a = (j * (n + 1) + i) as u32;
                mesh.add_triangle(a, a + 1, a + n as u32 + 2);
                mesh.add_triangle(a, a + n as u32 + 2, a + n as u32 + 1);
            }
        }
        let mesh = Arc::new(mesh);
        if self.meshes.len() >= MESH_CANVASES {
            self.meshes.remove(0);
        }
        self.meshes.push((*view, Arc::clone(&mesh)));
        mesh
    }

    fn invalidate(&mut self) {
        // Layout guide edits do not change the identity mapping of the photo.
        if self.show_result || self.grid_size != GRID {
            self.invalidate_mesh();
        }
        self.solution = None;
        self.show_result = false;
        self.grid_size = GRID;
    }

    fn move_corner(&mut self, id: u64, index: usize, p: [f64; 2]) {
        // Guide handles may extend into the pasteboard, outside the image itself.
        // validate_guides still rejects non-finite, excessive or folded geometry.
        let Some(guide) = self.guides.iter().find(|g| g.id == id) else { return };
        let mut next = guide.clone();
        let Some(c) = next.corners.get_mut(index) else { return };
        *c = p;
        if next.corners == guide.corners {
            return;
        }
        if validate_guides(std::slice::from_ref(&next)).is_ok() {
            if let Some(guide) = self.guides.iter_mut().find(|g| g.id == id) {
                *guide = next;
            }
            self.invalidate();
        }
    }

    fn translate_edge(&mut self, id: u64, edge: usize, original: [[f64; 2]; 4], delta: [f64; 2]) {
        let Some(guide) = self.guides.iter().find(|g| g.id == id) else { return };
        let mut next = guide.clone();
        let neighbor = (edge + 1) % 4;
        // Preserve the complete edge's displacement even outside the image.
        // Both endpoints share the exact delta; invalid quads are rejected atomically.
        for corner in [edge, neighbor] {
            next.corners[corner] = [original[corner][0] + delta[0], original[corner][1] + delta[1]];
        }
        if next.corners == guide.corners {
            return;
        }
        if validate_guides(std::slice::from_ref(&next)).is_ok() {
            if let Some(guide) = self.guides.iter_mut().find(|g| g.id == id) {
                *guide = next;
            }
            self.invalidate();
        }
    }

    fn drag_to(&mut self, p: [f64; 2], keys: Keys) {
        match self.drag.clone() {
            Some(Drag::New { start, .. }) => {
                self.drag = Some(Drag::New { start, current: p });
            }
            Some(Drag::Corner { id, corner, offset, start, axis, local_axes, local_axis, activation_distance }) => {
                let proposed = [p[0] + offset[0], p[1] + offset[1]];
                let delta = [proposed[0] - start[0], proposed[1] - start[1]];
                // Ctrl remains the image-axis lock, including when both modifiers are held.
                // Shift uses the original adjacent edge directions.
                let (delta, axis, local_axis) = if keys.ctrl {
                    let (d, a) = axis_restrict(delta, true, axis, activation_distance);
                    (d, a, None)
                } else if keys.shift {
                    let (d, a) = local_axes.map_or(([0.0, 0.0], None), |axes| axes.restrict(delta, local_axis, activation_distance));
                    (d, None, a)
                } else {
                    (delta, None, None)
                };
                self.move_corner(id, corner, [start[0] + delta[0], start[1] + delta[1]]);
                self.drag = Some(Drag::Corner { id, corner, offset, start, axis, local_axes, local_axis, activation_distance });
            }
            Some(Drag::Edge { id, edge, mouse_start, original, axis, activation_distance }) => {
                let raw = [p[0] - mouse_start[0], p[1] - mouse_start[1]];
                let (delta, axis) = axis_restrict(raw, keys.ctrl, axis, activation_distance);
                self.translate_edge(id, edge, original, delta);
                self.drag = Some(Drag::Edge { id, edge, mouse_start, original, axis, activation_distance });
            }
            None => {}
        }
    }

    fn delete_selected(&mut self) {
        let before = self.guides.len();
        self.guides.retain(|g| Some(g.id) != self.selected);
        self.selected = self.guides.last().map(|g| g.id);
        self.drag = None;
        if self.guides.len() != before {
            self.invalidate();
        }
    }
}

/// Whether the session still belongs to the active document, as the same snapshot, under the colour display it opened on.
pub(crate) fn bound_to_active_document(app: &PhotocraftApp, s: &GlobalSession) -> bool {
    app.session.active().is_some_and(|st| st.doc.id == s.document && Arc::ptr_eq(&st.doc, &s.source_doc))
        && Screen::key(app, &s.source_doc, None) == s.display_key
}

/// The raw mesh and the document with its source layer hidden must use the same admission rule.
/// Other canvases keep the committed document when their monitor needs a color conversion.
pub(crate) fn preview_visible(app: &PhotocraftApp, idx: usize, output: Option<u32>) -> bool {
    let Some(s) = app.distort.global_perspective.as_ref() else { return false };
    if app.ui.view.pattern_preview || app.session.documents().get(idx).is_none_or(|st| st.doc.id != s.document) || !bound_to_active_document(app, s) {
        return false;
    }
    let screen = Screen::of(app, &s.source_doc, output);
    !screen.encode_srgb && screen.transform.is_none()
}

/// Why the open session cannot act now, or the session's reason to stay editable.
fn validate_session(app: &PhotocraftApp) -> Result<(), String> {
    let s = app.distort.global_perspective.as_ref().ok_or(tl!("Global Perspective Warp is not active"))?;
    let st = app.session.active().ok_or(tl!("no document open"))?;
    if st.doc.id != s.document {
        return Err(tl!("Global Perspective Warp belongs to another document").into());
    }
    if !Arc::ptr_eq(&st.doc, &s.source_doc) {
        return Err(tl!("The source document changed; cancel and reopen Global Perspective Warp to avoid a stale commit").into());
    }
    if !bound_to_active_document(app, s) {
        return Err(tl!("The source or color display changed; cancel and reopen Global Perspective Warp").into());
    }
    validate_target(&app.session, s.layer)?;
    Ok(())
}

/// Whether the session takes the primary pointer on the canvas of document `idx`: it is open, belongs to the active
/// document as the same snapshot, and `idx` is that document. A stale session returns false, so the selected tool works.
pub(crate) fn owns_pointer(app: &PhotocraftApp, idx: usize) -> bool {
    app.session.active_index() == Some(idx) && app.distort.global_perspective.as_ref().is_some_and(|s| bound_to_active_document(app, s))
}

/// Whether a drag is in progress, for the tool host's move filter.
pub(crate) fn dragging(app: &PhotocraftApp) -> bool {
    app.distort.global_perspective.as_ref().is_some_and(|s| s.drag.is_some())
}

fn fail(app: &mut PhotocraftApp, error: String) -> String {
    app.ui.status = error.clone();
    app.ui.status_error = true;
    error
}

/// Starts Global Perspective Warp on the active layer.
pub fn begin(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<(), String> {
    if app.ui.transform.is_some() || app.ui.symmetry_transform.is_some() || app.distort.active() || crate::distort_ui::modal_workspace(app).is_some() {
        return Err(tl!("finish or cancel the current transformation first").into());
    }
    if app.ui.view.pattern_preview {
        return Err(tl!("Turn off Pattern Preview before starting Global Perspective Warp").into());
    }
    let (layer, surface, _) = crate::distort_ui::active_pixels(app)?;
    let bounds = validate_target(&app.session, layer)?;
    let st = app.session.active().ok_or(tl!("no document"))?;
    let l = st.doc.layer(layer).ok_or(tl!("no layer"))?;
    // The mesh uploads raw RGBA8 without the document-to-monitor LUT or HDR data.
    // Reject incompatible previews instead of changing colors on open/commit.
    if st.doc.mode != photocraft_doc::ColorMode::Rgb || st.doc.depth != photocraft_doc::SampleType::U8 {
        return Err(tl!("Global Perspective Warp preview currently requires an 8-bit RGB document; use a converted copy").into());
    }
    let screen = Screen::of(app, &st.doc, None);
    if screen.encode_srgb || screen.transform.is_some() {
        return Err(
            tl!("Global Perspective Warp preview cannot reproduce this document/monitor color transform; use an sRGB display or a converted copy").into()
        );
    }
    // A raw mesh cannot render layer styles: removing the original would also
    // remove its shadow/stroke, while commit retains those effects. Fail closed
    // rather than showing a preview that disagrees with the committed image.
    if l.effects.enabled && (l.effects.items.iter().any(photocraft_doc::Effect::enabled) || l.effects.psd_raw.is_some()) {
        return Err(tl!("Global Perspective Warp preview cannot represent active layer effects; disable or rasterize the effects first").into());
    }
    // The preview is an ordinary alpha-blended mesh ABOVE the composited canvas.
    // It cannot reproduce non-Normal blending, clipping, advanced blending, nested
    // groups or foreground layers painted over the source at commit time.
    let path = st.doc.path_of(layer).ok_or(tl!("no target layer path"))?;
    let index = path.first().copied().ok_or(tl!("no target layer path"))?;
    if !l.visible
        || l.blend != photocraft_doc::BlendMode::Normal
        || l.clipped
        || l.excluded_channels != 0
        || !l.blend_if.is_default()
        || !l.advanced.is_default()
        || path.len() != 1
        || st.doc.layers.iter().skip(index.saturating_add(1)).any(|above| above.visible)
    {
        return Err(tl!("Global Perspective Warp preview requires a visible topmost raster layer with Normal blending and no clipping or advanced blending; flatten incompatible layers first").into());
    }
    let source_doc = st.doc.clone();
    let document = st.doc.id;
    let preview_doc = crate::distort_ui::without_layer(&st.doc, layer);
    let opacity = l.opacity * l.fill_opacity;
    let display_key = Screen::key(app, &source_doc, None);
    let key = app.ui.alloc_id();
    let texture = ctx.load_texture(format!("global-perspective-{key}"), crate::distort_ui::surface_image(&surface, bounds, 2048), egui::TextureOptions::LINEAR);
    app.distort.global_perspective = Some(GlobalSession {
        key,
        document,
        layer,
        source_doc,
        preview_doc,
        bounds,
        guides: Vec::new(),
        options: Options::default(),
        selected: None,
        solution: None,
        show_result: false,
        drag: None,
        texture,
        opacity,
        display_key,
        grid_size: GRID,
        meshes: Vec::new(),
    });
    app.ui.status = tl!("Global Perspective Warp: draw independent guides on doors and windows, then rectify the whole photo").into();
    app.ui.status_error = false;
    Ok(())
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Control {
    guides: Option<Vec<Guide>>,
    options: Option<Options>,
    mode: Option<String>,
    selected_guide: Option<u64>,
    #[serde(default)]
    solve: bool,
    #[serde(default)]
    commit: bool,
    #[serde(default)]
    cancel: bool,
}

/// Control channel: `edit.perspectiveWarpGlobal {"ui": {...}}` while the mode is active. Everything is validated, and
/// the solution computed, before the session is touched, so a refused request leaves the preview as it was.
pub fn control(app: &mut PhotocraftApp, value: &Value) -> Result<Value, String> {
    if value.get("guides").and_then(Value::as_array).is_some_and(|a| a.len() > MAX_GUIDES) {
        return Err(tl!("too many guides (maximum 128)").into());
    }
    let p: Control = serde_json::from_value(value.clone()).map_err(|e| format!("invalid global perspective UI parameters: {e}"))?;
    if (p.commit || p.cancel) && value.as_object().is_none_or(|v| v.len() != 1) {
        return Err(tl!("commit or cancel must be a standalone action").into());
    }
    if p.cancel {
        cancel(app);
        return Ok(json!({"cancelled":true}));
    }
    validate_session(app)?;
    if p.commit {
        return commit(app);
    }
    if p.mode.as_ref().is_some_and(|m| m != "layout" && m != "result") {
        return Err(tl!("mode must be layout|result").into());
    }
    let s = app.distort.global_perspective.as_mut().ok_or(tl!("Global Perspective Warp is not active"))?;
    let replaced = p.guides.is_some() || p.options.is_some();
    let guides = p.guides.unwrap_or_else(|| s.guides.clone());
    let options = p.options.unwrap_or_else(|| s.options.clone());
    validate_guides(&guides)?;
    validate_options(&options)?;
    let selected = match p.selected_guide {
        Some(id) if guides.iter().any(|g| g.id == id) => Some(id),
        Some(_) => return Err(tl!("selectedGuide does not exist").into()),
        None => s.selected.filter(|id| guides.iter().any(|g| g.id == *id)).or_else(|| guides.last().map(|g| g.id)),
    };
    let should_solve = p.solve || p.mode.as_deref() == Some("result");
    // Solve and validate all candidate data before mutating the existing session.
    let solution = if should_solve {
        Some(solve(&guides, s.bounds, &options)?)
    } else if replaced {
        None
    } else {
        s.solution.clone()
    };
    let show_result = if p.mode.as_deref() == Some("layout") {
        false
    } else if should_solve {
        true
    } else if replaced {
        false
    } else {
        s.show_result
    };
    // Compute the safe tessellation before modifying the current session.
    // Never display a coarse mesh when its affine UVs diverge from the solver.
    let geometry_changed = replaced || should_solve || show_result != s.show_result;
    let grid_size = if !geometry_changed {
        s.grid_size
    } else if show_result {
        preview_grid_size(s.bounds, &solution.as_ref().ok_or(tl!("no current solution"))?.homography())?
    } else {
        GRID
    };
    s.guides = guides;
    s.options = options;
    s.selected = selected;
    s.solution = solution;
    s.show_result = show_result;
    s.grid_size = grid_size;
    s.drag = None;
    if geometry_changed {
        s.invalidate_mesh();
    }
    let described = s.describe();
    let status = status_line(s);
    if let Some(status) = status {
        app.ui.status = status;
        app.ui.status_error = false;
    }
    Ok(described)
}

/// The solver's diagnostics as one status line: used / enabled guides, the worst angle, and the rejected guides.
fn status_line(s: &GlobalSession) -> Option<String> {
    let sol = s.solution.as_ref()?;
    let mut line = format!(
        "{}/{} | {:.2} deg | {}",
        sol.used_guide_ids.len(),
        s.guides.iter().filter(|g| g.enabled).count(),
        sol.max_used_error_deg,
        tl!("One global transform; guides are not image regions")
    );
    if !sol.rejected_guide_ids.is_empty() {
        line.push_str(&format!(" | {}: {:?}", tl!("Inconsistent guides"), sol.rejected_guide_ids));
    }
    Some(line)
}

/// Ends the session without touching the document or its history. The preview texture goes with it.
pub fn cancel(app: &mut PhotocraftApp) {
    app.distort.global_perspective = None;
}

/// Commits the session: one `edit.perspectiveWarpGlobal` step with the guides. A session with no current solution, or
/// one that is stale, stays open and nothing runs; an engine refusal keeps the session so its guides can be fixed.
pub fn commit(app: &mut PhotocraftApp) -> Result<Value, String> {
    validate_session(app).map_err(|e| fail(app, e))?;
    let s = app.distort.global_perspective.take().ok_or(tl!("Global Perspective Warp is not active"))?;
    if s.solution.is_none() {
        app.distort.global_perspective = Some(s);
        return Err(fail(app, tl!("Rectify globally before committing; the guides have no current solution").into()));
    }
    let p = json!({"document":s.document.0,"layer":s.layer.0,"guides":s.guides,"options":s.options});
    match app.run(COMMAND, p) {
        Ok(result) => Ok(json!({"committed":result["changed"]==true,"result":result})),
        Err(error) => {
            app.distort.global_perspective = Some(s);
            Err(fail(app, error))
        }
    }
}

/// The reason command `id` cannot start while the session is open, if it is one that would strand or invalidate it.
/// The session's own controls, undo and navigation stay available; Pattern Preview cannot tile the raw mesh.
pub(crate) fn refuses(app: &PhotocraftApp, id: &str) -> Option<String> {
    app.distort.global_perspective.as_ref()?;
    if id == "view.patternPreview" {
        return Some(tl!("Finish or cancel Global Perspective Warp before changing Pattern Preview").into());
    }
    let blocked = matches!(
        id,
        "edit.freeTransform"
            | "edit.freeTransformCopy"
            | "layer.smartObjects.warp"
            | "edit.contentAwareScale"
            | "edit.puppetWarp"
            | "layer.smartObjects.puppetWarp"
            | "edit.perspectiveWarp"
            | "layer.smartObjects.perspectiveWarp"
            | "filter.liquify"
            | "filter.filterGallery"
            | "filter.vanishingPoint"
            | "file.placeEmbedded"
            | "file.placeLinked"
    ) || (id.starts_with("edit.transform.") && id != "edit.transform.again");
    blocked.then(|| tl!("Finish or cancel Global Perspective Warp before starting another transformation").to_string())
}

// ------------------------------------------------------------------------------------------------ pointer

fn can_create_guide(s: &GlobalSession, p: [f64; 2]) -> bool {
    !s.show_result
        && s.guides.len() < MAX_GUIDES
        && p[0] >= f64::from(s.bounds.x0)
        && p[0] <= f64::from(s.bounds.x1)
        && p[1] >= f64::from(s.bounds.y0)
        && p[1] <= f64::from(s.bounds.y1)
}

/// A press at document point `p`, `tol` document units from a handle being a hit. True when it began a drag; the press is
/// the session's either way (a click selects a guide, a click on the result starts nothing).
pub(crate) fn pointer_down(app: &mut PhotocraftApp, p: [f64; 2], tol: f64, keys: Keys) -> bool {
    if let Err(e) = validate_session(app) {
        fail(app, e);
        return false;
    }
    let Some(s) = app.distort.global_perspective.as_mut() else { return false };
    // Any earlier grab ended with its release; a leftover one (a release the host never delivered) is dropped.
    s.drag = None;
    let (x, y) = (p[0], p[1]);
    if let Some(handle) = hit_handle(s, p, tol) {
        let id = match handle {
            Handle::Corner { id, .. } | Handle::Edge { id, .. } => id,
        };
        s.selected = Some(id);
        if !s.show_result {
            match handle {
                Handle::Corner { id, corner } => {
                    let snapshot = s
                        .guides
                        .iter()
                        .find(|g| g.id == id)
                        .and_then(|g| g.corners.get(corner).map(|start| (*start, LocalAxes::at_corner(&g.corners, corner))));
                    if let Some((start, local_axes)) = snapshot {
                        s.drag = Some(Drag::Corner {
                            id,
                            corner,
                            offset: [start[0] - x, start[1] - y],
                            start,
                            axis: None,
                            local_axes,
                            local_axis: None,
                            activation_distance: tol * (AXIS_ACTIVATION_POINTS / HIT_POINTS),
                        });
                    }
                }
                Handle::Edge { id, edge } => {
                    let original = s.guides.iter().find(|g| g.id == id).map(|g| g.corners);
                    if let Some(original) = original {
                        s.drag = Some(Drag::Edge {
                            id,
                            edge,
                            mouse_start: p,
                            original,
                            axis: None,
                            activation_distance: tol * (AXIS_ACTIVATION_POINTS / HIT_POINTS),
                        });
                    }
                }
            }
        }
    } else if !keys.shift
        && let Some(g) = s.guides.iter().rev().find(|g| s.display_corners(g).is_some_and(|corners| inside(p, &corners)))
    {
        s.selected = Some(g.id);
    } else if can_create_guide(s, p) {
        s.drag = Some(Drag::New { start: p, current: p });
    }
    s.drag.is_some()
}

/// The pointer moved with the button down.
pub(crate) fn pointer_move(app: &mut PhotocraftApp, p: [f64; 2], keys: Keys) {
    if let Some(s) = app.distort.global_perspective.as_mut() {
        s.drag_to(p, keys);
    }
}

/// The button came up at `p`: the final sample counts, with the modifiers held now.
pub(crate) fn pointer_up(app: &mut PhotocraftApp, p: [f64; 2], keys: Keys) {
    let Some(s) = app.distort.global_perspective.as_mut() else { return };
    if let Some(Drag::New { start, .. }) = s.drag.clone() {
        s.drag = None;
        let end = [p[0].clamp(f64::from(s.bounds.x0), f64::from(s.bounds.x1)), p[1].clamp(f64::from(s.bounds.y0), f64::from(s.bounds.y1))];
        let (x0, y0, x1, y1) = (start[0].min(end[0]), start[1].min(end[1]), start[0].max(end[0]), start[1].max(end[1]));
        if x1 - x0 > 4.0 && y1 - y0 > 4.0 && s.guides.len() < MAX_GUIDES {
            let Some(id) = (1..=(MAX_GUIDES as u64 + 1)).find(|id| s.guides.iter().all(|g| g.id != *id)) else { return };
            s.guides.push(Guide { id, corners: [[x0, y0], [x1, y0], [x1, y1], [x0, y1]], enabled: true, weight: 1.0 });
            s.selected = Some(id);
            s.invalidate();
        }
    } else {
        // Apply the final cursor sample with the active modifier state before releasing the grab.
        s.drag_to(p, keys);
        s.drag = None;
    }
}

/// The host ended the gesture without a release (Escape, a lost pointer, another tool or a navigation gesture took it):
/// the last accepted guide coordinates stay, the grab is released, and an unfinished new guide is discarded.
pub(crate) fn pointer_cancel(app: &mut PhotocraftApp) {
    if let Some(s) = app.distort.global_perspective.as_mut() {
        s.drag = None;
    }
}

/// Deletes the selected guide (the Delete key).
pub(crate) fn delete_selected(app: &mut PhotocraftApp) -> bool {
    match app.distort.global_perspective.as_mut() {
        Some(s) if s.selected.is_some() => {
            s.delete_selected();
            true
        }
        _ => false,
    }
}

/// What the pointer is over at document point `p`, for the cursor.
pub(crate) fn hover(app: &PhotocraftApp, p: [f64; 2], tol: f64) -> Hover {
    let Some(s) = app.distort.global_perspective.as_ref() else { return Hover::Idle };
    if s.drag.is_some() {
        return Hover::Grabbing;
    }
    if s.show_result {
        return Hover::Idle;
    }
    match hit_handle(s, p, tol) {
        Some(_) => Hover::Handle,
        None if can_create_guide(s, p) => Hover::Draw,
        None => Hover::Idle,
    }
}

// ------------------------------------------------------------------------------------------------ canvas

/// The preview and the guides as shapes in the canvas's picture slot, for the canvas of document `idx` showing it through
/// `view` on `output` (`None`: the main display). Foreign/stale sessions and incompatible displays or repeating views
/// show nothing and keep the source layer in the base composite.
pub(crate) fn canvas_shapes(app: &mut PhotocraftApp, ctx: &egui::Context, view: &CanvasView, idx: usize, output: Option<u32>) -> Vec<Shape> {
    if !preview_visible(app, idx, output) {
        return Vec::new();
    }
    let Some(s) = app.distort.global_perspective.as_mut() else { return Vec::new() };
    let mut shapes = vec![Shape::mesh(s.preview_mesh(view))];
    let t = crate::theme::Tokens::get(ctx);
    let scr = |p: [f64; 2]| view.to_screen(Point::new(p[0], p[1]));
    for g in &s.guides {
        let rejected = s.solution.as_ref().is_some_and(|sol| sol.rejected_guide_ids.contains(&g.id));
        let color = if !g.enabled {
            t.text_faint
        } else if rejected {
            t.warning
        } else {
            t.accent
        };
        let Some(corners) = s.display_corners(g) else { continue };
        let pts: Vec<Pos2> = corners.iter().map(|p| scr(*p)).collect();
        if pts.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
            continue;
        }
        let selected = s.selected == Some(g.id);
        shapes.push(Shape::closed_line(pts.clone(), Stroke::new(if selected { 2.5 } else { 1.3 }, color)));
        // A guide the solver rejected is crossed out, so the state is not carried by the colour alone.
        if rejected {
            shapes.push(Shape::line_segment([pts[0], pts[2]], Stroke::new(1.0, color)));
            shapes.push(Shape::line_segment([pts[1], pts[3]], Stroke::new(1.0, color)));
        }
        for p in &pts {
            shapes.push(Shape::circle_filled(*p, 4.5, t.card));
            shapes.push(Shape::circle_stroke(*p, 4.5, Stroke::new(1.4, color)));
        }
    }
    // A thin, theme-coloured rail makes the chosen perspective direction visible.
    // The view handles zoom, canvas rotation, mirroring and display pixel aspect.
    if let Some((anchor, direction)) = s.active_local_axis() {
        let origin = scr(anchor);
        let vector = scr([anchor[0] + direction[0], anchor[1] + direction[1]]) - origin;
        if vector.length_sq().is_finite() && vector.length_sq() > f32::EPSILON {
            let reach = vector.normalized() * view.rect.size().length();
            shapes.push(Shape::line_segment([origin - reach, origin + reach], Stroke::new(1.0, t.accent.gamma_multiply(0.55))));
        }
    }
    if let Some(Drag::New { start, current }) = &s.drag {
        let (a, b) = (*start, *current);
        let pts = [[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]].map(scr).to_vec();
        shapes.push(Shape::closed_line(pts, Stroke::new(1.0, t.accent)));
    }
    shapes
}

// ------------------------------------------------------------------------------------------------ options bar

/// The options bar's members: the mode, Rectify, the guide list and its settings, and the solver's diagnostics. The bar's
/// own commit and cancel buttons (`panels::options_bar::End::GlobalPerspective`) follow them. A session that no longer
/// matches the document shows only why; its Cancel stays.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let stale = app.distort.global_perspective.as_ref().is_some_and(|s| !bound_to_active_document(app, s));
    if stale {
        Text::new(Label::from(tl!("The source or color display changed; cancel and reopen Global Perspective Warp"))).show(ui);
        return;
    }
    let Some(s) = app.distort.global_perspective.as_mut() else { return };
    let mut do_solve = false;
    let mut changed = false;
    Text::new(Label::from(tl!("Ctrl: image axes; Shift: perspective axes (corners)"))).show(ui);
    if Button::new(tl!("Edit guides")).show(ui).clicked() && s.show_result {
        s.show_result = false;
        s.grid_size = GRID;
        s.invalidate_mesh();
    }
    if Button::new(tl!("Rectify globally")).show(ui).clicked() {
        do_solve = true;
    }
    let entries: Vec<Entry<String>> = s.guides.iter().map(|g| Entry::new(g.id.to_string(), format!("#{}", g.id))).collect();
    let shown = s.selected.map(|id| id.to_string()).unwrap_or_default();
    if let Some(picked) = Select::new(tl!("Guide:"), Bind::given(shown), &entries).show(ui).events.committed().cloned() {
        s.selected = picked.parse().ok().filter(|id| s.guides.iter().any(|g| g.id == *id));
    }
    if let Some(g) = s.guides.iter_mut().find(|g| Some(g.id) == s.selected) {
        let mut on = g.enabled;
        Checkbox::new(Bind::bound(&mut on)).label(tl!("Enabled").to_owned()).show(ui);
        if on != g.enabled {
            g.enabled = on;
            changed = true;
        }
        let mut weight = g.weight;
        SliderField::new(tl!("Guide weight"), Bind::bound(&mut weight), 0.05..=20.0).show(ui);
        if weight != g.weight {
            g.weight = weight;
            changed = true;
        }
    }
    if Button::new(tl!("Delete guide")).show(ui).clicked() {
        s.delete_selected();
    }
    let mut fit = s.options.fit_canvas;
    Checkbox::new(Bind::bound(&mut fit)).label(tl!("Fit whole photo").to_owned()).show(ui);
    if fit != s.options.fit_canvas {
        s.options.fit_canvas = fit;
        changed = true;
    }
    let mut aspect = s.options.aspect;
    SliderField::new(tl!("Global aspect"), Bind::bound(&mut aspect), 0.25..=4.0).show(ui);
    if aspect != s.options.aspect {
        s.options.aspect = aspect;
        changed = true;
    }
    if changed {
        s.invalidate();
    }
    if let Some(line) = status_line(s) {
        Text::new(Label::from(line)).show(ui);
    }
    if do_solve && let Err(e) = control(app, &json!({"solve":true})) {
        fail(app, e);
    }
}
