//! Free Transform (⌘T): bounding box with handles over the canvas, live preview, commit via the
//! engine's `edit.transform` (one quad for scale/rotate/skew/distort/perspective).
//!
//! The box, its gestures, cursors, snapping and the session's own undo steps are the framework's
//! `TransformSession` and `TransformOverlay`; this module opens the session, previews the moving
//! pixels and commits through the engine. ↩ commits, Esc cancels. Undo and Redo step through the
//! session's own changes, not the document's history.

use std::sync::Arc;

use craft::behavior::geometry::{SnapLine, TransformMode};
use craft::draw::ui::transform::{Interpolation, TransformPreset, TransformSettings, TransformTool};
use craft::imaging::ui::transform_preview::{PreviewPixels, PreviewTextures, StyledPreview};
use craft::imaging::ui::warp::WarpSplit;
use egui::Color32;
use photocraft_algo::transform::Homography;
use photocraft_doc::{DocId, Document, LayerContent, LayerId};
use photocraft_engine::jobs::{JobEvent, JobId, JobOutcome};
use photocraft_geom::warp::{BezierMesh, Warp, WarpStyle};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{FreeTransform, MadeLayer, SessionWarp, Tool};

mod begin;
mod commit;
mod draw;
mod options_bar;
mod preview;
mod warp;

pub(crate) use begin::begin_content_aware;
pub(crate) use begin::moving_layers;
#[cfg(test)]
pub(crate) use begin::moving_texel;
pub use begin::{
    WARP_ONE_LAYER, begin, begin_copy, begin_from_frame, begin_placed, begin_selection, begin_warp, enter_warp, leave_warp, warps_active_layer, warps_type,
};
pub(crate) use commit::{SAVE_FOR_WEB, committing};
pub use commit::{apply_click, auto_commit, cancel, commit, end_if_left, follow_edits, intercept, is_enabled, on_job_event};
#[cfg(test)]
pub(crate) use draw::styled_composite;
pub use draw::{SessionShown, show_in_canvas};
pub(crate) use draw::{moves_selection, selection_map};
pub use options_bar::{options_bar, reset_warp};
pub(crate) use preview::read_surface;
pub(crate) use preview::solo_layer;
use preview::{Above, composite_image, show_only};
#[cfg(test)]
use preview::{covering, preview_image};
pub use warp::{edit_session_warp, split};

/// Opens a prepared box once its job has landed (see [`Preparing`]).
type FinishPrepared = Box<dyn FnOnce(&mut PhotocraftApp, begin::Prepared) -> Result<(), String>>;

/// A large layer's box being prepared as a job (`begin::on_prepared` opens it when it lands).
pub(crate) struct Preparing {
    job: JobId,
    out: Arc<std::sync::Mutex<Option<begin::Prepared>>>,
    finish: FinishPrepared,
    /// The document and layer the box is for.
    layer: (DocId, LayerId),
    /// The layer was made for this box (⌥⌘T's copy, a placed file): the opened session gets the
    /// marker, and a box that never opens takes the layer back, as Esc would.
    made: Option<MadeLayer>,
}

/// The canvas key of an open box's preview document (`display_doc`).
pub(crate) fn preview_key(session: u64) -> u64 {
    (1 << 40) + session
}

/// Where the document under an open box differs from the document itself: the moving layer
/// (hidden, or its selected pixels lifted out) and the layers drawn over the box instead
/// ([`preview::Above`]). Kept from when the box opens until the next one, so opening, canceling
/// and committing redraw that area instead of the whole canvas.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OpenArea {
    pub doc: DocId,
    pub key: u64,
    /// The document revision the preview was built from.
    pub revision: u64,
    pub area: photocraft_geom::Rect,
}

/// The canvas damage between the document and an open box's preview of it, either way, at the
/// revision the box opened on; on a commit (one revision later) also what the commit changed.
/// `None` (redraw everything) for anything else, e.g. a box that followed an edit while open.
pub(crate) fn damage(
    app: &PhotocraftApp,
    doc: DocId,
    seen: (u64, u64),
    now: (u64, u64),
    last_damage: Option<photocraft_geom::Rect>,
) -> Option<photocraft_geom::Rect> {
    let o = app.transform_area.filter(|o| o.doc == doc)?;
    let opening = seen.1 == 0 && now.1 == o.key && seen.0 == o.revision && now.0 == o.revision;
    let canceled = seen.1 == o.key && now.1 == 0 && seen.0 == o.revision && now.0 == o.revision;
    if opening || canceled {
        return Some(o.area);
    }
    // Committed: the box's preview, replaced by the document with the transform applied.
    if seen.1 == o.key && now.1 == 0 && seen.0 == o.revision && now.0 == o.revision + 1 {
        let done = last_damage?;
        return Some(if done.is_empty() { o.area } else { o.area.union(&done) });
    }
    None
}

/// Preview state that isn't serialisable: the document without the transformed pixels, and
/// full-resolution textures of those pixels.
pub struct OpenPreview {
    pub session: u64,
    pub doc: Arc<Document>,
    pub textures: PreviewTextures,
    pub opacity: f32,
    /// The layers stacked over the moving one, drawn over its pixels ([`Above`]).
    above: Option<Above>,
    /// A layer with a style: its pixels composited with the style as the box has them now,
    /// drawn instead of the bare pixels (#3653).
    styled: Option<StyledPreview>,
    /// The armed split tool of the Warp phase, one-shot (#12498). Hover, placing and the quick split are the
    /// warp grid's.
    split: Option<WarpSplit>,
    /// The tool the session began with: picking another one applies the transform.
    tool: Tool,
    /// Free Transform Path: the path as it was, drawn through the box instead of pixels.
    path: Option<photocraft_doc::vector::Path>,
    /// A layer's session keeps `doc` and `opacity` up to date with edits made while the box is
    /// open ([`follow_edits`]); `None` for a lone mask or channel and Transform Selection.
    follow: Option<Follow>,
    /// The commit running as a background job (#1015): the moving pixels stay on screen, without
    /// the box, until it lands; Esc cancels it. The session ends with the job.
    committing: Option<JobId>,
    /// Content-Aware Scale's live preview (cas_transform.rs).
    pub(crate) content_aware: Option<crate::cas_transform::Live>,
    /// While a smart object's stored warp is edited (begin.rs `preview_without_smart_warp`):
    /// the texels of the mode the session is not in. Warp mode shows the source without the
    /// warp, Free Transform the layer's render with it; leaving or re-entering Warp mode, and
    /// Undo across it, swap them.
    other_texels: Option<OtherTexels>,
    /// A box over several layers (#6501): their composite, which `textures` holds.
    several: Option<Several>,
}

impl OpenPreview {
    /// A preview of `doc` with `textures` as the moving pixels, with nothing else shown or running.
    fn new(session: u64, doc: Arc<Document>, textures: PreviewTextures, tool: Tool) -> Self {
        Self {
            session,
            doc,
            textures,
            opacity: 1.0,
            above: None,
            styled: None,
            split: None,
            tool,
            path: None,
            follow: None,
            committing: None,
            content_aware: None,
            other_texels: None,
            several: None,
        }
    }
}

/// The moving pixels of a box over several layers ([`begin_several`]): the layers composited on
/// their own, with their styles, opacity and masks. Composited again when an edit made while the
/// box is open changes how they look ([`follow_edits`]), since nothing is tinted at draw time.
struct Several {
    name: String,
    max_side: usize,
    ids: Vec<LayerId>,
    /// The pixels a selection lifted out of moving pixel layers, as the box opened. Every other
    /// moving layer (groups, type, shapes, smart objects, pixel layers without a selection) moves
    /// whole, so the live document holds what it moves, children and their looks included.
    lifted: Vec<(LayerId, photocraft_raster::Surface)>,
    /// The document showing only the moving layers, as last composited.
    moving: Document,
    /// The box's frame.
    frame: photocraft_geom::Rect,
    /// The document area the texture covers: the frame grown by the reach of the moving layers'
    /// styles, so a shadow, glow or outside stroke past their content moves with the box.
    rect: photocraft_geom::Rect,
}

impl Several {
    /// `live` showing only the layers `ids`, with the `lifted` pixels in place of their layers'.
    /// Unclipped: a clipping base that isn't moving would clip them away (#5239).
    fn shown(live: &Document, ids: &[LayerId], lifted: &[(LayerId, photocraft_raster::Surface)]) -> Document {
        let mut d = live.clone();
        show_only(&mut d.layers, ids);
        for id in ids {
            if let Some(path) = d.path_of(*id) {
                for n in 1..path.len() {
                    if let Some(ancestor) = path.get(..n).and_then(|p| d.layer_at_mut(p)) {
                        ancestor.clipped = false;
                    }
                }
            }
            if let Some(l) = d.layer_mut(*id) {
                l.clipped = false;
            }
        }
        for (id, surf) in lifted {
            if let Some(s) = d.layer_mut(*id).and_then(|l| l.surface_mut()) {
                *s = surf.clone();
            }
        }
        d
    }

    /// Composite the moving layers over their frame and their styles' reach.
    fn render(&mut self) -> PreviewTextures {
        self.rect = self.frame.inflate(style_reach(&self.moving, &self.ids));
        let (image, uv) = composite_image(&self.moving, self.rect, self.max_side);
        PreviewTextures::new(self.name.clone(), PreviewPixels::new(image, uv))
    }
}

/// How far past their content the styles of the layers `ids` can draw, in document pixels: their
/// own, the layers' inside them and the groups' around them, since a group's style draws around
/// its children's composite and so adds to theirs.
fn style_reach(doc: &Document, ids: &[LayerId]) -> i32 {
    let reach_of =
        |l: &photocraft_doc::Layer| if l.visible && photocraft_compose::effects::has_effects(l) { photocraft_compose::effects::margin(l).max(0) } else { 0 };
    // The groups around each layer: everything on its path but itself.
    let around =
        |id: LayerId| doc.path_of(id).map_or(0, |p| (1..p.len()).filter_map(|n| doc.layer_at(p.get(..n)?)).fold(0i32, |r, g| r.saturating_add(reach_of(g))));
    // An explicit stack rather than recursion: documents can nest deeply.
    let mut stack: Vec<(&photocraft_doc::Layer, i32)> = ids.iter().filter_map(|id| Some((doc.layer(*id)?, around(*id)))).collect();
    let mut reach = 0;
    while let Some((l, outer)) = stack.pop() {
        let here = outer.saturating_add(reach_of(l));
        reach = reach.max(here);
        if let LayerContent::Group(g) = &l.content {
            stack.extend(g.children.iter().map(|c| (c, here)));
        }
    }
    reach
}

/// The preview texels of the transform mode the session is not in ([`OpenPreview::other_texels`]).
/// The box each covers is the session's own: its undo steps bring the box with the warp.
struct OtherTexels {
    texture: PreviewTextures,
}

/// Swaps the session's texels with the other mode's, when there are any: the stored warp's edit
/// shows the source without the warp, the box the render with it.
fn swap_mode_texels(pv: &mut OpenPreview) {
    if let Some(o) = pv.other_texels.as_mut() {
        std::mem::swap(&mut pv.textures, &mut o.texture);
    }
}

/// What [`follow_edits`] needs to rebuild the preview document.
#[derive(Clone, Debug)]
struct Follow {
    /// The document revision `doc` was built from.
    revision: u64,
    /// The moving layers hidden in `doc` (nothing was lifted out of them).
    hidden: Vec<LayerId>,
    /// The Layers panel's selection when the box opened, sorted; `None` when the box doesn't
    /// follow it. Any change to it applies the box (`end_if_left`, #11101).
    selected: Option<Vec<LayerId>>,
}

/// Photoshop's alert when Free Transform (⌘T, Edit › Transform, the Move tool's transform controls)
/// starts on a layer locked in place, by its own lock or a group's.
pub const LOCKED: &str = "Could not complete the Free Transform command because the layer is locked.";

/// Why Edit › Transform's modes and Warp refuse while Content-Aware Scale's box is open.
pub const CAS_ONLY_SCALES: &str = "Content-Aware Scale only scales: commit or cancel it first";

#[cfg(test)]
pub(crate) fn corners(r: [f64; 4]) -> [[f64; 2]; 4] {
    [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]]
}

/// The interpolation a new session starts with: the General preference. The bar's own pick is
/// `TransformSettings::interpolation`, which the session reads first.
pub(crate) fn default_interpolation(app: &PhotocraftApp) -> Interpolation {
    use photocraft_engine::prefs::Interpolation as General;
    match app.session.prefs().general.image_interpolation {
        General::Nearest => Interpolation::NearestNeighbor,
        General::Bilinear => Interpolation::Bilinear,
        _ => Interpolation::Bicubic,
    }
}

/// The mode an `edit.transform.*` / `edit.freeTransform` menu id starts.
pub fn mode_for_command(id: &str) -> TransformMode {
    match id {
        "edit.transform.rotate" => TransformMode::Rotate,
        "edit.transform.skew" => TransformMode::Skew,
        "edit.transform.distort" => TransformMode::Distort,
        "edit.transform.perspective" => TransformMode::Perspective,
        _ => TransformMode::Free,
    }
}

/// The Rotate and Flip preset an `edit.transform.*` menu id runs on an open box.
pub fn preset_for_command(id: &str) -> Option<TransformPreset> {
    Some(match id {
        "edit.transform.rotate180" => TransformPreset::Rotate180,
        "edit.transform.rotate90Cw" => TransformPreset::Rotate90Clockwise,
        "edit.transform.rotate90Ccw" => TransformPreset::Rotate90CounterClockwise,
        "edit.transform.flipHorizontal" => TransformPreset::FlipHorizontal,
        "edit.transform.flipVertical" => TransformPreset::FlipVertical,
        _ => return None,
    })
}

/// The split a Split Warp menu id arms.
pub fn split_for_command(id: &str) -> Option<WarpSplit> {
    Some(match id {
        "edit.transform.splitWarpCrosswise" => WarpSplit::Cross,
        "edit.transform.splitWarpVertically" => WarpSplit::Vertical,
        "edit.transform.splitWarpHorizontally" => WarpSplit::Horizontal,
        _ => return None,
    })
}

/// The id salt the session's `TransformOverlay` and the Move tool's transform controls share in the canvas body, so
/// a drag begun on the controls carries on in the session's box.
pub(crate) const BOX_ID_SALT: &str = "transform-box";

/// The persisted Free Transform settings: the stored section with Use Legacy Free Transform from the preferences.
pub(crate) fn settings(app: &PhotocraftApp) -> TransformSettings {
    // PLAN: `ToolOptions::transform: TransformSettings` is written by tools-w71 (M14); the errors in this set come from it and from hub-w1's `state.rs` re-exports, not from this file.
    let mut settings = app.ui.tool_options.transform.clone();
    settings.legacy = app.session.prefs().general.use_legacy_free_transform;
    settings
}

/// The canvas tool of an open session (Enter and Escape in the Warp phase), with the box's snap lines of this frame's
/// body ([`show_in_canvas`]). The canvas pass holds the session while it runs, so the tool borrows it, not the app.
pub fn canvas_tool(session: &FreeTransform, snapped: Vec<SnapLine>) -> TransformTool<'_, SessionWarp> {
    TransformTool::new(&session.session, snapped)
}

/// Applies what the tool's keys asked for: `(apply, discard)` from `TransformTool::take_keys`.
pub fn tool_keys(app: &mut PhotocraftApp, (apply, discard): (bool, bool)) {
    if discard {
        cancel(app);
    } else if apply {
        commit(app);
    }
}

#[cfg(test)]
mod tests;
