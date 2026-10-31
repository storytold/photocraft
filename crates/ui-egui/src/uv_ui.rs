//! Optional native 3D / UV workspace. All edits dispatch engine commands.
use crate::{PhotocraftApp, theme::Tokens};
use egui::{Color32, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, pos2, vec2};
use photocraft_uv::preview::{self, Camera, Channel, Image};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{Duration, Instant};

/// View-only navigation preference, independent of texture and mesh editing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrbitPivot {
    #[default]
    ViewCenter,
    Cursor,
    Fixed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionTarget {
    #[default]
    UvShells,
    Polygons,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionDepth {
    #[default]
    VisibleOnly,
    ThroughMesh,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionGesture {
    #[default]
    Rectangle,
    Lasso,
}

#[derive(Clone, Copy)]
struct FillDragState {
    start: [f64; 2],
    bounds: [i32; 4],
    rotation: f32,
    handle: Option<usize>,
}
impl OrbitPivot {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::ViewCenter => tl!("View center → surface"),
            Self::Cursor => tl!("Under cursor"),
            Self::Fixed => tl!("Fixed"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub open: bool,
    pub show_overlay: bool,
    pub show_wireframe: bool,
    pub wireframe_opacity: f32,
    pub wireframe_color: [u8; 4],
    #[serde(skip)]
    pub wireframe_picker_open: bool,
    pub select_shells: bool,
    #[serde(skip)]
    pub selection_target: SelectionTarget,
    /// Temporary wireframe request for the current polygon-selection interaction.
    #[serde(skip)]
    pub temporary_polygon_wireframe: bool,
    #[serde(skip)]
    pub selection_depth: SelectionDepth,
    #[serde(skip)]
    pub selection_gesture: SelectionGesture,
    #[serde(skip)]
    pub selected_polygons: std::collections::BTreeSet<usize>,
    /// Texture document whose UV space owns the transient polygon selection.
    #[serde(skip)]
    pub polygon_document: Option<photocraft_doc::DocId>,
    pub paint_wrap: bool,
    #[serde(skip)]
    pub picker_3d: bool,
    #[serde(skip)]
    pub picker_viewport: Option<egui::ViewportId>,
    pub model_path: String,
    pub texture_path: String,
    pub project_path: String,
    pub uv_set: usize,
    pub guard: f64,
    pub horizontal: f64,
    pub vertical: f64,
    pub feather: f64,
    pub padding: f64,
    pub open_borders: bool,
    pub shaded: bool,
    pub channel: Channel,
    pub channel_2d: Channel,
    pub shell_enclosed: bool,
    pub view_3d: bool,
    pub split_view: bool,
    pub swap_views: bool,
    pub split_ratio: f32,
    pub floating_3d: bool,
    pub floating_2d: bool,
    pub import_settings: bool,
    pub perspective: bool,
    pub antialias: bool,
    pub fast_navigation: bool,
    pub orbit_pivot: OrbitPivot,
    pub cull_backfaces: bool,
    pub flip_faces: bool,
    pub lighting: photocraft_uv::lighting::Settings,
    pub normal_strength: f64,
    pub flip_normal_y: bool,
    pub pan: [f64; 2],
    pub target: Option<[f64; 3]>,
    pub yaw: f64,
    pub pitch: f64,
    pub zoom: f64,
    pub status: String,
    pub overlap: Vec<usize>,
}
impl Default for State {
    fn default() -> Self {
        let c = Camera::default();
        Self {
            open: false,
            show_overlay: true,
            show_wireframe: false,
            wireframe_opacity: 0.75,
            wireframe_color: [240, 240, 240, 255],
            wireframe_picker_open: false,
            select_shells: false,
            selection_target: SelectionTarget::UvShells,
            temporary_polygon_wireframe: false,
            selection_depth: SelectionDepth::VisibleOnly,
            selection_gesture: SelectionGesture::Rectangle,
            selected_polygons: std::collections::BTreeSet::new(),
            polygon_document: None,
            paint_wrap: false,
            picker_3d: false,
            picker_viewport: None,
            model_path: String::new(),
            texture_path: String::new(),
            project_path: String::new(),
            uv_set: 0,
            guard: 0.0,
            horizontal: 0.0,
            vertical: 0.0,
            feather: 0.0,
            padding: 3.0,
            open_borders: false,
            shaded: true,
            channel: Channel::Material,
            channel_2d: Channel::Color,
            shell_enclosed: false,
            view_3d: false,
            split_view: false,
            swap_views: false,
            split_ratio: 0.5,
            floating_3d: false,
            floating_2d: false,
            import_settings: false,
            perspective: false,
            antialias: true,
            fast_navigation: true,
            orbit_pivot: OrbitPivot::default(),
            cull_backfaces: true,
            flip_faces: false,
            lighting: Default::default(),
            normal_strength: 1.0,
            flip_normal_y: false,
            pan: [0.0; 2],
            target: None,
            yaw: c.yaw,
            pitch: c.pitch,
            zoom: c.zoom,
            status: String::new(),
            overlap: Vec::new(),
        }
    }
}

fn polygon_selection_wireframe_active(state: &State) -> bool {
    state.select_shells && state.selection_target == SelectionTarget::Polygons && state.temporary_polygon_wireframe
}

fn wireframe_visible(state: &State, snap_pending: bool) -> bool {
    state.show_wireframe || snap_pending || polygon_selection_wireframe_active(state)
}

fn update_polygon_wireframe_for_mode(state: &mut State, previous_target: SelectionTarget) {
    if !state.select_shells || state.selection_target != SelectionTarget::Polygons {
        state.temporary_polygon_wireframe = false;
    } else if previous_target != SelectionTarget::Polygons {
        state.temporary_polygon_wireframe = true;
    }
}

pub(crate) fn begin_polygon_selection_wireframe(state: &mut State) {
    if state.select_shells && state.selection_target == SelectionTarget::Polygons {
        state.temporary_polygon_wireframe = true;
    }
}

fn finish_polygon_selection_wireframe(state: &mut State) {
    state.temporary_polygon_wireframe = false;
}

fn cancel_temporary_wireframes_for_gray_mask(state: &mut State, ctx: &egui::Context, gray_mask_view: bool, polygon_gesture_active: bool) {
    if gray_mask_view {
        if !polygon_gesture_active {
            finish_polygon_selection_wireframe(state);
        }
        crate::uv_symmetry::cancel_pending_snap(ctx);
    }
}

impl State {
    fn camera(&self) -> Camera {
        Camera {
            perspective: self.perspective,
            yaw: self.yaw,
            pitch: self.pitch,
            zoom: self.zoom,
            pan: self.pan,
            target: self.target,
            shaded: self.shaded,
            channel: self.channel,
            cull_backfaces: self.cull_backfaces,
            flip_faces: self.flip_faces,
            authored_normals: true,
            normal_strength: self.normal_strength,
            flip_normal_y: self.flip_normal_y,
        }
    }
}

#[derive(Default)]
pub(crate) struct Cache {
    pub(crate) gpu: Option<crate::uv_gpu::Renderer>,
    pub(crate) library: crate::uv_library::Cache,
    pub(crate) project: crate::uv_project::Ui,
    pub(crate) workspace_views: std::collections::HashMap<photocraft_doc::DocId, crate::state::View>,
    pub(crate) parked_dock: Option<ParkedDock>,
    masked: std::collections::BTreeSet<usize>,
    pub(crate) lighting: crate::uv_lighting::Cache,
    geometry: Option<(std::sync::Arc<photocraft_uv::Scene>, std::sync::Arc<photocraft_uv::prepared::Geometry>)>,
    textures: Vec<Option<Image>>,
    // Unmasked source used to reapply a transient 3D mask overlay without recompositing color.
    mask_base: Vec<Option<Image>>,
    normals: Vec<Option<Image>>,
    opacity: Vec<Option<Image>>,
    opacity_keys: Vec<(u64, u64, u64)>,
    normal_keys: Vec<(u64, u64, u64)>,

    keys: Vec<(u64, u64, u64, u64)>,
    snapshots: Vec<Option<std::sync::Arc<photocraft_doc::Document>>>,
    normal_snapshots: Vec<Option<std::sync::Arc<photocraft_doc::Document>>>,
    frame: Option<preview::Frame>,
    frame_handle: Option<TextureHandle>,
    rendered: Option<(u64, Camera, usize, usize, bool)>,
    refreshed: Option<Instant>,
    gpu_mask_preview_was_active: bool,
    gpu_mask_preview_flush_pending: bool,
    gray_mask_shells: Option<(u64, [u32; 2], Vec<usize>, std::sync::Arc<Vec<u32>>)>,
    texture_side: usize,
    navigation: Option<Navigation>,
    wheel_active: Option<Instant>,
    display_channel: Channel,
    pub(crate) viewport: Option<Rect>,
    pub(crate) viewport_id: Option<egui::ViewportId>,
    pub(crate) area: Option<Rect>,
    pub(crate) shell_tool_owner: Option<crate::state::Tool>,
    paint: crate::uv_paint::State,
    shell_box_3d: Option<crate::uv_box::Drag>,
    shell_box_2d: Option<crate::uv_box::Drag>,
    pub(crate) mask_settings_pending: bool,
    source_checked: Option<Instant>,
    source_generation: u64,
    source_message: String,
    // Immutable slot previews: keyed by source document, never by its live paint revision.
    thumbnails: std::collections::BTreeMap<(u64, bool), TextureHandle>,
    texture_targets: Vec<(Rect, usize, Channel)>,
    texture_target_frame: u64,
}

/// UV workspace dock state parked while a built-in non-UV preset is active.
/// The regular preset transition still resets its own layout; this snapshot is used only when
/// returning to 3D, whose native UV panel layout belongs to the isolated UV workspace.
#[derive(Clone)]
pub(crate) struct ParkedDock {
    pub(crate) dock: crate::dock::DockLayout,
    pub(crate) docking: Option<craft_ui::docking::Workspace<String>>,
    pub(crate) returns: std::collections::BTreeMap<String, craft_ui::docking::Location<String>>,
    pub(crate) kept: Option<serde_json::Map<String, serde_json::Value>>,
    pub(crate) collapsed: Vec<String>,
    pub(crate) tabs: crate::state::DockTabs,
    pub(crate) panels: crate::state::Panels,
    pub(crate) dock_visible: bool,
    pub(crate) uv_open: bool,
}

#[cfg(test)]
impl Cache {
    pub(crate) fn paint_for_test(&mut self, state: crate::uv_paint::State) {
        self.paint = state;
    }
}

pub(crate) fn pending_paint(app: &PhotocraftApp) -> bool {
    crate::uv_paint::pending(&app.uv_cache.paint)
}

pub(crate) fn finish_paint(app: &mut PhotocraftApp) {
    let mut paint = std::mem::take(&mut app.uv_cache.paint);
    crate::uv_paint::finish(app, &mut paint);
    app.uv_cache.paint = paint;
}
pub(crate) fn reimport(app: &mut PhotocraftApp) {
    finish_paint(app);
    match app.run("uv.model.reimport", json!({"uvSet":app.ui.uv_workbench.uv_set})) {
        Ok(result) => {
            app.ui.uv_workbench.overlap.clear();
            let message: String = if result["uvChanged"] == true {
                tl!("Mesh reimported. UVs changed: check existing seam protection masks.").into()
            } else {
                tl!("Mesh reimported; textures and layers preserved.").into()
            };
            app.ui.uv_workbench.status = if result["uvChanged"] == true { message.clone() } else { String::new() };
            app.ui.status = message;
            app.ui.status_error = false;
            app.uv_cache.source_message.clear();
            app.uv_cache.source_checked = None;
        }
        Err(error) => app.ui.uv_workbench.status = error,
    }
}
fn check_source(app: &mut PhotocraftApp) {
    if app.session.uv.scene.is_none() || app.drag.is_some() || crate::uv_mask_paint_preview::pending(app).is_some() {
        return;
    }
    if app.uv_cache.source_generation == app.session.uv.generation && app.uv_cache.source_checked.is_some_and(|at| at.elapsed() < Duration::from_secs(1)) {
        return;
    }
    app.uv_cache.source_checked = Some(Instant::now());
    app.uv_cache.source_generation = app.session.uv.generation;
    if let Ok(result) = app.run("uv.model.checkSource", json!({})) {
        app.uv_cache.source_message = if result["available"] == false {
            tl!("The source model is unavailable. The current mesh is retained.").into()
        } else if result["changed"] == true {
            tl!("The source model changed — click ReImport").into()
        } else {
            String::new()
        };
    }
}

fn command(app: &mut PhotocraftApp, state: &mut State, id: &str, params: serde_json::Value) -> bool {
    match app.run(id, params) {
        Ok(_) => {
            if id == "uv.layer.create"
                && let Err(e) = app.run("channel.target.composite", json!({}))
            {
                state.status = e;
                return false;
            }
            state.status = String::new();
            true
        }
        Err(e) => {
            state.status = e.to_string();
            false
        }
    }
}

pub(crate) fn patch_image(target: &mut Option<Image>, size: [u32; 2], rect: photocraft_geom::Rect, pixels: Vec<[f32; 4]>) -> Result<(), String> {
    let [w, h] = size.map(|v| v as usize);
    if pixels.len() != rect.width() as usize * rect.height() as usize {
        return Err("Invalid UV preview update".into());
    }
    if rect == photocraft_geom::Rect::from_xywh(0, 0, size[0], size[1]) {
        *target = Some(Image { width: w, height: h, rgba: pixels });
        return Ok(());
    }
    let image = target.as_mut().filter(|im| im.width == w && im.height == h).ok_or("Missing UV preview cache")?;
    if rect.is_empty() {
        return Ok(());
    }
    if rect.x0 < 0 || rect.y0 < 0 || rect.x1 as usize > w || rect.y1 as usize > h {
        return Err("UV preview update exceeds image".into());
    }
    for (row, src) in pixels.chunks_exact(rect.width() as usize).enumerate() {
        let start = (rect.y0 as usize + row) * w + rect.x0 as usize;
        image.rgba.get_mut(start..start + src.len()).ok_or("Invalid UV preview cache")?.copy_from_slice(src);
    }
    Ok(())
}
fn region(rect: photocraft_geom::Rect) -> [u32; 4] {
    if rect.is_empty() { [0; 4] } else { [rect.x0 as u32, rect.y0 as u32, rect.width(), rect.height()] }
}
fn preview_refresh_due(
    moving: bool,
    active_preview_gpu: bool,
    mask_surface_preview_gpu: bool,
    opacity_preview: bool,
    settle: bool,
    interval_elapsed: bool,
    preview_ended: bool,
) -> bool {
    !moving && ((active_preview_gpu && !mask_surface_preview_gpu) || preview_ended || ((opacity_preview || active_preview_gpu || !settle) && interval_elapsed))
}
fn preview_interval_elapsed(refreshed: Option<Instant>, now: Instant) -> bool {
    refreshed.is_none_or(|at| now.duration_since(at) >= Duration::from_millis(33))
}
fn should_throttle_mask_surface_preview(
    active_preview_gpu: bool,
    opacity_preview: bool,
    native_surface_stroke: bool,
    workspace_3d: bool,
    mask_target: bool,
) -> bool {
    active_preview_gpu && !opacity_preview && native_surface_stroke && workspace_3d && mask_target
}
fn showing_gray_layer_mask(app: &PhotocraftApp) -> bool {
    app.session
        .active()
        .is_some_and(|st| photocraft_engine::mask_view_cmds::current(st).is_some_and(|view| view.mode == photocraft_engine::mask_view_cmds::MaskViewMode::Gray))
}
fn gray_mask_hides_all_materials(app: &PhotocraftApp, cache: &Cache) -> bool {
    showing_gray_layer_mask(app)
        && app.session.uv.scene.as_ref().is_some_and(|scene| scene.materials.iter().enumerate().all(|(material, _)| cache.masked.contains(&material)))
}
fn gray_mask_gpu_source(app: &mut PhotocraftApp, cache: &mut Cache, scene: &photocraft_uv::Scene) -> Option<crate::uv_gpu::GrayMaskGpuSource> {
    let gpu_brush_active = crate::canvas::native_mask_gpu_brush_active(app);
    let source = gray_mask_gpu_source_inner(app, cache, scene);
    if gpu_brush_active && source.is_none() {
        crate::canvas::fail_native_mask_gpu_brush(app);
    }
    source
}

fn gray_mask_gpu_source_inner(app: &mut PhotocraftApp, cache: &mut Cache, scene: &photocraft_uv::Scene) -> Option<crate::uv_gpu::GrayMaskGpuSource> {
    let index = app.session.active_index()?;
    let (view, revision, doc_id) = {
        let state = app.session.documents().get(index)?;
        (photocraft_engine::mask_view_cmds::current(state)?, state.revision, state.doc.id)
    };
    if view.mode != photocraft_engine::mask_view_cmds::MaskViewMode::Gray || view.filter {
        return None;
    }
    // Use the same layered display snapshot as the canvas. `gpu_display_doc` only covers
    // temporary effect previews; an in-progress native brush stroke is exposed by
    // `canvas::display_doc` and must advance this key before the stroke is committed.
    let (document, live_key) = crate::canvas::display_doc(app, index);
    let mask = document.layer(view.layer)?.mask.as_ref()?;
    if mask.feather > 0.0 {
        return None;
    }
    mask.paint.as_ref()?;
    let size = [document.size.width, document.size.height];
    let materials: Vec<_> = cache.masked.iter().copied().filter(|material| crate::uv_mask::source(app, *material) == Some(index)).collect();
    if materials.is_empty() {
        return None;
    }
    let topology = app.session.uv.topology.clone()?;
    let topology_key = app.session.uv.generation;
    let map =
        cache.gray_mask_shells.as_ref().filter(|(key, map_size, map_materials, _)| *key == topology_key && *map_size == size && *map_materials == materials);
    let shell_ids = if let Some((_, _, _, ids)) = map {
        ids.clone()
    } else {
        let ids = topology.shell_id_map_for_materials(scene, size[0], size[1], &materials, [0, 0]).ok()?;
        let ids = std::sync::Arc::new(ids);
        cache.gray_mask_shells = Some((topology_key, size, materials.clone(), ids.clone()));
        ids
    };
    let view_key = crate::uv_mask::key(app, index);
    let mut owner_hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&doc_id.0, &mut owner_hasher);
    std::hash::Hash::hash(&view.layer.0, &mut owner_hasher);
    let owner_key = std::hash::Hasher::finish(&owner_hasher);
    let mut shell_hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&(topology_key, size, &materials), &mut shell_hasher);
    let shell_key = std::hash::Hasher::finish(&shell_hasher);
    let materials = materials.into_iter().collect();
    let brush_preview = crate::canvas::native_mask_gpu_input(app);
    Some(crate::uv_gpu::GrayMaskGpuSource {
        document,
        layer: view.layer,
        shell_ids,
        owner_key,
        shell_key,
        materials,
        key: (doc_id.0, revision, live_key, view_key ^ topology_key),
        brush_preview,
    })
}
fn should_refresh_surface_maps(app: &PhotocraftApp, cache: &Cache) -> bool {
    !gray_mask_hides_all_materials(app, cache)
}

#[cfg(test)]
fn refresh(app: &mut PhotocraftApp, ctx: &egui::Context, cache: &mut Cache) -> Result<(), String> {
    refresh_with_lighting(app, ctx, cache, photocraft_uv::lighting::Settings::default())
}
fn refresh_with_lighting(app: &mut PhotocraftApp, ctx: &egui::Context, cache: &mut Cache, lighting: photocraft_uv::lighting::Settings) -> Result<(), String> {
    let refresh_started = crate::gpu_canvas::now_ms();
    let mut cpu_compose_ms = 0.0;
    if let Some(index) = app.session.active_index()
        && crate::uv_mask_paint_preview::fill_manipulation_active(app, index)
    {
        // Schedule/poll the shared proxy before lighting keeps full-resolution source maps
        // canonical for this gesture. This also works when the mask is not the 3D display mode.
        let _ = crate::uv_mask_paint_preview::fill_mask_proxy(app, index, ctx);
    }
    // UV hit testing and the brush cursor consult CPU texture alpha. An opacity slider can
    // skip that work while the pointer is elsewhere, but refresh it on demand before the
    // pointer returns to the 3D surface.
    let cpu_pick_needed = crate::uv_mask_paint_preview::active_document(app).is_some()
        && cache.viewport.is_some_and(|viewport| ctx.input(|input| input.pointer.hover_pos().is_some_and(|pos| viewport.contains(pos))));
    let map_kind = if matches!(cache.display_channel, Channel::Material | Channel::Normal) { Channel::Color } else { cache.display_channel };
    let mut bindings = app.session.uv.map_bindings(map_kind).to_vec();
    cache.masked.clear();
    let showing_layer_mask = app.session.active().is_some_and(|st| photocraft_engine::mask_view_cmds::current(st).is_some());
    let showing_gray_mask = showing_gray_layer_mask(app);
    for (material, id) in bindings.iter_mut().enumerate() {
        if let Some(index) = crate::uv_mask::source(app, material) {
            *id = app.session.documents().get(index).map(|st| st.doc.id);
            cache.masked.insert(material);
        } else if showing_layer_mask {
            // A different texture may be bound here, but it is still unavailable for this mask
            // view. Do not show that document's PBR/color map as if it were the active mask.
            *id = None;
            cache.masked.insert(material);
        }
    }
    let gray_mask_hides_materials = gray_mask_hides_all_materials(app, cache);
    let slots = app.session.uv.all_bindings().filter(|id| id.is_some()).count().max(1);
    let gpu_limit = cache.gpu.as_ref().map(|g| g.max_texture_side());
    if let Some(limit) = gpu_limit {
        let mut bytes = 0u64;
        for id in app.session.uv.all_bindings().flatten() {
            if let Some(st) = app.session.documents().iter().find(|st| st.doc.id == *id) {
                if st.doc.size.width > limit || st.doc.size.height > limit {
                    return Err(crate::i18n::fmt(
                        tl!("Source map {width} × {height} exceeds the GPU limit of {limit}. Maps are not downscaled automatically."),
                        &[("width", &st.doc.size.width.to_string()), ("height", &st.doc.size.height.to_string()), ("limit", &limit.to_string())],
                    ));
                }
                bytes = bytes.saturating_add(u64::from(st.doc.size.width) * u64::from(st.doc.size.height) * 16);
            }
        }
        if bytes > 4 * 1024 * 1024 * 1024 {
            return Err(tl!("Source maps require more than 4 GiB of CPU cache. Remove unused material bindings; maps are not downscaled automatically.").into());
        }
    }
    // GPU uses the source size. The small float-map budget belongs only to the CPU fallback.
    let texture_side = gpu_limit.map_or_else(|| (16_777_216.0 / slots as f64).sqrt().floor().clamp(1.0, 2048.0), f64::from);
    let mut textures_changed = cache.texture_side != texture_side as usize;
    if textures_changed {
        cache.keys.fill((0, 0, 0, 0));
        cache.normal_keys.fill((0, 0, 0));
        cache.texture_side = texture_side as usize;
    }
    let opacity_bindings = app.session.uv.map_bindings(Channel::Opacity).to_vec();
    if cache.opacity.len() != bindings.len() {
        cache.opacity = vec![None; bindings.len()];
        cache.opacity_keys = vec![(0, 0, 0); bindings.len()];
    }
    // Every surface is marked as a mask in Gray view, whose shader returns the grayscale map
    // directly; opacity does not participate in that preview.
    for material in (0..bindings.len()).filter(|_| !gray_mask_hides_materials) {
        let index = opacity_bindings.get(material).copied().flatten().and_then(|id| app.session.documents().iter().position(|st| st.doc.id == id));
        let Some(index) = index else {
            if let Some(dst) = cache.opacity.get_mut(material) {
                *dst = None;
            }
            if let Some(key) = cache.opacity_keys.get_mut(material) {
                *key = (0, 0, 0);
            }
            continue;
        };
        let revision = app.session.documents().get(index).map_or(0, |st| st.revision);
        let (doc, preview) = crate::canvas::display_doc(app, index);
        let doc = crate::uv_mask_paint_preview::cpu_document(app, doc, preview);
        let key = (doc.id.0, revision, preview);
        if cache.opacity_keys.get(material) == Some(&key) {
            continue;
        }
        let scale = (texture_side / f64::from(doc.size.width.max(doc.size.height).max(1))).min(1.0);
        let (w, h) = ((f64::from(doc.size.width) * scale).round().max(1.0) as u32, (f64::from(doc.size.height) * scale).round().max(1.0) as u32);
        let buffer = photocraft_compose::render_reduced(&doc, w, h);
        if let Some(dst) = cache.opacity.get_mut(material) {
            *dst = Some(Image { width: w as usize, height: h as usize, rgba: buffer.px });
        }
        if let Some(dst) = cache.opacity_keys.get_mut(material) {
            *dst = key;
        }
    }
    if bindings.len() != cache.textures.len() {
        cache.textures = vec![None; bindings.len()];
        cache.mask_base = vec![None; bindings.len()];

        cache.keys = vec![(0, 0, 0, 0); bindings.len()];
        cache.snapshots = vec![None; bindings.len()];
        textures_changed = true;
    }
    if cache.mask_base.len() != bindings.len() {
        cache.mask_base.resize_with(bindings.len(), || None);
    }
    for (material, id) in bindings.iter().enumerate() {
        let Some(index) = id.and_then(|id| app.session.documents().iter().position(|d| d.doc.id == id)) else {
            if showing_layer_mask {
                // Materials without a texture bound to the displayed mask have no grayscale
                // value. Use the neutral checker used by texturing tools instead of leaking
                // their original PBR appearance into the mask preview.
                let key = (u64::MAX, app.session.uv.generation, 0, 0x6d61_736b_6368_6563);
                if cache.keys.get(material) != Some(&key) || cache.textures.get(material).is_none_or(Option::is_none) {
                    let side = 64usize;
                    let rgba = (0..side * side)
                        .map(|pixel| {
                            let x = pixel % side;
                            let y = pixel / side;
                            let gray = if (x / 8 + y / 8).is_multiple_of(2) { 0.72 } else { 0.84 };
                            [gray, gray, gray, 1.0]
                        })
                        .collect();
                    if let Some(dst) = cache.textures.get_mut(material) {
                        *dst = Some(Image { width: side, height: side, rgba });
                    }
                    if let Some(dst) = cache.keys.get_mut(material) {
                        *dst = key;
                    }
                    textures_changed = true;
                }
                cache.masked.insert(material);
                cache.snapshots[material] = None;
                continue;
            }
            let opacity = cache
                .opacity
                .get(material)
                .and_then(Option::as_ref)
                .filter(|_| matches!(cache.display_channel, Channel::Material | Channel::Color | Channel::Normal));
            if let Some(opacity) = opacity {
                let k = cache.opacity_keys.get(material).copied().unwrap_or_default();
                let key = (0, k.1, k.2, k.0.wrapping_mul(0x9e3779b97f4a7c15));
                if cache.keys.get(material) != Some(&key) || cache.textures.get(material).is_none_or(Option::is_none) {
                    let image = Image {
                        width: opacity.width,
                        height: opacity.height,
                        rgba: opacity.rgba.iter().map(|p| [1.0, 1.0, 1.0, p[0].clamp(0.0, 1.0) * p[3].clamp(0.0, 1.0)]).collect(),
                    };
                    if let Some(dst) = cache.textures.get_mut(material) {
                        *dst = Some(image);
                    }
                    if let Some(dst) = cache.keys.get_mut(material) {
                        *dst = key;
                    }
                    textures_changed = true;
                }
            } else if let Some(image) = cache.textures.get_mut(material)
                && image.take().is_some()
            {
                textures_changed = true;
            }
            continue;
        };
        let revision = app.session.documents().get(index).map_or(0, |d| d.revision);
        // The SAME display snapshot as the canvas includes an in-progress brush stroke,
        // adjustment dialog, gradient and undo/redo, without waiting for a file save.
        let (doc, preview_key) = if cache.masked.contains(&material) {
            crate::canvas::filter_channel_preview(app, index, ctx).unwrap_or_else(|| crate::canvas::display_doc(app, index))
        } else {
            crate::canvas::display_doc(app, index)
        };
        // A spatial Fill proxy is sampled off-thread at most 20 times per second. The same
        // accepted low-resolution result drives 2D and 3D; all other material maps stay at their
        // canonical image until the gesture commits.
        if showing_gray_mask && cache.masked.contains(&material) {
            // This cache is intentionally omitted in the opaque Gray view; a later Overlay
            // fill preview will rebuild it through the color path if it needs the base.
            if let Some(base) = cache.mask_base.get_mut(material) {
                *base = None;
            }
        }
        let live_gray_mask = if cache.masked.contains(&material) {
            crate::uv_mask_paint_preview::sampled_fill_proxy(app, index).and_then(|proxy| {
                let mut view = app.session.documents().get(index)?.channel_view.clone();
                if view.target == photocraft_engine::channel_cmds::ChannelTarget::LayerMask {
                    view.layer_mask = Some(photocraft_engine::mask_view_cmds::LayerMaskView {
                        layer: proxy.layer,
                        filter: false,
                        mode: photocraft_engine::mask_view_cmds::MaskViewMode::Gray,
                    });
                }
                (view.layer_mask.is_some_and(|shown| shown.layer == proxy.layer) && view.shown_layer_mask(&doc).is_some()).then_some((proxy, view))
            })
        } else {
            None
        };
        let overlay_fill_needs_color_base = live_gray_mask.as_ref().is_some_and(|(_, view)| {
            view.shown_layer_mask(&doc).is_some_and(|(_, mode)| mode == photocraft_engine::mask_view_cmds::MaskViewMode::Overlay)
                && cache.mask_base.get(material).and_then(Option::as_ref).is_none()
        });
        if !overlay_fill_needs_color_base && let Some((proxy, view)) = live_gray_mask {
            let mode = view.shown_layer_mask(&doc).map(|(_, mode)| mode);
            let (w, h) = (proxy.size[0], proxy.size[1]);
            let mask_key = crate::uv_mask::key(app, index);
            let key = (doc.id.0, revision, proxy.key, mask_key);
            if cache.keys.get(material) == Some(&key)
                && cache.textures.get(material).and_then(Option::as_ref).is_some_and(|im| im.width == w as usize && im.height == h as usize)
            {
                continue;
            }
            let Some(pixels) = crate::channel_view::render_fill_mask_proxy(
                &doc,
                &view,
                photocraft_geom::Rect::new(0, 0, w as i32, h as i32),
                [w, h],
                crate::channel_view::Look::of(&app.session),
                &proxy,
            ) else {
                continue;
            };
            let mut rgba: Vec<_> = pixels.into_iter().map(|p| p.to_array().map(|v| f32::from(v) / 255.0)).collect();
            if mode == Some(photocraft_engine::mask_view_cmds::MaskViewMode::Overlay) {
                let Some(base) = cache.mask_base.get(material).and_then(Option::as_ref) else { continue };
                for (i, pixel) in rgba.iter_mut().enumerate() {
                    let x = i % w as usize;
                    let y = i / w as usize;
                    let bx = x * base.width / w as usize;
                    let by = y * base.height / h as usize;
                    let Some(background) = base.rgba.get(by * base.width + bx) else { continue };
                    let alpha = pixel[3].clamp(0.0, 1.0);
                    for channel in 0..3 {
                        pixel[channel] += background[channel] * (1.0 - alpha);
                    }
                    pixel[3] = alpha + background[3] * (1.0 - alpha);
                }
            }
            let image = Image { width: w as usize, height: h as usize, rgba };
            if let Some(dst) = cache.textures.get_mut(material) {
                *dst = Some(image);
            }
            if let Some(dst) = cache.keys.get_mut(material) {
                *dst = key;
            }
            cache.snapshots[material] = None;
            textures_changed = true;
            continue;
        }
        if crate::uv_mask_paint_preview::fill_manipulation_active(app, index) && !overlay_fill_needs_color_base {
            // Until a proxy exists (or when this is a non-mask material view), retain the last
            // UV image. Never fall through to a full-resolution document composite on pointer move.
            continue;
        }
        let has_opacity_map = app.session.uv.map_bindings(Channel::Opacity).get(material).is_some_and(Option::is_some);
        let gpu_pbr_color = cache.gpu.is_some()
            && crate::uv_mask_paint_preview::active_document(app).is_some()
            && !cpu_pick_needed
            && lighting.pbr
            && cache.display_channel == Channel::Material
            && !cache.masked.contains(&material)
            && !has_opacity_map
            && crate::uv_lighting::gpu_color_encoding(&doc, Channel::Color).is_some()
            && cache.textures.get(material).and_then(Option::as_ref).is_some();
        if gpu_pbr_color {
            // PBR binds the full-resolution, linear compositor map prepared by
            // uv_lighting. Leave CPU cache keys untouched so toggling PBR back
            // off forces a fresh display image.
            continue;
        }
        let doc = crate::uv_mask_paint_preview::cpu_document(app, doc, preview_key);
        let display = app.session.color.canvas_display(&doc).map_err(|e| e.to_string())?;
        let opacity_key = if matches!(cache.display_channel, Channel::Material | Channel::Color | Channel::Normal) {
            cache.opacity_keys.get(material).copied().unwrap_or_default()
        } else {
            (0, 0, 0)
        };
        let alpha_key = opacity_key.0.wrapping_mul(0x9e3779b97f4a7c15) ^ opacity_key.1.rotate_left(17) ^ opacity_key.2.rotate_left(39);
        let masked = cache.masked.contains(&material);
        let view_key = if masked { crate::uv_mask::key(app, index) } else { 0 };
        let key = (doc.id.0, revision, preview_key, display.key ^ alpha_key ^ view_key);
        if cache.keys.get(material) == Some(&key) && cache.textures.get(material).is_some_and(Option::is_some) {
            continue;
        }
        let scale = texture_side / f64::from(doc.size.width.max(doc.size.height).max(1));
        let w = (f64::from(doc.size.width) * scale.min(1.0)).round().max(1.0) as u32;
        let h = (f64::from(doc.size.height) * scale.min(1.0)).round().max(1.0) as u32;
        let old = cache.keys.get(material).copied().unwrap_or_default();
        let damage = if old.0 == key.0
            && old.3 == key.3
            && cache.textures.get(material).and_then(Option::as_ref).is_some_and(|im| im.width == w as usize && im.height == h as usize)
        {
            crate::canvas::damage_since(
                app,
                index,
                (old.1, old.2 ^ display.key),
                (revision, preview_key),
                display.key,
                app.session.documents().get(index).and_then(|st| st.last_damage),
            )
        } else {
            None
        };
        if old.3 != key.3 || cache.textures[material].is_none() {
            cache.snapshots[material] = None;
        }
        let regions = crate::uv_damage::regions(&mut cache.snapshots[material], &doc, damage);
        // At native map resolution Gray mask view is already the final UV image. The regular
        // path below composites and colour-converts the document only to replace every pixel
        // with this opaque mask image in `uv_mask::overlay`.
        let direct_gray_mask =
            showing_gray_mask && masked && w == doc.size.width && h == doc.size.height && !crate::uv_mask_paint_preview::fill_manipulation_active(app, index);
        let mut hints = Vec::new();
        for rect in regions {
            if direct_gray_mask {
                if let Some((view, active_layer)) = app.session.documents().get(index).map(|st| (st.channel_view.clone(), st.active_layer))
                    && let Some(pixels) = crate::channel_view::render(&doc, &view, rect, 1, crate::channel_view::Look::of(&app.session), active_layer)
                {
                    let rgba = pixels.into_iter().map(|p| p.to_array().map(|v| f32::from(v) / 255.0)).collect::<Vec<_>>();
                    if let Some(texture) = cache.textures.get_mut(material) {
                        patch_image(texture, [w, h], rect, rgba)?;
                    }
                    hints.push(region(rect));
                    continue;
                }
            }
            let compose_started = crate::gpu_canvas::now_ms();
            let buffer = photocraft_compose::render_reduced_damage(&doc, w, h, rect);
            cpu_compose_ms += crate::gpu_canvas::now_ms() - compose_started;
            let image = display.to_rgba8(&buffer);
            let mut rgba: Vec<[f32; 4]> = if matches!(map_kind, Channel::Color | Channel::Emissive) {
                image.pixels.as_chunks::<4>().0.iter().map(|p| p.map(|v| f32::from(v) / 255.0)).collect()
            } else {
                buffer.px
            };
            if opacity_key.0 != 0
                && let Some(opacity) = cache.opacity.get(material).and_then(Option::as_ref)
            {
                for (i, pixel) in rgba.iter_mut().enumerate() {
                    let x = buffer.rect.x0 as usize + i % buffer.rect.width() as usize;
                    let y = buffer.rect.y0 as usize + i / buffer.rect.width() as usize;
                    let sample = preview::sample(opacity, [(x as f64 + 0.5) / f64::from(w), 1.0 - (y as f64 + 0.5) / f64::from(h)]);
                    pixel[3] *= sample[0].clamp(0.0, 1.0) * sample[3].clamp(0.0, 1.0);
                }
            }
            if masked {
                // Keep the unmasked material image so a live mask Fill can update its overlay
                // independently of the document compositor.
                patch_image(&mut cache.mask_base[material], [w, h], buffer.rect, rgba.clone())?;
                let factor = doc.size.width.max(doc.size.height).div_ceil(w.max(h)).max(1);
                crate::uv_mask::overlay(app, index, &doc, rect, [w, h], &mut rgba, factor);
            }
            if let Some(t) = cache.textures.get_mut(material) {
                patch_image(t, [w, h], buffer.rect, rgba)?;
            }
            hints.push(region(buffer.rect));
        }
        if old.0 == key.0
            && let Some(gpu) = &mut cache.gpu
        {
            gpu.hint_region(material, false, (old.0, old.1, old.2, old.3 ^ ((u64::from(w)) << 32) ^ u64::from(h)), hints);
        }
        if let Some(k) = cache.keys.get_mut(material) {
            *k = key;
        }
        textures_changed = true;
    }
    let normal_bindings = app.session.uv.normal_bindings.clone();
    if cache.normals.len() != normal_bindings.len() {
        cache.normals = vec![None; normal_bindings.len()];
        cache.normal_keys = vec![(0, 0, 0); normal_bindings.len()];
        cache.normal_snapshots = vec![None; normal_bindings.len()];
        textures_changed = true;
    }
    // The same unlit mask branch never samples tangent-space normals.
    for (material, id) in normal_bindings.iter().enumerate().filter(|_| !gray_mask_hides_materials) {
        let Some(index) = id.and_then(|id| app.session.documents().iter().position(|d| d.doc.id == id)) else {
            if let Some(img) = cache.normals.get_mut(material)
                && img.take().is_some()
            {
                textures_changed = true;
            }
            continue;
        };
        let revision = app.session.documents().get(index).map_or(0, |d| d.revision);
        let (doc, preview_key) = crate::canvas::display_doc(app, index);
        let doc = crate::uv_mask_paint_preview::cpu_document(app, doc, preview_key);
        let key = (doc.id.0, revision, preview_key);
        if cache.normal_keys.get(material) == Some(&key) && cache.normals.get(material).is_some_and(Option::is_some) {
            continue;
        }
        let scale = (texture_side / f64::from(doc.size.width.max(doc.size.height).max(1))).min(1.0);
        let w = (f64::from(doc.size.width) * scale).round().max(1.0) as u32;
        let h = (f64::from(doc.size.height) * scale).round().max(1.0) as u32;
        // Normal maps are data: sample native numeric RGB, bypass display ICC/proof/gamma.
        let old = cache.normal_keys.get(material).copied().unwrap_or_default();
        let damage =
            if old.0 == key.0 && cache.normals.get(material).and_then(Option::as_ref).is_some_and(|im| im.width == w as usize && im.height == h as usize) {
                crate::canvas::damage_since(
                    app,
                    index,
                    (old.1, old.2),
                    (revision, preview_key),
                    0,
                    app.session.documents().get(index).and_then(|st| st.last_damage),
                )
            } else {
                None
            };
        if cache.normals[material].as_ref().is_none_or(|im| im.width != w as usize || im.height != h as usize) {
            cache.normal_snapshots[material] = None;
        }
        let regions = crate::uv_damage::regions(&mut cache.normal_snapshots[material], &doc, damage);
        let mut hints = Vec::new();
        for rect in regions {
            let compose_started = crate::gpu_canvas::now_ms();
            let buffer = photocraft_compose::render_reduced_damage(&doc, w, h, rect);
            cpu_compose_ms += crate::gpu_canvas::now_ms() - compose_started;
            hints.push(region(buffer.rect));
            if let Some(image) = cache.normals.get_mut(material) {
                patch_image(image, [w, h], buffer.rect, buffer.px)?;
            }
        }
        if old.0 == key.0
            && let Some(gpu) = &mut cache.gpu
        {
            gpu.hint_region(material, true, (old.0, old.1, old.2, (u64::from(w) << 32) | u64::from(h)), hints);
        }
        if let Some(k) = cache.normal_keys.get_mut(material) {
            *k = key;
        }
        textures_changed = true;
    }
    if textures_changed {
        cache.rendered = None;
    }
    let refresh_ms = crate::gpu_canvas::now_ms() - refresh_started;
    app.perf.span("uv.refresh", refresh_ms);
    app.perf.span("uv.compose.cpu", cpu_compose_ms);
    app.perf.span("uv.prepare", (refresh_ms - cpu_compose_ms).max(0.0));
    Ok(())
}

pub(crate) fn reveal(app: &mut PhotocraftApp) -> Result<(), String> {
    if app.ui.workspace != "3D" {
        let previous = app.ui.clone();
        app.ui.workspace = "3D".into();
        if let Err(error) = crate::uv_workspace::sync_workspace(app) {
            app.ui = previous;
            return Err(error);
        }
        crate::menus::apply_workspace(app);
    } else {
        crate::uv_workspace::sync_workspace(app)?;
    }
    app.ui.uv_workbench.open = true;
    app.ui.panels.dock = true;
    if app.ui.docking.is_some() {
        if let Some(result) = crate::panel_docking::command(app, "window.panel.activate", &json!({"panel":"uvWorkbench"})) {
            result?;
        }
    } else {
        app.ui.dock_tabs.properties = 2;
        crate::dock::reveal(app, crate::dock::Group::Properties);
        app.ui.dock.heights.entry(crate::dock::Group::Properties).or_insert(440.0);
    }
    Ok(())
}
pub(crate) fn pick_file(app: &mut PhotocraftApp, kind: &'static str, material: usize, normal: bool) -> Result<serde_json::Value, String> {
    reveal(app)?;
    pick_map_file(app, kind, material, if normal { Channel::Normal } else { Channel::Color })
}
fn pick_map_file(app: &mut PhotocraftApp, kind: &'static str, material: usize, channel: Channel) -> Result<serde_json::Value, String> {
    use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
    let extensions = match kind {
        "model" => vec!["fbx", "obj"],
        "project" => vec!["pcuv"],
        _ => vec!["png", "tga", "jpg", "jpeg", "tif", "tiff", "dds", "exr", "bmp", "pcraft"],
    }
    .into_iter()
    .map(str::to_owned)
    .collect();
    let uv_set = app.ui.uv_workbench.uv_set;
    let initial_dir = if kind == "texture" {
        let slots = app.session.uv.map_bindings(channel);
        slots
            .get(material)
            .copied()
            .flatten()
            .and_then(|id| app.session.documents().iter().find(|d| d.doc.id == id))
            .and_then(|d| d.path.as_deref())
            .and_then(|path| std::path::Path::new(path).parent())
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.to_string_lossy().into_owned())
    } else {
        None
    };
    app.ask_file(FileDialogRequest::Open { multiple: false, initial_dir, extensions: Some(extensions) }, move |app, answer| {
        let FileDialogAnswer::Paths(paths) = answer else { return Err("This import needs a local file".into()) };
        let path = paths.first().ok_or("No file selected")?;
        let (id, params) = match kind {
            "model" => ("uv.model.import", json!({"path":path,"uvSet":uv_set})),
            "project" => ("uv.project.load", json!({"path":path})),
            _ => ("uv.texture.open", json!({"path":path,"material":material,"channel":channel.key()})),
        };
        finish_paint(app);
        let result = app.run(id, params);
        match &result {
            Ok(_) => {
                app.ui.uv_workbench.status.clear();
                if kind == "model" {
                    app.ui.uv_workbench.model_path = path.clone();
                    app.ui.uv_workbench.project_path.clear();
                    app.ui.uv_workbench.view_3d = true;
                }
                if kind == "project" {
                    app.ui.uv_workbench.project_path = path.clone();
                    app.ui.uv_workbench.view_3d = true;
                }
                app.ui.uv_workbench.guard = app.session.uv.guard;
                app.ui.uv_workbench.horizontal = app.session.uv.horizontal;
                app.ui.uv_workbench.vertical = app.session.uv.vertical;
                app.ui.uv_workbench.feather = app.session.uv.feather;
                app.ui.uv_workbench.padding = app.session.uv.padding;
                app.ui.uv_workbench.open_borders = app.session.uv.open_borders;
                app.ui.uv_workbench.uv_set = app.session.uv.uv_set;
                // Reset scene resources, retaining the device attached at application startup.
                // Dropping the renderer here silently selected CPU rasterization after every picker.
                let mut gpu = app.uv_cache.gpu.take();
                if let Some(renderer) = &mut gpu {
                    renderer.clear();
                }
                app.uv_cache = Cache {
                    gpu,
                    lighting: std::mem::take(&mut app.uv_cache.lighting),
                    thumbnails: std::mem::take(&mut app.uv_cache.thumbnails),
                    ..Cache::default()
                };
                if kind == "project"
                    && let Ok(value) = &result
                    && let Err(error) = crate::uv_project::restore(app, &value["view"])
                {
                    app.ui.uv_workbench.status = error;
                }
            }
            Err(e) => app.ui.uv_workbench.status = e.clone(),
        }
        result
    })
}
pub(crate) fn save_project(app: &mut PhotocraftApp) -> Result<serde_json::Value, String> {
    if !app.session.uv.project_path.is_empty() {
        return crate::uv_project::save_to(app, app.session.uv.project_path.clone());
    }
    save_project_as(app)
}
pub(crate) fn save_project_as(app: &mut PhotocraftApp) -> Result<serde_json::Value, String> {
    let suggested = if app.ui.uv_workbench.project_path.is_empty() { "model.pcuv".into() } else { app.ui.uv_workbench.project_path.clone() };
    app.pick_save(&suggested, |app, path| {
        let path = if path.to_ascii_lowercase().ends_with(".pcuv") { path } else { format!("{path}.pcuv") };
        crate::uv_project::save_to(app, path)
    })
}
/// Settings belong to the shell-selection tool, not to a material.
pub(crate) fn shell_settings(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let mut state = std::mem::take(&mut app.ui.uv_workbench);
    let previous_target = state.selection_target;
    ui.horizontal(|ui| {
        ui.label(tl!("Selection mode:"));
        ui.selectable_value(&mut state.selection_target, SelectionTarget::UvShells, tl!("UV shells"));
        ui.selectable_value(&mut state.selection_target, SelectionTarget::Polygons, tl!("Polygons"));
    });
    ui.horizontal(|ui| {
        ui.label(tl!("Selection depth:"));
        ui.selectable_value(&mut state.selection_depth, SelectionDepth::VisibleOnly, tl!("Visible only"));
        ui.selectable_value(&mut state.selection_depth, SelectionDepth::ThroughMesh, tl!("Through mesh"));
    });
    if state.selection_target == SelectionTarget::Polygons {
        ui.horizontal(|ui| {
            ui.label(tl!("Gesture:"));
            ui.selectable_value(&mut state.selection_gesture, SelectionGesture::Rectangle, tl!("Rectangle"));
            ui.selectable_value(&mut state.selection_gesture, SelectionGesture::Lasso, tl!("Lasso"));
        });
    }
    update_polygon_wireframe_for_mode(&mut state, previous_target);
    if previous_target == SelectionTarget::Polygons && state.selection_target != SelectionTarget::Polygons {
        let next_target = state.selection_target;
        state.selection_target = previous_target;
        let committed = commit_polygon_selection(app, &mut state);
        state.selection_target = if committed { next_target } else { previous_target };
    }
    if previous_target != state.selection_target {
        state.selected_polygons.clear();
        state.polygon_document = None;
        app.uv_cache.rendered = None;
    }
    ui.horizontal(|ui| {
        ui.label(tl!("Marquee:"));
        ui.selectable_value(&mut state.shell_enclosed, false, tl!("Intersecting"));
        ui.selectable_value(&mut state.shell_enclosed, true, tl!("Fully enclosed"));
    });
    if state.selection_target != SelectionTarget::Polygons {
        let previous = (state.guard, state.horizontal, state.vertical, state.feather, state.padding);
        for (label, value) in [
            (tl!("General margin, px"), &mut state.guard),
            (tl!("U margin, px"), &mut state.horizontal),
            (tl!("V margin, px"), &mut state.vertical),
            (tl!("Inward feather, px"), &mut state.feather),
            (tl!("Outward padding, px"), &mut state.padding),
        ] {
            ui.horizontal(|ui| {
                ui.label(label);
                ui.add(egui::DragValue::new(value).range(0.0..=1024.0));
            });
        }
        if previous != (state.guard, state.horizontal, state.vertical, state.feather, state.padding) && app.session.uv.scene.is_some() {
            let p = mask_params(&state);
            if command(app, &mut state, "uv.settings", p) {
                app.uv_cache.mask_settings_pending = true;
            }
        }
    }
    app.ui.uv_workbench = state;
}
fn mask_params(state: &State) -> serde_json::Value {
    json!({"guard":state.guard,"horizontal":state.horizontal,"vertical":state.vertical,"feather":state.feather,"openBorders":state.open_borders,"padding":state.padding})
}
fn flush_mask_settings(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // Dragging numbers must not rasterize a 4K selection on every mouse move.
    if app.uv_cache.mask_settings_pending && !ctx.input(|i| i.pointer.primary_down()) {
        app.uv_cache.mask_settings_pending = false;
        if !app.session.uv.selected.is_empty() {
            let p = mask_params(&app.ui.uv_workbench);
            if let Err(e) = app.run("uv.mask.apply", p) {
                app.ui.uv_workbench.status = e;
            }
        }
    }
}
/// Selecting a map also chooses its editable 2D document and native paint channel.
pub(crate) fn select_view_channel(app: &mut PhotocraftApp, channel: Channel) {
    app.ui.uv_workbench.channel = channel;
    if channel != Channel::Material {
        select_2d_channel(app, channel);
    }
}
pub(crate) fn select_2d_channel(app: &mut PhotocraftApp, channel: Channel) {
    finish_paint(app);
    app.ui.uv_workbench.channel_2d = channel;
    if channel == Channel::Material {
        return;
    }
    let slots = app.session.uv.map_bindings(channel);
    let valid = |material: usize| slots.get(material).copied().flatten().is_some_and(|id| app.session.documents().iter().any(|d| d.doc.id == id));
    let material = if valid(app.session.uv.material) {
        Some(app.session.uv.material)
    } else {
        (0..slots.len()).find(|m| !app.session.uv.hidden_materials.contains(m) && valid(*m))
    };
    if let Some(material) = material {
        match app.run("uv.material.select", json!({"material":material,"channel":channel.key()})) {
            Ok(_) => {
                use photocraft_engine::channel_cmds::ChannelTarget;
                app.ui.mask_target = app.session.active().is_some_and(|st| st.channel_view.target == ChannelTarget::LayerMask);
                app.ui.vector_mask_target = app.session.active().is_some_and(|st| st.channel_view.target == ChannelTarget::VectorMask);
                app.sync_mask_targets();

                if !app.session.uv.selected.is_empty() {
                    let p = mask_params(&app.ui.uv_workbench);
                    if let Err(e) = app.run("uv.mask.apply", p) {
                        app.ui.uv_workbench.status = e;
                    }
                }
            }
            Err(e) => app.ui.uv_workbench.status = e,
        }
    }
}

pub(crate) fn has_channel(app: &PhotocraftApp, channel: Channel) -> bool {
    let bindings = app.session.uv.map_bindings(channel);
    bindings.iter().flatten().any(|id| app.session.documents().iter().any(|d| d.doc.id == *id))
}

/// A small immutable preview is composed once on assignment. Brush strokes never invalidate it.
fn source_thumbnail(app: &mut PhotocraftApp, ctx: &egui::Context, id: photocraft_doc::DocId, normal: bool) -> Option<TextureHandle> {
    let key = (id.0, normal);
    if let Some(handle) = app.uv_cache.thumbnails.get(&key) {
        return Some(handle.clone());
    }
    let st = app.session.documents().iter().find(|d| d.doc.id == id)?;
    let size = st.doc.size;
    let scale = 64.0 / f64::from(size.width.max(size.height).max(1));
    let w = (f64::from(size.width) * scale.min(1.0)).round().max(1.0) as u32;
    let h = (f64::from(size.height) * scale.min(1.0)).round().max(1.0) as u32;
    let buffer = photocraft_compose::render_reduced(&st.doc, w, h);
    let bytes = if normal {
        buffer.px.iter().flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)).collect()
    } else {
        match app.session.color.canvas_display(&st.doc) {
            Ok(display) => display.to_rgba8(&buffer).pixels,
            Err(_) => return None,
        }
    };
    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &bytes);
    let handle = ctx.load_texture(format!("uv-source-{}-{normal}", id.0), image, TextureOptions::LINEAR);
    app.uv_cache.thumbnails.insert(key, handle.clone());
    Some(handle)
}

/// Called before the generic OS-drop dispatcher. A drop over a slot is consumed exactly once.
pub(crate) fn texture_drop(app: &mut PhotocraftApp, ctx: &egui::Context, at: Option<Pos2>, files: &mut Vec<egui::DroppedFileHandle>) {
    if ctx.viewport_id() != egui::ViewportId::ROOT {
        return;
    }
    if app.uv_cache.texture_target_frame.saturating_add(1) < ctx.cumulative_frame_nr() {
        return;
    }
    let Some(at) = at.or_else(|| ctx.input(|i| i.pointer.latest_pos())) else {
        return;
    };
    if ctx.layer_id_at(at).is_some_and(|layer| layer.order != egui::Order::Background) {
        return;
    }
    let Some((_, material, channel)) = app.uv_cache.texture_targets.iter().find(|(rect, _, _)| rect.contains(at)).copied() else {
        return;
    };
    if files.is_empty() {
        return;
    }
    // One card represents one map. Reject ambiguous multi-file drops without opening extra documents.
    let result = if files.len() != 1 {
        Err(tl!("Drop one file onto the texture slot").into())
    } else if let Some(path) = files.first().map(|f| f.path()).filter(|p| p.is_absolute()) {
        finish_paint(app);
        app.run("uv.texture.open", json!({"path":path,"material":material,"channel":channel.key()}))
    } else {
        Err(tl!("Assigning a texture requires a local file").into())
    };
    files.clear();
    match result {
        Ok(_) => {
            app.ui.uv_workbench.status.clear();
            app.uv_cache.refreshed = None;
        }
        Err(e) => app.ui.uv_workbench.status = e,
    }
}

fn material_cards(app: &mut PhotocraftApp, state: &mut State, ui: &mut egui::Ui, material: usize) -> Option<(&'static str, usize, Channel)> {
    let mut pick = None;
    app.uv_cache.thumbnails.retain(|(id, normal), _| {
        let _ = normal;
        app.session.uv.all_bindings().flatten().any(|doc| doc.0 == *id)
    });
    app.uv_cache.texture_targets.clear();
    app.uv_cache.texture_target_frame = ui.ctx().cumulative_frame_nr();
    let hover_at = if ui.input(|i| !i.raw.hovered_files.is_empty()) {
        app.services.cursor_pos.as_mut().and_then(|f| f(ui.ctx())).or_else(|| ui.input(|i| i.pointer.latest_pos()))
    } else {
        None
    };
    for kind in Channel::MAPS {
        let label = kind.label();
        let channel = kind.key();
        let normal = !matches!(kind, Channel::Color | Channel::Emissive);
        let bindings = app.session.uv.map_bindings(kind);
        let bound = bindings.get(material).copied().flatten().and_then(|id| app.session.documents().iter().find(|d| d.doc.id == id));
        let info = bound.map(|st| {
            (st.doc.id, std::path::Path::new(&st.doc.name).file_name().unwrap_or_default().to_string_lossy().into_owned(), st.doc.name.clone(), st.doc.size)
        });
        let thumbnail = info.as_ref().and_then(|(id, _, _, _)| source_thumbnail(app, ui.ctx(), *id, normal));
        let selected = app.session.uv.current_channel() == kind;
        ui.push_id(("uv-map-card", kind), |ui| {
            let t = Tokens::get(ui.ctx());
            let frame = egui::Frame::NONE
                .fill(t.chrome)
                .stroke(Stroke::new(1.0, if selected { t.accent } else { ui.visuals().widgets.noninteractive.bg_stroke.color }))
                .corner_radius(3)
                .inner_margin(3);
            let row = frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    let menu_width = (16.0 + 2.0 * ui.spacing().button_padding.x).max(26.0);
                    let width = (ui.available_width() - menu_width - ui.spacing().item_spacing.x).max(48.0);
                    let (rect, response) = ui.allocate_exact_size(vec2(width, if info.is_some() { 30.0 } else { 24.0 }), Sense::click());
                    if response.hovered() {
                        ui.painter().rect_filled(rect, 2, ui.visuals().widgets.hovered.weak_bg_fill);
                    }
                    let image_rect = Rect::from_min_size(rect.min + vec2(0.0, 1.0), vec2(28.0, 28.0));
                    if let Some(handle) = &thumbnail {
                        egui::Image::new(handle).paint_at(ui, image_rect);
                    }
                    let name = info.as_ref().map_or("", |(_, name, _, _)| name.as_str());
                    for (text, y, color) in
                        [(label, if info.is_some() { 0.0 } else { 4.0 }, ui.visuals().text_color()), (name, 15.0, ui.visuals().weak_text_color())]
                    {
                        let mut job = egui::text::LayoutJob::simple_singleline(text.into(), egui::FontId::proportional(11.0), color);
                        job.wrap.max_width = (width - 34.0).max(0.0);
                        job.wrap.max_rows = 1;
                        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
                        ui.painter().galley(rect.min + vec2(if info.is_some() { 34.0 } else { 4.0 }, y), galley, color);
                    }
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{label}: {name}")));
                    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(info.as_ref().map_or(
                        tl!("Choose a file or drop an image here").into(),
                        |(_, _, path, size)| {
                            format!("{path}\n{} × {} · {}\n{}", size.width, size.height, tl!("Source thumbnail"), tl!("Click to replace or drop a file"))
                        },
                    ));
                    if response.clicked() {
                        pick = Some(("texture", material, kind));
                    }
                    let button = egui::Button::image(crate::icons::image("ellipsis", 16.0, t.icon)).min_size(vec2(26.0, 24.0));
                    let (menu_response, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                        if ui.add_enabled(info.is_some(), egui::Button::new(tl!("Edit in 2D"))).clicked() {
                            command(app, state, "uv.material.select", json!({"material":material,"channel":channel}));
                            state.view_3d = false;
                            state.split_view = false;
                            ui.close();
                        }
                        if ui.add_enabled(info.is_some(), egui::Button::new(tl!("Paint on this map"))).clicked() {
                            if commit_polygon_selection(app, state) {
                                command(app, state, "uv.material.select", json!({"material":material,"channel":channel}));
                                state.select_shells = false;
                            }
                            ui.close();
                        }
                        if ui.add_enabled(app.session.active().is_some(), egui::Button::new(tl!("Use active document"))).clicked() {
                            finish_paint(app);
                            command(app, state, "uv.texture.bind", json!({"material":material,"channel":channel}));
                            ui.close();
                        }
                        if ui.add_enabled(info.is_some(), egui::Button::new(tl!("New layer"))).clicked() {
                            if command(app, state, "uv.material.select", json!({"material":material,"channel":channel})) {
                                command(app, state, "layer.new.layer", json!({"name":"3D paint"}));
                                command(app, state, "channel.target.composite", json!({}));
                            }
                            ui.close();
                        }
                        if ui.button(tl!("New 1K texture")).clicked() {
                            command(app, state, "uv.texture.create", json!({"material":material,"channel":channel}));
                            ui.close();
                        }
                    });
                    menu_response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{} options", label)));
                    menu_response.on_hover_text(format!("{} options", label));
                });
            });
            app.uv_cache.texture_targets.push((row.response.rect, material, kind));
            if hover_at.is_some_and(|p| row.response.rect.contains(p)) {
                ui.painter().rect_stroke(row.response.rect, 3, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
            }
        });
    }
    pick
}

pub(crate) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if app.ui.workspace != "3D" {
        if ui.button(tl!("Open the 3D workspace")).clicked() {
            let _ = reveal(app);
        }
        return;
    }
    let mut state = std::mem::take(&mut app.ui.uv_workbench);
    state.open = true;
    let t = Tokens::get(ui.ctx());
    let mut pick = None;
    app.uv_cache.texture_targets.clear();
    if !app.session.uv.source.is_empty() {
        ui.label(std::path::Path::new(&app.session.uv.source).file_name().unwrap_or_default().to_string_lossy()).on_hover_text(&app.session.uv.source);
    }
    if !app.uv_cache.source_message.is_empty() {
        ui.colored_label(t.warning, &app.uv_cache.source_message);
    }
    if let (Some(scene), Some(topology)) = (app.session.uv.scene.clone(), app.session.uv.topology.clone()) {
        ui.separator();
        ui.label(crate::i18n::fmt(
            tl!("Triangles: {triangles} · UV shells: {shells}"),
            &[("triangles", &scene.triangles.len().to_string()), ("shells", &topology.shells.len().to_string())],
        ));
        egui::CollapsingHeader::new(tl!("Materials / visibility")).default_open(true).show(ui, |ui| {
            for (id, material) in scene.materials.iter().enumerate() {
                ui.horizontal(|ui| {
                    let mut visible = !app.session.uv.hidden_materials.contains(&id);
                    if ui.checkbox(&mut visible, "").on_hover_text(tl!("Show mesh parts using this material")).changed() {
                        command(app, &mut state, "uv.material.visible", json!({"material":id,"visible":visible}));
                    }
                    if ui.selectable_label(app.session.uv.material == id, &material.name).clicked() {
                        let channel = state.channel_2d.key();
                        command(app, &mut state, "uv.material.select", json!({"material":id,"channel":channel}));
                        state.overlap.clear();
                    }
                });
            }
        });
        let material = app.session.uv.material;
        egui::CollapsingHeader::new(tl!("Material textures")).default_open(true).show(ui, |ui| {
            pick = material_cards(app, &mut state, ui, material);
        });
    } else {
        ui.label(tl!("Import a UV-mapped FBX or OBJ. Materials and available textures are loaded automatically."));
    }
    crate::uv_lighting::panel(app, &mut state, ui);
    if !state.status.is_empty() {
        ui.colored_label(t.danger, &state.status);
    }
    app.ui.uv_workbench = state;
    if let Some((kind, material, normal)) = pick
        && let Err(e) = pick_map_file(app, kind, material, normal)
    {
        app.ui.uv_workbench.status = e;
    }
}
/// Draw the mode switch before the no-document screen, so an untextured mesh is usable.
pub(crate) fn workspace(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> bool {
    if let Err(error) = crate::uv_workspace::sync_workspace(app) {
        app.ui.status = error.clone();
        app.ui.status_error = true;
        ui.colored_label(Tokens::get(ui.ctx()).danger, error);
        return true;
    }
    if app.ui.workspace != "3D" {
        return false;
    }
    check_source(app);
    flush_mask_settings(app, ui.ctx());
    crate::uv_workspace::show(app, ui)
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Navigation {
    Orbit,
    Pan,
    Dolly,
}
fn preview_size(size: egui::Vec2, pixels_per_point: f32, antialias: bool, texture_limit: usize) -> [usize; 2] {
    let density = if pixels_per_point.is_finite() { pixels_per_point.clamp(0.25, 8.0) } else { 1.0 };
    let multiplier = f64::from(density) * if antialias { 2.0 } else { 1.0 };
    let finite = |v: f32| if v.is_finite() { v.max(1.0) } else { 1.0 };
    let w = f64::from(finite(size.x)) * multiplier;
    let h = f64::from(finite(size.y)) * multiplier;
    let cap = texture_limit.clamp(1, 4096) as f64;
    let limit = cap / w.max(h).max(1.0);
    let scale = limit.min(1.0);
    [(w * scale).round().clamp(1.0, cap) as usize, (h * scale).round().clamp(1.0, cap) as usize]
}
fn navigation_limit(fast: bool, moving: bool, hardware: usize) -> usize {
    if fast && moving { hardware.min(1600) } else { hardware }
}
fn scene_extent(scene: &photocraft_uv::Scene) -> f64 {
    let (lo, hi) = scene.positions.iter().fold(([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]), |(lo, hi), p| {
        ([lo[0].min(p[0]), lo[1].min(p[1]), lo[2].min(p[2])], [hi[0].max(p[0]), hi[1].max(p[1]), hi[2].max(p[2])])
    });
    ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt().max(1e-12)
}
/// A gesture belongs to navigation from Ctrl+press until mouse release, even if Ctrl is released first.
struct NavigationResult {
    gesture: bool,
    orbit_started: bool,
}
fn wheel_delta(ui: &egui::Ui) -> f32 {
    let line = ui.ctx().options(|options| options.input_options.line_scroll_speed).clamp(1.0, 500.0);
    let page = ui.clip_rect().height().max(1.0);
    ui.input(|input| {
        input
            .raw
            .events
            .iter()
            .filter_map(|event| {
                let egui::Event::MouseWheel { unit, delta, phase, .. } = event else { return None };
                if *phase == egui::TouchPhase::Cancel {
                    return None;
                }
                let delta = delta.y
                    * match unit {
                        egui::MouseWheelUnit::Point => 1.0,
                        egui::MouseWheelUnit::Line => line,
                        egui::MouseWheelUnit::Page => page,
                    };
                delta.is_finite().then_some(delta.clamp(-2000.0, 2000.0))
            })
            .sum::<f32>()
            .clamp(-2000.0, 2000.0)
    })
}
fn navigate(ui: &egui::Ui, response: &egui::Response, state: &mut State, nav: &mut Option<Navigation>, extent: f64) -> NavigationResult {
    let previous = nav.is_some();
    let scroll = wheel_delta(ui);
    let (ctrl, pressed, left, middle, right, delta, scroll) = ui.input(|i| {
        (i.modifiers.alt, i.pointer.any_pressed(), i.pointer.primary_down(), i.pointer.middle_down(), i.pointer.secondary_down(), i.pointer.delta(), scroll)
    });
    let mut started = false;
    if ctrl && pressed && response.contains_pointer() {
        *nav = if right || (left && middle) {
            Some(Navigation::Dolly)
        } else if middle {
            Some(Navigation::Pan)
        } else if left {
            Some(Navigation::Orbit)
        } else {
            None
        };
        started = nav.is_some();
    }
    let held = match nav {
        Some(Navigation::Orbit) => left,
        Some(Navigation::Pan) => middle,
        Some(Navigation::Dolly) => right || (left && middle),
        None => false,
    };
    if !held {
        *nav = None;
    }
    if !started && let Some(mode) = nav {
        match mode {
            Navigation::Orbit => {
                state.yaw -= f64::from(delta.x) * 0.008;
                state.pitch = (state.pitch + f64::from(delta.y) * 0.008).clamp(-1.55, 1.55);
            }
            Navigation::Pan => {
                let unit = extent / (f64::from(response.rect.width().min(response.rect.height()).max(1.0)) * 0.8 * state.zoom);
                state.pan[0] += f64::from(delta.x) * unit;
                state.pan[1] -= f64::from(delta.y) * unit;
            }
            Navigation::Dolly => {
                state.zoom = (state.zoom * (f64::from(delta.x - delta.y) * 0.01).exp()).clamp(0.01, if state.perspective { 1_000_000.0 } else { 100.0 })
            }
        }
    }
    if response.hovered() {
        state.zoom = (state.zoom * (f64::from(scroll) * 0.002).exp()).clamp(0.01, if state.perspective { 1_000_000.0 } else { 100.0 });
    }
    NavigationResult { gesture: previous || nav.is_some() || started, orbit_started: started && *nav == Some(Navigation::Orbit) }
}
/// One BVH query on orbit press, never a GPU readback or a per-frame mesh scan.
fn surface_pivot(
    state: &mut State,
    geometry: &photocraft_uv::prepared::Geometry,
    scene: &photocraft_uv::Scene,
    maps: &preview::Materials<'_>,
    cursor: Option<[f64; 2]>,
    size: [u32; 2],
) {
    let at = match state.orbit_pivot {
        OrbitPivot::ViewCenter => [0.5, 0.5],
        OrbitPivot::Cursor => match cursor {
            Some(p) => p,
            None => return,
        },
        OrbitPivot::Fixed => return,
    };
    if let Some(hit) = geometry.hit(scene, state.camera(), maps, at, size) {
        let camera = geometry.retarget(state.camera(), hit.position);
        state.target = camera.target;
        state.pan = camera.pan;
        state.zoom = camera.zoom;
    }
}

fn frame_model(app: &mut PhotocraftApp, selected: bool) {
    let (Some(scene), Some(topology)) = (app.session.uv.scene.as_ref(), app.session.uv.topology.as_ref()) else { return };
    let state = &mut app.ui.uv_workbench;
    state.pan = [0.0; 2];
    state.target = None;
    state.zoom = 1.0;
    if !selected || app.session.uv.selected.is_empty() {
        return;
    }
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    let mut found = false;
    for (id, triangle) in scene.triangles.iter().enumerate() {
        if !topology.triangle_shell.get(id).is_some_and(|s| app.session.uv.selected.contains(s)) || app.session.uv.hidden_materials.contains(&triangle.material)
        {
            continue;
        }
        for corner in triangle.corners {
            if let Some(p) = scene.positions.get(corner.vertex) {
                for axis in 0..3 {
                    lo[axis] = lo[axis].min(p[axis]);
                    hi[axis] = hi[axis].max(p[axis]);
                }
                found = true;
            }
        }
    }
    if found {
        state.target = Some(std::array::from_fn(|i| (lo[i] + hi[i]) * 0.5));
        let extent = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt().max(1e-12);
        state.zoom = (scene_extent(scene) / extent).clamp(0.01, 100.0);
    }
}
/// Give the 3D viewport F/A before Photoshop's screen-mode/tool shortcuts.
pub(crate) fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    if app.ui.workspace != "3D" {
        return false;
    }
    if crate::uv_workspace::keys(app, ctx) {
        return true;
    }
    if (!app.ui.uv_workbench.view_3d && !app.ui.uv_workbench.split_view && !app.ui.uv_workbench.floating_3d)
        || app.session.uv.scene.is_none()
        || app.uv_cache.viewport_id.is_some_and(|id| id != ctx.viewport_id())
        || !ctx.input(|i| i.pointer.hover_pos().is_some_and(|p| app.uv_cache.viewport.is_some_and(|r| r.contains(p))))
    {
        return false;
    }
    let f = crate::uv_workspace::plain_key(ctx, egui::Key::F);
    let a = crate::uv_workspace::plain_key(ctx, egui::Key::A);
    if f || a {
        frame_model(app, f);
        return true;
    }
    false
}

pub(crate) fn main_view(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if ui.rect_contains_pointer(ui.available_rect_before_wrap()) {
        app.hover_doc = None;
        app.drop_canvas_rect = None;
    }
    let (Some(scene), Some(topology)) = (app.session.uv.scene.clone(), app.session.uv.topology.clone()) else { return };
    let mut state = std::mem::take(&mut app.ui.uv_workbench);
    let mut cache = std::mem::take(&mut app.uv_cache);
    let size = ui.available_size().max(vec2(1.0, 1.0));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let mode_menu_blocks = crate::brush_mode_menu::blocks_canvas(app, ui.ctx());
    cache.viewport = Some(rect);
    cache.viewport_id = Some(ui.ctx().viewport_id());
    if cache.geometry.as_ref().is_some_and(|(s, _)| !std::sync::Arc::ptr_eq(s, &scene)) {
        crate::uv_paint::finish(app, &mut cache.paint);
        cache.geometry = None;
        if let Some(gpu) = &mut cache.gpu {
            gpu.clear();
        }
        cache.refreshed = None;
    }
    if cache.geometry.as_ref().is_none_or(|(s, _)| !std::sync::Arc::ptr_eq(s, &scene)) {
        match photocraft_uv::prepared::Geometry::new(&scene, &topology) {
            Ok(g) => cache.geometry = Some((scene.clone(), std::sync::Arc::new(g))),
            Err(e) => {
                state.status = e.to_string();
            }
        }
    }
    cache.display_channel = state.channel;
    if let Some(e) = crate::uv_lighting::poll(&mut cache.lighting, ui.ctx()) {
        state.status = e;
    }
    // Canvas presses release numeric/text focus, so the next Undo belongs to the
    // document rather than a previously edited brush-size field.
    if ui.input(|i| i.pointer.any_pressed())
        && response.contains_pointer()
        && ui.ctx().layer_id_at(ui.input(|i| i.pointer.interact_pos()).unwrap_or(rect.center())).is_none_or(|layer| layer == ui.layer_id())
    {
        ui.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
    }
    let extent = cache.geometry.as_ref().map_or_else(|| scene_extent(&scene), |(_, g)| g.diameter);
    let resizing = !mode_menu_blocks && crate::uv_paint::resize_brush(app, ui, &response, !state.select_shells && !ui.input(|i| i.modifiers.alt));
    let navigation = if mode_menu_blocks {
        NavigationResult { gesture: false, orbit_started: false }
    } else if resizing {
        NavigationResult { gesture: true, orbit_started: false }
    } else {
        navigate(ui, &response, &mut state, &mut cache.navigation, extent)
    };
    let wheel = if !mode_menu_blocks && response.hovered() { wheel_delta(ui) } else { 0.0 };
    if wheel != 0.0 {
        cache.wheel_active = Some(Instant::now());
    } else if response.contains_pointer() && ui.input(|i| i.pointer.any_pressed()) {
        cache.wheel_active = None;
    }
    let moving = cache.navigation.is_some() || cache.wheel_active.is_some_and(|at| at.elapsed() < Duration::from_millis(120));
    // The quality settle interval owns no input: a click immediately after zoom
    // must still begin a stroke or select a shell.
    let gesture = navigation.gesture || wheel != 0.0;
    let polygon_selection_started = !mode_menu_blocks
        && state.select_shells
        && state.selection_target == SelectionTarget::Polygons
        && !gesture
        && response.contains_pointer()
        && ui.input(|i| i.pointer.primary_pressed() && !i.modifiers.alt);
    if polygon_selection_started {
        begin_polygon_selection_wireframe(&mut state);
    }
    let opacity_preview = crate::uv_mask_paint_preview::active_document(app);
    let active_preview = opacity_preview.or_else(|| crate::uv_paint::active_document(&cache.paint));
    let active_preview_gpu = active_preview.is_some_and(|id| {
        state.lighting.pbr
            && state.channel == Channel::Material
            && cache.gpu.is_some()
            && cache.masked.is_empty()
            && app.session.uv.map_bindings(Channel::Color).contains(&Some(id))
            && app.session.uv.map_bindings(Channel::Opacity).iter().all(Option::is_none)
            && app
                .session
                .documents()
                .iter()
                .find(|st| st.doc.id == id)
                .is_some_and(|st| crate::uv_lighting::gpu_color_encoding(&st.doc, Channel::Color).is_some())
    });
    // Native 3D mask strokes can invalidate the full material preview on each
    // pointer move. Keep their live refreshes at the normal cadence; pixel
    // strokes and 2D previews retain the immediate GPU refresh path.
    let mask_surface_preview_gpu = should_throttle_mask_surface_preview(
        active_preview_gpu,
        opacity_preview.is_some(),
        crate::uv_paint::active_document(&cache.paint).is_some(),
        app.ui.workspace == "3D",
        app.ui.mask_target,
    );
    let preview_ended = cache.gpu_mask_preview_was_active && !mask_surface_preview_gpu;
    cache.gpu_mask_preview_was_active = mask_surface_preview_gpu;
    cache.gpu_mask_preview_flush_pending |= preview_ended;
    let settle = crate::adjust_preview::settling(app, 180.0);
    let interval_elapsed = preview_interval_elapsed(cache.refreshed, Instant::now());
    if preview_refresh_due(
        moving,
        active_preview_gpu,
        mask_surface_preview_gpu,
        opacity_preview.is_some(),
        settle,
        interval_elapsed,
        cache.gpu_mask_preview_flush_pending,
    ) {
        if let Err(e) = refresh_with_lighting(app, ui.ctx(), &mut cache, state.lighting) {
            state.status = e;
        }
        // Masked Gray materials bypass PBR in the UV shader, so these source maps cannot affect
        // the frame. Their revision keys remain intact and refresh normally after leaving Gray.
        if should_refresh_surface_maps(app, &cache) {
            let lighting_refresh = if cache.gpu.is_some() {
                crate::uv_lighting::refresh_gpu(app, &mut cache.lighting.surface)
            } else {
                crate::uv_lighting::refresh(app, &mut cache.lighting.surface)
            };
            if let Err(e) = lighting_refresh {
                state.status = e;
            }
        }
        cache.refreshed = Some(Instant::now());
        cache.gpu_mask_preview_flush_pending = false;
    }
    let limit = navigation_limit(state.fast_navigation, moving, ui.input(|i| i.max_texture_side));
    let [w, h] = preview_size(size, ui.ctx().pixels_per_point(), cache.gpu.is_none() && state.antialias && !moving, limit);
    let gray_mask_view = app
        .session
        .active()
        .and_then(photocraft_engine::mask_view_cmds::current)
        .is_some_and(|view| view.mode == photocraft_engine::mask_view_cmds::MaskViewMode::Gray);
    let gray_mask_source = gray_mask_view.then(|| gray_mask_gpu_source(app, &mut cache, &scene)).flatten();
    let maps = preview::Materials { colors: &cache.textures, normals: &cache.normals, hidden: app.session.uv.hidden_materials.clone() };
    if navigation.orbit_started
        && let Some((_, geometry)) = &cache.geometry
    {
        let cursor = response.interact_pointer_pos().map(|p| {
            let at = (p - rect.min) / rect.size();
            [f64::from(at.x), f64::from(at.y)]
        });
        surface_pivot(&mut state, geometry, &scene, &maps, cursor, [w as u32, h as u32]);
    }
    let camera = state.camera();
    let polygon_gesture_active = state.select_shells
        && state.selection_target == SelectionTarget::Polygons
        && ui.input(|input| input.focused && (polygon_selection_started || (cache.shell_box_3d.is_some() && input.pointer.primary_down())));
    cancel_temporary_wireframes_for_gray_mask(&mut state, ui.ctx(), gray_mask_view, polygon_gesture_active);
    // This single visibility decision feeds both the GPU renderer and CPU line overlay.
    // Temporary requests never write the user's persisted show_wireframe setting.
    let draw_wireframe = wireframe_visible(&state, crate::uv_symmetry::get(ui.ctx()).snap_pending);
    let key = (app.session.uv.generation, camera, w, h, state.select_shells);
    let mut gpu_image = None;
    let no_highlight = std::collections::BTreeSet::new();
    let mut face_selection = std::collections::BTreeSet::new();
    let highlight = if state.select_shells {
        match state.selection_target {
            SelectionTarget::UvShells => &app.session.uv.selected,
            SelectionTarget::Polygons => {
                face_selection.extend(state.selected_polygons.iter().copied().map(crate::uv_gpu::selected_face_key));
                &face_selection
            }
        }
    } else {
        &no_highlight
    };
    if let (Some(renderer), Some((_, geometry))) = (&mut cache.gpu, &cache.geometry) {
        renderer.set_wireframe(draw_wireframe, state.wireframe_color, state.wireframe_opacity);
        renderer.set_gray_mask_source(gray_mask_source.as_ref());
        let draw = crate::uv_gpu::Draw {
            scene: &scene,
            geometry: geometry.clone(),
            camera,
            size: [w as u32, h as u32],
            antialias: state.antialias && !moving,
            selected: highlight,
            hidden: &app.session.uv.hidden_materials,
            generation: app.session.uv.generation,
            colors: &cache.textures,
            normals: &cache.normals,
            keys: &cache.keys,
            normal_keys: &cache.normal_keys,
            masked: &cache.masked,
            symmetry_plane: if state.view_3d { crate::uv_symmetry::plane_corners(geometry, crate::uv_symmetry::get(ui.ctx())) } else { None },
        };
        let environment = crate::uv_lighting::ensure(&mut cache.lighting);
        let map_started = crate::gpu_canvas::now_ms();
        let map_result = renderer.set_lighting(state.lighting, &cache.lighting.surface, environment);
        app.perf.span("uv.pbr.prepare", crate::gpu_canvas::now_ms() - map_started);
        if let Some(stats) = renderer.take_map_stats()
            && let Some(timing) = stats.gpu_time
        {
            app.perf.record_gpu(timing);
            app.perf.span("uv.pbr.gpu", timing.ms);
        }
        let draw_started = crate::gpu_canvas::now_ms();
        let result = map_result.and_then(|()| renderer.draw(draw));
        app.perf.span("uv.draw.gpu", crate::gpu_canvas::now_ms() - draw_started);
        match result {
            Ok(id) => gpu_image = Some(id),
            Err(e) => {
                state.status = format!("3D GPU: {e}. CPU fallback.");
                cache.gpu = None;
                cache.keys.fill((0, 0, 0, 0));
                cache.refreshed = None;
            }
        }
    }
    if gpu_image.is_none() && cache.rendered != Some(key) {
        let selected: Vec<bool> = if state.selection_target == SelectionTarget::Polygons && state.select_shells {
            (0..topology.triangle_shell.len()).map(|id| state.selected_polygons.contains(&id)).collect()
        } else {
            topology.triangle_shell.iter().map(|s| highlight.contains(s)).collect()
        };
        let fallback_camera = if cache.masked.is_empty() { camera } else { Camera { channel: Channel::Color, shaded: false, ..camera } };
        match preview::render_materials(&scene, fallback_camera, &maps, &selected, w, h) {
            Ok(frame) => {
                let data: Vec<u8> = frame.rgba.iter().flatten().copied().collect();
                let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &data);
                if let Some(handle) = cache.frame_handle.as_mut() {
                    handle.set(image, TextureOptions::LINEAR);
                } else {
                    cache.frame_handle = Some(ui.ctx().load_texture("uv-main-3d", image, TextureOptions::LINEAR));
                }
                cache.frame = Some(frame);
                cache.rendered = Some(key);
            }
            Err(e) => state.status = e.to_string(),
        }
    }
    if let Some(id) = gpu_image {
        ui.painter().image(id, rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    } else if let Some(handle) = &cache.frame_handle {
        ui.painter().image(handle.id(), rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    }
    if draw_wireframe
        && gpu_image.is_none()
        && let Some((_, geometry)) = &cache.geometry
    {
        let painter = ui.painter_at(rect);
        let [r, g, b, a] = state.wireframe_color;
        let alpha = (f32::from(a) * state.wireframe_opacity.clamp(0.0, 1.0)).round() as u8;
        let stroke = Stroke::new(0.7, Color32::from_rgba_unmultiplied(r, g, b, alpha));
        for edge in wireframe_segments(geometry, camera, [w as u32, h as u32], rect, &app.session.uv.hidden_materials) {
            painter.line_segment(edge, stroke);
        }
    }
    if state.view_3d
        && gpu_image.is_none()
        && let Some((_, geometry)) = &cache.geometry
    {
        crate::uv_symmetry::paint_plane(ui, geometry, camera, [w as u32, h as u32], rect);
    }
    if state.select_shells && !gesture && !mode_menu_blocks {
        let lasso = state.selection_target == SelectionTarget::Polygons && state.selection_gesture == SelectionGesture::Lasso;
        let selection_drag_was_active = cache.shell_box_3d.is_some();
        let selection = crate::uv_box::input(ui, &response, &mut cache.shell_box_3d, lasso);
        let polygon_selection_cancelled = state.selection_target == SelectionTarget::Polygons
            && state.temporary_polygon_wireframe
            && ui.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused);
        let polygon_selection_finished = state.selection_target == SelectionTarget::Polygons
            && ((selection_drag_was_active || polygon_selection_started)
                && cache.shell_box_3d.is_none()
                && ui.input(|i| i.pointer.primary_released() || !i.pointer.primary_down() || !i.focused)
                || polygon_selection_cancelled);
        if polygon_selection_finished {
            finish_polygon_selection_wireframe(&mut state);
            ui.ctx().request_repaint();
        }
        if let Some((area, mods)) = selection
            && let Some((_, g)) = &cache.geometry
        {
            if state.selection_target == SelectionTarget::UvShells {
                if let crate::uv_box::SelectionArea::Rect(area) = area {
                    let material = app.session.uv.material;
                    let visible_only = state.selection_depth == SelectionDepth::VisibleOnly;
                    crate::uv_box::select(
                        app,
                        &mut state,
                        area,
                        mods,
                        material,
                        visible_only,
                        |id| {
                            let tri = g.triangles.get(id)?;
                            projected_face(g, tri.positions, camera, [w as u32, h as u32], rect)
                        },
                        |id, points, selection| face_is_visible(g, &scene, &maps, (camera, [w as u32, h as u32], rect), id, points, selection),
                    );
                }
            } else {
                let mut selected = Vec::new();
                let active_materials = app.session.active_index().map(|index| canvas_materials(app, index)).unwrap_or_default();
                for (id, tri) in g.triangles.iter().enumerate() {
                    if !active_materials.contains(&tri.material)
                        || app.session.uv.hidden_materials.contains(&tri.material)
                        || (state.selection_depth == SelectionDepth::VisibleOnly && camera.cull_backfaces && !wireframe_front_facing(tri.face, camera))
                    {
                        continue;
                    }
                    let Some(points) = projected_face(g, tri.positions, camera, [w as u32, h as u32], rect) else { continue };
                    let touches =
                        if state.shell_enclosed { selection_area_contains_triangle(&area, points) } else { selection_area_touches_triangle(&area, points) };
                    let visible = state.selection_depth == SelectionDepth::ThroughMesh
                        || face_is_visible_in_area(g, &scene, &maps, (camera, [w as u32, h as u32], rect), id, points, &area);
                    if touches && visible {
                        selected.push(id);
                    }
                }
                if let Some(owner) = app.session.active().map(|st| st.doc.id) {
                    update_polygon_selection(&mut state, &selected, mods, owner);
                }
                cache.rendered = None;
            }
        }
    } else {
        cache.shell_box_3d = None;
    }
    if response.clicked()
        && !mode_menu_blocks
        && state.select_shells
        && !gesture
        && !ui.input(|i| i.modifiers.alt)
        && let Some(p) = response.interact_pointer_pos()
    {
        let at = (p - rect.min) / rect.size();
        let triangle = if gpu_image.is_some() {
            cache.geometry.as_ref().and_then(|(_, g)| g.pick(&scene, camera, &maps, [f64::from(at.x), f64::from(at.y)], [w as u32, h as u32]))
        } else {
            cache.frame.as_ref().and_then(|frame| {
                let x = ((at.x * frame.width as f32) as usize).min(frame.width - 1);
                let y = ((at.y * frame.height as f32) as usize).min(frame.height - 1);
                frame.triangles.get(y * frame.width + x).copied().flatten()
            })
        };
        if let Some(triangle) = triangle {
            if state.selection_target == SelectionTarget::Polygons {
                let modifiers = ui.input(|i| i.modifiers);
                let active_materials = app.session.active_index().map(|index| canvas_materials(app, index)).unwrap_or_default();
                let through_faces: Vec<usize> = if state.selection_depth == SelectionDepth::ThroughMesh {
                    cache.geometry.as_ref().map_or_else(Vec::new, |(_, g)| {
                        g.triangles
                            .iter()
                            .enumerate()
                            .filter_map(|(id, tri)| {
                                if !active_materials.contains(&tri.material) || app.session.uv.hidden_materials.contains(&tri.material) {
                                    return None;
                                }
                                let points = projected_face(g, tri.positions, camera, [w as u32, h as u32], rect)?;
                                point_in_triangle(p, points).then_some(id)
                            })
                            .collect()
                    })
                } else {
                    vec![triangle]
                };
                if !through_faces.is_empty()
                    && through_faces.iter().all(|&id| {
                        scene
                            .triangles
                            .get(id)
                            .is_some_and(|tri| active_materials.contains(&tri.material) && !app.session.uv.hidden_materials.contains(&tri.material))
                    })
                    && let Some(owner) = app.session.active().map(|st| st.doc.id)
                {
                    update_polygon_selection(&mut state, &through_faces, modifiers, owner);
                    cache.rendered = None;
                }
            } else if let Some(&shell) = topology.triangle_shell.get(triangle)
                && let Some(sh) = topology.shells.get(shell)
            {
                if state.selection_depth == SelectionDepth::ThroughMesh {
                    let pointer = p;
                    let mut shells = std::collections::BTreeSet::new();
                    if let Some((_, g)) = &cache.geometry {
                        for (id, tri) in g.triangles.iter().enumerate() {
                            if tri.material != app.session.uv.material
                                || app.session.uv.hidden_materials.contains(&tri.material)
                                || (camera.cull_backfaces && state.selection_depth == SelectionDepth::VisibleOnly && !wireframe_front_facing(tri.face, camera))
                            {
                                continue;
                            }
                            if projected_face(g, tri.positions, camera, [w as u32, h as u32], rect).is_some_and(|points| point_in_triangle(pointer, points))
                                && let Some(shell) = topology.triangle_shell.get(id)
                            {
                                shells.insert(*shell);
                            }
                        }
                    }
                    let modifiers = ui.input(|i| i.modifiers);
                    if !shells.is_empty() {
                        crate::uv_box::select_ids(app, &mut state, sh.material, &shells.into_iter().collect::<Vec<_>>(), modifiers);
                    }
                } else {
                    choose(app, &mut state, ui, sh.material, shell);
                }
            }
        }
    }
    if let Some((_, geometry)) = &cache.geometry {
        let uv_input_started = crate::gpu_canvas::now_ms();
        let native_uv_surface_input = std::mem::replace(&mut app.native_uv_surface_input, true);
        crate::uv_paint::input(
            app,
            ui,
            &response,
            &mut cache.paint,
            geometry,
            &scene,
            camera,
            &maps,
            [w as u32, h as u32],
            !mode_menu_blocks && !state.select_shells && !gesture && cache.navigation.is_none() && !ui.input(|i| i.modifiers.alt),
            state.paint_wrap,
        );
        app.native_uv_surface_input = native_uv_surface_input;
        app.perf.span("uv.input", crate::gpu_canvas::now_ms() - uv_input_started);
    }
    if !state.status.is_empty() {
        ui.painter().text(
            rect.left_bottom() + vec2(12.0, -14.0),
            egui::Align2::LEFT_BOTTOM,
            &state.status,
            egui::FontId::proportional(13.0),
            Tokens::get(ui.ctx()).danger,
        );
    }
    if cache.navigation.is_some() {
        ui.ctx().request_repaint();
    }
    let pointer_modifiers = ui.input(|i| i.modifiers);
    let menu_modifiers = crate::workspace_ui::sticky_mods(app, crate::brush_mode_menu::gesture_mods(ui.ctx(), pointer_modifiers));
    let mode_menu = !mode_menu_blocks
        && response.secondary_clicked()
        && !gesture
        && !state.select_shells
        && crate::brush_mode_menu::is_gesture(app.ui.tool, menu_modifiers);
    let picker = !mode_menu_blocks
        && response.secondary_clicked()
        && !mode_menu
        && !gesture
        && !state.select_shells
        && !pointer_modifiers.alt
        && crate::paint_mouse::has_brush_picker(app.ui.tool);
    app.ui.uv_workbench = state;
    app.uv_cache = cache;
    if mode_menu && let Some(p) = response.interact_pointer_pos() {
        crate::brush_mode_menu::open(app, ui.ctx(), [p.x, p.y]);
    } else if picker && let Some(p) = response.interact_pointer_pos() {
        crate::brush_picker::open(&mut app.ui, ui.ctx(), [p.x, p.y]);
        app.ui.uv_workbench.picker_3d = true;
    }
    ui.ctx().request_repaint_after(Duration::from_millis(33));
}

/// Resolve by the displayed document, rather than the currently chosen material. Other
/// image tabs never acquire unrelated UVs. Several materials may share the same image.
fn canvas_materials(app: &PhotocraftApp, index: usize) -> Vec<usize> {
    let Some(document) = app.session.documents().get(index) else { return Vec::new() };
    (0..app.session.uv.bindings.len())
        .filter(|m| Channel::MAPS.into_iter().any(|kind| app.session.uv.map_bindings(kind).get(*m).copied().flatten() == Some(document.doc.id)))
        .collect()
}

/// Whether the active 2D canvas is showing the selected mask Fill's own controls.
/// The canvas uses this to keep the generic Move/Free Transform state machine from
/// seeing the same pointer events as the Fill manipulator.
pub(crate) fn fill_manipulator_active(app: &PhotocraftApp, ctx: &egui::Context, index: usize) -> bool {
    if app.ui.workspace != "3D" || !app.ui.mask_target || app.session.uv.topology.is_none() || canvas_materials(app, index).is_empty() {
        return false;
    }
    let Some(document) = app.session.documents().get(index) else { return false };
    app.session.active().is_some_and(|st| {
        if st.doc.id != document.doc.id {
            return false;
        }
        let Some(layer) = st.active_layer.and_then(|id| st.doc.layer(id)) else { return false };
        let Some(stack) = layer.mask.as_ref().and_then(|mask| mask.paint.as_ref()) else { return false };
        crate::uv_mask_paint::selected_effect(ctx, st.doc.id.0, layer.id.0)
            .filter(|index| *index == stack.active)
            .and_then(|index| stack.layers.get(index))
            .is_some_and(|effect| matches!(&effect.kind, photocraft_doc::mask_paint::EffectKind::Fill { .. }))
    })
}

/// View-only overlay: it never changes the image pixels or the persistent seam mask.
pub(crate) fn canvas_overlay(app: &mut PhotocraftApp, ui: &mut egui::Ui, xf: &crate::canvas::ViewXform, index: usize, _primary: bool) -> (Option<Rect>, bool) {
    if app.ui.workspace != "3D" {
        return (None, false);
    }
    let Some(topology) = app.session.uv.topology.clone() else { return (None, false) };
    let materials = canvas_materials(app, index);
    if materials.is_empty() {
        return (None, false);
    }
    let Some(document) = app.session.documents().get(index) else { return (None, false) };
    let size = [document.doc.size.width, document.doc.size.height];
    let t = Tokens::get(ui.ctx());
    let mut fill_area = None;
    let mut fill_owns_pointer = false;
    // A selected Fill exposes an editable rectangle directly on the active 2D map.
    // The document pixels remain untouched; only the Fill's destination bounds change.
    if let Some((layer_id, effect_index, bounds, rotation)) = app.session.active().and_then(|st| {
        if st.doc.id != document.doc.id || !app.ui.mask_target {
            return None;
        }
        let layer = st.active_layer.and_then(|id| st.doc.layer(id))?;
        let stack = layer.mask.as_ref()?.paint.as_ref()?;
        let selected = crate::uv_mask_paint::selected_effect(ui.ctx(), st.doc.id.0, layer.id.0)?;
        if selected != stack.active {
            return None;
        }
        let effect = stack.layers.get(selected)?;
        let photocraft_doc::mask_paint::EffectKind::Fill { rotation, .. } = &effect.kind else { return None };
        let bounds = effect.fill_bounds.unwrap_or([0, 0, size[0] as i32, size[1] as i32]).map(|value| value.clamp(-1_000_000, 1_000_000));
        if bounds[0] >= bounds[2] || bounds[1] >= bounds[3] {
            return None;
        }
        Some((layer.id.0, stack.active, bounds, *rotation))
    }) {
        let (shown_bounds, shown_rotation) =
            crate::uv_mask_paint_preview::fill_transform(app, photocraft_doc::LayerId(layer_id), effect_index, bounds, rotation);
        let id = ui.scope_id().with(("fill-mask-manipulator", document.doc.id.0, layer_id, effect_index));
        let screen = |x: f32, y: f32| xf.to_screen(x, y);
        let corners = fill_corners(shown_bounds, shown_rotation).map(|[x, y]| screen(x, y));
        let box_rect = corners.iter().copied().fold(Rect::NOTHING, |rect, point| rect.union(Rect::from_min_max(point, point)));
        let painter = ui.painter_at(xf.rect);
        let handles = fill_handles(corners);
        let rotation_handle = fill_rotation_handle(corners, xf.rect);
        let draw_bounds = |bounds: [i32; 4], rotation: f32| {
            let points = fill_corners(bounds, rotation).map(|[x, y]| screen(x, y));
            for index in 0..4 {
                painter.line_segment([points[index], points[(index + 1) % 4]], Stroke::new(1.5, t.accent));
            }
            for point in fill_handles(points) {
                painter.rect_filled(Rect::from_center_size(point, vec2(9.0, 9.0)), 1.0, t.accent);
            }
            let top = points[0].lerp(points[1], 0.5);
            let rotate = fill_rotation_handle(points, xf.rect);
            painter.line_segment([top, rotate], Stroke::new(1.0, t.accent));
            painter.circle_filled(rotate, 5.0, t.accent);
        };
        let mut displayed_bounds = shown_bounds;
        let mut displayed_rotation = shown_rotation;
        let response_rect = box_rect.expand(30.0).intersect(xf.rect).intersect(ui.clip_rect());
        fill_area = Some(response_rect);
        let drag_id = id.with("drag-state");
        let has_drag = ui.ctx().data_mut(|data| data.get_temp::<FillDragState>(drag_id)).is_some();
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) && has_drag {
            ui.ctx().data_mut(|data| data.remove::<FillDragState>(drag_id));
            crate::uv_mask_paint_preview::cancel_fill(app, photocraft_doc::LayerId(layer_id), effect_index);
        }
        let navigating = app.ui.tool == crate::state::Tool::Hand || ui.input(|i| i.key_down(egui::Key::Space) || i.pointer.middle_down() || i.modifiers.alt);
        let owns_gesture = ui.ctx().data_mut(|data| data.get_temp::<FillDragState>(drag_id)).is_some();
        let response = ((owns_gesture || !navigating) && response_rect.width() > 0.0 && response_rect.height() > 0.0)
            .then(|| ui.interact(response_rect, id, Sense::click_and_drag()));
        let hit_overlay = |point: Pos2| fill_overlay_contains(point, corners, rotation_handle);
        if !owns_gesture
            && let Some(response) = response.as_ref()
            && response.drag_started_by(egui::PointerButton::Primary)
        {
            let pointer = ui.input(|i| i.pointer.press_origin()).or_else(|| response.interact_pointer_pos());
            if let Some(press_origin) = pointer {
                let handle = if rotation_handle.distance(press_origin) <= 12.0 {
                    Some(8)
                } else {
                    handles
                        .iter()
                        .enumerate()
                        .map(|(index, point)| (index, point.distance_sq(press_origin)))
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .filter(|(index, _)| handles[*index].distance(press_origin) <= 12.0)
                        .map(|(index, _)| index)
                };
                if handle.is_some() || point_in_quad(press_origin, corners) {
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(drag_id, FillDragState { start: xf.to_doc(press_origin), bounds: shown_bounds, rotation: shown_rotation, handle })
                    });
                }
            }
        }
        // Once the Fill owns the pointer, read global pointer state as well as the widget
        // response. This keeps resize and release working after the pointer leaves the frame.
        let (latest_pos, primary_down, primary_released, press_origin, focused) =
            ui.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down(), i.pointer.primary_released(), i.pointer.press_origin(), i.focused));
        let drag = ui.ctx().data_mut(|data| data.get_temp::<FillDragState>(drag_id));
        let drag_active = drag.is_some();
        let mut drag_stopped = false;
        if let Some(drag) = drag {
            let response_pointer = response.as_ref().and_then(egui::Response::interact_pointer_pos);
            let stopped = fill_drag_stopped(
                owns_gesture,
                primary_down,
                primary_released,
                focused,
                response.as_ref().is_some_and(|response| response.drag_stopped_by(egui::PointerButton::Primary)),
            );
            drag_stopped = stopped;
            // On blur, commit the last rendered preview. The OS may leave primary_down true
            // and the reported pointer position may be stale, so do not derive a new transform.
            let pointer = if !focused {
                None
            } else if owns_gesture {
                latest_pos.or(response_pointer)
            } else {
                response_pointer
            };
            let active =
                response.as_ref().is_some_and(|response| response.dragged_by(egui::PointerButton::Primary)) || (owns_gesture && primary_down) || stopped;
            if active {
                let mut next = shown_bounds;
                let mut next_rotation = shown_rotation;
                if let Some(pointer) = pointer {
                    let doc_point = xf.to_doc(pointer);
                    if doc_point.iter().all(|value| value.is_finite()) {
                        if drag.handle == Some(8) {
                            let center =
                                [(f64::from(drag.bounds[0]) + f64::from(drag.bounds[2])) * 0.5, (f64::from(drag.bounds[1]) + f64::from(drag.bounds[3])) * 0.5];
                            let start_angle = (drag.start[1] - center[1]).atan2(drag.start[0] - center[0]);
                            let current_angle = (doc_point[1] - center[1]).atan2(doc_point[0] - center[0]);
                            let angle = drag.rotation + (current_angle - start_angle).to_degrees() as f32;
                            next_rotation = if angle.is_finite() { angle } else { drag.rotation };
                        } else if let Some(handle) = drag.handle.filter(|handle| *handle < 8) {
                            next = resize_fill_bounds(drag.bounds, drag.rotation, handle, drag.start, doc_point).unwrap_or(drag.bounds);
                        } else if drag.handle.is_none() {
                            next = move_fill_bounds(drag.bounds, drag.start, doc_point);
                        }
                    }
                }
                crate::uv_mask_paint_preview::set_fill(app, photocraft_doc::LayerId(layer_id), effect_index, next, next_rotation);
                if next != shown_bounds || next_rotation.to_bits() != shown_rotation.to_bits() {
                    ui.ctx().request_repaint();
                }
                displayed_bounds = next;
                displayed_rotation = next_rotation;
                if stopped {
                    ui.ctx().data_mut(|data| data.remove::<FillDragState>(drag_id));
                    if (next != drag.bounds || next_rotation.to_bits() != drag.rotation.to_bits())
                        && let Some(commit) = crate::uv_mask_paint_preview::pending(app)
                        && let Err(error) = app.run("uv.maskPaint.modify", commit.params())
                    {
                        app.ui.uv_workbench.status = error;
                    }
                }
            }
        }
        let press_captured =
            !navigating && press_origin.is_some_and(hit_overlay) && response.as_ref().is_some_and(|response| response.is_pointer_button_down_on());
        fill_owns_pointer = owns_gesture || drag_active || drag_stopped || press_captured;
        draw_bounds(displayed_bounds, displayed_rotation);
        if crate::uv_mask_paint_preview::fill_preview_pending(app, photocraft_doc::LayerId(layer_id), effect_index) {
            let pending_rect = Rect::from_min_size(xf.rect.left_top() + vec2(8.0, 8.0), vec2(154.0, 22.0));
            ui.painter().rect_filled(pending_rect, 3.0, t.canvas);
            ui.painter().text(
                pending_rect.left_center() + vec2(7.0, 0.0),
                egui::Align2::LEFT_CENTER,
                tl!("Refining mask preview…"),
                egui::FontId::proportional(11.0),
                t.accent,
            );
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
    if app.ui.uv_workbench.show_overlay {
        let painter = ui.painter_at(xf.rect);
        for edge in &topology.boundaries {
            if !topology.shells.get(edge.shell).is_some_and(|s| materials.contains(&s.material)) {
                continue;
            }
            let Some(segment) = clip_tile(edge.uv) else { continue };
            let points = segment.map(|uv| xf.to_screen((uv[0] * f64::from(size[0])) as f32, ((1.0 - uv[1]) * f64::from(size[1])) as f32));
            let selected = app.session.uv.selected.contains(&edge.shell);
            // A contrasting backing keeps the wire visible over light and dark maps.
            painter.line_segment(points, Stroke::new(if selected { 3.5 } else { 2.5 }, t.canvas));
            painter.line_segment(points, Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { t.accent } else { t.text }));
        }
        if app.ui.uv_workbench.select_shells
            && app.ui.uv_workbench.selection_target == SelectionTarget::Polygons
            && let Some(scene) = app.session.uv.scene.as_ref()
        {
            for (face_id, triangle) in scene.triangles.iter().enumerate() {
                if !app.ui.uv_workbench.selected_polygons.contains(&face_id) || !materials.contains(&triangle.material) {
                    continue;
                }
                let points =
                    triangle.corners.map(|corner| xf.to_screen((corner.uv[0] * f64::from(size[0])) as f32, ((1.0 - corner.uv[1]) * f64::from(size[1])) as f32));
                painter.add(egui::Shape::convex_polygon(points.to_vec(), t.accent.gamma_multiply(0.15), Stroke::new(1.5, t.accent)));
            }
        }
    }
    (fill_area, fill_owns_pointer)
}

pub(crate) fn fill_corners(bounds: [i32; 4], rotation: f32) -> [[f32; 2]; 4] {
    let [x0, y0, x1, y1] = bounds;
    let corners = [[x0 as f32, y0 as f32], [x1 as f32, y0 as f32], [x1 as f32, y1 as f32], [x0 as f32, y1 as f32]];
    let angle = rotation.rem_euclid(360.0);
    if angle == 0.0 || !angle.is_finite() {
        return corners;
    }
    let (sin, cos) = angle.to_radians().sin_cos();
    let center = [(x0 as f32 + x1 as f32) * 0.5, (y0 as f32 + y1 as f32) * 0.5];
    corners.map(|[x, y]| {
        let (dx, dy) = (x - center[0], y - center[1]);
        [center[0] + cos * dx - sin * dy, center[1] + sin * dx + cos * dy]
    })
}

fn fill_handles(corners: [Pos2; 4]) -> [Pos2; 8] {
    [
        corners[0],
        corners[0].lerp(corners[1], 0.5),
        corners[1],
        corners[1].lerp(corners[2], 0.5),
        corners[2],
        corners[2].lerp(corners[3], 0.5),
        corners[3],
        corners[3].lerp(corners[0], 0.5),
    ]
}

fn fill_overlay_contains(point: Pos2, corners: [Pos2; 4], rotation_handle: Pos2) -> bool {
    rotation_handle.distance(point) <= 12.0 || fill_handles(corners).iter().any(|handle| handle.distance(point) <= 12.0) || point_in_quad(point, corners)
}

pub(crate) fn fill_rotation_handle(corners: [Pos2; 4], canvas: Rect) -> Pos2 {
    let top = corners[0].lerp(corners[1], 0.5);
    let center = corners[0].lerp(corners[2], 0.5);
    let outward = (top - center).normalized();
    let outside = top + outward * 24.0;
    if canvas.contains(outside) { outside } else { top - outward * 24.0 }
}

#[cfg(test)]
fn inverse_rotate_point(point: [f64; 2], bounds: [i32; 4], rotation: f32) -> [f64; 2] {
    let angle = f64::from(rotation.rem_euclid(360.0)).to_radians();
    let (sin, cos) = angle.sin_cos();
    let center = [(f64::from(bounds[0]) + f64::from(bounds[2])) * 0.5, (f64::from(bounds[1]) + f64::from(bounds[3])) * 0.5];
    let (dx, dy) = (point[0] - center[0], point[1] - center[1]);
    [center[0] + cos * dx + sin * dy, center[1] - sin * dx + cos * dy]
}

const FILL_BOUNDS_LIMIT: i32 = 1_000_000;
const FILL_HANDLE_SIGNS: [[f64; 2]; 8] = [[-1.0, -1.0], [0.0, -1.0], [1.0, -1.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [-1.0, 1.0], [-1.0, 0.0]];

fn fill_drag_stopped(owns_gesture: bool, primary_down: bool, primary_released: bool, focused: bool, widget_stopped: bool) -> bool {
    if owns_gesture { primary_released || !primary_down || !focused } else { widget_stopped }
}

fn rotate_fill_vector([x, y]: [f64; 2], rotation: f32) -> [f64; 2] {
    let angle = f64::from(rotation.rem_euclid(360.0)).to_radians();
    let (sin, cos) = angle.sin_cos();
    [cos * x - sin * y, sin * x + cos * y]
}

/// Resize from the immutable gesture origin. The opposite edge/corner stays fixed in document
/// space even though the Fill's rotation pivot (its center) moves when its dimensions change.
fn resize_fill_bounds(bounds: [i32; 4], rotation: f32, handle: usize, start: [f64; 2], current: [f64; 2]) -> Option<[i32; 4]> {
    let moving_sign = *FILL_HANDLE_SIGNS.get(handle)?;
    if !rotation.is_finite() || start.iter().chain(current.iter()).any(|value| !value.is_finite()) {
        return None;
    }
    let old_width = f64::from(bounds[2]) - f64::from(bounds[0]);
    let old_height = f64::from(bounds[3]) - f64::from(bounds[1]);
    if old_width < 1.0 || old_height < 1.0 {
        return None;
    }
    let anchor_sign = FILL_HANDLE_SIGNS[(handle + 4) % 8];
    let center = [(f64::from(bounds[0]) + f64::from(bounds[2])) * 0.5, (f64::from(bounds[1]) + f64::from(bounds[3])) * 0.5];
    let moving_world = {
        let offset = rotate_fill_vector([moving_sign[0] * old_width * 0.5, moving_sign[1] * old_height * 0.5], rotation);
        [center[0] + offset[0] + current[0] - start[0], center[1] + offset[1] + current[1] - start[1]]
    };
    let anchor_world = {
        let offset = rotate_fill_vector([anchor_sign[0] * old_width * 0.5, anchor_sign[1] * old_height * 0.5], rotation);
        [center[0] + offset[0], center[1] + offset[1]]
    };
    let delta_world = [moving_world[0] - anchor_world[0], moving_world[1] - anchor_world[1]];
    let delta_local = inverse_rotate_vector(delta_world, rotation);
    let max_extent = f64::from(FILL_BOUNDS_LIMIT) * 2.0;
    let width = if moving_sign[0] == 0.0 { old_width } else { (delta_local[0] * moving_sign[0]).clamp(1.0, max_extent).round() };
    let height = if moving_sign[1] == 0.0 { old_height } else { (delta_local[1] * moving_sign[1]).clamp(1.0, max_extent).round() };
    let anchor_offset = rotate_fill_vector([anchor_sign[0] * width * 0.5, anchor_sign[1] * height * 0.5], rotation);
    let new_center = [anchor_world[0] - anchor_offset[0], anchor_world[1] - anchor_offset[1]];
    if new_center.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let width = width as i32;
    let height = height as i32;
    let x0 = (new_center[0] - f64::from(width) * 0.5).round().clamp(-f64::from(FILL_BOUNDS_LIMIT), f64::from(FILL_BOUNDS_LIMIT - width)) as i32;
    let y0 = (new_center[1] - f64::from(height) * 0.5).round().clamp(-f64::from(FILL_BOUNDS_LIMIT), f64::from(FILL_BOUNDS_LIMIT - height)) as i32;
    Some([x0, y0, x0.saturating_add(width), y0.saturating_add(height)])
}

fn inverse_rotate_vector([x, y]: [f64; 2], rotation: f32) -> [f64; 2] {
    let angle = f64::from(rotation.rem_euclid(360.0)).to_radians();
    let (sin, cos) = angle.sin_cos();
    [cos * x + sin * y, -sin * x + cos * y]
}

fn move_fill_bounds(bounds: [i32; 4], start: [f64; 2], current: [f64; 2]) -> [i32; 4] {
    let width = bounds[2].saturating_sub(bounds[0]).clamp(1, FILL_BOUNDS_LIMIT * 2);
    let height = bounds[3].saturating_sub(bounds[1]).clamp(1, FILL_BOUNDS_LIMIT * 2);
    let dx = if current[0].is_finite() && start[0].is_finite() { (current[0] - start[0]).round().clamp(-2_000_000.0, 2_000_000.0) as i32 } else { 0 };
    let dy = if current[1].is_finite() && start[1].is_finite() { (current[1] - start[1]).round().clamp(-2_000_000.0, 2_000_000.0) as i32 } else { 0 };
    let x0 = bounds[0].saturating_add(dx).clamp(-FILL_BOUNDS_LIMIT, FILL_BOUNDS_LIMIT - width);
    let y0 = bounds[1].saturating_add(dy).clamp(-FILL_BOUNDS_LIMIT, FILL_BOUNDS_LIMIT - height);
    [x0, y0, x0.saturating_add(width), y0.saturating_add(height)]
}

fn wireframe_segments(
    geometry: &photocraft_uv::prepared::Geometry,
    camera: Camera,
    size: [u32; 2],
    rect: Rect,
    hidden_materials: &std::collections::BTreeSet<usize>,
) -> Vec<[Pos2; 2]> {
    let mut segments = Vec::with_capacity(geometry.triangles.len().saturating_mul(3));
    for triangle in &geometry.triangles {
        if hidden_materials.contains(&triangle.material) {
            continue;
        }
        if camera.cull_backfaces && !wireframe_front_facing(triangle.face, camera) {
            continue;
        }
        let Some(points) = triangle
            .positions
            .iter()
            .map(|position| {
                let point = geometry.project_normalized(camera, position.map(f64::from), size)?;
                Some(rect.min + vec2(point[0] as f32 * rect.width(), point[1] as f32 * rect.height()))
            })
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        if let Ok(points) = <Vec<Pos2> as TryInto<[Pos2; 3]>>::try_into(points) {
            for edge in 0..3 {
                segments.push([points[edge], points[(edge + 1) % 3]]);
            }
        }
    }
    segments
}

fn wireframe_front_facing(face: [f32; 3], camera: Camera) -> bool {
    let (sy, cy) = camera.yaw.sin_cos();
    let (sp, cp) = camera.pitch.sin_cos();
    let normal = face.map(f64::from);
    let orientation = if camera.flip_faces { -1.0 } else { 1.0 };
    let view_z = (sp * normal[1] + cp * (-sy * normal[0] + cy * normal[2])) * orientation;
    view_z.is_finite() && view_z > 0.0
}

fn point_in_quad(point: Pos2, quad: [Pos2; 4]) -> bool {
    let mut sign = 0.0_f32;
    for index in 0..4 {
        let a = quad[index];
        let b = quad[(index + 1) % 4];
        let cross = (b.x - a.x) * (point.y - a.y) - (b.y - a.y) * (point.x - a.x);
        if cross.abs() <= 0.01 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

fn projected_face(geometry: &photocraft_uv::prepared::Geometry, positions: [[f32; 3]; 3], camera: Camera, size: [u32; 2], rect: Rect) -> Option<[Pos2; 3]> {
    positions
        .map(|p| {
            let uv = geometry.project_normalized(camera, p.map(f64::from), size)?;
            Some(rect.min + vec2(uv[0] as f32 * rect.width(), uv[1] as f32 * rect.height()))
        })
        .into_iter()
        .collect::<Option<Vec<_>>>()?
        .try_into()
        .ok()
}

fn clip_triangle_to_rect(rect: Rect, triangle: [Pos2; 3]) -> Vec<Pos2> {
    let mut polygon = triangle.to_vec();
    for (axis, edge, sign) in [(0, rect.left(), -1.0_f32), (0, rect.right(), 1.0), (1, rect.top(), -1.0), (1, rect.bottom(), 1.0)] {
        let mut output = Vec::new();
        for (a, b) in polygon.iter().zip(polygon.iter().cycle().skip(1)).take(polygon.len()) {
            let value = |p: Pos2| ((if axis == 0 { p.x } else { p.y }) - edge) * sign;
            let (va, vb) = (value(*a), value(*b));
            if va <= 0.0 {
                output.push(*a);
            }
            if (va <= 0.0) != (vb <= 0.0) {
                output.push(a.lerp(*b, va / (va - vb)));
            }
        }
        polygon = output;
    }
    polygon
}

fn triangle_touches_rect(rect: Rect, triangle: [Pos2; 3]) -> bool {
    !clip_triangle_to_rect(rect, triangle).is_empty()
}

fn point_on_segment(point: Pos2, start: Pos2, end: Pos2) -> bool {
    let edge = end - start;
    let offset = point - start;
    edge.x * offset.y - edge.y * offset.x <= 0.25
        && edge.x * offset.y - edge.y * offset.x >= -0.25
        && offset.dot(edge) >= -0.25
        && offset.dot(edge) <= edge.length_sq() + 0.25
}

fn point_in_polygon(point: Pos2, polygon: &[Pos2]) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    for (a, b) in polygon.iter().zip(polygon.iter().cycle().skip(1)).take(polygon.len()) {
        if point_on_segment(point, *a, *b) {
            return true;
        }
        if (a.y > point.y) != (b.y > point.y) {
            let crossing_x = (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x;
            if point.x < crossing_x {
                inside = !inside;
            }
        }
    }
    inside
}

fn point_in_triangle(point: Pos2, triangle: [Pos2; 3]) -> bool {
    let cross = |a: Pos2, b: Pos2, p: Pos2| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    let a = cross(triangle[0], triangle[1], point);
    let b = cross(triangle[1], triangle[2], point);
    let c = cross(triangle[2], triangle[0], point);
    (a >= -0.01 && b >= -0.01 && c >= -0.01) || (a <= 0.01 && b <= 0.01 && c <= 0.01)
}

fn segment_intersection(a: Pos2, b: Pos2, c: Pos2, d: Pos2) -> Option<Pos2> {
    let ab = b - a;
    let cd = d - c;
    let cross = |u: egui::Vec2, v: egui::Vec2| u.x * v.y - u.y * v.x;
    let denominator = cross(ab, cd);
    if denominator.abs() < 1e-5 {
        return [a, b, c, d].into_iter().find(|p| point_on_segment(*p, a, b) && point_on_segment(*p, c, d));
    }
    let ac = c - a;
    let t = cross(ac, cd) / denominator;
    let u = cross(ac, ab) / denominator;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some(a + ab * t)
}

fn triangle_lasso_intersection_points(polygon: &[Pos2], triangle: [Pos2; 3]) -> Vec<Pos2> {
    if polygon.len() < 3 {
        return Vec::new();
    }
    let mut points: Vec<_> = triangle.iter().copied().filter(|point| point_in_polygon(*point, polygon)).collect();
    points.extend(polygon.iter().copied().filter(|point| point_in_triangle(*point, triangle)));
    for (ta, tb) in triangle.iter().copied().zip(triangle.iter().copied().cycle().skip(1)).take(3) {
        for (la, lb) in polygon.iter().copied().zip(polygon.iter().copied().cycle().skip(1)).take(polygon.len()) {
            if let Some(point) = segment_intersection(ta, tb, la, lb) {
                points.push(point);
            }
        }
    }
    points.dedup_by(|a, b| a.distance_sq(*b) < 0.25);
    points
}

fn selection_area_touches_triangle(area: &crate::uv_box::SelectionArea, triangle: [Pos2; 3]) -> bool {
    match area {
        crate::uv_box::SelectionArea::Rect(rect) => triangle_touches_rect(*rect, triangle),
        crate::uv_box::SelectionArea::Lasso(polygon) => !triangle_lasso_intersection_points(polygon, triangle).is_empty(),
    }
}

fn selection_area_contains_triangle(area: &crate::uv_box::SelectionArea, triangle: [Pos2; 3]) -> bool {
    match area {
        crate::uv_box::SelectionArea::Rect(rect) => triangle.iter().all(|point| rect.contains(*point)),
        crate::uv_box::SelectionArea::Lasso(polygon) => triangle.iter().all(|point| point_in_polygon(*point, polygon)),
    }
}

fn selection_area_intersection_points(area: &crate::uv_box::SelectionArea, triangle: [Pos2; 3]) -> Vec<Pos2> {
    match area {
        crate::uv_box::SelectionArea::Rect(rect) => clip_triangle_to_rect(*rect, triangle),
        crate::uv_box::SelectionArea::Lasso(polygon) => triangle_lasso_intersection_points(polygon, triangle),
    }
}

fn face_is_visible(
    geometry: &photocraft_uv::prepared::Geometry,
    scene: &photocraft_uv::Scene,
    maps: &preview::Materials<'_>,
    frame: (Camera, [u32; 2], Rect),
    face_id: usize,
    triangle: [Pos2; 3],
    area: Rect,
) -> bool {
    let clipped = clip_triangle_to_rect(area, triangle);
    face_visible_at_samples(geometry, scene, maps, frame, face_id, &clipped)
}

fn face_is_visible_in_area(
    geometry: &photocraft_uv::prepared::Geometry,
    scene: &photocraft_uv::Scene,
    maps: &preview::Materials<'_>,
    frame: (Camera, [u32; 2], Rect),
    face_id: usize,
    triangle: [Pos2; 3],
    area: &crate::uv_box::SelectionArea,
) -> bool {
    let samples = selection_area_intersection_points(area, triangle);
    face_visible_at_samples(geometry, scene, maps, frame, face_id, &samples)
}

fn face_visible_at_samples(
    geometry: &photocraft_uv::prepared::Geometry,
    scene: &photocraft_uv::Scene,
    maps: &preview::Materials<'_>,
    frame: (Camera, [u32; 2], Rect),
    face_id: usize,
    samples: &[Pos2],
) -> bool {
    let (camera, size, viewport) = frame;
    if samples.is_empty() || viewport.width() <= 0.0 || viewport.height() <= 0.0 {
        return false;
    }
    let sum = samples.iter().fold(Pos2::ZERO, |sum, point| pos2(sum.x + point.x, sum.y + point.y));
    let center = pos2(sum.x / samples.len() as f32, sum.y / samples.len() as f32);
    samples.iter().any(|point| {
        let sample = point.lerp(center, 0.08);
        let at = [f64::from((sample.x - viewport.left()) / viewport.width()), f64::from((sample.y - viewport.top()) / viewport.height())];
        geometry.pick(scene, camera, maps, at, size) == Some(face_id)
    })
}

fn update_polygon_selection(state: &mut State, faces: &[usize], modifiers: egui::Modifiers, owner: photocraft_doc::DocId) {
    if state.polygon_document.is_some_and(|previous| previous != owner) {
        state.selected_polygons.clear();
    }
    if modifiers.ctrl {
        for face in faces {
            state.selected_polygons.remove(face);
        }
    } else {
        if !modifiers.shift {
            state.selected_polygons.clear();
        }
        state.selected_polygons.extend(faces.iter().copied());
    }
    state.polygon_document = (!state.selected_polygons.is_empty()).then_some(owner);
}

/// Persist the temporary 3D face highlight as the document's ordinary selection before another
/// tool takes over. The engine rasterizes all triangles in one history step.
pub(crate) fn commit_polygon_selection(app: &mut PhotocraftApp, state: &mut State) -> bool {
    if !state.select_shells || state.selection_target != SelectionTarget::Polygons {
        return true;
    }
    let owner = state.polygon_document;
    let polygons = app
        .session
        .uv
        .scene
        .as_ref()
        .zip(owner.and_then(|owner| app.session.documents().iter().find(|st| st.doc.id == owner)))
        .map(|(scene, st)| {
            state
                .selected_polygons
                .iter()
                .filter_map(|&id| {
                    let tri = scene.triangles.get(id)?;
                    Some(tri.corners.map(|corner| [corner.uv[0] * f64::from(st.doc.size.width), (1.0 - corner.uv[1]) * f64::from(st.doc.size.height)]))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !polygons.is_empty()
        && let Err(error) = app.run("select.uvPolygons", json!({"document":owner.map(|id|id.0),"polygons":polygons}))
    {
        app.ui.status = error;
        app.ui.status_error = true;
        return false;
    }
    state.selected_polygons.clear();
    state.polygon_document = None;
    app.uv_cache.rendered = None;
    true
}

pub(crate) fn commit_current_polygon_selection(app: &mut PhotocraftApp) -> bool {
    let mut state = std::mem::take(&mut app.ui.uv_workbench);
    let committed = commit_polygon_selection(app, &mut state);
    app.ui.uv_workbench = state;
    committed
}

pub(crate) fn clear_polygon_selection(app: &mut PhotocraftApp) {
    app.ui.uv_workbench.selected_polygons.clear();
    app.ui.uv_workbench.polygon_document = None;
    app.uv_cache.rendered = None;
}

fn selection_mode(app: &PhotocraftApp, index: usize) -> bool {
    app.ui.uv_workbench.show_overlay
        && app.ui.uv_workbench.select_shells
        && app.session.uv.topology.is_some()
        && !canvas_materials(app, index).is_empty()
        && app.drag.is_none()
        && app.ui.transform.is_none()
}

/// Called before the ordinary tool's press handling, so a shell click cannot paint.
/// Space/middle-button navigation and modal tools are handled by the canvas first.
pub(crate) fn canvas_input(
    app: &mut PhotocraftApp,
    ui: &egui::Ui,
    xf: &crate::canvas::ViewXform,
    response: &egui::Response,
    index: usize,
    toolbar: Option<Rect>,
) -> bool {
    if app.ui.workspace != "3D" {
        return false;
    }
    let over_toolbar = ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| toolbar.is_some_and(|r| r.contains(p)));
    if over_toolbar {
        return true;
    }
    if !selection_mode(app, index) {
        return false;
    }
    let lasso = app.ui.uv_workbench.selection_target == SelectionTarget::Polygons && app.ui.uv_workbench.selection_gesture == SelectionGesture::Lasso;
    let polygon_selection = app.ui.uv_workbench.selection_target == SelectionTarget::Polygons;
    let polygon_selection_started = polygon_selection && response.contains_pointer() && ui.input(|i| i.pointer.primary_pressed() && !i.modifiers.alt);
    if polygon_selection_started {
        begin_polygon_selection_wireframe(&mut app.ui.uv_workbench);
    }
    let selection_drag_was_active = app.uv_cache.shell_box_2d.is_some();
    let selection = crate::uv_box::input(ui, response, &mut app.uv_cache.shell_box_2d, lasso);
    let polygon_selection_cancelled =
        polygon_selection && app.ui.uv_workbench.temporary_polygon_wireframe && ui.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused);
    if polygon_selection
        && ((selection_drag_was_active || polygon_selection_started)
            && app.uv_cache.shell_box_2d.is_none()
            && ui.input(|i| i.pointer.primary_released() || !i.pointer.primary_down() || !i.focused)
            || polygon_selection_cancelled)
    {
        finish_polygon_selection_wireframe(&mut app.ui.uv_workbench);
        ui.ctx().request_repaint();
    }
    if let Some((area, mods)) = selection {
        let scene = app.session.uv.scene.clone();
        let doc = app.session.documents().get(index).map(|st| st.doc.clone());
        if let (Some(scene), Some(doc)) = (scene, doc) {
            if app.ui.uv_workbench.selection_target == SelectionTarget::Polygons {
                let materials = canvas_materials(app, index);
                let faces: Vec<_> = scene
                    .triangles
                    .iter()
                    .enumerate()
                    .filter(|(_, tri)| materials.contains(&tri.material) && !app.session.uv.hidden_materials.contains(&tri.material))
                    .filter_map(|(id, tri)| {
                        let points = tri
                            .corners
                            .map(|c| xf.to_screen((c.uv[0] * f64::from(doc.size.width)) as f32, ((1.0 - c.uv[1]) * f64::from(doc.size.height)) as f32));
                        let selected = if app.ui.uv_workbench.shell_enclosed {
                            selection_area_contains_triangle(&area, points)
                        } else {
                            selection_area_touches_triangle(&area, points)
                        };
                        selected.then_some(id)
                    })
                    .collect();
                update_polygon_selection(&mut app.ui.uv_workbench, &faces, mods, doc.id);
                app.uv_cache.rendered = None;
            } else if let crate::uv_box::SelectionArea::Rect(area) = area {
                let materials = canvas_materials(app, index);
                let material = if materials.contains(&app.session.uv.material) { app.session.uv.material } else { materials.first().copied().unwrap_or(0) };
                let mut state = std::mem::take(&mut app.ui.uv_workbench);
                crate::uv_box::select(
                    app,
                    &mut state,
                    area,
                    mods,
                    material,
                    false,
                    |id| {
                        let t = scene.triangles.get(id)?;
                        Some(
                            t.corners
                                .map(|c| xf.to_screen((c.uv[0] * f64::from(doc.size.width)) as f32, ((1.0 - c.uv[1]) * f64::from(doc.size.height)) as f32)),
                        )
                    },
                    |_, _, _| true,
                );
                app.ui.uv_workbench = state;
            }
        }
    }
    if response.clicked()
        && let Some(p) = response.interact_pointer_pos()
    {
        if app.ui.uv_workbench.selection_target == SelectionTarget::Polygons {
            select_polygon_at_canvas(app, index, xf.to_doc(p), ui.input(|i| i.modifiers));
        } else {
            pick_canvas(app, ui, index, xf.to_doc(p));
        }
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    true
}

fn select_polygon_at_canvas(app: &mut PhotocraftApp, index: usize, point: [f64; 2], modifiers: egui::Modifiers) {
    let (Some(scene), Some(document)) = (app.session.uv.scene.as_ref(), app.session.documents().get(index)) else { return };
    let uv = [point[0] / f64::from(document.doc.size.width.max(1)), 1.0 - point[1] / f64::from(document.doc.size.height.max(1))];
    if uv.iter().any(|value| !value.is_finite()) {
        return;
    }
    let materials = canvas_materials(app, index);
    let through = app.ui.uv_workbench.selection_depth == SelectionDepth::ThroughMesh;
    let mut candidates: Vec<_> = scene
        .triangles
        .iter()
        .enumerate()
        .filter(|(_, triangle)| materials.contains(&triangle.material) && !app.session.uv.hidden_materials.contains(&triangle.material))
        .filter_map(|(id, triangle)| photocraft_uv::topology::barycentric(uv, triangle.corners.map(|corner| corner.uv)).map(|_| (id, triangle.material)))
        .collect();
    candidates.sort_by_key(|(_, material)| (*material != app.session.uv.material, *material));
    let faces: Vec<_> = if through { candidates.iter().map(|(id, _)| *id).collect() } else { candidates.first().map(|(id, _)| vec![*id]).unwrap_or_default() };
    update_polygon_selection(&mut app.ui.uv_workbench, &faces, modifiers, document.doc.id);
    app.uv_cache.rendered = None;
}

fn pick_canvas(app: &mut PhotocraftApp, ui: &egui::Ui, index: usize, point: [f64; 2]) {
    let (Some(scene), Some(topology), Some(document)) = (app.session.uv.scene.clone(), app.session.uv.topology.clone(), app.session.documents().get(index))
    else {
        return;
    };
    let document_id = document.doc.id;
    let uv = [point[0] / f64::from(document.doc.size.width.max(1)), 1.0 - point[1] / f64::from(document.doc.size.height.max(1))];
    if uv.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return;
    }
    let mut state = std::mem::take(&mut app.ui.uv_workbench);
    state.overlap = canvas_materials(app, index)
        .into_iter()
        .filter(|material| !app.session.uv.hidden_materials.contains(material))
        .flat_map(|m| topology.pick_uv(&scene, uv, m))
        .collect();
    // Prefer the current material for stacked shells sharing a texture document.
    let shell =
        state.overlap.iter().find(|&&s| topology.shells.get(s).is_some_and(|s| s.material == app.session.uv.material)).or(state.overlap.first()).copied();
    if let Some(shell) = shell
        && let Some(s) = topology.shells.get(shell)
    {
        let current = app.session.uv.current_channel();
        let kind = std::iter::once(current)
            .chain(Channel::MAPS)
            .find(|kind| app.session.uv.map_bindings(*kind).get(s.material).copied().flatten() == Some(document_id))
            .unwrap_or(Channel::Color);
        command(app, &mut state, "uv.material.select", json!({"material":s.material,"channel":kind.key()}));
        if state.selection_depth == SelectionDepth::ThroughMesh {
            let shells: Vec<_> =
                state.overlap.iter().copied().filter(|id| topology.shells.get(*id).is_some_and(|shell| shell.material == s.material)).collect();
            let modifiers = ui.input(|i| i.modifiers);
            choose_many(app, &mut state, ui, s.material, &shells, modifiers);
        } else {
            choose(app, &mut state, ui, s.material, shell);
        }
    }
    app.ui.uv_workbench = state;
}

/// Clip a wire segment in UV space before applying the canvas's rotation/flip.
fn clip_tile(segment: [[f64; 2]; 2]) -> Option<[[f64; 2]; 2]> {
    let [a, b] = segment;
    let delta = [b[0] - a[0], b[1] - a[1]];
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for (p, q) in [(-delta[0], a[0]), (delta[0], 1.0 - a[0]), (-delta[1], a[1]), (delta[1], 1.0 - a[1])] {
        if p.abs() < 1e-15 {
            if q < 0.0 {
                return None;
            }
        } else if p < 0.0 {
            lo = lo.max(q / p);
        } else {
            hi = hi.min(q / p);
        }
        if lo > hi {
            return None;
        }
    }
    Some([lo, hi].map(|t| [a[0] + delta[0] * t, a[1] + delta[1] * t]))
}

fn choose(app: &mut PhotocraftApp, state: &mut State, ui: &egui::Ui, material: usize, shell: usize) {
    choose_many(app, state, ui, material, &[shell], ui.input(|i| i.modifiers));
}

fn choose_many(app: &mut PhotocraftApp, state: &mut State, _ui: &egui::Ui, material: usize, shells: &[usize], modifiers: egui::Modifiers) {
    let mode = if modifiers.ctrl {
        "subtract"
    } else if modifiers.shift {
        "add"
    } else {
        "replace"
    };
    if command(app, state, "uv.shell.select", json!({"material":material,"shells":shells,"mode":mode})) {
        let p = mask_params(state);
        command(app, state, "uv.mask.apply", p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygon_lasso_selects_intersection_and_supports_full_enclosure() {
        let triangle = [pos2(0.0, 0.0), pos2(12.0, 0.0), pos2(0.0, 12.0)];
        let tiny_lasso = vec![pos2(1.0, 1.0), pos2(4.0, 1.0), pos2(1.0, 4.0)];
        let crossing_lasso = vec![pos2(8.0, -2.0), pos2(14.0, -2.0), pos2(14.0, 3.0), pos2(8.0, 3.0)];
        let enclosing_lasso = vec![pos2(-2.0, -2.0), pos2(14.0, -2.0), pos2(14.0, 14.0), pos2(-2.0, 14.0)];
        let separate_lasso = vec![pos2(8.0, 8.0), pos2(12.0, 8.0), pos2(12.0, 12.0), pos2(8.0, 12.0)];
        assert!(selection_area_touches_triangle(&crate::uv_box::SelectionArea::Lasso(tiny_lasso.clone()), triangle));
        assert!(selection_area_touches_triangle(&crate::uv_box::SelectionArea::Lasso(crossing_lasso), triangle), "lasso edges crossing a face select it");
        assert!(
            !selection_area_touches_triangle(&crate::uv_box::SelectionArea::Lasso(separate_lasso), triangle),
            "bounding-box overlap alone does not select a face"
        );
        assert!(!selection_area_contains_triangle(&crate::uv_box::SelectionArea::Lasso(tiny_lasso), triangle));
        assert!(selection_area_contains_triangle(&crate::uv_box::SelectionArea::Lasso(enclosing_lasso), triangle));
    }

    #[test]
    fn polygon_selection_keeps_replace_add_and_subtract_modifier_semantics() {
        let owner = photocraft_doc::DocId(7);
        let mut state = State::default();
        update_polygon_selection(&mut state, &[1, 2], egui::Modifiers::NONE, owner);
        update_polygon_selection(&mut state, &[3], egui::Modifiers { shift: true, ..Default::default() }, owner);
        assert_eq!(state.selected_polygons, std::collections::BTreeSet::from([1, 2, 3]));
        update_polygon_selection(&mut state, &[2], egui::Modifiers { ctrl: true, ..Default::default() }, owner);
        assert_eq!(state.selected_polygons, std::collections::BTreeSet::from([1, 3]));
        update_polygon_selection(&mut state, &[4], egui::Modifiers::NONE, owner);
        assert_eq!(state.selected_polygons, std::collections::BTreeSet::from([4]));
    }

    #[test]
    fn polygon_settings_offer_lasso_and_depth_modes_without_mask_margins() {
        use egui_kittest::kittest::Queryable as _;

        let session = photocraft_engine::Session::new();
        let mut app = PhotocraftApp::new(session, Default::default());
        app.ui.uv_workbench.selection_target = SelectionTarget::Polygons;
        let mut harness =
            egui_kittest::Harness::builder().with_size(vec2(700.0, 500.0)).build_ui_state(|ui, app: &mut PhotocraftApp| shell_settings(app, ui), app);
        harness.run_steps(3);
        assert!(harness.query_all_by_label("Rectangle").next().is_some());
        assert!(harness.query_all_by_label("Lasso").next().is_some());
        assert!(harness.query_all_by_label("Visible only").next().is_some());
        assert!(harness.query_all_by_label("Through mesh").next().is_some());
        for label in ["General margin, px", "U margin, px", "V margin, px", "Inward feather, px", "Outward padding, px"] {
            assert!(harness.query_all_by_label(label).next().is_none(), "{label} is hidden for polygon selection");
        }
    }

    #[test]
    fn mask_surface_preview_is_throttled_but_pixels_and_final_flush_are_not() {
        let now = Instant::now();
        assert!(!preview_interval_elapsed(Some(now.checked_sub(Duration::from_millis(32)).unwrap()), now), "mask live updates stay below 30 Hz");
        assert!(preview_interval_elapsed(Some(now.checked_sub(Duration::from_millis(33)).unwrap()), now), "mask live updates resume at the 33 ms boundary");
        assert!(!preview_refresh_due(false, true, true, false, false, false, false), "mask surface waits for the cadence interval");
        assert!(preview_refresh_due(false, true, true, false, false, true, false), "mask surface refreshes when the interval expires");
        assert!(preview_refresh_due(false, true, false, false, false, false, false), "pixel preview remains immediate");
        assert!(preview_refresh_due(false, false, false, false, true, false, true), "ending a deferred preview forces its final refresh");
    }

    #[test]
    fn mask_surface_preview_throttle_is_wired_only_to_native_3d_mask_strokes() {
        let native_3d_mask_preview =
            |native_stroke, workspace_3d, mask_target| should_throttle_mask_surface_preview(true, false, native_stroke, workspace_3d, mask_target);
        let native_3d_mask = native_3d_mask_preview(true, true, true);
        let two_d_mask = native_3d_mask_preview(false, true, true);
        assert!(native_3d_mask, "native 3D mask preview is throttled");
        assert!(!two_d_mask, "2D UV canvas mask preview stays immediate");
        assert!(!native_3d_mask_preview(true, true, false), "native 3D pixel strokes stay immediate");
        assert!(!native_3d_mask_preview(true, false, true), "non-3D preview paths stay immediate");
        assert!(!should_throttle_mask_surface_preview(true, true, true, true, true), "opacity previews keep their existing cadence");
        assert!(!preview_refresh_due(false, true, native_3d_mask, false, true, false, false), "native 3D mask waits for cadence");
        assert!(preview_refresh_due(false, true, two_d_mask, false, true, false, false), "2D mask keeps immediate refresh");
    }

    fn anchor_world(bounds: [i32; 4], rotation: f32, moving_handle: usize) -> [f64; 2] {
        let sign = FILL_HANDLE_SIGNS[(moving_handle + 4) % 8];
        let center = [(f64::from(bounds[0]) + f64::from(bounds[2])) * 0.5, (f64::from(bounds[1]) + f64::from(bounds[3])) * 0.5];
        let offset = rotate_fill_vector([sign[0] * f64::from(bounds[2] - bounds[0]) * 0.5, sign[1] * f64::from(bounds[3] - bounds[1]) * 0.5], rotation);
        [center[0] + offset[0], center[1] + offset[1]]
    }

    fn moved_handle_world(bounds: [i32; 4], rotation: f32, handle: usize) -> [f64; 2] {
        let sign = FILL_HANDLE_SIGNS[handle];
        let center = [(f64::from(bounds[0]) + f64::from(bounds[2])) * 0.5, (f64::from(bounds[1]) + f64::from(bounds[3])) * 0.5];
        let offset = rotate_fill_vector([sign[0] * f64::from(bounds[2] - bounds[0]) * 0.5, sign[1] * f64::from(bounds[3] - bounds[1]) * 0.5], rotation);
        [center[0] + offset[0], center[1] + offset[1]]
    }

    #[test]
    fn rotated_fill_resize_keeps_opposite_anchor_with_press_offset_and_coalesced_steps() {
        let bounds = [100, 200, 300, 400];
        let rotation = 37.0;
        for handle in [4, 3] {
            // bottom-right corner and right side
            let handle_world = moved_handle_world(bounds, rotation, handle);
            let press = [handle_world[0] + 2.25, handle_world[1] - 1.75];
            let large_delta = rotate_fill_vector([350.0, 270.0], rotation);
            let final_pointer = [press[0] + large_delta[0], press[1] + large_delta[1]];
            let expected = resize_fill_bounds(bounds, rotation, handle, press, final_pointer).unwrap();
            // A worker may observe only the latest coalesced pointer value; results must not
            // depend on the intermediate frames that were (or were not) rendered.
            let mut sequential = bounds;
            let first_delta = rotate_fill_vector([90.0, 70.0], rotation);
            let second_delta = rotate_fill_vector([200.0, 160.0], rotation);
            for pointer in [[press[0] + first_delta[0], press[1] + first_delta[1]], [press[0] + second_delta[0], press[1] + second_delta[1]], final_pointer] {
                sequential = resize_fill_bounds(bounds, rotation, handle, press, pointer).unwrap();
            }
            let coalesced = resize_fill_bounds(bounds, rotation, handle, press, final_pointer).unwrap();
            assert_eq!(sequential, expected);
            assert_eq!(coalesced, expected);
            assert!(coalesced[2] - coalesced[0] >= 1 && coalesced[3] - coalesced[1] >= 1);
            let before = anchor_world(bounds, rotation, handle);
            let after = anchor_world(coalesced, rotation, handle);
            assert!((before[0] - after[0]).abs() <= 1.25 && (before[1] - after[1]).abs() <= 1.25, "handle {handle}: anchor moved {before:?} → {after:?}");
        }
    }

    #[test]
    fn fill_resize_clamps_only_at_anchor_crossing_and_rejects_non_finite_input() {
        let bounds = [0, 0, 100, 80];
        let handle = 4;
        let press = moved_handle_world(bounds, 45.0, handle);
        let shrink_delta = rotate_fill_vector([-40.0, -30.0], 45.0);
        let shrunk = resize_fill_bounds(bounds, 45.0, handle, press, [press[0] + shrink_delta[0], press[1] + shrink_delta[1]]).unwrap();
        assert_eq!([shrunk[2] - shrunk[0], shrunk[3] - shrunk[1]], [60, 50], "valid shrinking remains continuous above the anchor");
        let crossing_delta = rotate_fill_vector([-150.0, -120.0], 45.0);
        let crossed = resize_fill_bounds(bounds, 45.0, handle, press, [press[0] + crossing_delta[0], press[1] + crossing_delta[1]]).unwrap();
        assert_eq!([crossed[2] - crossed[0], crossed[3] - crossed[1]], [1, 1], "minimum size applies once the moving corner crosses its anchor");
        assert!(resize_fill_bounds(bounds, 45.0, handle, press, [f64::NAN, press[1]]).is_none());
        assert!(resize_fill_bounds(bounds, f32::INFINITY, handle, press, press).is_none());
    }

    #[test]
    fn fill_drag_recovers_when_input_loses_the_release_event() {
        assert!(!fill_drag_stopped(false, false, false, true, false), "drag start cannot be mistaken for a lost release");
        assert!(!fill_drag_stopped(true, true, false, true, false), "an owned drag stays active while focused and primary remains down");
        assert!(!fill_drag_stopped(true, true, false, true, true), "a widget stop cannot end an owned gesture while primary remains down");
        assert!(fill_drag_stopped(true, false, false, true, false), "input reset ends an owned drag without a release event");
        assert!(fill_drag_stopped(true, true, false, false, false), "blur ends an owned drag even when the OS still reports primary down");
    }

    #[test]
    fn wireframe_projects_visible_polygons_and_default_shell_margin_is_zero() {
        let app = fixture_app();
        let scene = app.session.uv.scene.as_ref().unwrap();
        let topology = app.session.uv.topology.as_ref().unwrap();
        let geometry = photocraft_uv::prepared::Geometry::new(scene, topology).unwrap();
        let camera = Camera::default();
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0));
        let segments = wireframe_segments(&geometry, camera, [400, 400], rect, &Default::default());
        let visible = geometry.triangles.iter().filter(|triangle| wireframe_front_facing(triangle.face, camera)).count();
        assert_eq!(segments.len(), visible * 3, "back-facing triangles are omitted when culling is enabled");
        let state = State::default();
        assert_eq!(state.guard, 0.0);
        assert!(!state.show_wireframe);
    }

    #[test]
    fn temporary_wireframe_covers_snap_and_polygon_selection_without_mutating_manual_setting() {
        let mut state = State::default();
        assert!(!wireframe_visible(&state, false));

        assert!(wireframe_visible(&state, true), "pending symmetry snap reveals the polygon grid");
        assert!(!state.show_wireframe, "snap visibility does not change the persisted manual setting");
        assert!(!wireframe_visible(&state, false), "the grid hides when snap is completed or cancelled");

        state.select_shells = true;
        state.selection_target = SelectionTarget::Polygons;
        update_polygon_wireframe_for_mode(&mut state, SelectionTarget::UvShells);
        assert!(wireframe_visible(&state, false), "entering polygon selection reveals the grid");
        finish_polygon_selection_wireframe(&mut state);
        assert!(!wireframe_visible(&state, false), "finishing or cancelling the polygon gesture hides the temporary grid");
        begin_polygon_selection_wireframe(&mut state);
        assert!(wireframe_visible(&state, false), "a later polygon gesture temporarily reveals the grid again");
        finish_polygon_selection_wireframe(&mut state);

        state.show_wireframe = true;
        assert!(wireframe_visible(&state, false), "manual wireframe remains visible after temporary modes end");
        assert!(state.show_wireframe, "temporary modes never mutate the manual preference");

        state.show_wireframe = false;
        state.temporary_polygon_wireframe = true;
        state.selection_target = SelectionTarget::UvShells;
        update_polygon_wireframe_for_mode(&mut state, SelectionTarget::Polygons);
        assert!(!wireframe_visible(&state, false), "leaving polygon selection clears its temporary request");
    }

    #[test]
    fn gray_mask_view_clears_idle_overlays_but_preserves_active_polygon_gesture() {
        let ctx = egui::Context::default();
        let mut state = State { select_shells: true, selection_target: SelectionTarget::Polygons, temporary_polygon_wireframe: true, ..Default::default() };
        crate::uv_symmetry::put(&ctx, crate::uv_symmetry::Settings { enabled: true, snap_pending: true, ..Default::default() });

        cancel_temporary_wireframes_for_gray_mask(&mut state, &ctx, false, false);
        assert!(state.temporary_polygon_wireframe, "non-mask views leave the polygon-selection request intact");
        assert!(crate::uv_symmetry::get(&ctx).snap_pending, "non-mask views leave vertex snapping armed");

        cancel_temporary_wireframes_for_gray_mask(&mut state, &ctx, true, true);
        assert!(state.temporary_polygon_wireframe, "an active polygon gesture keeps its grid in Gray view");
        assert!(!crate::uv_symmetry::get(&ctx).snap_pending, "Gray view cancels pending vertex snapping");
        assert!(wireframe_visible(&state, false));

        cancel_temporary_wireframes_for_gray_mask(&mut state, &ctx, true, false);

        assert!(!state.temporary_polygon_wireframe);
        assert!(!crate::uv_symmetry::get(&ctx).snap_pending);
        assert!(!wireframe_visible(&state, crate::uv_symmetry::get(&ctx).snap_pending));

        state.show_wireframe = true;
        assert!(wireframe_visible(&state, false), "explicit manual wireframe remains available in Gray view");
    }

    #[test]
    fn rotated_fill_manipulator_uses_the_transformed_quad_for_outline_and_hit_testing() {
        let corners = fill_corners([0, 0, 8, 4], 45.0).map(|[x, y]| pos2(x, y));
        assert!(point_in_quad(pos2(4.0, 2.0), corners));
        assert!(!point_in_quad(pos2(0.0, -2.0), corners), "empty corners of the rotated AABB must not start a drag");
        let local = inverse_rotate_point([4.0, 2.0], [0, 0, 8, 4], 45.0);
        assert!((local[0] - 4.0).abs() < 1e-5 && (local[1] - 2.0).abs() < 1e-5);
    }

    #[test]
    fn fill_overlay_hit_testing_does_not_claim_nearby_canvas_clicks() {
        let corners = [pos2(0.0, 0.0), pos2(100.0, 0.0), pos2(100.0, 100.0), pos2(0.0, 100.0)];
        let rotation = pos2(50.0, -24.0);
        assert!(fill_overlay_contains(pos2(50.0, 50.0), corners, rotation), "the Fill interior belongs to the overlay");
        assert!(fill_overlay_contains(pos2(0.0, 0.0), corners, rotation), "resize handles belong to the overlay");
        assert!(fill_overlay_contains(rotation, corners, rotation), "the rotation handle belongs to the overlay");
        assert!(
            !fill_overlay_contains(pos2(20.0, -20.0), corners, rotation),
            "a nearby canvas point outside the quad and handles remains available to the active tool"
        );
    }

    fn input_key(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, key: egui::Key, mods: egui::Modifiers) {
        h.event(egui::Event::ModifiersChanged(mods));
        h.event(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: mods });
        h.step();
        h.event(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: mods });
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.step();
    }
    #[test]
    fn opacity_affects_render_and_pick_with_and_without_color_but_raw_view_stays_visible() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        let ctx = egui::Context::default();
        let mut cache = Cache::default();
        let scene = app.session.uv.scene.clone().unwrap();
        let g = photocraft_uv::prepared::Geometry::new(&scene, app.session.uv.topology.as_ref().unwrap()).unwrap();
        let camera = Camera::default();
        let t = &g.triangles[0];
        let p = std::array::from_fn(|i| t.positions.iter().map(|p| f64::from(p[i])).sum::<f64>() / 3.0);
        let at = g.project_normalized(camera, p, [400, 400]).unwrap();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        assert!(g.hit(&scene, camera, &preview::Materials { colors: &cache.textures, ..Default::default() }, at, [400, 400]).is_some());
        app.run("uv.texture.create", json!({"channel":"opacity","width":16,"height":16})).unwrap();
        app.run("edit.fill", json!({"color":"#000000"})).unwrap();
        for color in [true, false] {
            if !color {
                app.session.uv.bindings.fill(None);
            }
            cache.display_channel = Channel::Material;
            refresh(&mut app, &ctx, &mut cache).unwrap();
            let maps = preview::Materials { colors: &cache.textures, ..Default::default() };
            assert!(g.hit(&scene, camera, &maps, at, [400, 400]).is_none());
            cache.display_channel = Channel::Opacity;
            refresh(&mut app, &ctx, &mut cache).unwrap();
            let maps = preview::Materials { colors: &cache.textures, ..Default::default() };
            assert!(g.hit(&scene, Camera { channel: Channel::Opacity, ..camera }, &maps, at, [400, 400]).is_some());
            assert!(cache.textures.iter().flatten().flat_map(|im| &im.rgba).all(|p| p[0] == 0.0 && p[3] == 1.0));
        }
    }
    #[test]
    fn compact_slots_wrap_popup_wheel_fast_path_and_single_undo_key() {
        let _gpu = photocraft_testkit::gpu_lock();
        use egui_kittest::kittest::Queryable as _;
        assert!(crate::icons::exists("uv-perspective") && crate::icons::exists("uv-orthographic"));
        let mut app = fixture_app();
        app.ui.workspace = "3D".into();
        crate::menus::apply_workspace(&mut app);
        app.run("uv.texture.create", json!({"channel":"normal","width":32,"height":32})).unwrap();
        app.run("uv.material.select", json!({"channel":"baseColor"})).unwrap();
        app.ui.tool = crate::state::Tool::Brush;
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 1000.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            app.set_uv_wgpu(cc.wgpu_render_state.clone().unwrap()).unwrap();
            app
        });
        h.step();
        h.step();
        assert_eq!(h.state().uv_cache.texture_targets.len(), 9);
        assert!(h.state().uv_cache.texture_targets.iter().all(|(r, _, _)| r.height() <= 40.0));
        let point = h.state().uv_cache.viewport.unwrap().center();
        h.event(egui::Event::PointerMoved(point));
        h.step();
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, 60.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        assert!(h.state().uv_cache.wheel_active.is_some());
        assert_eq!(h.state().uv_cache.gpu.as_ref().unwrap().frame_info().unwrap().1, 1, "wheel uses navigation MSAA policy");
        // Wait egui's smooth-wheel tail out using frames; clear the wall-clock latch for this deterministic harness.
        for _ in 0..40 {
            h.step();
        }
        h.state_mut().uv_cache.wheel_active = None;
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.step();
        h.step();
        h.get_by_label("Paint Wrap").click();
        h.step();
        assert!(h.state().ui.uv_workbench.paint_wrap);
        if let Ok(path) = std::env::var("PHOTOCRAFT_UV_WRAP_SCREENSHOT") {
            h.step();
            h.render().unwrap().save(path).unwrap();
        }
        let revision = h.state().session.active().unwrap().revision;
        let dismiss = {
            let g = &h.state().uv_cache.geometry.as_ref().unwrap().1;
            let t = &g.triangles[0];
            let center = std::array::from_fn(|i| t.positions.iter().map(|p| f64::from(p[i])).sum::<f64>() / 3.0);
            let size = h.state().uv_cache.gpu.as_ref().unwrap().frame_info().unwrap().0;
            let at = g.project_normalized(h.state().ui.uv_workbench.camera(), center, size).unwrap();
            let rect = h.state().uv_cache.viewport.unwrap();
            rect.min + vec2(at[0] as f32 * rect.width(), at[1] as f32 * rect.height())
        };
        h.event(egui::Event::PointerMoved(dismiss));
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: dismiss, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        assert!(h.state().ui.brush_picker.is_none());
        assert_eq!(h.state().session.active().unwrap().revision, revision, "dismissing the picker must not paint the model");
        let index = h.state().session.active_index().unwrap();
        let before = h.state().session.documents()[index].doc.clone();
        h.state_mut().run("edit.fill", json!({"color":"#ff0000"})).unwrap();
        input_key(&mut h, egui::Key::Z, egui::Modifiers { ctrl: true, command: true, ..egui::Modifiers::NONE });
        assert_eq!(h.state().session.documents()[index].doc.layers[0].surface(), before.layers[0].surface(), "first Ctrl+Z undoes pixels");
        // Empty History plus a still-held first stroke must enable the *keyboard*
        // command before the live surface is committed by run(edit.undo).
        h.state_mut().ui.uv_workbench.paint_wrap = false;
        let (scene, geometry) = h.state().uv_cache.geometry.as_ref().unwrap();
        let t = &geometry.triangles[0];
        let center = std::array::from_fn(|i| t.positions.iter().map(|p| f64::from(p[i])).sum::<f64>() / 3.0);
        let camera = h.state().ui.uv_workbench.camera();
        let size = h.state().uv_cache.gpu.as_ref().unwrap().frame_info().unwrap().0;
        let at = geometry.project_normalized(camera, center, size).unwrap();
        let rect = h.state().uv_cache.viewport.unwrap();
        let pos = rect.min + vec2(at[0] as f32 * rect.width(), at[1] as f32 * rect.height());
        assert!(geometry.hit(scene, camera, &preview::Materials::default(), at, size).is_some());
        h.event(egui::Event::PointerMoved(pos));
        h.step();
        h.state_mut().uv_cache.wheel_active = Some(Instant::now());
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.step();
        assert!(pending_paint(h.state()));
        input_key(&mut h, egui::Key::Z, egui::Modifiers { ctrl: true, command: true, ..egui::Modifiers::NONE });
        assert!(!pending_paint(h.state()));
        assert_eq!(h.state().session.documents()[index].doc.layers[0].surface(), before.layers[0].surface());
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.step();
    }
    #[test]
    fn ctrl_right_drag_resizes_in_split_3d_and_2d_without_painting() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        app.ui.workspace = "3D".into();
        crate::menus::apply_workspace(&mut app);
        app.ui.uv_workbench.split_view = true;
        app.ui.tool = crate::state::Tool::Brush;
        app.run("tools.setBrush", json!({"brush":{"size":20,"hardness":0.333,"opacity":0.617}})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 900.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            app.set_uv_wgpu(cc.wgpu_render_state.clone().unwrap()).unwrap();
            app
        });
        h.step();
        h.step();
        let revision = h.state().session.active().unwrap().revision;
        for model in [true, false] {
            if !model {
                h.state_mut().ui.uv_workbench.view_3d = false;
                h.state_mut().ui.uv_workbench.split_view = false;
                h.step();
            }
            let at = if model { h.state().uv_cache.viewport.unwrap().center() } else { h.state().last_canvas_rect.center() };
            let camera = (h.state().ui.uv_workbench.yaw, h.state().ui.uv_workbench.pitch, h.state().ui.uv_workbench.zoom);
            let brush = h.state().session.tools.brush.clone();
            h.event(egui::Event::PointerMoved(at));
            h.step();
            h.event(egui::Event::ModifiersChanged(egui::Modifiers { ctrl: true, command: true, ..egui::Modifiers::NONE }));
            h.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers: egui::Modifiers { ctrl: true, command: true, ..egui::Modifiers::NONE },
            });
            h.step();
            h.event(egui::Event::PointerMoved(at + vec2(10.0, 20.0)));
            h.step();
            h.event(egui::Event::PointerMoved(at + vec2(30.0, 60.0)));
            h.step();
            assert!(h.state().session.tools.brush.size > brush.size, "model={model}");
            assert_eq!(h.state().session.tools.brush.hardness, brush.hardness);
            assert_eq!(h.state().session.tools.brush.opacity, brush.opacity);
            // Releasing Ctrl first must not turn the rest of the drag into painting/navigation.
            h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
            h.event(egui::Event::PointerMoved(at + vec2(40.0, 80.0)));
            h.step();
            h.event(egui::Event::PointerButton {
                pos: at + vec2(40.0, 80.0),
                button: egui::PointerButton::Secondary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
            assert!(h.state().brush_resize.is_none() && h.state().drag.is_none() && h.state().ui.brush_picker.is_none());
            assert_eq!(h.state().session.active().unwrap().revision, revision, "resize must not change texture pixels");
            assert_eq!((h.state().ui.uv_workbench.yaw, h.state().ui.uv_workbench.pitch, h.state().ui.uv_workbench.zoom), camera);
        }
    }

    #[test]
    fn shift_right_click_opens_brush_mode_menu_in_3d_and_2d_uv_views() {
        let _gpu = photocraft_testkit::gpu_lock();
        use egui_kittest::kittest::Queryable as _;
        for model_view in [true, false] {
            let mut app = fixture_app();
            app.run("uv.texture.create", json!({"width":64,"height":64})).unwrap();
            app.ui.uv_workbench.view_3d = model_view;
            app.ui.uv_workbench.split_view = false;
            app.ui.tool = crate::state::Tool::Brush;
            let mut h = egui_kittest::Harness::builder().with_size(vec2(1200.0, 850.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                app.set_uv_wgpu(cc.wgpu_render_state.clone().unwrap()).unwrap();
                app
            });
            h.step();
            h.step();
            let point = if model_view { h.state().uv_cache.viewport.unwrap().center() } else { h.state().last_canvas_rect.center() };
            let before_revision = h.state().session.active().unwrap().revision;
            h.event(egui::Event::PointerMoved(point));
            h.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
            h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Secondary, pressed: true, modifiers: egui::Modifiers::SHIFT });
            h.step();
            h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Secondary, pressed: false, modifiers: egui::Modifiers::SHIFT });
            h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
            h.run_steps(2);
            assert!(h.state().ui.brush_mode_menu.is_some(), "model_view={model_view}");
            assert_eq!(h.state().ui.brush_mode_menu_viewport, Some(egui::ViewportId::ROOT));
            assert!(h.state().ui.brush_picker.is_none() && h.state().drag.is_none(), "gesture must not open the preset picker or paint");
            assert_eq!(h.state().session.active().unwrap().revision, before_revision, "opening the menu must not edit the UV document");
            assert!(h.ctx.memory(|memory| memory.area_rect(crate::brush_mode_menu::menu_id())).is_some(), "menu must render in its opening viewport");
            h.get_by_label("Multiply").click();
            h.run_steps(2);
            assert!(h.state().ui.brush_mode_menu.is_none());
            assert_eq!(h.state().ui.brush_mode_menu_viewport, None);
            assert_eq!(h.state().session.tools.brush.mode, photocraft_doc::BlendMode::Multiply);
        }
    }

    #[derive(Debug)]
    struct MapDrop(std::path::PathBuf);
    impl egui::DroppedFile for MapDrop {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            Err("native import uses the path".into())
        }
    }
    #[test]
    fn material_cards_picker_drop_source_thumbnails_and_scoped_tabless_views() {
        let _gpu = photocraft_testkit::gpu_lock();
        use crate::file_dialog::FileDialogAnswer;
        use egui_kittest::kittest::Queryable as _;
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/checker.png");
        let (dialog, asked) = crate::file_dialog::fake(vec![Some(FileDialogAnswer::Paths(vec![source.to_string_lossy().into_owned()]))]);
        let mut app = fixture_app();
        app.session.uv.normal_bindings.fill(None); // Explicitly exercise the absent-channel state despite sibling auto-discovery.
        app.services.file_dialog = Some(dialog);
        app.ui.workspace = "3D".into();
        crate::menus::apply_workspace(&mut app);
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 900.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            app.set_uv_wgpu(cc.wgpu_render_state.clone().unwrap()).unwrap();
            app
        });
        h.step();
        h.step();
        assert!(h.query_by_label("UV and seam protection").is_none());
        assert!(h.query_by_label("3D view / normals").is_none());
        assert!(h.query_by_label("Import information").is_none());
        h.get_by_label("Display channel").click();
        h.run_steps(2);
        assert!(h.query_by_label("Normal").is_none());
        h.get_by_label("Color").click();
        h.step();
        assert_eq!(h.state().ui.uv_workbench.channel, Channel::Color);
        let material = h.state().session.uv.material;
        let docs = h.state().session.documents().len();
        let rect = h.state().uv_cache.texture_targets.iter().find(|(_, _, n)| *n == Channel::Color).unwrap().0;
        h.event(egui::Event::PointerMoved(rect.center()));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: rect.center(), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.step();
        h.step();
        assert_eq!(asked.borrow().len(), 1, "card click must open a file picker");
        assert_eq!(h.state().session.documents().len(), docs + 1);
        assert!(h.state().uv_gpu_active());
        let rect = h.state().uv_cache.texture_targets.iter().find(|(_, _, n)| *n == Channel::Normal).unwrap().0;
        h.event(egui::Event::PointerMoved(rect.center()));
        h.step();
        h.input_mut().dropped_files.push(std::sync::Arc::new(MapDrop(source)));
        h.step();
        h.step();
        assert_eq!(h.state().session.documents().len(), docs + 2, "drop opens exactly one document");
        assert!(h.state().session.uv.normal_bindings[material].is_some());
        assert!(h.state().session.uv.normal_channel);
        assert!(h.state().drop_places.is_empty());
        assert!(h.state().uv_gpu_active());
        let id = h.state().session.uv.normal_bindings[material].unwrap();
        let thumb = h.state().uv_cache.thumbnails[&(id.0, true)].id();
        h.state_mut().run("layer.setProps", json!({"locks":{"all":false,"pixels":false}})).unwrap();
        h.state_mut().run("edit.fill", json!({"color":"#00ff00"})).unwrap();
        h.step();
        assert_eq!(thumb, h.state().uv_cache.thumbnails[&(id.0, true)].id(), "painting must not regenerate source thumbnails");
        h.get_by_label("Display channel").click();
        h.run_steps(2);
        h.get_by_label("Normal").click();
        h.step();
        assert_eq!(h.state().ui.uv_workbench.channel, Channel::Normal);
        assert_eq!(h.state().session.active().unwrap().doc.id, id, "2D must follow the selected map");
        h.get_by_label("UV shell selection").click();
        h.run_steps(2);
        assert!(h.query_by_label("General margin, px").is_some());
        assert!(h.query_by_label("U margin, px").is_some());
        assert!(h.query_by_label("V margin, px").is_some());
        for removed in ["Apply selection", "Protected layer", "Also protect open borders"] {
            assert!(h.query_by_label(removed).is_none());
        }
        if let Ok(path) = std::env::var("PHOTOCRAFT_UV_MARGINS_SCREENSHOT") {
            h.render().unwrap().save(path).unwrap();
        }
        // Close the tool flyout before checking the canvas layout.
        h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
        h.step();
        let point = h.state().uv_cache.viewport.unwrap().center();
        h.event(egui::Event::PointerMoved(point));
        h.step();
        h.state_mut().run("uv.shell.select", json!({"shells":[0]})).unwrap();
        h.state_mut().ui.uv_workbench.guard = 0.0;
        h.state_mut().ui.uv_workbench.horizontal = 4.0;
        h.state_mut().ui.uv_workbench.vertical = 2.0;
        h.state_mut().uv_cache.mask_settings_pending = true;
        h.step();
        assert_eq!((h.state().session.uv.horizontal, h.state().session.uv.vertical), (4.0, 2.0));
        assert!(h.state().session.active().unwrap().doc.selection.is_some(), "changed margins apply automatically");
        h.state_mut().run("tools.setColors", json!({"foreground":[1.0,0.0,0.0,1.0]})).unwrap();
        let projection = h.state().ui.uv_workbench.perspective;
        input_key(&mut h, egui::Key::F5, egui::Modifiers::SHIFT);
        assert_eq!(h.state().ui.uv_workbench.perspective, projection, "Shift+F5 must not switch the camera");
        assert!(h.state().ui.dialogs.iter().any(|d| crate::fill_ui::owns(&d.fields)), "shell mode opens native Fill");
        h.get_by_label("OK").click();
        h.step();
        let surface = h.state().session.active().unwrap().doc.layers[0].surface().unwrap();
        assert!(surface.rgba(30, 64)[0] > 0.9 && surface.rgba(30, 64)[1] < 0.1);
        assert!(surface.rgba(90, 64)[1] > 0.9, "fill respects the selected shell");
        input_key(&mut h, egui::Key::D, egui::Modifiers { ctrl: true, command: true, ..egui::Modifiers::NONE });
        assert!(h.state().session.uv.selected.is_empty());
        assert!(h.state().session.active().unwrap().doc.selection.is_none());
        h.state_mut().ui.uv_workbench.select_shells = false;
        input_key(&mut h, egui::Key::F5, egui::Modifiers::SHIFT);
        assert!(h.state().ui.dialogs.iter().any(|d| crate::fill_ui::owns(&d.fields)), "paint mode opens native Fill too");
        h.get_by_label("Cancel").click();
        h.step();
        h.state_mut().ui.uv_workbench.view_3d = false;
        h.state_mut().ui.uv_workbench.split_view = false;
        h.step();
        assert!(h.state().tab_strip.is_none(), "UV texture view has no document header");
        if let Ok(path) = std::env::var("PHOTOCRAFT_UV_CARDS_SCREENSHOT") {
            h.render().unwrap().save(path).unwrap();
        }
        h.state_mut().session.uv.scene = None;
        h.state_mut().ui.workspace = "Essentials".into();
        h.step();
        assert!(h.state().tab_strip.is_some(), "ordinary PhotoCraft documents keep their tabs");
    }
    #[test]
    fn fast_navigation_is_optional_and_does_not_limit_idle_or_source_maps() {
        let _gpu = photocraft_testkit::gpu_lock();
        let state: State = serde_json::from_value(json!({})).unwrap();
        assert!(state.fast_navigation);
        assert_eq!(navigation_limit(true, true, 8192), 1600);
        assert_eq!(navigation_limit(true, false, 8192), 8192);
        assert_eq!(navigation_limit(false, true, 8192), 8192);
        assert_eq!(navigation_limit(true, true, 1024), 1024);
    }
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn gpu_refresh_keeps_original_resolution_above_two_k_and_normal_values() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        let material = app.session.uv.material;
        app.run("uv.texture.create", json!({"channel":"normal","width":4096,"height":32,"depth":16})).unwrap();
        let rs = egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default());
        app.set_uv_wgpu(rs).unwrap();
        let mut cache = std::mem::take(&mut app.uv_cache);
        refresh(&mut app, &egui::Context::default(), &mut cache).unwrap();
        let image = cache.normals[material].as_ref().unwrap();
        assert_eq!([image.width, image.height], [4096, 32]);
        assert!((image.rgba[0][0] - 0.5).abs() < 0.001 && image.rgba[0][2] > 0.999);
        assert_eq!(app.session.active().unwrap().doc.size.width, 4096);
    }
    #[test]
    fn canonical_color_document_still_uses_gpu_map_request_without_fill_preview() {
        let mut app = fixture_app();
        app.run("uv.texture.create", json!({"channel":"baseColor","width":16,"height":16})).unwrap();
        let material = app.session.uv.material;
        let mut maps = crate::uv_lighting::SurfaceMaps::default();

        crate::uv_lighting::refresh_gpu(&mut app, &mut maps).unwrap();

        assert!(maps.gpu[0].get(material).is_some_and(Option::is_some), "canonical RGB maps retain the direct GPU request path");
    }
    #[test]
    fn pbr_linear_sources_persisted_settings_always_normals_and_panel() {
        let _gpu = photocraft_testkit::gpu_lock();
        use egui_kittest::kittest::Queryable as _;
        let mut app = fixture_app();
        for channel in [Channel::Color, Channel::Metallic, Channel::Roughness, Channel::Emissive] {
            app.run("uv.texture.create", json!({"channel":channel.key(),"width":16,"height":16,"depth":16})).unwrap();
            app.run("edit.fill", json!({"color":"#808080"})).unwrap();
        }
        let mut maps = crate::uv_lighting::SurfaceMaps::default();
        crate::uv_lighting::refresh(&mut app, &mut maps).unwrap();
        let mat = app.session.uv.material;
        assert!((maps.images[0][mat].as_ref().unwrap().rgba[0][0] - 0.21586).abs() < 0.003);
        assert!((maps.images[4][mat].as_ref().unwrap().rgba[0][0] - 0.21586).abs() < 0.003);
        assert!((maps.images[1][mat].as_ref().unwrap().rgba[0][0] - 0.50196).abs() < 0.003);
        let pointer = maps.images[0][mat].as_ref().unwrap().rgba.as_ptr();
        crate::uv_lighting::refresh(&mut app, &mut maps).unwrap();
        assert_eq!(maps.images[0][mat].as_ref().unwrap().rgba.as_ptr(), pointer);
        crate::uv_workspace::restore_preferences(&mut app, Some(&json!({"authored_normals":false,"lighting":{"pbr":false,"rotation":45.0}})));
        assert!(app.ui.uv_workbench.camera().authored_normals);
        assert!(!app.ui.uv_workbench.lighting.pbr);
        let saved = crate::uv_workspace::preferences(&app);
        assert_eq!(saved["lighting"]["rotation"], 45.0);
        assert!(saved.get("authored_normals").is_none());
        app.ui.uv_workbench.lighting.pbr = true;
        let mut h = egui_kittest::Harness::builder().with_size(vec2(450.0, 1000.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.step();
        h.step();
        assert!(h.state().uv_cache.texture_targets.iter().any(|(_, _, channel)| *channel == Channel::Emissive));
        assert!(h.query_by_label("Model normals").is_none());
        assert!(h.query_by_label("HDR / EXR…").is_some());
        if let Ok(path) = std::env::var("PHOTOCRAFT_UV_PBR_SCREENSHOT") {
            h.render().unwrap().save(path).unwrap();
        }
    }
    #[test]
    #[ignore = "explicit full-resolution 4K native brush benchmark"]
    fn benchmark_native_paint_4k_2d_split_and_3d() {
        let _gpu = photocraft_testkit::gpu_lock();
        for mode in 0..3 {
            let mut app = if let Ok(source) = std::env::var("PHOTOCRAFT_UV_PAINT_MODEL") {
                let mut session = photocraft_engine::Session::new();
                session.execute("uv.model.import", json!({"path":source})).unwrap();
                PhotocraftApp::new(session, Default::default())
            } else {
                fixture_app()
            };
            // The imported-model branch is a bare application state, unlike fixture_app(),
            // and the 3D viewport (including its WGPU render state) is created only in the
            // UV workspace.
            app.ui.workspace = "3D".into();
            app.run("uv.texture.create", json!({"width":4096,"height":4096,"depth":16})).unwrap();
            app.run("tools.setColors", json!({"foreground":"#ff0000"})).unwrap();
            app.run("tools.setBrush", json!({"brush":{"size":32.0,"hardness":0.5}})).unwrap();
            app.ui.uv_workbench.view_3d = mode != 0;
            app.ui.uv_workbench.split_view = mode == 1;
            app.ui.tool = crate::state::Tool::Brush;
            let shared = std::sync::Arc::new(std::sync::Mutex::new(None));
            let capture = shared.clone();
            let mut h = egui_kittest::Harness::builder().with_size(vec2(1600.0, 1000.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                let rs = cc.wgpu_render_state.clone().unwrap();
                *capture.lock().unwrap() = Some(rs.clone());
                app.set_wgpu(rs);
                app
            });
            h.step();
            h.step();
            let rs = shared.lock().unwrap().clone().unwrap();
            let point = if mode == 0 {
                pos2(600.0, 450.0)
            } else {
                let app = h.state();
                let (_, g) = app.uv_cache.geometry.as_ref().unwrap();
                let camera = app.ui.uv_workbench.camera();
                let size = app.uv_cache.gpu.as_ref().unwrap().frame_info().unwrap().0;
                let maps = preview::Materials { colors: &app.uv_cache.textures, ..Default::default() };
                let at = g
                    .triangles
                    .iter()
                    .enumerate()
                    .find_map(|(id, t)| {
                        let p = std::array::from_fn(|axis| t.positions.iter().map(|p| f64::from(p[axis])).sum::<f64>() / 3.0);
                        let at = g.project_normalized(camera, p, size)?;
                        (t.material == app.session.uv.material && g.pick(app.session.uv.scene.as_ref().unwrap(), camera, &maps, at, size) == Some(id))
                            .then_some(at)
                    })
                    .unwrap();
                let rect = app.uv_cache.viewport.unwrap();
                rect.min + vec2(at[0] as f32 * rect.width(), at[1] as f32 * rect.height())
            };
            h.event(egui::Event::PointerMoved(point));
            h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
            h.step();
            assert!(h.state().drag.is_some());
            let mut times = Vec::new();
            for frame in 0..24 {
                h.state_mut().uv_cache.refreshed = None;
                h.event(egui::Event::PointerMoved(point + vec2(frame as f32 * 0.5, frame as f32 * 0.15)));
                let start = Instant::now();
                h.step();
                rs.device.poll(eframe::wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
                if frame >= 4 {
                    times.push(start.elapsed().as_secs_f64() * 1000.0);
                }
            }
            times.sort_by(f64::total_cmp);
            println!("4K U16 paint mode={mode} CPU+completed GPU median={:.2}ms p95={:.2}ms", times[times.len() / 2], times[times.len() * 95 / 100]);
            h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
            h.step();
            assert!(h.state().drag.is_none());
        }
    }

    #[test]
    #[ignore = "explicit 24 MP mask-stack input and GPU-wait benchmark; run in release"]
    fn benchmark_native_mask_paint_stack_24mp_split_2d_and_3d() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (width, height) = std::env::var("PHOTOCRAFT_MASK_BENCH_SIZE")
            .ok()
            .and_then(|size| {
                let (w, h) = size.split_once('x')?;
                Some((w.parse().ok()?, h.parse().ok()?))
            })
            .unwrap_or((6_000, 4_000));
        const PAINT_LAYERS: usize = 8;
        const WARMUP: usize = 2;
        const RUNS: usize = 10;
        const MOVES: usize = 24;
        let percentile = |values: &[f64], p: f64| {
            let mut sorted = values.to_vec();
            sorted.sort_by(f64::total_cmp);
            sorted.get(((sorted.len().saturating_sub(1) as f64 * p).round() as usize).min(sorted.len().saturating_sub(1))).copied().unwrap_or(0.0)
        };

        let mut app = fixture_app();
        app.ui.workspace = "3D".into();
        app.run("uv.texture.create", json!({"width":4096,"height":4096,"depth":16})).unwrap();
        app.run("image.imageSize", json!({"width":width,"height":height,"resample":"bicubic"})).unwrap();
        let mask_layer = app.session.active().unwrap().active_layer.unwrap();
        app.run("edit.fill", json!({"contents":"color","color":"#9b6347"})).unwrap();
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        for index in 0..PAINT_LAYERS {
            app.run("uv.maskPaint.add", json!({"layer":mask_layer.0,"kind":"paint","name":format!("Paint {index}")})).unwrap();
        }
        let pixel_layer = app.run("layer.new.layer", json!({"name":"Pixel target"})).unwrap().get("layer").and_then(serde_json::Value::as_u64).unwrap();
        app.run("edit.fill", json!({"contents":"color","color":"#346c9a"})).unwrap();
        app.run("tools.setColors", json!({"foreground":"#ff0000"})).unwrap();
        app.run("tools.setBrush", json!({"brush":{"size":30.0,"hardness":0.8,"opacity":1.0,"flow":1.0}})).unwrap();
        app.ui.tool = crate::state::Tool::Brush;
        app.ui.uv_workbench.view_3d = true;
        app.ui.uv_workbench.split_view = true;
        app.ui.uv_workbench.select_shells = false;
        app.ui.mask_target = true;

        let shared = std::sync::Arc::new(std::sync::Mutex::new(None));
        let capture = shared.clone();
        let mut h = egui_kittest::Harness::builder()
            .with_size(vec2(1600.0, 1000.0))
            .with_pixels_per_point(1.0)
            .wgpu_setup(crate::gpu_canvas::wgpu_setup())
            .build_eframe(move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                let rs = cc.wgpu_render_state.clone().unwrap();
                *capture.lock().unwrap() = Some(rs.clone());
                app.set_wgpu(rs);
                app
            });
        let rs = shared.lock().unwrap().clone().unwrap();
        assert_ne!(rs.adapter.get_info().device_type, eframe::wgpu::DeviceType::Cpu, "hardware GPU required");
        h.input_mut().max_texture_side = Some(rs.device.limits().max_texture_dimension_2d as usize);
        for _ in 0..8 {
            h.step();
        }
        assert!(h.state().uv_gpu_active(), "UV renderer must remain on GPU: {}", h.state().ui.uv_workbench.status);
        let (request, _) = crate::control::ControlRequest::new("ui.inspect", serde_json::Value::Null);
        let ctx = h.ctx.clone();
        let crate::control::Outcome::Done(inspect) = crate::control::handle(h.state_mut(), &ctx, &request) else {
            panic!("ui.inspect must complete synchronously")
        };
        assert_eq!(inspect["ok"], true, "{inspect}");
        assert_eq!(inspect["result"]["uvViewport"]["rendererActive"], true, "{inspect}");
        assert_eq!(inspect["result"]["uvViewport"]["mapCompositorActive"], true, "{inspect}");
        println!(
            "adapter={:?} texture={width}x{height} mask_paint_layers={PAINT_LAYERS} moves_per_stroke={MOVES} runs={RUNS} warmup={WARMUP}",
            rs.adapter.get_info()
        );
        let active_preview_gpu = |app: &PhotocraftApp| {
            let active = crate::uv_mask_paint_preview::active_document(app).or_else(|| crate::uv_paint::active_document(&app.uv_cache.paint));
            active.is_some_and(|id| {
                app.ui.uv_workbench.lighting.pbr
                    && app.ui.uv_workbench.channel == Channel::Material
                    && app.uv_cache.gpu.is_some()
                    && app.uv_cache.masked.is_empty()
                    && app.session.uv.map_bindings(Channel::Color).contains(&Some(id))
                    && app.session.uv.map_bindings(Channel::Opacity).iter().all(Option::is_none)
                    && app
                        .session
                        .documents()
                        .iter()
                        .find(|st| st.doc.id == id)
                        .is_some_and(|st| crate::uv_lighting::gpu_color_encoding(&st.doc, Channel::Color).is_some())
            })
        };
        #[derive(Clone)]
        struct BenchRow {
            target: &'static str,
            view: &'static str,
            cadence: &'static str,
            input_p50: f64,
            input_p95: f64,
            wait_p50: f64,
            wait_p95: f64,
            uv_input_p50: f64,
            uv_input_p95: f64,
            refreshed_moves: usize,
            measured_moves: usize,
        }
        let mut paired_rows = Vec::<BenchRow>::new();

        for view_3d in [false, true] {
            let view_name = if view_3d { "3D" } else { "2D" };
            for (target, layer, mask_target) in [("mask-stack", mask_layer.0, true), ("pixels", pixel_layer, false)] {
                h.state_mut().run("layer.select", json!({"layer":layer,"mode":"replace"})).unwrap();
                h.state_mut().ui.mask_target = mask_target;
                h.state_mut().ui.tool = crate::state::Tool::Brush;
                h.state_mut().ui.uv_workbench.view_3d = view_3d;
                h.state_mut().ui.uv_workbench.split_view = true;
                let expected_target = if mask_target { "mask" } else { "pixels" };
                assert_eq!(crate::canvas::paint_target(h.state()), json!(expected_target), "{target}: native brush target");
                if mask_target {
                    let stack_layers = h
                        .state()
                        .session
                        .active()
                        .and_then(|st| st.active_layer.and_then(|id| st.doc.layer(id)))
                        .and_then(|layer| layer.mask.as_ref())
                        .and_then(|mask| mask.paint.as_ref())
                        .map(|stack| stack.layers.len());
                    assert_eq!(stack_layers, Some(PAINT_LAYERS), "mask benchmark must keep the requested Paint stack active");
                }
                for _ in 0..3 {
                    h.step();
                }
                assert!(h.state().uv_gpu_active(), "{target} {view_name}: UV renderer must remain on GPU");
                let (request, _) = crate::control::ControlRequest::new("ui.inspect", serde_json::Value::Null);
                let ctx = h.ctx.clone();
                let crate::control::Outcome::Done(inspect) = crate::control::handle(h.state_mut(), &ctx, &request) else {
                    panic!("{target} {view_name}: ui.inspect must complete synchronously")
                };
                assert_eq!(inspect["result"]["uvViewport"]["rendererActive"], true, "{inspect}");
                assert_eq!(inspect["result"]["uvViewport"]["mapCompositorActive"], true, "{inspect}");
                println!("target={target} view={view_name} uv_maps={}", inspect["result"]["uvViewport"]["maps"]);
                let point = if view_3d {
                    let app = h.state();
                    let (_, geometry) = app.uv_cache.geometry.as_ref().expect("prepared geometry");
                    let camera = app.ui.uv_workbench.camera();
                    let size = app.uv_cache.gpu.as_ref().and_then(|gpu| gpu.frame_info()).expect("GPU viewport frame").0;
                    let rect = app.uv_cache.viewport.expect("3D viewport rectangle");
                    let maps = preview::Materials { colors: &app.uv_cache.textures, normals: &app.uv_cache.normals, ..Default::default() };
                    geometry
                        .triangles
                        .iter()
                        .enumerate()
                        .find_map(|(id, triangle)| {
                            if triangle.material != app.session.uv.material {
                                return None;
                            }
                            let position = std::array::from_fn(|axis| triangle.positions.iter().map(|p| f64::from(p[axis])).sum::<f64>() / 3.0);
                            let normalized = geometry.project_normalized(camera, position, size)?;
                            (geometry.pick(app.session.uv.scene.as_ref()?, camera, &maps, normalized, size) == Some(id))
                                .then_some(rect.min + vec2(normalized[0] as f32 * rect.width(), normalized[1] as f32 * rect.height()))
                        })
                        .expect("visible picked face on the active material")
                } else {
                    h.state().last_canvas_rect.center()
                };
                assert_eq!(
                    h.state().uv_cache.viewport.is_some_and(|viewport| viewport.contains(point)),
                    view_3d,
                    "{target} {view_name}: pointer must be inside only the requested UV surface region"
                );
                for force_refresh in [false, true] {
                    let cadence = if force_refresh { "forced-each-move" } else { "default-scheduler" };
                    let (mut input_step, mut gpu_wait) = (Vec::new(), Vec::new());
                    let mut uv_input = Vec::new();
                    let (mut active_preview_gpu_moves, mut refresh_intervals) = (0usize, Vec::new());
                    let (mut uv_refresh, mut cpu_compose, mut prepare, mut draw_gpu) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
                    let (mut map_updates, mut map_chunks, mut map_uploads, mut map_upload_bytes) = (0usize, 0usize, 0usize, 0usize);
                    let (mut map_gpu_ms, mut map_gpu_px) = (Vec::new(), Vec::new());
                    for stroke in 0..(WARMUP + RUNS) {
                        let stroke_point = point + vec2((stroke % 4) as f32 * 0.3, (stroke % 3) as f32 * 0.2);
                        let mut last_refresh_at = None;
                        h.event(egui::Event::PointerMoved(stroke_point));
                        h.event(egui::Event::PointerButton {
                            pos: stroke_point,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::NONE,
                        });
                        h.step();
                        assert!(h.state().live_stroke.is_some(), "{target} {view_name} press must begin a native brush stroke");
                        if view_3d {
                            assert!(pending_paint(h.state()), "{target}: native 3D hit-test must own the active surface stroke");
                        } else {
                            assert!(!pending_paint(h.state()), "{target}: split 2D canvas input must bypass UV surface hit-testing");
                        }
                        for step in 1..=MOVES {
                            if force_refresh {
                                h.state_mut().uv_cache.refreshed = None;
                            }
                            let refresh_before = h.state().uv_cache.refreshed;
                            let offset =
                                if view_3d { vec2((step % 7) as f32 * 0.4, (step % 5) as f32 * 0.3) } else { vec2(step as f32 * 1.5, (step % 3) as f32 * 0.5) };
                            let start = Instant::now();
                            h.event(egui::Event::PointerMoved(stroke_point + offset));
                            h.step();
                            let ui_elapsed = start.elapsed().as_secs_f64() * 1000.0;
                            let app = h.state();
                            if stroke >= WARMUP && active_preview_gpu(app) {
                                active_preview_gpu_moves += 1;
                            }
                            if stroke >= WARMUP
                                && let Some(value) = app.perf.spans.get("uv.input")
                            {
                                uv_input.push(*value);
                            }
                            if stroke >= WARMUP && app.uv_cache.refreshed != refresh_before {
                                let now = Instant::now();
                                if stroke >= WARMUP
                                    && let Some(previous) = last_refresh_at
                                {
                                    refresh_intervals.push(now.duration_since(previous).as_secs_f64() * 1000.0);
                                }
                                last_refresh_at = Some(now);
                                for (name, values) in [
                                    ("uv.refresh", &mut uv_refresh),
                                    ("uv.compose.cpu", &mut cpu_compose),
                                    ("uv.prepare", &mut prepare),
                                    ("uv.draw.gpu", &mut draw_gpu),
                                ] {
                                    if let Some(value) = app.perf.spans.get(name) {
                                        values.push(*value);
                                    }
                                }
                            }
                            if stroke >= WARMUP
                                && let Some(stats) = app.uv_cache.gpu.as_ref().and_then(|gpu| gpu.last_map_counters)
                            {
                                map_updates += 1;
                                map_chunks += stats.chunks;
                                map_uploads += stats.tiles_uploaded;
                                map_upload_bytes += stats.bytes_uploaded;
                                if let Some(timing) = stats.gpu_time {
                                    map_gpu_ms.push(timing.ms);
                                    map_gpu_px.push(timing.px as f64);
                                }
                            }
                            let wait_start = Instant::now();
                            rs.device.poll(eframe::wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
                            let wait_elapsed = wait_start.elapsed().as_secs_f64() * 1000.0;
                            if stroke >= WARMUP {
                                input_step.push(ui_elapsed);
                                gpu_wait.push(wait_elapsed);
                            }
                        }
                        let refresh_marker = Instant::now();
                        if view_3d && mask_target {
                            h.state_mut().uv_cache.refreshed = Some(refresh_marker);
                        }
                        h.event(egui::Event::PointerButton {
                            pos: stroke_point + vec2(MOVES as f32 * 0.5, 0.0),
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: egui::Modifiers::NONE,
                        });
                        h.step();
                        h.step();
                        assert!(h.state().live_stroke.is_none(), "release leaves no live stroke active");
                        if view_3d && mask_target {
                            assert_ne!(h.state().uv_cache.refreshed, Some(refresh_marker), "ending the native mask stroke must flush the last map update");
                        }
                    }
                    assert!(!input_step.is_empty() && !gpu_wait.is_empty(), "{target} {view_name} has no measured input/GPU samples");
                    let measured_moves = input_step.len();
                    let refreshed_moves = uv_refresh.len();
                    println!(
                        "target={target} view={view_name} cadence={cadence} input_plus_step_ms_p50/p95={:.3}/{:.3} uv_input_ms_p50/p95={:.3}/{:.3} gpu_wait_ms_p50/p95={:.3}/{:.3} active_preview_gpu_moves={active_preview_gpu_moves}/{measured_moves} refreshed_moves={refreshed_moves}/{measured_moves} refresh_interval_ms_p50/p95={:.3}/{:.3} uv_refresh_ms_p50/p95={:.3}/{:.3} cpu_compose_ms_p50/p95={:.3}/{:.3} prepare_ms_p50/p95={:.3}/{:.3} draw_gpu_ms_p50/p95={:.3}/{:.3} map_updates={map_updates} map_chunks={map_chunks} map_tile_uploads={map_uploads} map_tile_upload_bytes={map_upload_bytes} map_gpu_ms_p50/p95={:.3}/{:.3} map_gpu_px_p50/p95={:.0}/{:.0}",
                        percentile(&input_step, 0.5),
                        percentile(&input_step, 0.95),
                        percentile(&uv_input, 0.5),
                        percentile(&uv_input, 0.95),
                        percentile(&gpu_wait, 0.5),
                        percentile(&gpu_wait, 0.95),
                        percentile(&refresh_intervals, 0.5),
                        percentile(&refresh_intervals, 0.95),
                        percentile(&uv_refresh, 0.5),
                        percentile(&uv_refresh, 0.95),
                        percentile(&cpu_compose, 0.5),
                        percentile(&cpu_compose, 0.95),
                        percentile(&prepare, 0.5),
                        percentile(&prepare, 0.95),
                        percentile(&draw_gpu, 0.5),
                        percentile(&draw_gpu, 0.95),
                        percentile(&map_gpu_ms, 0.5),
                        percentile(&map_gpu_ms, 0.95),
                        percentile(&map_gpu_px, 0.5),
                        percentile(&map_gpu_px, 0.95),
                    );
                    paired_rows.push(BenchRow {
                        target,
                        view: view_name,
                        cadence,
                        input_p50: percentile(&input_step, 0.5),
                        input_p95: percentile(&input_step, 0.95),
                        wait_p50: percentile(&gpu_wait, 0.5),
                        wait_p95: percentile(&gpu_wait, 0.95),
                        uv_input_p50: percentile(&uv_input, 0.5),
                        uv_input_p95: percentile(&uv_input, 0.95),
                        refreshed_moves,
                        measured_moves,
                    });
                }
            }
        }
        for view in ["2D", "3D"] {
            for cadence in ["default-scheduler", "forced-each-move"] {
                let mask = paired_rows.iter().find(|row| row.target == "mask-stack" && row.view == view && row.cadence == cadence);
                let pixels = paired_rows.iter().find(|row| row.target == "pixels" && row.view == view && row.cadence == cadence);
                if let (Some(mask), Some(pixels)) = (mask, pixels) {
                    let ratio = |a: f64, b: f64| if b > 0.0 { a / b } else { 0.0 };
                    println!(
                        "paired view={view} cadence={cadence} mask_over_pixels_input_p50/p95={:.3}/{:.3} uv_input_p50/p95={:.3}/{:.3} gpu_wait_p50/p95={:.3}/{:.3} refresh_fraction_mask/pixels={:.3}/{:.3}",
                        ratio(mask.input_p50, pixels.input_p50),
                        ratio(mask.input_p95, pixels.input_p95),
                        ratio(mask.uv_input_p50, pixels.uv_input_p50),
                        ratio(mask.uv_input_p95, pixels.uv_input_p95),
                        ratio(mask.wait_p50, pixels.wait_p50),
                        ratio(mask.wait_p95, pixels.wait_p95),
                        ratio(mask.refreshed_moves as f64, mask.measured_moves as f64),
                        ratio(pixels.refreshed_moves as f64, pixels.measured_moves as f64),
                    );
                }
            }
        }
    }

    #[test]
    fn independent_2d_channel_follows_global_changes_and_round_trips_preferences() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        app.run("uv.texture.create", json!({"channel":"normal","width":64,"height":64})).unwrap();
        let normal = app.session.active().unwrap().doc.id;
        select_view_channel(&mut app, Channel::Normal);
        assert_eq!(app.ui.uv_workbench.channel_2d, Channel::Normal);
        select_2d_channel(&mut app, Channel::Color);
        assert_eq!(app.ui.uv_workbench.channel, Channel::Normal);
        assert_ne!(app.session.active().unwrap().doc.id, normal);
        select_view_channel(&mut app, Channel::Material);
        assert_eq!(app.ui.uv_workbench.channel_2d, Channel::Color);
        select_view_channel(&mut app, Channel::Normal);
        assert_eq!(app.session.active().unwrap().doc.id, normal);
        let prefs = crate::uv_workspace::preferences(&app);
        app.ui.uv_workbench.channel_2d = Channel::Color;
        crate::uv_workspace::restore_preferences(&mut app, Some(&prefs));
        assert_eq!(app.ui.uv_workbench.channel_2d, Channel::Normal);
        let mut legacy = prefs;
        legacy.as_object_mut().unwrap().remove("channel_2d");
        app.ui.uv_workbench.channel_2d = Channel::Color;
        crate::uv_workspace::restore_preferences(&mut app, Some(&legacy));
        assert_eq!(app.ui.uv_workbench.channel_2d, Channel::Normal);
    }
    #[test]
    fn uv_mask_view_replaces_live_map_and_restores_material_without_touching_texture() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        let ctx = egui::Context::default();
        app.run("uv.shell.select", json!({"shells":[0]})).unwrap();
        app.run("uv.layer.create", json!({"guard":8})).unwrap();
        app.run("view.layerMask", json!({"mode":"gray"})).unwrap();
        let doc = app.session.active().unwrap().doc.clone();
        let material = app.session.uv.material;
        let mask = doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.clone();
        let mut cache = Cache { display_channel: Channel::Material, ..Default::default() };
        refresh(&mut app, &ctx, &mut cache).unwrap();
        assert!(cache.masked.contains(&material));
        let im = cache.textures[material].as_ref().unwrap();
        for (x, y) in [(30, 64), (52, 64)] {
            let p = im.rgba[y * im.width + x];
            let g = mask.pixel(x as i32, y as i32)[0];
            assert!((p[0] - g).abs() < 0.01 && p[0] == p[1] && p[1] == p[2] && p[3] == 1.0, "mask shown on mesh at {x},{y}");
        }
        assert!(std::sync::Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
        app.run("channel.target.layerMask", json!({})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        app.run("tools.setColors", json!({"foreground":"#000000"})).unwrap();
        app.run("tools.setBrush", json!({"brush":{"size":8.0,"hardness":1.0}})).unwrap();
        app.ui.tool = crate::state::Tool::Brush;
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Down { x: 30.0, y: 64.0, pressure: 1.0 }, egui::Modifiers::NONE);
        refresh(&mut app, &ctx, &mut cache).unwrap();
        assert!(cache.textures[material].as_ref().unwrap().rgba[64 * 128 + 30][0] < 0.01, "3D mask display includes a live native brush stroke");
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Up { x: 30.0, y: 64.0 }, egui::Modifiers::NONE);
        app.run("edit.undo", json!({})).unwrap();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        assert!(cache.textures[material].as_ref().unwrap().rgba[64 * 128 + 30][0] > 0.99, "Undo refreshes mask display");
        let old = cache.keys[material];
        app.run("view.layerMask", json!({"mode":"off"})).unwrap();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        assert!(cache.masked.is_empty());
        assert_ne!(cache.keys[material], old);
    }
    #[test]
    fn targeted_layer_mask_replaces_uv_preview_with_grayscale_mask() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        let ctx = egui::Context::default();
        app.run("uv.shell.select", json!({"shells":[0]})).unwrap();
        app.run("uv.layer.create", json!({"guard":8})).unwrap();
        app.run("channel.target.layerMask", json!({})).unwrap();
        app.run("view.layerMask", json!({"mode":"gray"})).unwrap();

        let st = app.session.active().unwrap();
        let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
        let mask = layer.mask.as_ref().unwrap().surface.clone();
        let material = app.session.uv.material;
        let mut cache = Cache { display_channel: Channel::Material, ..Default::default() };
        refresh(&mut app, &ctx, &mut cache).unwrap();

        assert!(showing_gray_layer_mask(&app));
        assert!(gray_mask_hides_all_materials(&app, &cache), "all model materials show the mask in Gray view");
        assert!(!should_refresh_surface_maps(&app, &cache), "masked Gray view does not consume material lighting maps");
        assert_eq!(app.perf.spans.get("uv.compose.cpu"), Some(&0.0), "full-resolution Gray mask preview bypasses document color composition");
        assert!(cache.masked.contains(&material), "the targeted mask is used by the UV preview");
        let image = cache.textures[material].as_ref().unwrap();
        for (x, y) in [(30, 64), (52, 64)] {
            let shown = image.rgba[y * image.width + x];
            let expected = mask.pixel(x as i32, y as i32)[0];
            assert!(
                (shown[0] - expected).abs() < 0.01 && shown[0] == shown[1] && shown[1] == shown[2] && shown[3] == 1.0,
                "targeted mask is shown as grayscale at {x},{y}"
            );
        }
        cache.masked.remove(&material);
        assert!(should_refresh_surface_maps(&app, &cache), "keep material maps current if any visible surface is not masked");
        cache.masked.insert(material);
        app.run("view.layerMask", json!({"mode":"off"})).unwrap();
        assert!(!showing_gray_layer_mask(&app));
        assert!(should_refresh_surface_maps(&app, &cache), "normal material refresh resumes when Gray mask view closes");
    }

    #[test]
    fn gray_gpu_source_tracks_live_mask_paint_before_commit() {
        let mut app = fixture_app();
        app.run("uv.shell.select", json!({"shells":[0]})).unwrap();
        app.run("uv.texture.create", json!({"channel":"baseColor","width":512,"height":512,"depth":8})).unwrap();
        app.run("uv.layer.create", json!({"guard":8})).unwrap();
        app.run("channel.target.layerMask", json!({})).unwrap();
        app.run("view.layerMask", json!({"mode":"gray"})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        app.run("uv.maskPaint.add", json!({"layer":layer.0,"kind":"paint"})).unwrap();
        app.run("tools.setColors", json!({"foreground":"#ff0000"})).unwrap();
        app.run("tools.setBrush", json!({"brush":{"size":32.0,"hardness":1.0}})).unwrap();
        app.ui.tool = crate::state::Tool::Brush;
        app.ui.mask_target = true;
        let scene = app.session.uv.scene.clone().unwrap();
        let mut cache = Cache { display_channel: Channel::Material, ..Default::default() };
        refresh(&mut app, &egui::Context::default(), &mut cache).unwrap();
        let before = gray_mask_gpu_source(&mut app, &mut cache, &scene).unwrap();

        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Down { x: 200.0, y: 200.0, pressure: 1.0 }, egui::Modifiers::NONE);
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Move { x: 210.0, y: 205.0, pressure: 1.0 }, egui::Modifiers::NONE);
        assert!(app.live_stroke.is_some(), "native brush preview must remain live before mouse release");
        let during = gray_mask_gpu_source(&mut app, &mut cache, &scene).expect("Gray view must expose live Paint data");
        assert_ne!(during.key, before.key, "live stroke key must invalidate the GPU stack before document revision changes");
        assert_ne!(
            during.document.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].surface,
            before.document.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].surface,
            "GPU source must use the in-progress stroke snapshot, not the committed document"
        );
    }

    #[test]
    fn gray_uv_preview_has_no_blur_seams_after_mask_effect_changes() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        let ctx = egui::Context::default();
        app.run("uv.shell.select", json!({"shells":[0]})).unwrap();
        app.run("uv.texture.create", json!({"channel":"baseColor","width":512,"height":512,"depth":8})).unwrap();
        app.run("uv.layer.create", json!({"guard":8})).unwrap();
        app.run("channel.target.layerMask", json!({})).unwrap();
        app.run("view.layerMask", json!({"mode":"gray"})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        app.run("uv.maskPaint.add", json!({"layer":layer.0,"kind":"paint"})).unwrap();
        app.session
            .edit("seed mask blur seam test", |doc, _| {
                let mask = doc.layer_mut(layer).ok_or(photocraft_engine::EngineError::NoLayer(layer))?.mask.as_mut().unwrap();
                let stack = mask.paint.as_mut().unwrap();
                stack.layers[0].surface.write_region(photocraft_geom::Rect::new(248, 248, 264, 264), &vec![1.0; 16 * 16 * 4]);
                Ok(())
            })
            .unwrap();

        let material = app.session.uv.material;
        let mut cache = Cache { display_channel: Channel::Material, ..Default::default() };
        let assert_preview_matches_mask = |app: &PhotocraftApp, cache: &Cache, stage: &str| {
            let mask = app.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap();
            let image = cache.textures[material].as_ref().unwrap();
            assert_eq!([image.width, image.height], [512, 512], "{stage}: UV Gray cache size");
            for y in 0..512 {
                for x in 0..512 {
                    let shown = image.rgba[y * 512 + x];
                    let expected = mask.surface.sample_channel(x as i32, y as i32, 0);
                    assert!(
                        (shown[0] - expected).abs() < 0.01 && shown[0] == shown[1] && shown[1] == shown[2] && shown[3] == 1.0,
                        "{stage}: gray mask/cache seam at ({x},{y}): shown={shown:?}, expected={expected}"
                    );
                }
            }
        };

        refresh(&mut app, &ctx, &mut cache).unwrap();
        let scene = app.session.uv.scene.clone().unwrap();
        let state = app.session.active().unwrap();
        let view = photocraft_engine::mask_view_cmds::current(state).unwrap();
        assert!(!view.filter);
        let viewed_mask = state.doc.layer(view.layer).unwrap().mask.as_ref().unwrap();
        assert!(viewed_mask.paint.is_some());
        assert_eq!(viewed_mask.feather, 0.0);
        assert!(cache.masked.contains(&material));
        assert_eq!(crate::uv_mask::source(&app, material), app.session.active_index());
        assert!(app.session.uv.topology.as_ref().unwrap().shell_id_map_for_materials(&scene, 512, 512, &[material], [0, 0]).is_ok());
        let gpu_source = gray_mask_gpu_source(&mut app, &mut cache, &scene).expect("native Gray view should expose its Paint stack to the GPU renderer");
        assert_eq!(gpu_source.layer, layer);
        assert_eq!(gpu_source.materials, [material].into_iter().collect(), "GPU mask map is scoped to materials bound to this document");
        assert_eq!(gpu_source.shell_ids.len(), 512 * 512);
        assert!(gpu_source.shell_ids.iter().any(|id| *id > 0), "UV ownership map covers the selected model shell");
        let shell_cache = gpu_source.shell_ids.clone();
        let cached_source = gray_mask_gpu_source(&mut app, &mut cache, &scene).unwrap();
        assert!(std::sync::Arc::ptr_eq(&shell_cache, &cached_source.shell_ids), "shell IDs are cached across preview frames");
        app.run("uv.maskPaint.add", json!({"layer":layer.0,"kind":"blur","radius":8})).unwrap();
        assert_eq!(app.session.active().unwrap().last_damage, Some(app.session.active().unwrap().doc.bounds()));
        refresh(&mut app, &ctx, &mut cache).unwrap();
        assert_preview_matches_mask(&app, &cache, "Blur added");

        for (radius, opacity, visible) in [(13, 1.0, true), (13, 0.35, true), (13, 0.35, false), (5, 0.8, true)] {
            app.run("uv.maskPaint.modify", json!({"layer":layer.0,"index":1,"radius":radius,"opacity":opacity,"visible":visible})).unwrap();
            refresh(&mut app, &ctx, &mut cache).unwrap();
            assert_preview_matches_mask(&app, &cache, &format!("Blur radius={radius}, opacity={opacity}, visible={visible}"));
        }
    }

    #[test]
    fn fill_proxy_key_is_shared_by_2d_and_uv_until_the_next_sample_and_commit_uses_raw_latest() {
        let mut app = fixture_app();
        app.run("uv.texture.create", json!({"channel":"baseColor","width":128,"height":128,"depth":8})).unwrap();
        let index = app.session.active_index().unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        {
            let state = app.session.active_mut().unwrap();
            let mut mask = photocraft_doc::LayerMask::hide_all();
            mask.paint = Some(photocraft_doc::mask_paint::Stack {
                base: photocraft_raster::Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[0.2]),
                layers: vec![photocraft_doc::mask_paint::Paint {
                    name: "Fill preview".into(),
                    visible: true,
                    opacity: 1.0,
                    blend: photocraft_color::BlendMode::Normal,
                    surface: photocraft_raster::Surface::with_default(photocraft_color::PixelFormat::RGBA8, &[0.0, 0.0, 0.0, 0.0]),
                    fill_map: None,
                    fill_map_size: [0, 0],
                    fill_bounds: None,
                    kind: photocraft_doc::mask_paint::EffectKind::Fill {
                        value: 1.0,
                        rotation: 0.0,
                        repeat_u: false,
                        repeat_v: false,
                        mirror_u: false,
                        mirror_v: false,
                    },
                }],
                active: 0,
            });
            photocraft_compose::mask_paint::refresh_in_canvas(&mut mask, state.doc.bounds());
            let doc = std::sync::Arc::make_mut(&mut state.doc);
            doc.layer_mut(layer).unwrap().mask = Some(mask);
            state.channel_view.target = photocraft_engine::channel_cmds::ChannelTarget::LayerMask;
            state.channel_view.layer_mask =
                Some(photocraft_engine::mask_view_cmds::LayerMaskView { layer, filter: false, mode: photocraft_engine::mask_view_cmds::MaskViewMode::Gray });
        }
        let ctx = egui::Context::default();
        let material = app.session.uv.material;
        let mut uv = Cache { display_channel: Channel::Material, ..Default::default() };
        refresh_with_lighting(&mut app, &ctx, &mut uv, photocraft_uv::lighting::Settings::default()).unwrap();
        assert!(uv.masked.contains(&material), "the fixture must route its bound document through the UV mask display");
        assert!(crate::channel_view::ensure(&mut app, &ctx, index).is_some());

        let first_bounds = [8, 8, 48, 48];
        crate::uv_mask_paint_preview::set_fill(&mut app, layer, 0, first_bounds, 0.0);
        let started = Instant::now();
        let _ = crate::uv_mask_paint_preview::fill_mask_proxy_at(&mut app, index, started);
        let deadline = started + Duration::from_secs(2);
        let proxy = loop {
            let (proxy, _) = crate::uv_mask_paint_preview::fill_mask_proxy_at(&mut app, index, Instant::now());
            if let Some(proxy) = proxy
                && proxy.key == crate::uv_mask_paint_preview::current_fill_key(&app, index).unwrap()
            {
                break proxy;
            }
            assert!(Instant::now() < deadline, "Fill proxy becomes ready off-thread");
            std::thread::sleep(Duration::from_millis(1));
        };
        crate::channel_view::ensure(&mut app, &ctx, index).unwrap();
        refresh_with_lighting(&mut app, &ctx, &mut uv, photocraft_uv::lighting::Settings::default()).unwrap();
        let doc_id = app.session.active().unwrap().doc.id.0;
        let two_d_key = app.channel_views.get(&doc_id).unwrap().preview_key;
        let three_d_key = uv.keys[material].2;
        assert_eq!(two_d_key, proxy.key);
        assert_eq!(three_d_key, proxy.key, "2D and UV maps use the same accepted sample key");

        let held_at = Instant::now();
        crate::uv_mask_paint_preview::hold_fill_sample_schedule(&mut app, index, held_at);
        let latest_bounds = [40, 40, 88, 88];
        crate::uv_mask_paint_preview::set_fill(&mut app, layer, 0, latest_bounds, 23.0);
        let (still_shown, _) = crate::uv_mask_paint_preview::fill_mask_proxy_at(&mut app, index, held_at + Duration::from_millis(20));
        assert_eq!(still_shown.unwrap().key, proxy.key, "the display sample is intentionally throttled");
        assert_eq!(crate::uv_mask_paint_preview::fill_transform(&app, layer, 0, first_bounds, 0.0), (latest_bounds, 23.0));
        assert_eq!(crate::uv_mask_paint_preview::pending(&app).unwrap().bounds, Some(latest_bounds));
        crate::channel_view::ensure(&mut app, &ctx, index).unwrap();
        refresh_with_lighting(&mut app, &ctx, &mut uv, photocraft_uv::lighting::Settings::default()).unwrap();
        assert_eq!(app.channel_views.get(&doc_id).unwrap().preview_key, two_d_key);
        assert_eq!(uv.keys[material].2, three_d_key, "neither view claims the unsampled parameters");

        let commit = crate::uv_mask_paint_preview::pending(&app).unwrap();
        app.run("uv.maskPaint.modify", commit.params()).unwrap();
        let fill = &app.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0];
        assert_eq!(fill.fill_bounds, Some(latest_bounds));
        assert!(matches!(fill.kind, photocraft_doc::mask_paint::EffectKind::Fill { rotation, .. } if rotation == 23.0));
    }

    #[test]
    fn check_source_does_not_commit_pending_fill_manipulation() {
        let mut app = fixture_app();
        app.run("uv.texture.create", json!({"channel":"baseColor","width":128,"height":128,"depth":8})).unwrap();
        app.run("layer.layerMask.hideAll", json!({})).unwrap();
        app.run("uv.maskPaint.add", json!({"kind":"fill","value":1.0})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let canonical = app.session.active().unwrap().doc.layer(layer).unwrap();
        let original_bounds = canonical.mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].fill_bounds;
        let original_mask = canonical.mask.as_ref().unwrap().surface.clone();
        let history = app.session.active().unwrap().history.past_len();
        crate::uv_mask_paint_preview::set_fill(&mut app, layer, 0, [20, 16, 70, 80], 17.0);
        let pending = crate::uv_mask_paint_preview::pending(&app).expect("Fill manipulation remains transient");

        app.uv_cache.source_generation = app.session.uv.generation;
        app.uv_cache.source_checked = None;
        check_source(&mut app);

        assert_eq!(
            crate::uv_mask_paint_preview::pending(&app).unwrap().params(),
            pending.params(),
            "background source check leaves the latest preview pending"
        );
        let canonical = app.session.active().unwrap().doc.layer(layer).unwrap();
        assert_eq!(canonical.mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].fill_bounds, original_bounds);
        assert_eq!(canonical.mask.as_ref().unwrap().surface, original_mask);
        assert_eq!(app.session.active().unwrap().history.past_len(), history, "background polling does not create history");
    }

    fn fixture_app() -> PhotocraftApp {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.fbx");
        let mut session = photocraft_engine::Session::new();
        session.execute("uv.model.import", json!({"path":source.to_string_lossy()})).unwrap();
        {
            let mut app = PhotocraftApp::new(session, crate::Services::default());
            app.ui.workspace = "3D".into();
            app
        }
    }
    #[test]
    fn pbr_live_brush_updates_only_damage_and_reuses_source_buffers() {
        let _gpu = photocraft_testkit::gpu_lock();
        for channel in [Channel::Color, Channel::Metallic, Channel::Emissive] {
            let mut app = fixture_app();
            app.run("uv.texture.create", json!({"channel":channel.key(),"width":1024,"height":1024,"depth":16})).unwrap();
            app.run("tools.setColors", json!({"foreground":"#ff0000"})).unwrap();
            app.run("tools.setBrush", json!({"brush":{"size":16.0,"hardness":1.0}})).unwrap();
            app.ui.tool = crate::state::Tool::Brush;
            let mut maps = crate::uv_lighting::SurfaceMaps::default();
            let material = app.session.uv.material;
            let kind = crate::uv_lighting::MAPS.iter().position(|kind| *kind == channel).unwrap();
            crate::uv_lighting::refresh(&mut app, &mut maps).unwrap();
            let pointer = maps.images[kind][material].as_ref().unwrap().rgba.as_ptr();
            crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Down { x: 200.0, y: 200.0, pressure: 1.0 }, egui::Modifiers::NONE);
            crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Move { x: 215.0, y: 205.0, pressure: 1.0 }, egui::Modifiers::NONE);
            crate::uv_lighting::refresh(&mut app, &mut maps).unwrap();
            let (_, regions) = maps.regions[kind][material].clone().expect("live stroke must propagate its damage");
            let region = regions.first().copied().unwrap_or([0; 4]);
            assert!(region[2] * region[3] < 65536, "small dabs cannot recompose a 1K map: {region:?}");
            assert_eq!(maps.images[kind][material].as_ref().unwrap().rgba.as_ptr(), pointer);
            assert!(maps.images[kind][material].as_ref().unwrap().rgba[200 * 1024 + 200][1] < 0.02);
            crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Up { x: 215.0, y: 205.0 }, egui::Modifiers::NONE);
            crate::uv_lighting::refresh(&mut app, &mut maps).unwrap();
            assert_eq!(maps.images[kind][material].as_ref().unwrap().rgba.as_ptr(), pointer, "commit preserves the live texture cache");
            app.run("edit.undo", json!({})).unwrap();
            crate::uv_lighting::refresh(&mut app, &mut maps).unwrap();
            assert!(maps.images[kind][material].as_ref().unwrap().rgba[200 * 1024 + 200][1] > 0.9);
        }
    }
    #[test]
    fn painting_in_3d_keeps_zoomed_split_2d_view_fixed_on_both_sides() {
        let _gpu = photocraft_testkit::gpu_lock();
        for swapped in [false, true] {
            let mut app = fixture_app();
            app.run("uv.texture.create", json!({"width":512,"height":512})).unwrap();
            app.run("tools.setColors", json!({"foreground":"#ff0000"})).unwrap();
            app.ui.uv_workbench.split_view = true;
            app.ui.uv_workbench.swap_views = swapped;
            app.ui.uv_workbench.antialias = false;
            app.ui.tool = crate::state::Tool::Brush;
            let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 900.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                app.set_uv_wgpu(cc.wgpu_render_state.clone().unwrap()).unwrap();
                app
            });
            h.step();
            h.step();
            for view in &mut h.state_mut().ui.views {
                view.zoom = 8.0;
                view.center = [220.0, 240.0];
                view.fit_pending = false;
            }
            h.step();
            let app = h.state();
            let (_, g) = app.uv_cache.geometry.as_ref().unwrap();
            let camera = app.ui.uv_workbench.camera();
            let size = app.uv_cache.gpu.as_ref().unwrap().frame_info().unwrap().0;
            let maps = preview::Materials { colors: &app.uv_cache.textures, ..Default::default() };
            let at = g
                .triangles
                .iter()
                .enumerate()
                .find_map(|(id, t)| {
                    let p = std::array::from_fn(|axis| t.positions.iter().map(|p| f64::from(p[axis])).sum::<f64>() / 3.0);
                    let at = g.project_normalized(camera, p, size)?;
                    (t.material == app.session.uv.material && g.pick(app.session.uv.scene.as_ref().unwrap(), camera, &maps, at, size) == Some(id)).then_some(at)
                })
                .expect("visible model face");
            let rect = app.uv_cache.viewport.unwrap();
            let point = rect.min + vec2(at[0] as f32 * rect.width(), at[1] as f32 * rect.height());
            let centers: Vec<_> = app.ui.views.iter().map(|view| view.center).collect();
            h.event(egui::Event::PointerMoved(point));
            h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
            h.step();
            assert!(h.state().drag.is_some());
            for _ in 0..8 {
                h.step();
            }
            assert_eq!(centers, h.state().ui.views.iter().map(|view| view.center).collect::<Vec<_>>());
            h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
            h.step();
            assert!(h.state().drag.is_none());
        }
    }
    #[test]
    fn main_3d_pointer_uses_native_brush_and_refreshes_the_bound_gpu_map() {
        let _gpu = photocraft_testkit::gpu_lock();
        use egui_kittest::kittest::Queryable as _;
        let source = std::env::var_os("PHOTOCRAFT_UV_PAINT_MODEL")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj"));
        let side = if std::env::var_os("PHOTOCRAFT_UV_PAINT_MODEL").is_some() { 4096 } else { 128 };
        let mut session = photocraft_engine::Session::new();
        session.execute("uv.model.import", json!({"path":source})).unwrap();
        session.execute("uv.texture.create", json!({"width":side,"height":side})).unwrap();
        session.execute("tools.setColors", json!({"foreground":[1.0,0.0,0.0,1.0]})).unwrap();
        session.execute("tools.setBrush", json!({"brush":{"size":8,"hardness":1.0}})).unwrap();
        let mut harness = egui_kittest::Harness::builder().with_size(vec2(1200.0, 850.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(session, Default::default());
            app.ui.workspace = "3D".into();
            app.set_uv_wgpu(cc.wgpu_render_state.clone().unwrap()).unwrap();
            app.ui.uv_workbench.view_3d = true;
            app.ui.uv_workbench.split_view = true;
            app.ui.uv_workbench.antialias = false;
            app.ui.uv_workbench.yaw = 0.0;
            app.ui.uv_workbench.pitch = 0.0;
            app.ui.tool = crate::state::Tool::Brush;
            app
        });
        harness.step();
        let app = harness.state();
        let rect = app.uv_cache.viewport.unwrap();
        let g = &app.uv_cache.geometry.as_ref().unwrap().1;
        let size = app.uv_cache.gpu.as_ref().unwrap().frame_info().unwrap().0;
        let camera = Camera { yaw: 0.0, pitch: 0.0, ..Default::default() };
        let c = g.camera_values(camera, size);
        let scene = app.session.uv.scene.as_ref().unwrap();
        let maps = preview::Materials { colors: &app.uv_cache.textures, normals: &app.uv_cache.normals, ..Default::default() };
        let triangle = g
            .triangles
            .iter()
            .enumerate()
            .find_map(|(id, t)| {
                if t.material != app.session.uv.material {
                    return None;
                }
                let p: [f64; 2] = std::array::from_fn(|i| t.positions.iter().map(|p| f64::from(p[i])).sum::<f64>() / 3.0);
                let at = [((p[0] + f64::from(c[6])) * f64::from(c[4]) + 1.0) * 0.5, (1.0 - (p[1] + f64::from(c[7])) * f64::from(c[5])) * 0.5];
                if at.iter().all(|v| (0.0..=1.0).contains(v)) && g.pick(scene, camera, &maps, at, size) == Some(id) { Some(t) } else { None }
            })
            .expect("visible triangle of the active material");
        let xy: [f64; 2] = std::array::from_fn(|i| triangle.positions.iter().map(|p| f64::from(p[i])).sum::<f64>() / 3.0);
        let point = rect.min
            + vec2(
                ((xy[0] + f64::from(c[6])) * f64::from(c[4]) + 1.0) as f32 * 0.5 * rect.width(),
                (1.0 - (xy[1] + f64::from(c[7])) * f64::from(c[5])) as f32 * 0.5 * rect.height(),
            );
        let uv: [f64; 2] = std::array::from_fn(|i| triangle.uv.iter().map(|p| f64::from(p[i])).sum::<f64>() / 3.0);
        let pixel = [(uv[0] * f64::from(side)) as i32, ((1.0 - uv[1]) * f64::from(side)) as i32];
        let material = app.session.uv.material;
        let before = app.session.active().unwrap().doc.clone();
        harness.event(egui::Event::PointerMoved(point));
        harness.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        harness.step();
        assert!(harness.state().live_stroke.is_some(), "3D press must reach native live brush");
        let index = harness.state().session.active_index().unwrap();
        let (shown, key) = crate::canvas::display_doc(harness.state_mut(), index);
        assert_ne!(key, 0);
        assert!(shown.layers[0].surface().unwrap().rgba(pixel[0], pixel[1])[1] < 0.1);
        harness.state_mut().uv_cache.refreshed = None;
        let refresh_start = Instant::now();
        harness.step();
        eprintln!("3D paint {side}x{side}: partial compose/upload + UI {:.3}ms", refresh_start.elapsed().as_secs_f64() * 1000.0);
        let image = harness.state().uv_cache.textures[material].as_ref().unwrap();
        assert!(image.rgba[pixel[1] as usize * side as usize + pixel[0] as usize][1] < 0.1);
        harness.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        harness.step();
        assert!(harness.state().drag.is_none() && harness.state().uv_paint_key.is_none());
        assert!(harness.state().uv_gpu_active());
        let doc = harness.state().session.active().unwrap().doc.id;
        let docs = harness.state().session.documents().len();
        let old_scene = harness.state().session.uv.scene.clone().unwrap();
        reveal(harness.state_mut()).unwrap();
        harness.step();
        harness.get_by_label("File").click();
        harness.step();
        harness.get_by_label("3D Import").click();
        harness.step();
        harness.get_by_label("Reimport").click();
        harness.step();
        harness.step();
        assert!(harness.state().uv_gpu_active());
        assert!(!std::sync::Arc::ptr_eq(&old_scene, harness.state().session.uv.scene.as_ref().unwrap()));
        assert_eq!(harness.state().session.documents().len(), docs);
        assert_eq!(harness.state().session.active().unwrap().doc.id, doc);
        if let Ok(path) = std::env::var("PHOTOCRAFT_UV_PAINT_SCREENSHOT") {
            harness.state_mut().uv_cache.refreshed = None;
            harness.step();
            harness.render().unwrap().save(path).unwrap();
        }
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(harness.state().session.active().unwrap().doc.layers[0].surface().unwrap(), before.layers[0].surface().unwrap());
    }

    #[test]
    fn new_protected_layer_targets_pixels_after_editing_an_existing_mask() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        app.session.execute("uv.shell.select", json!({"shells":[0]})).unwrap();
        let mut state = State::default();
        assert!(command(&mut app, &mut state, "uv.layer.create", json!({"guard":8})));
        app.run("channel.target.layerMask", json!({})).unwrap();
        assert!(app.ui.mask_target);
        assert!(command(&mut app, &mut state, "uv.layer.create", json!({"guard":8})));
        assert!(!app.ui.mask_target);
        assert!(!app.ui.vector_mask_target);
        assert!(!app.layer_mask_targeted());
        assert_eq!(app.session.active().unwrap().channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::Composite);
        app.run("edit.fill", json!({"color":"#ff0000"})).unwrap();
        let doc = &app.session.active().unwrap().doc;
        let image = photocraft_compose::render(doc, doc.bounds());
        assert!(image.px[64 * 128 + 30][1] < 0.01, "fill changes the new layer pixels");
        assert!(doc.layers.last().unwrap().mask.as_ref().unwrap().surface.pixel(52, 64)[0] < 0.01, "the seam stays blocked");
    }

    #[test]
    fn main_canvas_shell_click_does_not_start_brush_and_paint_mode_resumes_it() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        app.ui.uv_workbench.select_shells = true;
        app.session.execute("layer.setProps", json!({"locks":{"all":false,"pixels":false}})).unwrap();
        app.sync_views();
        let index = app.session.active_index().unwrap();
        let mut view = app.ui.views[index].clone();
        view.zoom = 2.5;
        view.center = [50.0, 70.0];
        view.rotation = 31.0;
        view.fit_pending = false;
        view.fill_pending = false;
        app.ui.view.flip_horizontal = true;
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(600.0, 600.0));
        let xf = crate::canvas::ViewXform { rect, zoom: 2.5, center: view.center, rotation: view.rotation, flip: true, aspect: 1.0 };
        let at = xf.to_screen(32.0, 64.0);
        let ctx = egui::Context::default();
        let mut frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                view = crate::canvas::canvas_view(app, ui, index, rect, view.clone(), true);
            });
            out.textures_delta.clear();
        };
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(&mut app, vec![egui::Event::PointerMoved(at)]);
        frame(&mut app, vec![button(true)]);
        assert!(app.drag.is_none());
        assert!(!app.press_stroke, "UV press must not paint");
        frame(&mut app, vec![button(false)]);
        assert_eq!(app.session.uv.selected.iter().copied().collect::<Vec<_>>(), vec![0]);
        assert!(app.session.active().unwrap().doc.selection.is_some());
        app.ui.uv_workbench.select_shells = false;
        frame(&mut app, vec![button(true)]);
        assert!(app.drag.is_some() || app.press_stroke, "Paint mode reaches the ordinary brush");
        frame(&mut app, vec![button(false)]);
        assert!(app.drag.is_none());
    }

    #[test]
    fn hidden_uvs_and_unrelated_documents_leave_tools_available() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        let index = app.session.active_index().unwrap();
        app.ui.uv_workbench.select_shells = true;
        assert!(selection_mode(&app, index));
        app.ui.uv_workbench.show_overlay = false;
        assert!(!selection_mode(&app, index));
        app.ui.uv_workbench.show_overlay = true;
        app.session.execute("file.new", json!({"width":128,"height":128})).unwrap();
        let other = app.session.active_index().unwrap();
        assert!(canvas_materials(&app, other).is_empty());
        assert!(!selection_mode(&app, other));
        assert!(selection_mode(&app, index), "closing the separate 3D window does not disable the overlay");
    }

    #[test]
    fn uv_picking_uses_canvas_transform_and_shift_alt_modifiers() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        let color = app.session.uv.bindings[app.session.uv.material];
        *app.session.uv.normal_bindings.last_mut().unwrap() = color;

        let index = app.session.active_index().unwrap();
        let ctx = egui::Context::default();
        for (point, modifiers, expected) in
            [([32.0, 64.0], egui::Modifiers::NONE, vec![0]), ([96.0, 64.0], egui::Modifiers::SHIFT, vec![0, 1]), ([32.0, 64.0], egui::Modifiers::CTRL, vec![1])]
        {
            let xf = crate::canvas::ViewXform {
                rect: Rect::from_min_size(pos2(35.0, 80.0), vec2(800.0, 600.0)),
                zoom: 3.7,
                center: [70.0, 50.0],
                rotation: 43.0,
                flip: true,
                aspect: 1.8,
            };
            let p = xf.to_screen(point[0], point[1]);
            let mut out = ctx.run_ui(egui::RawInput { events: vec![egui::Event::ModifiersChanged(modifiers)], ..Default::default() }, |ui| {
                pick_canvas(&mut app, ui, index, xf.to_doc(p))
            });
            out.textures_delta.clear();
            assert_eq!(app.session.uv.selected.iter().copied().collect::<Vec<_>>(), expected);
            assert!(!app.session.uv.normal_channel, "a different material using this image as a normal map must not change the clicked material channel");
        }
        assert_eq!(clip_tile([[-1.0, 0.5], [2.0, 0.5]]), Some([[0.0, 0.5], [1.0, 0.5]]));
        assert!(clip_tile([[-1.0, 0.2], [-1.0, 0.8]]).is_none());
    }

    #[test]
    fn editor_fill_and_undo_update_preview_without_saving_files() {
        let _gpu = photocraft_testkit::gpu_lock();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.fbx");
        let mut session = photocraft_engine::Session::new();
        session.execute("uv.model.import", json!({"path":source.to_string_lossy()})).unwrap();
        let material = session.uv.material;
        let mut app = {
            let mut app = PhotocraftApp::new(session, crate::Services::default());
            app.ui.workspace = "3D".into();
            app
        };
        let ctx = egui::Context::default();
        let mut cache = Cache::default();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        let original = cache.textures[material].as_ref().unwrap().rgba.clone();
        app.session.execute("layer.setProps", json!({"locks":{"all":false,"pixels":false}})).unwrap();
        app.session.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        let changed = &cache.textures[material].as_ref().unwrap().rgba;
        assert_ne!(&original, changed);
        assert!(changed.iter().all(|p| p[0] > 0.99 && p[1] < 0.01 && p[2] < 0.01));
        app.session.execute("edit.undo", json!({})).unwrap();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        assert_eq!(original, cache.textures[material].as_ref().unwrap().rgba);
    }

    #[test]
    fn native_picker_import_cancel_and_integrated_view_without_texture_document() {
        let _gpu = photocraft_testkit::gpu_lock();
        use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.fbx").to_string_lossy().into_owned();
        let (dialog, asked) = crate::file_dialog::fake(vec![None, Some(FileDialogAnswer::Paths(vec![source.clone()]))]);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_dialog: Some(dialog), ..Default::default() });
        let ctx = egui::Context::default();
        pick_file(&mut app, "model", 0, false).unwrap();
        app.poll_file_dialog(&ctx, None);
        assert!(app.session.uv.scene.is_none());
        pick_file(&mut app, "model", 0, false).unwrap();
        app.poll_file_dialog(&ctx, None);
        assert_eq!(app.session.uv.source, source);
        assert!(app.ui.uv_workbench.view_3d);
        assert!(matches!(&asked.borrow()[1],FileDialogRequest::Open{multiple:false,extensions:Some(e),..} if e==&vec!["fbx".to_owned(),"obj".to_owned()]));
        reveal(&mut app).unwrap();
        assert!(app.ui.panels.properties);
        assert_eq!(app.ui.dock_tabs.properties, 2);
        // An untextured import has no document tab, but its 3D viewport must still draw.
        app.session.uv.bindings.fill(None);
        app.session.uv.normal_bindings.fill(None);
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 650.0))), ..Default::default() }, |ui| {
            assert!(workspace(&mut app, ui));
        });
        assert!(out.viewport_output.len() <= 1, "no separate native workbench window");
        out.textures_delta.clear();
        assert!(app.uv_cache.frame.as_ref().is_some_and(|f| f.width > f.height));
        assert!(
            app.uv_cache.frame.as_ref().is_some_and(|f| f.triangles.iter().any(Option::is_some)),
            "the initial camera must show the front of the imported FBX"
        );
    }
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn native_picker_keeps_gpu_after_model_texture_and_project_import() {
        let _gpu = photocraft_testkit::gpu_lock();
        use crate::file_dialog::FileDialogAnswer;
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures");
        let project = std::env::temp_dir().join(format!(
            "photocraft-uv-picker-gpu-{}-{}.pcuv",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fixture_app().run("uv.project.save", json!({"path":project})).unwrap();
        let answer = |path: std::path::PathBuf| Some(FileDialogAnswer::Paths(vec![path.to_string_lossy().into_owned()]));
        let (dialog, _) = crate::file_dialog::fake(vec![
            None,
            answer(fixtures.join("absent-model.fbx")),
            answer(fixtures.join("two-shells.fbx")),
            answer(fixtures.join("checker.png")),
            answer(project.clone()),
            answer(fixtures.join("normals-reflected.fbx")),
        ]);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_dialog: Some(dialog), ..Default::default() });
        let rs = egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default());
        app.set_uv_wgpu(rs).unwrap();
        let ctx = egui::Context::default();
        for (index, kind) in ["model", "model", "model", "texture", "project", "model"].into_iter().enumerate() {
            pick_file(&mut app, kind, 0, false).unwrap();
            app.poll_file_dialog(&ctx, None);
            assert!(app.uv_gpu_active(), "picker {kind}, step {index}: GPU renderer was lost");
            if index >= 2 {
                let mut out =
                    ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 650.0))), ..Default::default() }, |ui| {
                        assert!(workspace(&mut app, ui));
                    });
                out.textures_delta.clear();
                assert!(app.uv_cache.gpu.as_ref().unwrap().frame_info().is_some());
                assert!(app.uv_cache.frame.is_none(), "native picker must not select the CPU rasterizer");
            }
        }
        std::fs::remove_file(project).unwrap();
    }
    #[test]
    fn normal_preview_samples_data_without_display_transform_and_tracks_edits() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        let material = app.session.uv.material;
        app.run("uv.texture.create", json!({"channel":"normal","width":32,"height":32,"depth":32})).unwrap();
        let ctx = egui::Context::default();
        let mut cache = Cache::default();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        let pixel = cache.normals[material].as_ref().unwrap().rgba[0];
        assert!((pixel[0] - 0.5).abs() < 1e-6 && pixel[2] == 1.0);
        app.run("edit.fill", json!({"color":"#ff0000"})).unwrap();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        assert!(cache.normals[material].as_ref().unwrap().rgba.iter().all(|p| p[0] > 0.99 && p[1] < 0.01));
        app.run("edit.undo", json!({})).unwrap();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        assert_eq!(pixel, cache.normals[material].as_ref().unwrap().rgba[0]);
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;
    #[test]
    fn surface_orbit_updates_once_per_gesture_without_a_view_jump() {
        let _gpu = photocraft_testkit::gpu_lock();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
        let mut session = photocraft_engine::Session::new();
        session.execute("uv.model.import", json!({"path":source})).unwrap();
        let mut app = PhotocraftApp::new(session, Default::default());
        app.ui.uv_workbench.yaw = 0.0;
        app.ui.uv_workbench.pitch = 0.0;
        app.ui.uv_workbench.pan = [-0.4, 0.1];
        let ctx = egui::Context::default();
        let pump = |app: &mut PhotocraftApp, events| {
            let mut out = ctx
                .run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(320.0, 240.0))), events, ..Default::default() }, |ui| {
                    main_view(app, ui)
                });
            out.textures_delta.clear();
        };
        pump(&mut app, vec![]);
        let at = app.uv_cache.viewport.unwrap().center();
        let g = app.uv_cache.geometry.as_ref().unwrap().1.clone();
        let before = app.ui.uv_workbench.camera();
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::ALT };
        pump(&mut app, vec![egui::Event::PointerMoved(at), egui::Event::ModifiersChanged(egui::Modifiers::ALT), button(true)]);
        let pivot = app.ui.uv_workbench.target.expect("camera center intersects the sloped shell");
        let after = app.ui.uv_workbench.camera();
        let a = g.camera_values(before, [320, 240]);
        let b = g.camera_values(after, [320, 240]);
        for i in [4, 5, 6, 7] {
            assert!((a[i] - b[i]).abs() < 1e-6);
        }
        pump(&mut app, vec![egui::Event::PointerMoved(at + vec2(35.0, 15.0))]);
        assert_eq!(app.ui.uv_workbench.target, Some(pivot));
        assert_ne!(app.ui.uv_workbench.yaw, before.yaw);
        pump(&mut app, vec![button(false)]);
        // Pan to another area; the next Alt+LMB must find that surface afresh.
        app.ui.uv_workbench.yaw = 0.0;
        app.ui.uv_workbench.pitch = 0.0;
        app.ui.uv_workbench.pan[0] += 0.3;
        pump(&mut app, vec![button(true)]);
        assert_ne!(app.ui.uv_workbench.target, Some(pivot));
        assert!(app.drag.is_none(), "navigation must not paint");
        let previous = app.ui.uv_workbench.clone();
        let maps = preview::Materials::default();
        let scene = app.session.uv.scene.as_ref().unwrap();
        app.ui.uv_workbench.orbit_pivot = OrbitPivot::Cursor;
        surface_pivot(&mut app.ui.uv_workbench, &g, scene, &maps, Some([0.0, 0.0]), [320, 240]);
        assert_eq!(app.ui.uv_workbench.target, previous.target, "background retains the old pivot");
        app.ui.uv_workbench.orbit_pivot = OrbitPivot::Fixed;
        surface_pivot(&mut app.ui.uv_workbench, &g, scene, &maps, Some([0.5, 0.5]), [320, 240]);
        assert_eq!(app.ui.uv_workbench.target, previous.target);
        let mut cursor_state = State { orbit_pivot: OrbitPivot::Cursor, yaw: 0.0, pitch: 0.0, ..Default::default() };
        let cursor = [0.4, 0.5];
        let hit = g.hit(scene, cursor_state.camera(), &maps, cursor, [320, 240]).unwrap();
        surface_pivot(&mut cursor_state, &g, scene, &maps, Some(cursor), [320, 240]);
        assert_eq!(cursor_state.target, Some(hit.position));
        let maps = preview::Materials { hidden: (0..scene.materials.len()).collect(), ..Default::default() };
        let target = cursor_state.target;
        surface_pivot(&mut cursor_state, &g, scene, &maps, Some([0.6, 0.5]), [320, 240]);
        assert_eq!(cursor_state.target, target, "hidden geometry cannot change the pivot");
        let legacy: State = serde_json::from_value(json!({"zoom":2.0})).unwrap();
        assert_eq!(legacy.orbit_pivot, OrbitPivot::ViewCenter);
        assert_eq!(serde_json::from_value::<State>(serde_json::to_value(&app.ui.uv_workbench).unwrap()).unwrap(), app.ui.uv_workbench);
    }
    #[test]
    fn resolution_tracks_physical_pixels_and_supersampling_with_bounded_memory() {
        let _gpu = photocraft_testkit::gpu_lock();
        assert_eq!(preview_size(vec2(1800.0, 1000.0), 1.0, false, 4096), [1800, 1000]);
        assert_eq!(preview_size(vec2(1800.0, 1000.0), 1.0, true, 4096), [3600, 2000]);
        assert_eq!(preview_size(vec2(900.0, 500.0), 2.0, false, 4096), [1800, 1000]);
        assert_eq!(preview_size(vec2(4096.0, 2048.0), 1.0, true, 2048), [2048, 1024]);
        let [w, h] = preview_size(vec2(7680.0, 4320.0), 2.0, true, 4096);
        assert_eq!(w, 4096);
        assert!(w * h <= preview::MAX_PREVIEW_PIXELS);
        assert!((w as f32 / h as f32 - 16.0 / 9.0).abs() < 0.002);
        assert_eq!(preview_size(vec2(f32::NAN, f32::INFINITY), f32::NAN, false, 4096), [1, 1]);
    }
    #[test]
    fn maya_mouse_gestures_latch_alt_until_release_and_plain_buttons_do_not_navigate() {
        let _gpu = photocraft_testkit::gpu_lock();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
        let scene = photocraft_uv::import::load("test.obj", &std::fs::read(source).unwrap(), 0, |_| Err(photocraft_uv::Error("unused MTL".into()))).unwrap();
        for button in [egui::PointerButton::Primary, egui::PointerButton::Middle, egui::PointerButton::Secondary] {
            let ctx = egui::Context::default();
            let mut state = State::default();
            let mut nav = None;
            let mut pump = |state: &mut State, mut events: Vec<egui::Event>, modifiers: egui::Modifiers| {
                events.insert(0, egui::Event::ModifiersChanged(modifiers));
                let mut used = false;
                let mut out =
                    ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 500.0))), events, ..Default::default() }, |ui| {
                        let (_, response) = ui.allocate_exact_size(vec2(600.0, 500.0), Sense::click_and_drag());
                        used = navigate(ui, &response, state, &mut nav, scene_extent(&scene)).gesture;
                    });
                out.textures_delta.clear();
                used
            };
            let at = pos2(200.0, 200.0);
            let end = at + vec2(50.0, 20.0);
            let event = |pressed, pos, modifiers| egui::Event::PointerButton { pos, button, pressed, modifiers };
            pump(&mut state, vec![egui::Event::PointerMoved(at)], egui::Modifiers::NONE);
            assert!(!pump(&mut state, vec![event(true, at, egui::Modifiers::NONE)], egui::Modifiers::NONE));
            let before = state.clone();
            assert!(!pump(&mut state, vec![egui::Event::PointerMoved(end)], egui::Modifiers::NONE));
            assert_eq!(state, before);
            pump(&mut state, vec![event(false, end, egui::Modifiers::NONE), egui::Event::PointerMoved(at)], egui::Modifiers::NONE);
            assert!(!pump(&mut state, vec![event(true, at, egui::Modifiers::CTRL)], egui::Modifiers::CTRL), "Ctrl must remain available to tools");
            pump(&mut state, vec![event(false, at, egui::Modifiers::NONE)], egui::Modifiers::NONE);
            assert!(pump(&mut state, vec![event(true, at, egui::Modifiers::ALT)], egui::Modifiers::ALT));
            assert!(pump(&mut state, vec![egui::Event::PointerMoved(end)], egui::Modifiers::ALT));
            match button {
                egui::PointerButton::Primary => {
                    assert!(state.yaw < before.yaw);
                    assert!(state.pitch > before.pitch);
                    assert_eq!(state.zoom, before.zoom);
                }
                egui::PointerButton::Middle => {
                    assert!(state.pan[0] > 0.0 && state.pan[1] < 0.0);
                    assert_eq!(state.yaw, before.yaw);
                }
                _ => {
                    assert!(state.zoom > before.zoom);
                    assert_eq!(state.yaw, before.yaw);
                }
            }
            // Alt released first: continue the same gesture and suppress the release click.
            assert!(pump(&mut state, vec![egui::Event::PointerMoved(end + vec2(5.0, 0.0))], egui::Modifiers::NONE));
            assert!(pump(&mut state, vec![event(false, end + vec2(5.0, 0.0), egui::Modifiers::NONE)], egui::Modifiers::NONE));
            assert!(!pump(&mut state, vec![], egui::Modifiers::NONE));
        }
    }
    #[test]
    fn frame_selected_changes_the_orbit_pivot_and_frame_all_restores_it() {
        let _gpu = photocraft_testkit::gpu_lock();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.fbx");
        let mut session = photocraft_engine::Session::new();
        session.execute("uv.model.import", json!({"path":source})).unwrap();
        session.execute("uv.shell.select", json!({"shells":[0]})).unwrap();
        let mut app = {
            let mut app = PhotocraftApp::new(session, crate::Services::default());
            app.ui.workspace = "3D".into();
            app
        };
        app.ui.uv_workbench.view_3d = true;
        app.uv_cache.viewport = Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 500.0)));
        let ctx = egui::Context::default();
        for key in [egui::Key::F, egui::Key::A] {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events: vec![
                        egui::Event::PointerMoved(pos2(200.0, 200.0)),
                        egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
                    ],
                    ..Default::default()
                },
                |_| {
                    crate::shortcuts::handle(&mut app, &ctx);
                },
            );
            out.textures_delta.clear();
            if key == egui::Key::F {
                assert!(app.ui.uv_workbench.target.is_some());
                assert!(app.ui.uv_workbench.zoom > 1.0);
            } else {
                assert!(app.ui.uv_workbench.target.is_none());
                assert_eq!(app.ui.uv_workbench.zoom, 1.0);
            }
        }
    }
}
