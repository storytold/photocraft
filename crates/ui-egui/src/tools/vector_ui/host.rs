//! The Vector family of the tool host: the Pen, Curvature Pen, Freeform Pen, the anchor tools, Direct Selection, Path
//! Selection and the Shape tools.
//!
//! [`tool`] builds the selected tool for one pass from the app's options; [`drain`] turns what the tools reported into
//! the draft in `app.ui.pen`, engine commands and dialogs after the run. A tool switch cancels the Pen through
//! `ToolHost::cancel`, which ends it with `PenEnd::Interrupted`; the next `drain` commits the draft.

use craft::behavior::EditEvent;
use craft::draw::geometry::ShapeGeometry;
use craft::draw::ui::pen::{AnchorTool, PenDraft, PenEvent, PenTool};
use craft::draw::ui::{EditSubpath, KnotRef, PathTool, PenEnd, PenMode, ShapeDragEvent, ShapeDragOptions, ShapeDragTool, ShapeKind};
use craft::keymap::Mods;
use craft::ui::canvas::ToolHostResponse;
use serde_json::json;

use super::freeform::{Admit, FreeformPenTool, freeform_locked_shape};
use super::path_ops::{PathSelectionEvent, PathSelectionPass, PathSelectionTool};
use super::pen::{self, pen_path};
use super::pen_resume::{current_doc, draft_before_finish, remember_finish};
use super::{PenPath, PhotocraftApp, VectorOwner, cancel_stale_pen, capture_pen_owner};
use crate::canvas::document_display::CanvasHost;
use crate::canvas::tools::{FamilyTool, ToolScope};
use crate::direct_select::PathRef;
use crate::state::Tool;

/// The state of every vector tool, kept across passes in `CanvasTools::vector`.
pub(crate) struct VectorTools {
    /// The Pen and the Curvature Pen.
    pub(crate) pen: PenTool,
    /// The Shape tools.
    pub(crate) shapes: ShapeDragTool,
    /// The Freeform Pen.
    pub(crate) freeform: FreeformPenTool,
    /// Path Selection.
    pub(crate) path_selection: PathSelectionTool,
}

impl Default for VectorTools {
    fn default() -> Self {
        Self { pen: PenTool::new(), shapes: ShapeDragTool::new(), freeform: FreeformPenTool::default(), path_selection: Default::default() }
    }
}

/// Which of the family's tools a pass runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Pen,
    /// Add and Delete Anchor Point, Convert Point and Direct Selection: the path overlay takes their presses.
    Anchors(PathTool),
    Shapes,
    Freeform,
    PathSelection,
}

/// Configures the tool the app has selected from the app's options and returns which one it is.
fn configure(app: &PhotocraftApp, state: &mut VectorTools) -> Which {
    let tool = app.ui.tool;
    let o = &app.ui.tool_options;
    if super::shapes::is_shape_tool(tool) {
        let (sides, ratio) = match tool {
            Tool::Triangle => (3, 1.0),
            Tool::Star => (5, 0.5),
            _ => (o.polygon_sides, 1.0),
        };
        let mut options = ShapeDragOptions::new(super::kind_of(tool))
            .corner_radius(f64::from(o.corner_radius))
            .sides(sides)
            .star_ratio(ratio)
            .line_weight(f64::from(o.line_weight))
            .mode(o.shape_mode);
        // The gear belongs to the Rectangle tool alone.
        if tool == Tool::Rectangle {
            options = options.constraints(super::shapes::constraints_of(o));
        }
        state.shapes.options(options).preview(true);
        return Which::Shapes;
    }
    match tool {
        Tool::FreeformPen => {
            let admit = if freeform_locked_shape(app) {
                Admit::Locked
            } else if app.ui.pen.is_some() {
                Admit::Busy
            } else {
                Admit::Open
            };
            state.freeform.admission(admit);
            Which::Freeform
        }
        Tool::PathSelection => Which::PathSelection,
        _ => {
            let curvature = tool == Tool::CurvaturePen;
            let pen = app.ui.pen.as_ref();
            // The anchor tools run the Pen only to show the open draft (`AnchorTool::draft`).
            let anchors = crate::direct_select::path_tool(tool, Mods::NONE).filter(|t| !matches!(t, PathTool::Pen | PathTool::Curvature));
            // ⌘ makes the pointer Direct Selection: the draft is drawn and keeps its keys, but takes no presses.
            // A refused draft of another document waits for its explicit retry: no press adds to it (`cancel_stale`).
            let stale = pen.is_some_and(|p| p.commit_failed && p.owner.as_ref().is_some_and(|o| !o.valid(app)));
            let sensing = !craft::ui::runtime::pointer::frame(app.ctx()).mods.cmd && !stale;
            let idle = pen.is_none_or(|p| p.draft.knots.is_empty());
            let carry = if curvature && idle { pen::carry_on(app) } else { None };
            state
                .pen
                .mode(if curvature { PenMode::Curvature } else { PenMode::Pen })
                .sensing(sensing)
                .auto_add_delete(!curvature && o.pen_auto_add_delete)
                .path_options(o.path_options)
                .ends(&if idle { pen::ends(app) } else { Vec::new() })
                .carry_on(carry.as_ref().map(|(key, knot, draft)| (*key, *knot, draft)))
                // A draft the selected tool did not draw (a refused one kept after a tool switch) is foreign: Delete,
                // Backspace and Undo leave its anchors to that tool (`PenPath::owns_undo`).
                .draft(pen.map(|p| if p.owns_undo(tool) { PenDraft::Own(&p.draft) } else { PenDraft::Foreign(&p.draft) }))
                .kept(pen.map_or(0, |p| p.kept));
            anchors.map_or(Which::Pen, Which::Anchors)
        }
    }
}

/// The Vector family's tool for one pass: the selected tool, configured from the app.
pub(crate) fn tool<'a>(scope: &'a ToolScope<'a>, state: &'a mut VectorTools, _host: CanvasHost) -> FamilyTool<'a> {
    let which = scope.with_app(|app| configure(app, state));
    // `&mut T` is a `CanvasTool` where `T` is (R92): the selected tool runs as it is.
    match which {
        Which::Pen => Box::new(&mut state.pen),
        Which::Anchors(tool) => Box::new(AnchorTool::new(tool).draft(&mut state.pen)),
        Which::Shapes => Box::new(&mut state.shapes),
        Which::Freeform => Box::new(&mut state.freeform),
        Which::PathSelection => Box::new(PathSelectionPass { scope, tool: &mut state.path_selection }),
    }
}

/// After the run: the owner check, then what each tool reported. Every tool is drained, whichever is selected: a tool
/// switch ends the Pen's gesture with an event of the old tool.
pub(crate) fn drain(app: &mut PhotocraftApp, state: &mut VectorTools, _response: &ToolHostResponse) {
    // A live shape layer that moved under its draft since the last pass (Undo, Redo, the History panel) gives the Pen
    // the draft it has now (#12909). Here the app holds stand-in tools, so the Pen is told directly.
    if let Some(draft) = pen::sync_live_draft_from_layer(app) {
        state.pen.kept(0).replace(&draft);
    }
    cancel_stale_pen(app);
    let before = app.ui.pen.as_ref().map(|p| p.draft.clone());
    let events = state.pen.take_events();
    apply_pen(app, events);
    // A live shape layer takes the draft's anchors once a press is over (a release, or a key that changed the draft):
    // a drag's updates make no History steps of their own (#12909).
    let pointer = craft::ui::runtime::pointer::frame(app.ctx());
    let changed = app.ui.pen.as_ref().map(|p| &p.draft) != before.as_ref();
    if pointer.buttons.is_empty() && (changed || pointer.release.is_some()) {
        pen::sync_live_layer_from_draft(app);
    }
    end_for_other_tool(app);
    for event in state.shapes.take_events() {
        match event {
            ShapeDragEvent::Draw(EditEvent::Commit { after, .. }) => {
                let mods = craft::ui::runtime::pointer::frame(app.ctx()).mods;
                super::shapes::finish_drag(app, state.shapes.kind(), &after, mods);
            }
            ShapeDragEvent::Click { at, kind, .. } => {
                super::shapes::click(app, kind, [at.x, at.y]);
            }
            _ => {}
        }
    }
    if state.freeform.take_refused() {
        app.ui.status = tl!("Locked").into();
        app.ui.status_error = true;
    }
    // The destination is the one in effect when the trace began: this pass, as the trace's press ran in it.
    if state.freeform.owner.is_none() && (state.freeform.draft.is_open() || state.freeform.closed.is_some()) {
        state.freeform.owner = capture_pen_owner(app);
        state.freeform.target = VectorOwner::capture(app);
    }
    if let Some((points, straight, owner, target)) = state.freeform.take_finished()
        && target.is_none_or(|t| t.valid(app))
    {
        pen::freeform_finish_owned(app, &points, &straight, owner);
    }
    // PLAN: assumed `PathSelectionTool::owned_by(&mut self, owner: Option<VectorOwner>)` (tools-w35, R94).
    for event in state.path_selection.take_events() {
        match event {
            PathSelectionEvent::Press { at, shift } => {
                // A press on a shape layer, off every component of the targeted saved path, targets
                // the shape: the press and the drag act on it, not on the saved path (#13270).
                let tol = crate::path_components::press_tolerance(app);
                if app.session.active().is_some_and(|st| super::pick_shape_at(&st.doc, at, tol).is_some()) {
                    crate::path_components::untarget_saved_path_off(app, at, tol);
                }
                crate::path_components::press(app, at, shift);
                if let Some(owner) = VectorOwner::capture(app) {
                    state.path_selection.owned_by(owner);
                }
            }
            PathSelectionEvent::Release { start, end, owner } => {
                if owner.is_none_or(|o| o.valid(app)) {
                    super::path_selection_finish(app, start, end);
                }
            }
        }
    }
}

/// Gives back the open draft's last anchor (Edit › Undo while a draft is open). A resumed path's own anchors are not
/// the draft's to take back: undoing past the ones added since drawing resumed drops the draft and leaves the path as it
/// was (#6593). `false` with no draft.
pub(crate) fn remove_last(app: &mut PhotocraftApp) -> bool {
    if cancel_stale_pen(app) || !super::cancel_stale(app) {
        return false;
    }
    let Some(pen) = app.ui.pen.as_ref() else { return false };
    // A live shape layer's anchors are History steps (#12909): Undo takes back the last one the layer took. Anchors it
    // refused are the draft's alone, and the Pen takes those back first, one by one.
    if pen.live_layer && !pen::live_anchors_pending(app) {
        return pen::undo_live_step(app);
    }
    if pen.resumes.as_ref().is_some_and(|r| pen.draft.knots.len() <= r.was.knots.len()) {
        drop_draft(app);
        return true;
    }
    let (draft, kept) = (pen.draft.clone(), pen.kept);
    with_pen(app, |tool| {
        tool.kept(kept).replace(&draft);
        tool.remove_last()
    })
}

/// Replaces the open draft with `draft` (an edit of it with Direct Selection): the app's copy and the Pen's. A Curvature
/// Pen draft keeps following its anchors. `false` with no draft or while a press is down. Called between passes: a call
/// inside a run would reach the stand-in tools the run left in the app.
pub(crate) fn replace_draft(app: &mut PhotocraftApp, draft: &EditSubpath) -> bool {
    let Some(pen) = app.ui.pen.as_mut() else { return false };
    let mut next = draft.clone();
    if pen.curvature {
        next.reflow_kept(pen.kept);
    }
    pen.draft = next.clone();
    let kept = pen.kept;
    with_pen(app, |tool| tool.kept(kept).replace(&next))
}

/// Runs `f` on the Pen outside a pass and applies the events it raised.
fn with_pen<R>(app: &mut PhotocraftApp, f: impl FnOnce(&mut PenTool) -> R) -> R {
    let mut tools = std::mem::take(&mut app.canvas_tools);
    let result = f(&mut tools.vector.pen);
    let events = tools.vector.pen.take_events();
    app.canvas_tools = tools;
    apply_pen(app, events);
    result
}

/// The shape the Shape tool is drawing and its kind, while a drag has begun: the outline the app paints as a preview.
pub(crate) fn shape_preview(app: &PhotocraftApp) -> Option<(ShapeKind, &ShapeGeometry)> {
    let shapes = &app.canvas_tools.vector.shapes;
    shapes.shape().map(|geometry| (shapes.kind(), geometry))
}

/// Applies one pass's Pen events in order. The Pen reports an edit's `Commit` before the `Finished` that follows it.
fn apply_pen(app: &mut PhotocraftApp, events: Vec<PenEvent>) {
    let mut committed = false;
    for event in events {
        match event {
            PenEvent::Edit(edit) => committed |= edit_draft(app, edit),
            PenEvent::Finished(end) => finish(app, end, std::mem::take(&mut committed)),
            PenEvent::Resumed(key, knot) => resume(app, key, knot),
            _ => {}
        }
    }
}

/// The app's draft, created at the first edit with the destination captured then.
fn ensure_draft(app: &mut PhotocraftApp) -> Option<&mut PenPath> {
    if app.ui.pen.is_none() {
        let owner = capture_pen_owner(app)?;
        app.ui.pen_doc = app.session.active().map(|st| st.doc.id);
        app.ui.pen = Some(PenPath { owner: Some(owner), curvature: app.ui.tool == Tool::CurvaturePen, ..Default::default() });
    }
    app.ui.pen.as_mut()
}

/// Drops the draft; the path stays as it was.
pub(crate) fn drop_draft(app: &mut PhotocraftApp) {
    app.ui.pen = None;
    app.ui.pen_doc = None;
    if app.ui.direct_selection.target == Some(PathRef::Pen) {
        app.ui.direct_selection = Default::default();
    }
    release_draft(app);
}

/// Ends the Pen's open run once the app has finished or dropped the draft itself (a commit, a menu command, a stale
/// owner): the Pen keeps the draft it is drawing until its run ends, so the next press would otherwise carry on the
/// ended draft. Inside a pass the app holds stand-in tools and this does nothing; there the Pen reported the end
/// itself.
pub(crate) fn release_draft(app: &mut PhotocraftApp) {
    app.canvas_tools.vector.pen.draft(None);
}

/// Keeps the draft the Pen reports. Drawing on deselects an anchor Auto Add put on the draft, and a shorter draft loses
/// the selection of the removed anchors. Returns whether the draft was committed (the pen ended it with anchors).
fn edit_draft(app: &mut PhotocraftApp, edit: EditEvent<EditSubpath>) -> bool {
    match edit {
        EditEvent::Begin { .. } => {
            ensure_draft(app);
            false
        }
        EditEvent::Update { value, .. } => {
            set_draft(app, value);
            false
        }
        EditEvent::Commit { after, .. } => {
            // An empty commit removes the path a resumed draft held: nothing is left to commit.
            if after.knots.is_empty() {
                drop_draft(app);
                return false;
            }
            set_draft(app, after);
            true
        }
        EditEvent::Cancel { .. } => {
            drop_draft(app);
            false
        }
        _ => false,
    }
}

fn set_draft(app: &mut PhotocraftApp, value: EditSubpath) {
    let n = value.knots.len();
    let selection_on_draft = app.ui.direct_selection.target == Some(PathRef::Pen);
    // A refused draft whose document changed since takes no press, only Delete and Backspace: it stays refused, or
    // `cancel_stale_pen` would drop all of it on the next pass instead of the one anchor (#15488).
    let stale = app.ui.pen.as_ref().and_then(|p| p.owner.as_ref()).is_some_and(|o| !o.valid(app));
    let Some(pen) = ensure_draft(app) else { return };
    if pen.draft == value {
        return;
    }
    let grew = n > pen.draft.knots.len();
    let first = pen.draft.knots.is_empty();
    // Only the first anchor changing under a path of three or more is the press that closes it with a drag. The
    // Curvature Pen's close reflows every anchor's handles: the open draft is the one before the closing update.
    if n != pen.draft.knots.len() {
        pen.before_close = None;
    } else if pen.before_close.is_none() && n >= 3 {
        let first_dragged = value.knots[1..] == pen.draft.knots[1..] && value.knots[0] != pen.draft.knots[0];
        let closing_curve = pen.curvature && value.closed && !pen.draft.closed;
        if first_dragged || closing_curve {
            pen.before_close = Some(pen.draft.clone());
        }
    }
    pen.draft = value;
    pen.commit_failed &= stale;
    if selection_on_draft {
        let ds = &mut app.ui.direct_selection;
        if grew {
            ds.anchors.clear();
        } else {
            ds.anchors.retain(|a| a[1] < n);
            ds.segment = ds.segment.filter(|s| s[1] + 1 < n);
        }
    }
    // In Shape mode the first anchor makes the shape layer at once (#12909).
    if first {
        pen::start_live_shape(app);
    }
}

/// The Pen ended the draft. A draft whose commit failed waits for the user: Enter or Escape on it commits again.
fn finish(app: &mut PhotocraftApp, end: PenEnd, committed: bool) {
    let waiting = app.ui.pen.as_ref().is_some_and(|p| p.commit_failed);
    let has_draft = committed || waiting;
    match end {
        PenEnd::Submit if has_draft => submit(app),
        PenEnd::Escape if has_draft => end_keeping(app),
        PenEnd::Closed if has_draft => {
            pen::pen_commit(app, true);
        }
        // A refused draft the tool ends again on a tool switch stays as it is: it waits for an explicit retry.
        PenEnd::Interrupted if committed && !waiting => end_keeping(app),
        PenEnd::Removed | PenEnd::Discarded => drop_draft(app),
        _ => {}
    }
}

/// Choosing a tool that does not work on the Pen's path finishes it, as the Pen's own end does. The tool host cancels
/// only a gesture in progress, and an open path between presses is none, so the app ends it here once the Pen no
/// longer runs.
fn end_for_other_tool(app: &mut PhotocraftApp) {
    let Some(pen) = app.ui.pen.as_ref() else { return };
    let tool = crate::tool_memory::selected(app);
    let works_on_path = matches!(
        tool,
        Tool::Pen | Tool::CurvaturePen | Tool::FreeformPen | Tool::AddAnchorPoint | Tool::DeleteAnchorPoint | Tool::PathSelection | Tool::DirectSelection
    );
    if pen.draft.knots.is_empty() || works_on_path {
        return;
    }
    // A refused draft waits for an explicit retry, except a new shape's, which is kept as the Work Path.
    let shape = pen.owner.as_ref().is_some_and(|o| o.valid(app) && matches!(o.target, super::PenTarget::Shape { .. }));
    if pen.commit_failed && !shape {
        return;
    }
    end_keeping(app);
}

/// ↩ with a Pen path in progress: finish it as an open path. Photoshop then deselects the Work Path it went into: still
/// listed in the Paths panel, but not drawn, and the next path replaces it instead of joining it (#12892). Esc
/// ([`end_keeping`]) keeps it selected. The Curvature Pen, which shares the draft, keeps its Work Path selected.
fn submit(app: &mut PhotocraftApp) {
    let work_path = |app: &PhotocraftApp| app.session.active().and_then(|st| st.doc.work_path.clone());
    let curvature = app.ui.pen.as_ref().is_some_and(|p| p.curvature);
    let before = work_path(app);
    if pen::pen_commit(app, false) && !curvature && work_path(app) != before {
        pen::deselect_finished_work_path(app);
    }
}

/// End the Pen path for good (Esc, or switching to another tool): commit it, and when the document refuses that (a
/// locked shape layer or vector mask to add to…) keep it as the Work Path instead, so what was drawn is neither lost nor
/// retried on every frame. A draft even the Work Path refuses is dropped; the status bar says which happened.
fn end_keeping(app: &mut PhotocraftApp) {
    if pen::pen_commit(app, false) {
        return;
    }
    let active = app.session.active().map(|st| st.doc.id);
    if app.ui.pen_doc.is_some_and(|d| Some(d) != active) {
        // Drawn on another document that refused it (`cancel_stale`): left as it was.
        return;
    }
    let Some(pen) = app.ui.pen.clone() else { return };
    drop_draft(app);
    let draft = draft_before_finish(&pen);
    let refused = std::mem::take(&mut app.ui.status);
    let before = current_doc(app);
    let path = photocraft_engine::vector_cmds::path_json(&pen_path(&pen));
    let kept = app.run("path.set", json!({"name": "work", "op": "combine", "path": path})).is_ok();
    if kept {
        app.ui.selected_path = Some("work".into());
        // Undo gives the path back to the Pen here too (`pen_resume_on_undo`).
        remember_finish(app, draft, &before);
    }
    let outcome = if kept { tl!("The path was kept as the Work Path") } else { tl!("The path was discarded") };
    app.ui.status = format!("{refused} ({outcome})");
    app.ui.status_error = true;
}

/// A press on the end of an open path carries it on: its subpath becomes the draft, which the next pass hands to the Pen.
fn resume(app: &mut PhotocraftApp, key: u64, knot: KnotRef) {
    if app.ui.pen.as_ref().is_some_and(|p| !p.draft.knots.is_empty()) {
        return;
    }
    let Some(mut pen) = pen::resume_key(app, key, knot) else { return };
    let Some(owner) = capture_pen_owner(app) else { return };
    pen.owner = Some(owner);
    pen.curvature = app.ui.tool == Tool::CurvaturePen;
    app.ui.pen_doc = app.session.active().map(|st| st.doc.id);
    app.ui.pen = Some(pen);
}
