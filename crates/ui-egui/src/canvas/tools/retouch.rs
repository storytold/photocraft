//! The retouch family: Patch and Content-Aware Move (a selection drag or a lasso outline), Red Eye, Object
//! Selection, and the Alt-click source of Clone Stamp and Healing (their strokes are the paint family's).

use std::cell::RefCell;

use craft::behavior::PointerButton;
use craft::behavior::cursor::{Cursor, CursorIcon, PaintedCursor};
use craft::behavior::geometry::SelectionOperation;
use craft::behavior::kurbo::Point;
use craft::behavior::value::CancelReason;
use craft::imaging::ui::select::{DragKind, LassoTool, PressRule, SelectEvent, SelectionProbe, SelectionShape, drag_kind};
use craft::keymap::Mods;
use craft::ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolHostResponse, ToolPath, ToolPointer};
use craft::ui::primitives::Draw;

use super::dialog::doc_point;
use super::selection::{InsideSelectionDrag, PcSelection, drag_delta};
use super::{FamilyTool, ToolScope};
use crate::PhotocraftApp;
use crate::canvas::document_display::CanvasHost;
use crate::canvas::tool_feedback;
use crate::retouch_ui::{self, PatchPress};
use crate::state::Tool;

/// The retouch gestures in progress.
#[derive(Default)]
pub(crate) struct RetouchTools {
    /// The press of a Patch or Content-Aware Move selection drag, or of an Object Selection rectangle.
    press: Option<PatchPress>,
    /// The last drag point in document units.
    last: [f64; 2],
    /// Shift at the last drag sample.
    shift: bool,
    /// The outline a Patch or Content-Aware Move press outside the selection draws.
    lasso: LassoTool,
    /// The outline drag of an Object Selection press inside the selection (see `RetouchTool::drags_outline`).
    drag: InsideSelectionDrag,
}

impl RetouchTools {
    /// A lasso outline that makes a new selection is being drawn: the selection it replaces is hidden from the press
    /// on (#11024). Adding, subtracting and intersecting keep it visible.
    pub(crate) fn replaces_selection(&self) -> bool {
        self.lasso.replaces_selection()
    }

    /// The press of the selection drag in progress, for `cam_preview::wanted` and
    /// `patch_preview::drag_offset`.
    pub(crate) fn press(&self) -> Option<&PatchPress> {
        self.press.as_ref()
    }

    /// The last drag point in document units.
    pub(crate) fn last_point(&self) -> [f64; 2] {
        self.last
    }

    /// Whether Shift was held at the last drag sample.
    pub(crate) fn shift(&self) -> bool {
        self.shift
    }

    /// The whole-pixel offset of Object Selection's outline drag in progress, for `selection::shown_offset`.
    pub(crate) fn drag_offset(&self) -> Option<(i32, i32)> {
        drag_delta(self.drag.phase())
    }
}

/// The retouch tool for the selected tool (Patch, Content-Aware Move, Red Eye or Object Selection).
pub(crate) fn tool<'a>(scope: &'a ToolScope<'a>, state: &'a mut RetouchTools, _host: CanvasHost) -> FamilyTool<'a> {
    let (tool, probe, bar, finder) = scope.with_app(|app| {
        let tool = app.ui.tool;
        let bar = tool_feedback::selection_op(tool, app.ui.selection_mode, Mods::NONE, true);
        // Indexed Color and Bitmap refuse Object Selection: the press must reach `finish_object_selection` for the
        // tool's alert (#12867), never deselect or move the outline.
        let refused = app.session.active().is_some_and(|st| photocraft_engine::smartselect_cmds::object_mode_refusal(st.doc.mode).is_some());
        (tool, PcSelection::of(app), bar, app.ui.tool_options.object_finder && !refused)
    });
    state.drag.operation(bar);
    Box::new(RetouchTool { scope, state: RefCell::new(state), tool, probe, bar, finder })
}

/// A Red Eye click. CMYK, Grayscale and Indexed Color documents get the reference's alert (#12575); the alert host
/// shows one alert per message, so more clicks while it is open add nothing.
fn red_eye_click(app: &mut PhotocraftApp, at: [f64; 2]) {
    use photocraft_engine::redeye_cmds::MODE_REFUSAL;
    let o = app.ui.tool_options.clone();
    let result = app.run(
        "paint.redEye",
        serde_json::json!({
            "x": at[0].floor(),
            "y": at[1].floor(),
            "pupilSize": o.red_eye_pupil_size.clamp(1.0, 100.0),
            "darken": o.red_eye_darken.clamp(0.0, 100.0),
            "target": crate::canvas::strokes::paint_target(app),
        }),
    );
    if result.is_err_and(|e| e == MODE_REFUSAL) {
        app.ui.status.clear();
        app.ui.status_error = false;
        let _ = crate::dialogs::alerts::alert(app, tl!(MODE_REFUSAL).to_string());
    }
}

/// The Alt-click source of Clone Stamp and Healing: the stand-in `tools::run` installs for `StandInKey::Alt`.
pub(crate) fn source_tool<'a>(scope: &'a ToolScope<'a>, _state: &'a mut RetouchTools, _host: CanvasHost) -> FamilyTool<'a> {
    Box::new(SourceTool { scope })
}

/// Applies what the lasso outline and Object Selection's outline drag reported. A Patch or Content-Aware Move press
/// that dragged the selection applied itself on release.
pub(crate) fn drain(app: &mut PhotocraftApp, state: &mut RetouchTools, _response: &ToolHostResponse) {
    state.drag.drain(app, Tool::ObjectSelection);
    for event in state.lasso.take_events() {
        // A press inside the selection is routed to the selection drag, so the lasso's own inside drag and
        // deselect events do not arise here.
        let tool = app.ui.tool;
        // A click, or an outline of fewer than three points, reaches the app as the lasso's deselect: it selects no
        // pixel, which the Content-Aware Move tool answers with its alert.
        if matches!(event, SelectEvent::Deselect) && tool == Tool::ContentAwareMove {
            retouch_ui::content_aware_move_no_pixels(app, &[]);
            continue;
        }
        let SelectEvent::Commit { shape, operation, .. } = event else { continue };
        if tool == Tool::ContentAwareMove
            && let SelectionShape::Polygon(points) = &shape
        {
            let points: Vec<[f64; 2]> = points.iter().map(|p| [p.x, p.y]).collect();
            if retouch_ui::content_aware_move_no_pixels(app, &points) {
                continue;
            }
        }
        super::selection::run_commit(app, tool, shape, operation);
        if tool == Tool::Patch {
            retouch_ui::patch_hint(app);
        }
    }
}

struct RetouchTool<'a> {
    scope: &'a ToolScope<'a>,
    /// In a `RefCell` because the lasso paints through its pass, which needs `&mut` while `paint` takes `&self`.
    state: RefCell<&'a mut RetouchTools>,
    tool: Tool,
    probe: PcSelection,
    /// The options bar's operation.
    bar: SelectionOperation,
    /// Object Selection can drag the outline: Object Finder is on and the document's mode allows the tool.
    finder: bool,
}

impl RetouchTool<'_> {
    /// Whether a press outside the selection draws the outline (Patch and Content-Aware Move).
    fn lassos(&self) -> bool {
        matches!(self.tool, Tool::Patch | Tool::ContentAwareMove)
    }

    /// Object Selection with Object Finder on drags the outline like a marquee (#12853): a press at `at` inside the
    /// selection whose operation resolves to New deselects on a click and moves the outline on a drag. Any other press
    /// is a click or a rectangle: ⇧ and ⌥ combine, ⌘ is the Move stand-in's (its pixel drag is not taken here), and a
    /// floating piece keeps the rectangle.
    fn drags_outline(&self, at: [f64; 2], mods: Mods) -> bool {
        self.tool == Tool::ObjectSelection
            && self.finder
            && !self.probe.floating()
            && drag_kind(&self.probe, Point::new(at[0], at[1]), mods, self.bar, PressRule::Drags) == Some(DragKind::Outline)
    }
}

impl CanvasTool for RetouchTool<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let at = doc_point(pointer, cx);
        let (tool, mods) = (self.tool, pointer.mods);
        let mut state = self.state.borrow_mut();
        if tool == Tool::RedEye {
            self.scope.with_app(|app| red_eye_click(app, at));
            return ToolAction::Done;
        }
        if self.drags_outline(at, mods) {
            return state.drag.press(&self.probe, pointer, cx);
        }
        // ⌘ with Object Selection is the Move stand-in's: answered `Ignored`, the press goes to it and drags the
        // selected pixels.
        if tool == Tool::ObjectSelection && mods.cmd {
            return ToolAction::Ignored;
        }
        // A press of another document never reaches here: the canvas pass ends it (`end_foreign_gesture`), which
        // cancels this tool and drops its press.
        let drags = self.scope.with_app(|app| tool == Tool::ObjectSelection || retouch_ui::patch_drags_selection(app, at, mods));
        if drags {
            state.press = Some(PatchPress::new(tool, at, mods));
            state.last = at;
            state.shift = mods.shift;
            return ToolAction::Claimed;
        }
        if self.lassos() {
            return state.lasso.pass(&self.probe).press(pointer, cx);
        }
        ToolAction::Ignored
    }

    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        self.state.borrow_mut().drag.each_pass(&self.probe, cx);
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let mut state = self.state.borrow_mut();
        if state.drag.active() {
            state.drag.drag(&self.probe, pointer, cx);
        } else if state.press.is_some() {
            state.last = doc_point(pointer, cx);
            state.shift = pointer.mods.shift;
        } else if self.lassos() {
            state.lasso.pass(&self.probe).drag(pointer, cx);
        }
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let mut state = self.state.borrow_mut();
        if state.drag.active() {
            state.drag.release(&self.probe, pointer, cx);
            return;
        }
        let Some(press) = state.press.take() else {
            if self.lassos() {
                state.lasso.pass(&self.probe).release(pointer, cx);
            }
            return;
        };
        let end = doc_point(pointer, cx);
        let (tool, mods) = (self.tool, pointer.mods);
        let start = press.start;
        state.last = end;
        state.shift = mods.shift;
        self.scope.with_app(|app| match tool {
            // The rectangle combines as its press decided, as the cursor's badge showed.
            Tool::ObjectSelection => retouch_ui::finish_object_selection(app, start, end, press.modifiers),
            // A click with no drag moves nothing.
            _ if end == start => {}
            Tool::Patch => {
                // PLAN: assumed `patch_end(&PatchPress, end, shift) -> [f64; 2]` (tools-w54).
                let end = retouch_ui::patch_end(&press, end, mods.shift);
                retouch_ui::finish_patch(app, start, end);
            }
            Tool::ContentAwareMove => retouch_ui::finish_content_aware_move(app, start, end),
            _ => {}
        });
    }

    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if self.lassos() {
            self.state.borrow_mut().lasso.pass(&self.probe).hover(pointer, cx);
        }
    }

    fn cancel(&mut self, reason: CancelReason, cx: &mut ToolContext<'_>) {
        let mut state = self.state.borrow_mut();
        state.press = None;
        state.drag.cancel(&self.probe, reason, cx);
        if self.lassos() {
            state.lasso.pass(&self.probe).cancel(reason, cx);
        }
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        if self.lassos() {
            self.state.borrow_mut().lasso.pass(&self.probe).paint(draw, cx);
        }
    }

    /// The crosshair with the badge of what the gesture does: Object Selection's operation
    /// (`tool_feedback::selection_op`), and for Patch and Content-Aware Move the patch drag or the
    /// outline's operation (`tool_feedback::patch_badge`). A gesture keeps what its press decided. Where an Object
    /// Selection press would drag the outline, and while it does, the marquee's move cursor.
    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        {
            let mut state = self.state.borrow_mut();
            let over = state.press.is_none() && cx.pointer.as_ref().is_some_and(|pointer| self.drags_outline(doc_point(pointer, cx), cx.mods));
            if state.drag.active() || over {
                return state.drag.cursor(&self.probe, cx);
            }
        }
        let state = self.state.borrow();
        let press = state.press.as_ref().map(|p| (p.start, p.modifiers));
        let at = cx.pointer.as_ref().map(|pointer| doc_point(pointer, cx));
        let badge = self.scope.with_app(|app| match self.tool {
            Tool::ObjectSelection => {
                let mods = press.map_or(cx.mods, |(_, mods)| mods);
                tool_feedback::selection_op(Tool::ObjectSelection, app.ui.selection_mode, mods, tool_feedback::has_selection(app)).badge()
            }
            Tool::Patch | Tool::ContentAwareMove => {
                state.lasso.outline_operation().map_or_else(|| at.and_then(|at| tool_feedback::patch_badge(app, at, press, cx.mods)), |op| op.badge())
            }
            _ => None,
        });
        badge.map_or(Cursor::Painted(PaintedCursor::Crosshair), |badge| PaintedCursor::badged(CursorIcon::Crosshair, badge).into())
    }

    /// Object Selection's outline drag takes every move from the press, as the marquee's own does; the Patch and
    /// Content-Aware Move lasso takes them as the lasso says.
    fn takes_every_move(&self) -> bool {
        let mut state = self.state.borrow_mut();
        state.drag.active() || (self.lassos() && state.lasso.pass(&self.probe).takes_every_move())
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("a retouch gesture is drawn with the pointer")
    }
}

struct SourceTool<'a> {
    scope: &'a ToolScope<'a>,
}

impl CanvasTool for SourceTool<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let at = doc_point(pointer, cx);
        self.scope.with_app(|app| retouch_ui::set_source(app, at[0], at[1]));
        ToolAction::Done
    }

    fn drag(&mut self, _pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {}

    fn release(&mut self, _pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _reason: CancelReason, _cx: &mut ToolContext<'_>) {}

    fn takes(&self, button: PointerButton, mods: Mods) -> bool {
        // Control+Alt is the brush-resize drag, which the painting tool takes (`cursor_policy::alt_samples`).
        button == PointerButton::Primary && mods.alt && !mods.ctrl
    }

    fn paint(&self, _draw: &Draw<'_>, _cx: &ToolContext<'_>) {}

    /// The painting tool's brush cursor with ⌥ held: the tip with its centre mark, or the Healing
    /// Brush's sample target (`cursor_policy::source_target`).
    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        self.scope.with_app(|app| crate::brush_cursor::cursor(app, cx))
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the source is picked with the pointer")
    }
}
