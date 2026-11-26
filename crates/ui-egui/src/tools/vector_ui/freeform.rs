//! The Freeform Pen on the tool host: the framework's outline draft traces the pointer, and the closed trace goes to
//! `freeform_finish_owned` after the pass.

use craft::behavior::geometry::{DraftEvent, DraftKind, DraftPoint, DraftReach, OutlineDraft};
use craft::behavior::kurbo::Point;
use craft::behavior::{CancelReason, Cursor, CursorBadge, CursorIcon, PaintedCursor, PointerButton};
use craft::imaging::ui::select::paint_outline;
use craft::imaging::ui::select::{EdgeTracer, MagneticSettings};
use craft::keymap::Mods;
use craft::ui::canvas::{CanvasTool, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft::ui::primitives::Draw;
use egui::Vec2;

use super::options::selected_shape;
use super::pen::pen_path_op;
use super::{PenOwner, PhotocraftApp};
use crate::direct_select::{PathRef, still_valid};
use crate::magnetic_lasso_ui::PcEdges;
use crate::state::Tool;

/// Whether a primary press may begin a trace this pass, decided by the host from the app's state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Admit {
    /// A press begins a trace.
    #[default]
    Open,
    /// The selected shape is locked: the press is refused with a status message.
    Locked,
    /// A fitted path whose commit failed is the user's draft: another trace neither retries nor replaces it.
    Busy,
}

/// An open Work Path subpath's last anchor, where a normal Freeform gesture can start.
#[derive(Clone, Copy)]
struct WorkPathEnd {
    subpath: usize,
    point: Point,
    selected: bool,
}

/// A finished trace: its points, the straight-segment indices, the pen owner and the vector owner.
type FinishedTrace = (Vec<[f64; 3]>, Vec<usize>, PenOwner, Option<super::VectorOwner>);

/// The Freeform Pen's trace between presses and its finished trace until the host drains it.
pub(crate) struct FreeformPenTool {
    /// This pass's admission rule for a press that begins a trace.
    admit: Admit,
    /// A press was refused as `Locked` since the host last asked.
    refused: bool,
    /// The trace being drawn: a drag draws freehand, ⌥ switches to straight segments.
    pub(crate) draft: OutlineDraft,
    /// The destination captured when the trace began.
    pub(crate) owner: Option<PenOwner>,
    /// The document, selected path and mode the trace began on: a release on another target commits nothing.
    pub(crate) target: Option<super::VectorOwner>,
    /// The finished trace (its points and the indices of those reached by a straight segment) until `drain` commits it.
    pub(crate) closed: Option<(Vec<Point>, Vec<usize>)>,
    /// The Work Path end points and their current selection in this pass.
    ends: Vec<WorkPathEnd>,
    /// The subpath the press started on, until the trace closes or cancels.
    continuation: Option<usize>,
    /// The modifiers of the last pass, to report changes to the draft.
    mods: Mods,
    /// Magnetic: the settings of this pass, from the options bar; `None` while Magnetic is off.
    magnetic: Option<MagneticSettings>,
    /// Magnetic: the document's edges, the same adapter the Magnetic Lasso drives.
    edges: PcEdges,
    /// Magnetic: the pointer's path since the trace began, the guide the border is searched along.
    guide: Vec<Point>,
}

impl Default for FreeformPenTool {
    fn default() -> Self {
        // The reach comes from the pass's slop in every pass (`each_pass`).
        Self {
            admit: Admit::Open,
            refused: false,
            // ⇧ leaves the freehand trace as drawn, as the reference's Freeform Pen does (#12832).
            draft: OutlineDraft::new(DraftKind::Mixed, DraftReach::new(0.0, 0.0)),
            owner: None,
            target: None,
            closed: None,
            ends: Vec::new(),
            continuation: None,
            mods: Mods::NONE,
            magnetic: None,
            edges: PcEdges::default(),
            guide: Vec::new(),
        }
    }
}

/// Refuse a Freeform stroke before drawing when its selected shape cannot be edited.
pub(crate) fn freeform_locked_shape(app: &PhotocraftApp) -> bool {
    if app.ui.tool_options.vector_mode != craft::draw::ui::VectorMode::Shape || pen_path_op(&app.ui.tool_options).is_none() {
        return false;
    }
    let Some(id) = selected_shape(&app.session) else { return false };
    app.session.active().is_some_and(|st| {
        let locks = st.doc.effective_locks(id);
        locks.all || locks.pixels
    })
}

impl FreeformPenTool {
    /// Sets whether a press that begins a trace is taken this pass. A press inside an open trace is always taken.
    pub(super) fn admission(&mut self, admit: Admit) {
        self.admit = admit;
    }

    /// Magnetic: this pass's mode and detection settings, and the edge snapshot they search. This is
    /// the host's per-pass call that carries the app ( [`work_path_ends`](Self::work_path_ends)), so
    /// it plays the part `magnetic_lasso_ui::prepare` plays for the Magnetic Lasso.
    fn magnetic_pass(&mut self, app: &PhotocraftApp) {
        let o = &app.ui.tool_options;
        self.magnetic = o.freeform_magnetic_on.then_some(o.freeform_magnetic);
        self.edges.sync(&app.session);
    }

    /// Magnetic: the border found between the trace's last point and the edge near the pointer, with
    /// one point placed every [`MagneticSettings::spacing`] along it — the Magnetic Lasso's own
    /// cadence for fastening points, and here the points the trace (and so the path's anchors) is
    /// built from. `end` also places the border's last point, which is how a release puts the final
    /// anchor on the edge under the pointer.
    fn follow_edges(&mut self, pointer: &ToolPointer, at: DraftPoint, end: bool) -> Option<DraftEvent> {
        let settings = self.magnetic?;
        let search = settings.search(stylus_pressure(pointer));
        let spacing = settings.spacing(at.zoom).max(1.0);
        let target = self.edges.snap(at.at, search).unwrap_or(at.at);
        let from = self.draft.points().last().copied().unwrap_or(at.at);
        self.guide.push(at.at);
        // `PcEdges` reads its own snapshot: the guide is handed over while it is searched.
        let mut border = {
            let guide = std::mem::take(&mut self.guide);
            let traced = self.edges.trace(from, target, &guide, search);
            self.guide = guide;
            traced
        };
        if border.len() < 2 {
            // No border found: the segment runs straight to the edge point, as the Lasso's does.
            border = vec![from, target];
        }
        let mut event = None;
        // The border is searched from the point the trace last placed, so the pace is measured along
        // this call's border alone: one fastening point every `spacing` of edge, as the Lasso counts it.
        let mut along = 0.0;
        for i in 1..border.len() {
            let (Some(before), Some(next)) = (border.get(i - 1), border.get(i)) else { continue };
            along += next.distance(*before);
            let last = i + 1 == border.len();
            if along < spacing && !(end && last) {
                continue;
            }
            let placed = *next;
            // The fastening point goes in as a freehand sample of the draft, so the trace, its
            // painting and its fit stay the machinery they are.
            if self.draft.drag(DraftPoint::new(placed, at.mods, at.zoom)).is_some() {
                event = Some(DraftEvent::Changed);
            }
            along = 0.0;
            // Edges are looked for along the pointer's path ahead of the new point, the same rule the
            // Magnetic Lasso applies to its trail: without it the search would run over the whole
            // path the pointer has already walked.
            let ahead = self.guide.iter().enumerate().min_by(|a, b| a.1.distance(placed).total_cmp(&b.1.distance(placed))).map_or(0, |(at, _)| at);
            let rest: Vec<Point> = self.guide.iter().skip(ahead + 1).copied().collect();
            self.guide = std::iter::once(placed).chain(rest).collect();
        }
        event
    }

    /// The measured case is a normal Freeform trace into the displayed Work Path.
    pub(super) fn work_path_ends(&mut self, app: &PhotocraftApp, document: usize) {
        self.magnetic_pass(app);
        self.ends.clear();
        if self.admit != Admit::Open
            || app.session.active_index() != Some(document)
            || self.draft.is_open()
            || app.ui.selected_path.as_deref() != Some("work")
            || app.ui.tool_options.vector_mode != craft::draw::ui::VectorMode::Path
            || super::targeted_vector_mask_id(app).is_some()
        {
            return;
        }
        let Some(path) = app.session.active().and_then(|st| st.doc.work_path.as_ref()) else { return };
        let selection = &app.ui.direct_selection;
        let work_selected = selection.target == Some(PathRef::Work) && still_valid(app, selection);
        self.ends.extend(path.subpaths.iter().enumerate().filter_map(|(subpath, sub)| {
            if sub.closed || sub.knots.len() < 2 {
                return None;
            }
            let anchor = sub.knots.last()?.anchor;
            let last = sub.knots.len() - 1;
            let selected = work_selected && (selection.anchors.contains(&[subpath, last]) || selection.segment == Some([subpath, last - 1]));
            Some(WorkPathEnd { subpath, point: Point::new(anchor.x, anchor.y), selected })
        }));
    }

    /// The closest endpoint within the Pen tool's usual screen-point reach.
    fn end_at(&self, pointer: &ToolPointer, cx: &ToolContext<'_>) -> Option<WorkPathEnd> {
        if pointer.mods.alt || pointer.mods.cmd || self.draft.is_open() {
            return None;
        }
        let view = cx.space()?.view;
        self.ends
            .iter()
            .map(|end| (view.to_screen(end.point).distance(pointer.pos), end))
            .filter(|(distance, _)| *distance <= cx.slop)
            .min_by(|(a, end_a), (b, end_b)| a.total_cmp(b).then(end_b.selected.cmp(&end_a.selected)))
            .map(|(_, end)| *end)
    }

    /// Whether a press was refused for a locked shape since the last call; the host words the refusal.
    pub(super) fn take_refused(&mut self) -> bool {
        std::mem::take(&mut self.refused)
    }

    /// Keeps what the draft reported: a closed trace waits for the host.
    fn apply(&mut self, event: Option<DraftEvent>) {
        if let Some(DraftEvent::Closed { points, straight, .. }) = event {
            self.closed = Some((points, straight));
            self.drop_trace();
        }
    }

    /// The finished trace as the points `freeform_finish_owned` takes, with its straight-segment indices and owner.
    pub(super) fn take_finished(&mut self) -> Option<FinishedTrace> {
        let (points, straight) = self.closed.take()?;
        let owner = self.owner.take()?;
        Some((points.iter().map(|p| [p.x, p.y, 1.0]).collect(), straight, owner, self.target.take()))
    }

    /// The opened path chosen at press time, consumed with the finished trace.
    pub(super) fn take_continuation(&mut self) -> Option<usize> {
        self.continuation.take()
    }

    /// Magnetic: the trace's walk along the edges is over, so the next trace starts its guide from
    /// nothing.
    fn drop_trace(&mut self) {
        self.guide.clear();
    }
}

/// The draft's sample for a pointer, in document units.
fn sample(pointer: &ToolPointer, cx: &ToolContext<'_>) -> Option<DraftPoint> {
    let view = cx.space()?.view;
    Some(DraftPoint::new(view.to_doc(pointer.pos), pointer.mods, view.zoom))
}

/// The stylus pressure that narrows the Magnetic Width, `None` for a mouse: the framework's Magnetic
/// Lasso reads a pen the same way, and its own copy of this test is private. A pen at full pressure
/// with no tilt, rotation or eraser end reads as a mouse, which keeps Width as the options bar sets it.
fn stylus_pressure(pointer: &ToolPointer) -> Option<f32> {
    let sample = pointer.sample;
    (sample.pressure != 1.0 || sample.tilt != [0.0; 2] || sample.rotation != 0.0 || sample.eraser).then_some(sample.pressure)
}

impl CanvasTool for FreeformPenTool {
    /// Takes the reach and passes a change of the held modifiers to the draft (releasing ⌥ with the button up closes).
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        self.draft.set_reach(DraftReach::new(f64::from(cx.slop), f64::from(cx.slop)));
        if cx.mods != self.mods {
            self.mods = cx.mods;
            let event = self.draft.mods_changed(cx.mods);
            self.apply(event);
        }
    }

    fn claims(&self) -> ToolClaims {
        if self.draft.takes_retract() { ToolClaims::NONE.keyboard().deletes() } else { ToolClaims::NONE }
    }

    /// The primary button always, the secondary between straight-segment clicks (it retracts the last vertex).
    fn takes(&self, button: PointerButton, _mods: Mods) -> bool {
        button == PointerButton::Primary || (button == PointerButton::Secondary && self.draft.takes_retract())
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        if pointer.button == PointerButton::Secondary {
            self.draft.retract();
            return ToolAction::Done;
        }
        if !self.draft.is_open() {
            match self.admit {
                Admit::Open => {}
                Admit::Locked => {
                    self.refused = true;
                    return ToolAction::Ignored;
                }
                Admit::Busy => return ToolAction::Ignored,
            }
        }
        let Some(at) = sample(pointer, cx) else { return ToolAction::Ignored };
        if !self.draft.is_open() {
            self.continuation = match self.end_at(pointer, cx) {
                Some(end) if end.selected => Some(end.subpath),
                Some(_) => return ToolAction::Done,
                None => None,
            };
        }
        // With nothing selected, ⌥ starts straight segments at once: the Freeform Pen never subtracts.
        let event = if self.draft.is_open() && pointer.clicks >= 2 { self.draft.close() } else { self.draft.press(at, false) };
        if matches!(event, Some(DraftEvent::Started { .. })) {
            // A Magnetic trace begins at the click, unsnapped, and follows the edges from there.
            self.guide = vec![at.at];
        }
        self.apply(event);
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if let Some(at) = sample(pointer, cx) {
            // With Magnetic on the samples come from the edges near the pointer, not from the pointer.
            let event = if self.magnetic.is_some() && self.draft.is_down() { self.follow_edges(pointer, at, false) } else { self.draft.drag(at) };
            self.apply(event);
        }
    }

    /// The release closes a freehand trace unless straight segments are on; a click draws nothing.
    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if let Some(at) = sample(pointer, cx) {
            if self.magnetic.is_some() && self.draft.is_down() {
                // `OutlineDraft::release` only moves the rubber band, so the border's end point goes in
                // first: the trace ends on the edge under the pointer.
                let edge = self.follow_edges(pointer, at, true);
                self.apply(edge);
            }
            let event = self.draft.release(at);
            self.apply(event);
        }
    }

    /// Between clicks the rubber band follows the pointer; it never adds a vertex.
    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(mut at) = sample(pointer, cx) else { return };
        if let Some(settings) = self.magnetic
            && !self.draft.is_down()
        {
            // Magnetic between clicks: the live end sits on the edge near the pointer, as the
            // Magnetic Lasso's live segment does. It still adds no vertex.
            at.at = self.edges.snap(at.at, settings.search(stylus_pressure(pointer))).unwrap_or(at.at);
        }
        if self.draft.hover(at).is_some() {
            cx.request_repaint();
        }
    }
    /// Escape during a drag, a lost pointer or `ToolHost::cancel` drops the trace without a path.
    fn cancel(&mut self, _reason: CancelReason, _cx: &mut ToolContext<'_>) {
        self.draft.cancel();
        self.owner = None;
        self.target = None;
        self.continuation = None;
        self.drop_trace();
    }

    /// Between clicks: Enter closes, Escape cancels, Backspace and Delete retract.
    fn key(&mut self, key: &ToolKey, _cx: &mut ToolContext<'_>) -> bool {
        if !self.draft.takes_retract() {
            return false;
        }
        match key {
            ToolKey::Submit => {
                let event = self.draft.close();
                self.apply(event);
            }
            ToolKey::Cancel => {
                self.draft.cancel();
                self.owner = None;
                self.target = None;
                self.continuation = None;
                self.drop_trace();
            }
            ToolKey::Delete { .. } => {
                self.draft.retract();
                if self.magnetic.is_some() {
                    // Magnetic: the next border is searched from the point the trace kept.
                    self.guide = self.draft.points().last().copied().into_iter().collect();
                }
            }
            _ => return false,
        }
        true
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        paint_outline(&self.draft, draw, cx);
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        if cx.pointer.as_ref().and_then(|pointer| self.end_at(pointer, cx)).is_some_and(|end| end.selected) {
            // The nib is at the lower left of the same 24-point icon box the Pen uses.
            return Cursor::Painted(PaintedCursor::tool(crate::icons::tool_icon(Tool::FreeformPen), Vec2::Y * 24.0, Some(CursorBadge::Continue)));
        }
        Cursor::System(CursorIcon::Default)
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the Path commands take the same trace as points")
    }
}

#[cfg(test)]
#[path = "freeform_pen_tests.rs"]
mod freeform_pen_tests;

#[cfg(test)]
#[path = "freeform_continue_endpoint_tests.rs"]
mod freeform_continue_endpoint_tests;

#[cfg(test)]
#[path = "freeform_magnetic_tests.rs"]
mod freeform_magnetic_tests;
