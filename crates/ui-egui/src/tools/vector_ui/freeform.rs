//! The Freeform Pen on the tool host: the framework's outline draft traces the pointer, and the closed trace goes to
//! `freeform_finish_owned` after the pass.

use craft::behavior::geometry::{DraftEvent, DraftKind, DraftPoint, DraftReach, OutlineDraft};
use craft::behavior::kurbo::Point;
use craft::behavior::{CancelReason, PointerButton};
use craft::imaging::ui::select::paint_outline;
use craft::keymap::Mods;
use craft::ui::canvas::{CanvasTool, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft::ui::primitives::Draw;

use super::options::selected_shape;
use super::pen::pen_path_op;
use super::{PenOwner, PhotocraftApp};

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
    /// The modifiers of the last pass, to report changes to the draft.
    mods: Mods,
}

impl Default for FreeformPenTool {
    fn default() -> Self {
        // The reach comes from the pass's slop in every pass (`each_pass`).
        Self {
            admit: Admit::Open,
            refused: false,
            draft: OutlineDraft::new(DraftKind::Mixed, DraftReach::new(0.0, 0.0)).locking_shift_drags(),
            owner: None,
            target: None,
            closed: None,
            mods: Mods::NONE,
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

    /// Whether a press was refused for a locked shape since the last call; the host words the refusal.
    pub(super) fn take_refused(&mut self) -> bool {
        std::mem::take(&mut self.refused)
    }

    /// Keeps what the draft reported: a closed trace waits for the host.
    fn apply(&mut self, event: Option<DraftEvent>) {
        if let Some(DraftEvent::Closed { points, straight, .. }) = event {
            self.closed = Some((points, straight));
        }
    }

    /// The finished trace as the points `freeform_finish_owned` takes, with its straight-segment indices and owner.
    pub(super) fn take_finished(&mut self) -> Option<FinishedTrace> {
        let (points, straight) = self.closed.take()?;
        let owner = self.owner.take()?;
        Some((points.iter().map(|p| [p.x, p.y, 1.0]).collect(), straight, owner, self.target.take()))
    }
}

/// The draft's sample for a pointer, in document units.
fn sample(pointer: &ToolPointer, cx: &ToolContext<'_>) -> Option<DraftPoint> {
    let view = cx.space()?.view;
    Some(DraftPoint::new(view.to_doc(pointer.pos), pointer.mods, view.zoom))
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
        // With nothing selected, ⌥ starts straight segments at once: the Freeform Pen never subtracts.
        let event = if self.draft.is_open() && pointer.clicks >= 2 { self.draft.close() } else { self.draft.press(at, false) };
        self.apply(event);
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if let Some(at) = sample(pointer, cx) {
            let event = self.draft.drag(at);
            self.apply(event);
        }
    }

    /// The release closes a freehand trace unless straight segments are on; a click draws nothing.
    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if let Some(at) = sample(pointer, cx) {
            let event = self.draft.release(at);
            self.apply(event);
        }
    }

    /// Between clicks the rubber band follows the pointer; it never adds a vertex.
    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if let Some(at) = sample(pointer, cx)
            && self.draft.hover(at).is_some()
        {
            cx.request_repaint();
        }
    }

    /// Escape during a drag, a lost pointer or `ToolHost::cancel` drops the trace without a path.
    fn cancel(&mut self, _reason: CancelReason, _cx: &mut ToolContext<'_>) {
        self.draft.cancel();
        self.owner = None;
        self.target = None;
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
            }
            ToolKey::Delete { .. } => {
                self.draft.retract();
            }
            _ => return false,
        }
        true
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        paint_outline(&self.draft, draw, cx);
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the Path commands take the same trace as points")
    }
}

#[cfg(test)]
#[path = "freeform_pen_tests.rs"]
mod freeform_pen_tests;
