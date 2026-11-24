//! Direct Selection tool (A, #790), the Convert Point tool (#3204), the anchor point tools, the
//! Curvature Pen on finished paths, the Pens' ⌘/Ctrl (Direct Selection) and the Pen's ⌥/Alt
//! (Convert Point) modifiers, as in Photoshop. They edit the paths the canvas shows: the targeted
//! vector mask, the active shape layer's path or the active type layer's Type Path, the saved path
//! selected in the Paths panel, and the work path.
//!
//! * Click an anchor to select it (⇧ adds or removes, ⌥ selects its whole subpath), drag on empty
//!   canvas to marquee-select anchors, click empty canvas to deselect.
//! * Drag selected anchors to move them, a direction handle to turn it (a smooth point keeps its
//!   handles collinear; ⌥ breaks them apart), or a segment to bend it (straight segments move).
//!   The arrow keys nudge the selected anchors.
//! * Convert Point (the tool, or Pen + ⌥): click an anchor to make it a corner, drag from it to
//!   pull out smooth handles, drag a handle to move it alone.
//! * While a Pen path is being drawn, ⌘/Ctrl edits it in place (its anchors, handles and
//!   segments) and the Pen carries on drawing from its last anchor once ⌘/Ctrl is let go.
//!
//! The one `BezierOverlay` of `vector_ui::show_paths` takes the pointer and the keys: it previews
//! a gesture, and [`edits::apply`] commits one engine command per gesture (`path.moveAnchors`,
//! `path.moveHandle`, `path.bendSegment`, `path.convertPoint`, `path.addAnchor`,
//! `path.deleteAnchor`). One history step per gesture, and the same commands drive it over MCP and
//! the control channel. The Pen path being drawn is not in the document: its edits go to the pen
//! draft through `vector_ui::replace_draft`.

use craft::draw::ui::pen::{BezierTool, EditPath, KnotRef, PathSelection, PathTool};
use craft::keymap::Mods;
use photocraft_doc::vector::Path;
use photocraft_doc::{LayerContent, LayerId, Locks};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;

pub(crate) mod edits;

/// The bits of a [`PathRef::key`] that carry the id or index; the byte above them is the kind.
const KEY_MASK: u64 = (1 << 56) - 1;

/// A path Direct Selection edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathRef {
    Work,
    /// A layer's shape path, or its vector mask.
    Layer(u64),
    /// A type layer's Type Path (#9114), apart from a vector mask the layer may have.
    TypePath(u64),
    /// A saved path, by its index in the document's `paths`: the one selected in the Paths panel.
    Saved(usize),
    /// The Pen path being drawn (`ui.pen`): edited in place, not through the engine.
    Pen,
}

impl PathRef {
    /// The engine's target params for this path. A saved path goes by its name; one that is gone
    /// gets an empty name, which the engine refuses.
    pub(crate) fn params(self, app: &PhotocraftApp) -> Value {
        match self {
            PathRef::Work => json!({"name": "work"}),
            PathRef::Layer(id) => json!({"name": "layer", "layer": id}),
            PathRef::TypePath(id) => json!({"name": "layer", "layer": id, "typePath": true}),
            PathRef::Saved(i) => {
                let name = app.session.active().and_then(|st| st.doc.paths.get(i)).map(|p| p.name.clone()).unwrap_or_default();
                json!({"name": name})
            }
            // Never sent: edits to the Pen path being drawn don't go through the engine.
            PathRef::Pen => json!({}),
        }
    }

    /// The key every overlay `EditPath` of this path carries: the kind in the top byte, the layer
    /// id or path index below.
    pub(crate) fn key(&self) -> u64 {
        let (kind, low) = match *self {
            PathRef::Work => (1u64, 0),
            PathRef::Layer(id) => (2, id),
            PathRef::TypePath(id) => (3, id),
            PathRef::Saved(i) => (4, i as u64),
            PathRef::Pen => (5, 0),
        };
        debug_assert!(low <= KEY_MASK, "id {low} does not fit a path key");
        kind << 56 | (low & KEY_MASK)
    }

    /// The path a [`key`](Self::key) names; `None` for a key no path has.
    pub(crate) fn from_key(key: u64) -> Option<PathRef> {
        let low = key & KEY_MASK;
        match (key >> 56, low) {
            (1, 0) => Some(PathRef::Work),
            (2, id) => Some(PathRef::Layer(id)),
            (3, id) => Some(PathRef::TypePath(id)),
            (4, i) => usize::try_from(i).ok().map(PathRef::Saved),
            (5, 0) => Some(PathRef::Pen),
            _ => None,
        }
    }
}

/// Direct Selection state: which anchors are selected and the gesture in progress.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DirectSelection {
    /// The path whose anchors are selected.
    pub target: Option<PathRef>,
    /// Selected anchors as `[subpath, knot]`.
    pub anchors: Vec<[usize; 2]>,
    /// A clicked segment (`[subpath, knot]` it leaves): its two handles show.
    pub segment: Option<[usize; 2]>,
    /// The subpaths of the target whose anchors show; `None` shows every subpath's (a Delete Anchor click on a
    /// saved path narrows it, #12769). Mirrors `PathSelection::shown`.
    pub shown: Option<Vec<usize>>,
    /// The document, revision and target the running overlay gesture began on: its commit runs
    /// only while they still hold.
    #[serde(skip)]
    pub(crate) editing: Option<Owner>,
    /// The modifiers held when the running gesture began: the tool a gesture resolves to, and so the
    /// name of its History step, is the one its press chose, not the one the release's modifiers pick.
    #[serde(skip)]
    pub(crate) editing_mods: Mods,
    /// The overlay whose press runs the `editing` gesture: `cancel_stale` ends that press with the gesture.
    #[serde(skip)]
    pub(crate) editing_overlay: Option<egui::Id>,
    #[serde(skip)]
    pub(crate) owner: Option<Owner>,
    /// The saved path whose anchors the first Escape hid (#13072): the next Escape deactivates it,
    /// and targeting another path starts at the first Escape again.
    #[serde(skip)]
    pub(crate) escape_hidden: Option<usize>,
    /// Path Selection only: the target path's [`crate::path_components::SelectionSignature`] when
    /// the selection was made — a structural identity that survives history revisions (a click
    /// activates the layer, colours change, components move), so the component selection lives
    /// until the path's structure changes (#10854).
    #[serde(skip)]
    pub(crate) signature: Option<crate::path_components::SelectionSignature>,
}

/// The cached structural check for one document revision. Type Paths are generated only when
/// this cache misses, just like the other target paths are signed only then.
#[derive(Clone, Debug, Default)]
struct SelectionValidation {
    revision: Option<u64>,
    matches: bool,
    #[cfg(test)]
    computations: usize,
}

/// Runtime selection ownership is never restored from settings onto another document.
#[derive(Clone, Debug)]
pub(crate) struct Owner {
    document: photocraft_doc::DocId,
    source: std::sync::Weak<photocraft_doc::Document>,
    revision: u64,
    layer: Option<photocraft_doc::LayerId>,
    selected_path: Option<String>,
    vector_mask: bool,
    tool: Tool,
    validation: std::cell::RefCell<SelectionValidation>,
}

impl PartialEq for Owner {
    fn eq(&self, other: &Self) -> bool {
        self.source.ptr_eq(&other.source)
            && (self.document, self.revision, self.layer, &self.selected_path, self.vector_mask, self.tool)
                == (other.document, other.revision, other.layer, &other.selected_path, other.vector_mask, other.tool)
    }
}

impl Owner {
    pub(crate) fn current(app: &PhotocraftApp) -> Option<Self> {
        let st = app.session.active()?;
        Some(Self {
            document: st.doc.id,
            source: std::sync::Arc::downgrade(&st.doc),
            revision: st.revision,
            layer: st.active_layer,
            selected_path: app.ui.selected_path.clone(),
            vector_mask: app.ui.vector_mask_target || st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::VectorMask,
            tool: app.ui.tool,
            validation: Default::default(),
        })
    }

    #[cfg(test)]
    pub(crate) fn selection_computations(&self) -> usize {
        self.validation.borrow().computations
    }

    fn same_target(&self, app: &PhotocraftApp) -> bool {
        self.same_path_target(app) && app.ui.tool == self.tool
    }

    fn same_path_target(&self, app: &PhotocraftApp) -> bool {
        app.session.active().is_some_and(|st| {
            st.doc.id == self.document
                && st.active_layer == self.layer
                && (app.ui.vector_mask_target || st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::VectorMask) == self.vector_mask
        }) && app.ui.selected_path == self.selected_path
    }

    fn valid(&self, app: &PhotocraftApp) -> bool {
        self.valid_path(app) && app.ui.tool == self.tool
    }

    fn valid_path(&self, app: &PhotocraftApp) -> bool {
        self.same_path_target(app)
            && app.session.active().is_some_and(|st| st.revision == self.revision && self.source.ptr_eq(&std::sync::Arc::downgrade(&st.doc)))
    }
}

/// Is `ds` still about the document, revision and target it was made on? Convert Point works on
/// the handles selected with Direct Selection, so an idle selection crosses those two tools; a
/// gesture keeps its own tool.
pub(crate) fn still_valid(app: &PhotocraftApp, ds: &DirectSelection) -> bool {
    ds.owner.as_ref().is_some_and(|owner| owner.valid(app) || (ds.editing.is_none() && owns(owner.tool) && owns(app.ui.tool) && owner.valid_path(app)))
        || component_selection_matches(app, ds)
}

/// The live path `target` addresses on the active document, if it still exists. Existing paths
/// are borrowed; only a Type Path is generated, and the revision cache limits how often.
pub(crate) fn path_of<'a>(app: &'a PhotocraftApp, target: &PathRef) -> Option<std::borrow::Cow<'a, Path>> {
    let st = app.session.active()?;
    match target {
        PathRef::Layer(id) => st.doc.layer(photocraft_doc::LayerId(*id)).and_then(|l| match &l.content {
            photocraft_doc::LayerContent::Shape(sh) => Some(std::borrow::Cow::Borrowed(&sh.path)),
            _ => l.vector_mask.as_ref().map(|m| std::borrow::Cow::Borrowed(&m.path)),
        }),
        PathRef::Work => st.doc.work_path.as_ref().map(std::borrow::Cow::Borrowed),
        PathRef::Saved(i) => st.doc.paths.get(*i).map(|p| std::borrow::Cow::Borrowed(&p.path)),
        // The stored id is the layer whose Type Path this is; resolving by it confirms the match.
        PathRef::TypePath(id) => st.doc.layer(photocraft_doc::LayerId(*id)).and_then(|l| match &l.content {
            photocraft_doc::LayerContent::Text(t) => photocraft_engine::type_extra_cmds::type_path(t).map(std::borrow::Cow::Owned),
            _ => None,
        }),
        PathRef::Pen => None,
    }
}

/// A Path Selection component selection outlives history revisions: ordered subpath geometry and
/// the component partition are what the stored indices address. Valid while the target path still
/// exists on the same document and carries the same signature.
fn component_selection_matches(app: &PhotocraftApp, ds: &DirectSelection) -> bool {
    if ds.editing.is_some() || app.ui.tool != Tool::PathSelection {
        return false;
    }
    let (Some(owner), Some(target), Some(sig)) = (ds.owner.as_ref(), ds.target.as_ref(), ds.signature.as_ref()) else { return false };
    if owner.tool != Tool::PathSelection {
        return false;
    }
    let Some(st) = app.session.active() else { return false };
    if st.doc.id != owner.document {
        return false;
    }
    {
        let cached = owner.validation.borrow();
        if cached.revision == Some(st.revision) {
            return cached.matches;
        }
    }
    let matches = path_of(app, target).is_some_and(|p| crate::path_components::selection_signature(&p) == *sig);
    let mut cached = owner.validation.borrow_mut();
    cached.revision = Some(st.revision);
    cached.matches = matches;
    #[cfg(test)]
    {
        cached.computations += 1;
    }
    matches
}

/// Clear stale anchors and gestures. True when a gesture was running and is cancelled with them.
pub(crate) fn cancel_stale(app: &mut PhotocraftApp) -> bool {
    let ds = &app.ui.direct_selection;
    // A running gesture is held to the owner it began on, a selection to its own: a marquee begun with nothing
    // selected has a gesture and no selection yet.
    let gesture_valid = ds.editing.as_ref().is_none_or(|began| began.valid(app));
    let empty = ds.target.is_none() && ds.anchors.is_empty() && ds.segment.is_none();
    if gesture_valid && ((ds.owner.is_none() && empty) || still_valid(app, ds)) {
        return false;
    }
    let was_editing = ds.editing.is_some();
    if let Some(overlay) = ds.editing_overlay.filter(|_| was_editing) {
        craft::ui::primitives::cancel_press_of(app.ctx(), overlay);
    }
    app.ui.direct_selection = DirectSelection::default();
    was_editing
}

/// Rebind a selection to the revision its own command made; another target or tool drops it.
pub(crate) fn keep_after_command(app: &mut PhotocraftApp, mut selection: DirectSelection) {
    if selection.owner.as_ref().is_some_and(|owner| owner.same_target(app)) {
        selection.owner = Owner::current(app);
        app.ui.direct_selection = selection;
    }
}

/// Is an overlay gesture of the path tools running? Esc and the modal gates read it.
pub(crate) fn editing(app: &PhotocraftApp) -> bool {
    app.ui.direct_selection.editing.is_some()
}

/// The overlay's path tool for `tool` with `mods` held. ⌘/Ctrl makes the Freeform Pen Direct
/// Selection (the Pen's table resolves it); the Freeform Pen has no other overlay. Path Selection
/// with ⌘/Ctrl alone is Direct Selection too (#13284; with ⌥ or ⇧ too the reference was not
/// measured, so it stays Path Selection).
pub(crate) fn path_tool(tool: Tool, mods: Mods) -> Option<PathTool> {
    match tool {
        Tool::Pen => Some(PathTool::Pen),
        Tool::CurvaturePen => Some(PathTool::Curvature),
        Tool::AddAnchorPoint => Some(PathTool::AddAnchor),
        Tool::DeleteAnchorPoint => Some(PathTool::DeleteAnchor),
        Tool::ConvertPoint => Some(PathTool::ConvertPoint),
        Tool::DirectSelection => Some(PathTool::DirectSelection),
        Tool::FreeformPen if mods.cmd => Some(PathTool::Pen),
        Tool::PathSelection if mods.cmd && !mods.alt && !mods.shift => Some(PathTool::DirectSelection),
        _ => None,
    }
}

/// The selection the overlay shows for `bezier`: this selection while it still holds, else none.
/// Path Selection's ⌘/Ctrl (Direct Selection) moves the anchor pressed alone, not the selected
/// component it belongs to (#13284), so the component's anchors are not Direct Selection's.
pub(crate) fn selection(app: &PhotocraftApp, bezier: BezierTool) -> PathSelection {
    let ds = &app.ui.direct_selection;
    if !still_valid(app, ds) {
        return PathSelection::new(None, Vec::new(), None);
    }
    let knot = |&[s, k]: &[usize; 2]| KnotRef::new(s, k);
    let target = ds.target.as_ref().map(PathRef::key);
    if app.ui.tool == Tool::PathSelection && bezier == BezierTool::Direct {
        return PathSelection::new(target, Vec::new(), None).shown(ds.shown.clone());
    }
    PathSelection::new(target, ds.anchors.iter().map(knot).collect(), ds.segment.as_ref().map(knot)).shown(ds.shown.clone())
}

/// The paths the overlay edits with `bezier`, back to front, and their refs in the same order. A
/// locked layer's path is left out. The open pen draft comes last where it shows without sensing
/// (`PathMode::draft_senses` is false: Direct Selection, Convert Point, the anchor tools).
pub(crate) fn edit_paths(app: &PhotocraftApp, bezier: BezierTool) -> (Vec<EditPath>, Vec<PathRef>) {
    let draft_shown = matches!(bezier, BezierTool::Direct | BezierTool::Convert | BezierTool::AddAnchor | BezierTool::DeleteAnchor);
    let mut front: Vec<(PathRef, Path)> = match bezier {
        // ⌘/Ctrl on a Pen path being drawn edits only that path in place; a press anywhere else is a press off it,
        // which ends it (`edits::apply`, #4022).
        BezierTool::Direct if app.ui.tool != Tool::DirectSelection && drafted(app).is_some() => {
            candidates(app).into_iter().filter(|(r, _)| *r == PathRef::Pen).collect()
        }
        BezierTool::Direct => candidates(app),
        BezierTool::Convert => {
            // The tool's targets, and the Work Path behind them.
            let mut paths = targeted(app);
            if !paths.iter().any(|(r, _)| *r == PathRef::Work) {
                paths.extend(app.session.active().and_then(|st| st.doc.work_path.clone()).map(|p| (PathRef::Work, p)));
            }
            paths
        }
        BezierTool::AddAnchor | BezierTool::DeleteAnchor => {
            let mut paths: Vec<(PathRef, Path)> = targeted(app).into_iter().filter(|(r, _)| !ignores(app, r)).collect();
            // A path the Paths panel selects (a saved one or the work path) is the only target (#12762).
            if let Some((index, _)) = crate::vector_ui::selected_saved_path_index(app) {
                paths.retain(|(r, _)| *r == PathRef::Saved(index));
            } else if app.ui.selected_path.as_deref() == Some("work") {
                paths.retain(|(r, _)| *r == PathRef::Work);
            }
            paths
        }
        _ => active_paths(app).into_iter().take(1).collect(),
    };
    front.retain(|(r, _)| (draft_shown || *r != PathRef::Pen) && writable(app, r));
    let (mut paths, mut refs) = (Vec::new(), Vec::new());
    for (r, path) in front.into_iter().rev() {
        let edit = match (r, drafted(app)) {
            (PathRef::Pen, Some(pen)) => EditPath::new(r.key(), vec![pen.draft.clone()]).copies(false),
            _ => crate::vector_ui::edit_path(&path, r.key()),
        };
        paths.push(edit);
        refs.push(r);
    }
    (paths, refs)
}

/// The anchor tools ignore a click on a shape with only its own position lock set (#12730, #12825).
pub(crate) fn ignores(app: &PhotocraftApp, r: &PathRef) -> bool {
    if let PathRef::Layer(id) = *r
        && let Some(st) = app.session.active()
        && let Some(layer) = st.doc.layer(LayerId(id))
        && matches!(layer.content, LayerContent::Shape(_))
        && layer.locks == (Locks { position: true, ..Locks::default() })
        && let Some(at) = st.doc.path_of(layer.id)
        && at.split_last().is_some_and(|(_, ancestors)| !st.doc.locks_at(ancestors).any())
    {
        return true;
    }
    false
}

/// The Pen path being drawn, when it is on the active document and has an anchor.
fn drafted(app: &PhotocraftApp) -> Option<&crate::vector_ui::PenPath> {
    let active = app.session.active().map(|st| st.doc.id);
    app.ui.pen.as_ref().filter(|_| app.ui.pen_doc.is_none_or(|owner| Some(owner) == active)).filter(|p| !p.draft.knots.is_empty())
}

/// The selected anchor when it is the only one and an endpoint of an open subpath of the active
/// path: where the Curvature Pen carries the path on from (#10726), as Photoshop does after a
/// click, or ⌘/Ctrl-click, on the last point. The path, the subpath's index and the knot's.
pub(crate) fn selected_open_end(app: &PhotocraftApp) -> Option<(PathRef, usize, usize)> {
    let ds = &app.ui.direct_selection;
    let ([[s, k]], Some(target)) = (ds.anchors.as_slice(), ds.target) else { return None };
    let (_, path) = active_paths(app).into_iter().find(|(r, _)| *r == target)?;
    let sub = path.subpaths.get(*s).filter(|sub| !sub.closed)?;
    sub.knots.get(*k).filter(|_| *k == 0 || *k + 1 == sub.knots.len())?;
    Some((target, *s, *k))
}

/// The tools that keep a selection across each other: Direct Selection and Convert Point.
fn owns(tool: Tool) -> bool {
    matches!(tool, Tool::DirectSelection | Tool::ConvertPoint)
}

/// The active paths, front first: the targeted vector mask, the active shape layer's path or the
/// active type layer's Type Path (#9114), then the work path. The Pen's Auto Add/Delete edits these.
pub(crate) fn active_paths(app: &PhotocraftApp) -> Vec<(PathRef, Path)> {
    let layer = crate::vector_ui::targeted_vector_mask(app)
        .or_else(|| crate::vector_ui::active_shape_path(app))
        .map(|(id, p)| (PathRef::Layer(id), p))
        .or_else(|| crate::vector_ui::active_type_path(app).map(|(id, p)| (PathRef::TypePath(id), p)));
    let saved = crate::vector_ui::selected_saved_path_index(app).map(|(i, p)| (PathRef::Saved(i), p));
    let work = app.session.active().and_then(|st| st.doc.work_path.clone()).filter(|_| crate::vector_ui::work_path_shown(app)).map(|p| (PathRef::Work, p));
    let mut out: Vec<_> = saved.into_iter().chain(layer).chain(work).collect();
    if app.ui.selected_path.as_deref() == Some("work")
        && let Some(index) = out.iter().position(|(target, _)| *target == PathRef::Work)
    {
        out.swap(0, index);
    }
    out.retain(|(target, _)| writable(app, target));
    out
}

/// The paths Direct Selection edits, front first: the Pen path being drawn, the active paths, every
/// visible shape layer, then the work path.
pub(crate) fn candidates(app: &PhotocraftApp) -> Vec<(PathRef, Path)> {
    let pen = drafted(app).map(|p| (PathRef::Pen, crate::vector_ui::pen_path(p)));
    let mut out: Vec<_> = pen.into_iter().chain(active_paths(app)).collect();
    if let Some(st) = app.session.active() {
        for (path, _, l) in st.doc.walk().iter().rev() {
            if !l.visible || !(1..=path.len()).all(|n| st.doc.layer_at(&path[..n]).is_some_and(|a| a.visible)) {
                continue;
            }
            if let photocraft_doc::LayerContent::Shape(sh) = &l.content
                && !out.iter().any(|(r, _)| *r == PathRef::Layer(l.id.0))
            {
                out.push((PathRef::Layer(l.id.0), sh.path.clone()));
            }
        }
        if !out.iter().any(|(r, _)| *r == PathRef::Work) {
            out.extend(st.doc.work_path.clone().map(|p| (PathRef::Work, p)));
        }
    }
    out
}

/// Can `target` be edited: a layer's path only while the layer is not locked.
pub(crate) fn writable(app: &PhotocraftApp, target: &PathRef) -> bool {
    match target {
        PathRef::Layer(id) | PathRef::TypePath(id) => app.session.active().is_some_and(|st| {
            let id = photocraft_doc::LayerId(*id);
            let locks = st.doc.effective_locks(id);
            // A hidden layer's path (or one in a hidden group) shows nothing to edit.
            let shown = st
                .doc
                .walk()
                .iter()
                .find(|(_, _, l)| l.id == id)
                .is_some_and(|(path, _, l)| l.visible && (1..=path.len()).all(|n| st.doc.layer_at(&path[..n]).is_some_and(|a| a.visible)));
            shown && !locks.all && !locks.pixels
        }),
        _ => app.session.active().is_some(),
    }
}

/// The Pen draft (as `candidates` builds it), then the active paths, then (not measured, so as today,
/// #12762, #12705) every shown, unlocked shape layer inside a group, front first. A top-level shape
/// layer is a target only as the active layer.
fn targeted(app: &PhotocraftApp) -> Vec<(PathRef, Path)> {
    let Some(st) = app.session.active() else {
        return Vec::new();
    };
    // The draft's destination is private to `vector_ui`, so it is not filtered here.
    let pen = drafted(app).map(|p| (PathRef::Pen, crate::vector_ui::pen_path(p)));
    let mut out: Vec<(PathRef, Path)> = pen.into_iter().chain(active_paths(app)).collect();
    let layers = st.doc.walk();
    let grouped = layers.iter().rev().filter(|(at, depth, _)| *depth > 0 && st.doc.shown_at(at)).filter_map(|(_, _, l)| match &l.content {
        photocraft_doc::LayerContent::Shape(sh) => Some((PathRef::Layer(l.id.0), sh.path.clone())),
        _ => None,
    });
    for (r, path) in grouped {
        if writable(app, &r) && !out.iter().any(|(have, _)| *have == r) {
            out.push((r, path));
        }
    }
    out
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod convert_point_history_tests;

#[cfg(test)]
mod convert_point_targeted_path_tests;

#[cfg(test)]
mod convert_point_drag_segment_tests;

#[cfg(test)]
mod delete_key_tests;

#[cfg(test)]
mod history_names_tests;

#[cfg(test)]
mod options_bar_tests;

#[cfg(test)]
mod convert_point_delete_key_tests;

#[cfg(test)]
mod freeform_command_tests {
    use super::*;
    use crate::canvas::testing::{move_to, press, release};
    use egui::Modifiers;

    fn app(tool: Tool) -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        for name in ["work", "Saved drawing"] {
            app.run("path.set", json!({"name": name, "path":{"subpaths":[{"knots":[[20,20],[120,20]]}]}})).unwrap();
        }
        app.sync_views();
        app.ui.tool = tool;
        app.ui.selected_path = Some("Saved drawing".into());
        app
    }

    #[test]
    fn command_with_freeform_and_curvature_edits_the_selected_saved_path_after_modifier_release() {
        for tool in [Tool::FreeformPen, Tool::CurvaturePen] {
            let mut app = app(tool);
            let original = app.session.active().unwrap().doc.clone();
            let n = app.session.active().unwrap().history.past_len();
            let command = Modifiers { command: true, ctrl: true, ..Default::default() };

            press(&mut app, [20.0, 20.0], command);
            assert_eq!(app.ui.direct_selection.target, Some(PathRef::Saved(0)));
            assert!(!editing(&app), "{tool:?}: the gesture begins with the drag");
            move_to(&mut app, [30.0, 40.0], Modifiers::NONE);
            assert!(editing(&app), "{tool:?}: the press's drag goes on under its own tool after the modifier is released");
            release(&mut app, [30.0, 40.0], Modifiers::NONE);
            let st = app.session.active().unwrap();
            assert_eq!(st.doc.work_path, original.work_path, "{tool:?}: the Work Path stays");
            assert_eq!(st.doc.paths[0].path.subpaths[0].knots[0].anchor, photocraft_geom::Point::new(30.0, 40.0));
            assert_eq!(st.history.past_len(), n + 1);
            assert!(!editing(&app), "{tool:?}: the release ends the gesture");
            assert!(app.session.undo());
            assert_eq!(app.session.active().unwrap().doc.paths, original.paths);
        }
    }
}
