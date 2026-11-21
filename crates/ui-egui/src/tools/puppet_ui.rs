//! Edit › Puppet Warp on the canvas: the framework's `PinWarp` places, moves, turns and removes pins in the canvas
//! body, `PinWarpBar` is the options bar, and `PinWarpSession` holds the pins, the selection, the options and the
//! session's own undo steps (⌘Z / ⌘⇧Z step the session while it is open, not the document history). ↩ / Commit runs
//! `edit.puppetWarp` with the pins.
//!
//! The preview is the layer texture on the solver's deformed triangle mesh, drawn by `TransformPreview`. Every pin
//! change solves from the rest mesh with the engine's iteration count, so the preview, the release and the commit
//! show one shape whatever path the pointer took.

use std::sync::Arc;

use craft::behavior::kurbo::{Point, Rect as DocRect};
use craft::imaging::ui::transform_preview::{PreviewPixels, PreviewTextures};
use craft::imaging::ui::warp::pin::{
    Pin, PinDensity, PinEdit, PinMeshView, PinMode, PinSolver, PinWarp, PinWarpBar, PinWarpOptions, PinWarpSession, PinWarpSettings,
};
use craft::imaging::ui::{PreviewMap, TransformPreview};
use craft::ui::contract::{Bind, Show as _};
use craft::ui::runtime::memory::{self, SaveKey};
use photocraft_algo::puppet::{ITERATIONS, PuppetDensity, PuppetMesh, PuppetMode, PuppetPin, PuppetSolver, PuppetWarp, build_mesh};
use photocraft_doc::{DocId, Document, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// Where Mode, Density, Expansion and Show Mesh are remembered between runs.
const SETTINGS_KEY: SaveKey = SaveKey::new("photocraft.puppetWarp");

/// The text of a remesh that leaves no pixels to move.
const NO_PIXELS: &str = "Could not use Puppet Warp: no opaque pixels at this density and expansion";

fn point(v: [f64; 2]) -> Point {
    Point::new(v[0], v[1])
}

fn xy(p: Point) -> [f64; 2] {
    [p.x, p.y]
}

/// The engine's mode for the framework's. The framework enums are `#[non_exhaustive]`; a value this build does not
/// know is Normal, the default.
fn mode_of(mode: PinMode) -> PuppetMode {
    match mode {
        PinMode::Rigid => PuppetMode::Rigid,
        PinMode::Distort => PuppetMode::Distort,
        _ => PuppetMode::Normal,
    }
}

fn pin_mode_of(mode: PuppetMode) -> PinMode {
    match mode {
        PuppetMode::Rigid => PinMode::Rigid,
        PuppetMode::Normal => PinMode::Normal,
        PuppetMode::Distort => PinMode::Distort,
    }
}

fn density_of(density: PinDensity) -> PuppetDensity {
    match density {
        PinDensity::Fewer => PuppetDensity::Fewer,
        PinDensity::More => PuppetDensity::More,
        _ => PuppetDensity::Normal,
    }
}

fn pin_density_of(density: PuppetDensity) -> PinDensity {
    match density {
        PuppetDensity::Fewer => PinDensity::Fewer,
        PuppetDensity::Normal => PinDensity::Normal,
        PuppetDensity::More => PinDensity::More,
    }
}

/// The engine's pin for the framework's: `rest` is `src`, `at` is `dst`, `angle` is `rotate`.
fn to_algo(pin: &Pin) -> PuppetPin {
    PuppetPin { src: xy(pin.rest), dst: xy(pin.at), rotate: pin.angle, depth: pin.depth }
}

/// The engine's warp of `pins` under `options`: the command's params and the canonical deformation.
fn warp_of(pins: &[Pin], options: &PinWarpOptions) -> PuppetWarp {
    PuppetWarp { pins: pins.iter().map(to_algo).collect(), mode: mode_of(options.mode), density: density_of(options.density), expansion: options.expansion }
}

/// The app's [`PinSolver`]: the engine's `PuppetSolver` over the layer's mesh. The framework reads the mesh as
/// `Point`s, `u32` triangles and a `u32` draw order, converted once per call here.
pub struct PuppetPinSolver {
    surface: Surface,
    bounds: Rect,
    puppet_mesh: PuppetMesh,
    engine: Option<PuppetSolver>,
    rest: Vec<Point>,
    deformed: Vec<Point>,
    triangles: Vec<[u32; 3]>,
    order: Vec<u32>,
}

impl PuppetPinSolver {
    /// The solver of `surface`'s content within `bounds`, meshed under `options`, with no pins.
    fn new(surface: Surface, bounds: Rect, options: &PinWarpOptions) -> Self {
        let puppet_mesh = build_mesh(&surface, bounds, density_of(options.density), options.expansion);
        let mut solver = PuppetPinSolver {
            surface,
            bounds,
            puppet_mesh: PuppetMesh::default(),
            engine: None,
            rest: Vec::new(),
            deformed: Vec::new(),
            triangles: Vec::new(),
            order: Vec::new(),
        };
        solver.set_mesh(puppet_mesh);
        solver.rebind(&[], options);
        solver
    }

    /// Whether the mesh has no triangles: the layer has no opaque pixels at these options.
    fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    /// Takes `mesh` and its framework-typed copies; the deformation is the rest mesh until the next solve.
    fn set_mesh(&mut self, mesh: PuppetMesh) {
        self.rest = mesh.verts.iter().copied().map(point).collect();
        self.deformed = self.rest.clone();
        self.triangles = mesh.tris.iter().map(|t| t.map(|i| i as u32)).collect();
        self.order = (0..self.triangles.len() as u32).collect();
        self.puppet_mesh = mesh;
    }
}

impl PinSolver for PuppetPinSolver {
    fn rebind(&mut self, pins: &[Pin], options: &PinWarpOptions) {
        let src: Vec<[f64; 2]> = pins.iter().map(|p| xy(p.rest)).collect();
        let fixed: Vec<bool> = pins.iter().map(|p| p.angle.is_some()).collect();
        self.engine = Some(PuppetSolver::new(self.puppet_mesh.clone(), &src, &fixed));
        self.solve(pins, options);
    }

    fn solve(&mut self, pins: &[Pin], options: &PinWarpOptions) {
        let Some(engine) = &self.engine else { return };
        let dst: Vec<[f64; 2]> = pins.iter().map(|p| xy(p.at)).collect();
        let rotations: Vec<Option<f64>> = pins.iter().map(|p| p.angle).collect();
        let (deformed, _) = engine.solve(&dst, &rotations, mode_of(options.mode), ITERATIONS, None);
        let src: Vec<[f64; 2]> = pins.iter().map(|p| xy(p.rest)).collect();
        let depths: Vec<i32> = pins.iter().map(|p| p.depth).collect();
        self.order = engine.draw_order(&depths, &src).into_iter().map(|t| t as u32).collect();
        self.deformed = deformed.into_iter().map(point).collect();
    }

    fn remesh(&mut self, options: &PinWarpOptions) -> Result<(), String> {
        let mesh = build_mesh(&self.surface, self.bounds, density_of(options.density), options.expansion);
        if mesh.tris.is_empty() {
            return Err(NO_PIXELS.into());
        }
        self.set_mesh(mesh);
        Ok(())
    }

    fn mesh(&self) -> PinMeshView<'_> {
        PinMeshView::new(&self.rest, &self.deformed, &self.triangles, &self.order)
    }
}

/// An open Puppet Warp.
pub struct PuppetSession {
    /// Names the session's textures.
    pub key: u64,
    /// Document this session belongs to (#5115).
    pub document: DocId,
    /// The layer being warped.
    pub layer: LayerId,
    /// Command to commit with (`edit.puppetWarp` or `layer.smartObjects.puppetWarp`).
    command: String,
    /// The document without the layer: what the canvas shows under the preview.
    pub preview_doc: Arc<Document>,
    /// The layer's opacity times its fill.
    opacity: f32,
    /// The layer's content bounds: the preview's source frame.
    bounds: Rect,
    /// The pins, selection, options and undo steps.
    pub warp: PinWarpSession,
    solver: PuppetPinSolver,
    textures: PreviewTextures,
    /// The text of the last refused edit, shown by the bar until the next edit.
    notice: Option<String>,
}

impl PuppetSession {
    /// The session's state for `ui.inspect` and the control channel.
    pub fn describe(&self) -> Value {
        let options = self.warp.options();
        let mesh = self.solver.mesh();
        json!({
            "document": self.document.0,
            "layer": self.layer.0,
            "pins": self.warp.pins().iter().map(to_algo).collect::<Vec<_>>(),
            "mode": mode_of(options.mode),
            "density": density_of(options.density),
            "expansion": options.expansion,
            "vertices": mesh.rest.len(),
            "triangles": mesh.triangles.len(),
            "selected": self.warp.selected().first(),
        })
    }
}

/// Runs `f` on the open session and saves the settings when it changed them, so every path that changes one (the
/// canvas, the bar, the control channel, undo) saves it once. `None` without a session.
fn update<R>(app: &mut PhotocraftApp, f: impl FnOnce(&mut PuppetSession) -> R) -> Option<R> {
    let s = app.distort.puppet.as_mut()?;
    let before = s.warp.settings();
    let out = f(s);
    let after = s.warp.settings();
    if after != before {
        memory::set_saved_value(app.ctx(), &SETTINGS_KEY, after);
    }
    Some(out)
}

/// Applies one edit to the session. An `Err` (a refused remesh) is also kept as the bar's notice; any other edit
/// clears the notice.
fn apply_edit(app: &mut PhotocraftApp, edit: PinEdit) -> Result<(), String> {
    update(app, |s| {
        let result = s.warp.apply(edit, &mut s.solver);
        s.notice = result.clone().err();
        result
    })
    .unwrap_or_else(|| Err(tl!("Puppet Warp is not active").into()))
}

/// Applies a pin edit from a test, as the canvas or the bar would.
#[cfg(test)]
pub(crate) fn session_apply(app: &mut PhotocraftApp, edit: PinEdit) -> Result<(), String> {
    apply_edit(app, edit)
}

/// ⌘Z while the session is open: steps the session back, not the document history. The caller ends a running pin drag
/// first. False when there is nothing to undo or no session.
pub fn undo(app: &mut PhotocraftApp) -> bool {
    update(app, |s| s.warp.undo(&mut s.solver)).unwrap_or(false)
}

/// ⌘⇧Z while the session is open: steps the session forward. False when there is nothing to redo or no session.
pub fn redo(app: &mut PhotocraftApp) -> bool {
    update(app, |s| s.warp.redo(&mut s.solver)).unwrap_or(false)
}

/// Starts Puppet Warp on the active layer.
pub fn begin(app: &mut PhotocraftApp, ctx: &egui::Context, command: &str) -> Result<(), String> {
    if let Some(why) = app.session.disabled_reason(command) {
        return Err(why);
    }
    if app.ui.transform.is_some() {
        return Err("finish Free Transform first".into());
    }
    let (layer, surface, _) = crate::distort_ui::active_pixels(app)?;
    let st = app.session.active().ok_or("no document")?;
    let l = st.doc.layer(layer).ok_or("no layer")?;
    let bounds = surface.content_bounds();
    if bounds.is_empty() {
        return Err("Could not use Puppet Warp: the layer is empty".into());
    }
    let opacity = l.opacity * l.fill_opacity;
    let preview_doc = crate::distort_ui::without_layer(&st.doc, layer);
    let image = crate::distort_ui::surface_image(&surface, bounds, 2048);
    let document = st.doc.id;
    let key = app.ui.alloc_id();
    let settings: PinWarpSettings = memory::saved_value(ctx, &SETTINGS_KEY);
    let textures = PreviewTextures::new(format!("puppet-{key}"), PreviewPixels::new(image, [1.0, 1.0]));
    let solver = PuppetPinSolver::new(surface, bounds, &settings.options);
    if solver.is_empty() {
        return Err("Could not use Puppet Warp: no opaque pixels".into());
    }
    app.distort.puppet = Some(PuppetSession {
        key,
        document,
        layer,
        command: command.to_string(),
        preview_doc,
        opacity,
        bounds,
        warp: PinWarpSession::new(&settings),
        solver,
        textures,
        notice: None,
    });
    Ok(())
}

/// Commits the session: one `edit.puppetWarp` step. Pins that all sit where they attached run no command.
pub fn commit(app: &mut PhotocraftApp) {
    let Some(s) = app.distort.puppet.take() else { return };
    let Some(pins) = s.warp.result() else { return };
    let mut params = serde_json::to_value(warp_of(pins, s.warp.options())).unwrap_or_default();
    params["layer"] = json!(s.layer.0);
    let _ = app.run(&s.command, params);
}

/// Control channel: `edit.puppetWarp {"ui": {...}}` while the mode is active.
pub fn control(app: &mut PhotocraftApp, ui: &Value) -> Result<Value, String> {
    let flag = |key: &str| ui.get(key).and_then(Value::as_bool) == Some(true);
    if flag("commit") {
        commit(app);
        return Ok(json!({"committed": true}));
    }
    if flag("cancel") {
        app.distort.puppet = None;
        return Ok(json!({"cancelled": true}));
    }
    let s = app.distort.puppet.as_ref().ok_or(tl!("Puppet Warp is not active"))?;
    if flag("undo") {
        undo(app);
    } else if flag("redo") {
        redo(app);
    } else if flag("deleteSelectedPin") {
        let before = s.warp.pins().to_vec();
        let selected = s.warp.selected().to_vec();
        let after: Vec<Pin> = before.iter().enumerate().filter(|(i, _)| !selected.contains(i)).map(|(_, p)| *p).collect();
        if after.len() != before.len() {
            apply_edit(app, PinEdit::replace(&before, after, "Delete Pin"))?;
        }
    } else {
        let mut options = *s.warp.options();
        if let Some(m) = ui.get("mode").and_then(Value::as_str) {
            options.mode = pin_mode_of(PuppetMode::parse(m).ok_or("mode: rigid|normal|distort")?);
        }
        if let Some(d) = ui.get("density").and_then(Value::as_str) {
            options.density = pin_density_of(PuppetDensity::parse(d).ok_or("density: fewer|normal|more")?);
        }
        if let Some(e) = ui.get("expansion").and_then(Value::as_f64) {
            options.expansion = e.clamp(-200.0, 200.0);
        }
        let changed = options != *s.warp.options();
        if changed {
            apply_edit(app, PinEdit::Options(options))?;
        }
        if let Some(show) = ui.get("showMesh").and_then(Value::as_bool) {
            apply_edit(app, PinEdit::ShowMesh(show))?;
        }
    }
    app.distort.puppet.as_ref().map(PuppetSession::describe).ok_or_else(|| tl!("Puppet Warp is not active").into())
}

/// The warped pixels, the mesh lines and the pins, in the canvas body at `at`. Each pass applies only the edits its own
/// pointer and key input produced, so a second pass in one frame applies nothing the first did; Enter commits and
/// Escape with no drag cancels.
pub fn canvas(app: &mut PhotocraftApp, ui: &mut egui::Ui, at: egui::Rect) {
    let Some(size) = app.session.active().map(|st| [st.doc.size.width, st.doc.size.height]) else { return };
    let Some(s) = app.distort.puppet.as_ref() else { return };
    let b = s.bounds;
    let source = DocRect::new(f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1));
    TransformPreview::new("Moving layer", &s.textures).map(PreviewMap::pins(s.solver.mesh(), source)).opacity(s.opacity).clip(size).show_at(ui, at);
    let output = PinWarp::new("Pins", Bind::given(s.warp.pins().to_vec()), s.solver.mesh())
        .selected_pins(s.warp.selected())
        .show_mesh(s.warp.show_mesh())
        .show_at(ui, at);
    for edit in output.edits() {
        let _ = apply_edit(app, edit);
    }
    if output.apply {
        commit(app);
    } else if output.discard {
        app.distort.puppet = None;
    }
}

/// The options bar: Mode, Density, Expansion, Show Mesh, the selected pin's depth and rotation, Remove All Pins,
/// commit and cancel.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(s) = app.distort.puppet.as_ref() else { return };
    let output = PinWarpBar::new(&s.warp).notice(s.notice.as_deref()).show(ui);
    for edit in output.edits {
        let _ = apply_edit(app, edit);
    }
    if output.apply {
        commit(app);
    } else if output.discard {
        app.distort.puppet = None;
    }
}

#[cfg(test)]
mod tests;
