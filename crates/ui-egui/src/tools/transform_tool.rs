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
    WARP_ONE_LAYER, begin, begin_copy, begin_from_frame, begin_placed, begin_selection, begin_warp, enter_warp, hold_warp, leave_warp, warps_active_layer,
    warps_type,
};
pub(crate) use commit::{SAVE_FOR_WEB, committing};
pub use commit::{apply_click, auto_commit, cancel, commit, end_if_left, follow_edits, intercept, is_enabled, on_job_event};
#[cfg(test)]
pub(crate) use draw::styled_composite;
pub use draw::{SessionShown, show_in_canvas};
pub(crate) use draw::{moves_selection, selection_map};
pub(crate) use options_bar::bar_key;
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
    /// A rotated, skewed or distorted smart object's box opened on its placement (#3649):
    /// `textures` holds its pixels as they are over `rect`, which move with the box from `start`.
    pub(crate) placed: Option<Placed>,
    /// Warp mode on such a box ([`begin::enter_placed_warp`]): the map from the smart object's contents to the
    /// document the warp's control points were carried through (row-major 3×3), and whether the box had moved first.
    pub(crate) placed_warp: Option<([f64; 9], bool)>,
    /// A warp held while the box is in another mode ([`begin::hold_warp`]).
    pub(crate) held: Option<HeldWarp>,
    /// One entry per undone step that came back into Warp mode, newest last: whether it undid a hold, so its Redo
    /// holds the warp again rather than dropping it ([`commit::step`]). Emptied once the session has nothing to redo.
    pub(crate) redo_holds: Vec<bool>,
    /// During a warp drag on a placed smart object: the map from its contents as the drag began. The pull rule works
    /// through it, while `placed_warp` is what the drag has made of it ([`warp::grid_edit`]'s squeeze at the horizon).
    pub(crate) drag_placed: Option<([f64; 9], bool)>,
    /// The meshes the warp grid's rule proposed squeezed at the horizon this drag, each with the map it carried them
    /// through ([`warp::grid_rule`]), so [`warp::grid_edit`] lands each as proposed, with its map.
    pub(crate) rule_squeezes: warp::RuleSqueezes,
    /// A smart object's stored warp, for the box's preview once its move is projective ([`WarpedPreview`]).
    pub(crate) warped: Option<WarpedPreview>,
}

/// A smart object's stored warp as the render draws it under a Perspective or Distort move of the box: its control
/// points carried through the move and the placement, over its contents unplaced. The box's preview draws it so once
/// the move is projective, since the rendered pixels mapped through the move are not what the commit renders; under
/// an affine move they are, and the preview maps them.
pub(crate) struct WarpedPreview {
    /// The stored warp's control points in the contents.
    pub(crate) mesh: BezierMesh,
    /// The warp's box in the contents.
    pub(crate) bounds: [f64; 4],
    /// The placement, contents to document (row-major 3×3).
    pub(crate) place: [f64; 9],
    /// The box when it opened.
    pub(crate) start: [[f64; 2]; 4],
    /// The contents unplaced, over `bounds`.
    pub(crate) texture: PreviewTextures,
}

/// A warp kept when Edit › Transform's box modes are chosen in Warp mode: the reference keeps the warp, shows it
/// moving with the box, carries on with it in Warp mode again and applies both together.
pub(crate) struct HeldWarp {
    /// The warp as Warp mode left it: its control points in the document, over its bounds.
    pub(crate) warp: Warp,
    /// The box when the warp was held; the warp moves with the box from there.
    pub(crate) start: [[f64; 2]; 4],
    /// For a box that opened on a smart object's placement, the map from its contents to the document the points
    /// were carried through, and whether the box had moved ([`OpenPreview::placed_warp`]).
    pub(crate) placed: Option<([f64; 9], bool)>,
    /// The texels the warp maps when they aren't the box's (a smart object's source or contents).
    pub(crate) texture: Option<PreviewTextures>,
}

impl HeldWarp {
    /// The warp moved with the box from where it was held to `quad`: its control points carried through the box's
    /// move (exact for an affine move, the reference's own rule for a projective one).
    pub(crate) fn carried(&self, quad: [[f64; 2]; 4]) -> Warp {
        match Homography::quad_to_quad(self.start, quad).filter(|_| quad != self.start) {
            Some(h) => Warp::custom(
                self.warp.to_mesh(1, 1).map_points(|p| {
                    let (x, y) = h.apply(p[0], p[1]);
                    [x, y]
                }),
                self.warp.bounds,
            ),
            None => self.warp.clone(),
        }
    }

    /// The warp and its contents-to-document map once the box moved to `quad`, as [`carried`](Self::carried) and
    /// [`placed_at`](Self::placed_at), but where the move takes the warp's points to its horizon the map gives way, as
    /// it does under a pull ([`warp::horizon_squeeze`]): past the horizon the carried points would go through infinity.
    pub(crate) fn given_way(&self, quad: [[f64; 2]; 4]) -> (Warp, Option<([f64; 9], bool)>) {
        let (warp, placed) = (self.carried(quad), self.placed_at(quad));
        let Some(h) = Homography::quad_to_quad(self.start, quad).filter(|_| quad != self.start) else { return (warp, placed) };
        let source = self.warp.to_mesh(1, 1);
        let (total, source) = match self.placed {
            Some((m, _)) => {
                let Some(back) = Homography(m).inverse() else { return (warp, placed) };
                let contents = source.map_points(|p| {
                    let (x, y) = back.apply(p[0], p[1]);
                    [x, y]
                });
                (h.mul(&Homography(m)), contents)
            }
            None => (h, source),
        };
        let Some(squeeze) = warp::horizon_squeeze(total.0, &source.points, self.warp.bounds) else { return (warp, placed) };
        let given = total.mul(&Homography(squeeze));
        if !given.0.iter().all(|v| v.is_finite()) {
            return (warp, placed);
        }
        let doc = source.map_points(|p| {
            let (x, y) = given.apply(p[0], p[1]);
            [x, y]
        });
        (Warp::custom(doc, self.warp.bounds), placed.map(|_| (given.0, true)))
    }

    /// The contents-to-document map once the box moved from where the warp was held to `quad`, and whether the
    /// placement changed at all.
    pub(crate) fn placed_at(&self, quad: [[f64; 2]; 4]) -> Option<([f64; 9], bool)> {
        let (to_doc, moved) = self.placed?;
        match Homography::quad_to_quad(self.start, quad).filter(|_| quad != self.start) {
            Some(h) => Some((h.mul(&Homography(to_doc)).0, true)),
            None => Some((to_doc, moved)),
        }
    }
}

/// Where a box that opened on a smart object's placement started, and the pixels it moves.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placed {
    /// The placement's corners, where the box opened.
    pub(crate) start: [[f64; 2]; 4],
    /// The document area the preview's texels cover.
    pub(crate) rect: [f64; 4],
    /// The box's frame: the placement maps it onto `start`.
    pub(crate) frame: [f64; 4],
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
            placed: None,
            placed_warp: None,
            held: None,
            redo_holds: Vec::new(),
            drag_placed: None,
            rule_squeezes: warp::RuleSqueezes::default(),
            warped: None,
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
    fn render(&mut self, serials: &mut craft::ui::primitives::Serials) -> PreviewTextures {
        self.rect = self.frame.inflate(style_reach(&self.moving, &self.ids));
        let (image, uv) = composite_image(&self.moving, self.rect, self.max_side);
        PreviewTextures::new(serials, self.name.clone(), PreviewPixels::new(image, uv))
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
    /// The other mode's map from a smart object's contents ([`OpenPreview::placed_warp`]): Warp mode's when the box
    /// is showing, `None` (the box's) while Warp is. It goes with the texels, so Undo and Redo across Warp mode bring
    /// both back together.
    placed_warp: Option<([f64; 9], bool)>,
}

/// Swaps the session's texels with the other mode's, when there are any: the stored warp's edit
/// shows the source without the warp, the box the render with it; Warp on a placed smart object
/// shows its contents through `placed_warp`, which swaps with them.
fn swap_mode_texels(pv: &mut OpenPreview) {
    if let Some(o) = pv.other_texels.as_mut() {
        std::mem::swap(&mut pv.textures, &mut o.texture);
        std::mem::swap(&mut pv.placed_warp, &mut o.placed_warp);
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
