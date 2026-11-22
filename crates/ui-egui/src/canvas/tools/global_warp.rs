//! The Global Perspective Warp family: the canvas tool that runs while the session is open on the active document.
//!
//! The session (`global_perspective_ui`) holds the guides and the drag; this file only maps the host's pointer and key
//! calls onto its functions in document units. The tool takes the primary button whichever tool is selected, so a Brush
//! never paints under the guides, and the host keeps the grab until the release.

use craft::behavior::{CancelReason, Cursor, CursorIcon, PaintedCursor, PointerButton};
use craft::keymap::Mods;
use craft::ui::canvas::{CanvasTool, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft::ui::primitives::Draw;

use super::{FamilyTool, ToolScope};
use crate::global_perspective_ui::{self as session, HIT_POINTS, Hover, Keys};

/// The tool for one pass.
struct GlobalWarpTool<'a> {
    scope: &'a ToolScope<'a>,
    idx: usize,
    output: Option<u32>,
}

/// The global warp's tool for the canvas pass.
pub(crate) fn tool<'a>(scope: &'a ToolScope<'a>, idx: usize, output: Option<u32>) -> FamilyTool<'a> {
    Box::new(GlobalWarpTool { scope, idx, output })
}

impl GlobalWarpTool<'_> {
    /// A hidden/colour-incompatible canvas never changes guides behind an invisible overlay.
    /// Use the same canvas output and admission condition as its rendered guide mesh.
    fn preview_visible(&self) -> bool {
        self.scope.with_app(|app| session::preview_visible(app, self.idx, self.output))
    }
}

/// Ctrl, or ⌘ off the Mac where the host reports Ctrl as ⌘, locks to the image axes; Shift to the guide's own.
fn keys_of(mods: Mods) -> Keys {
    Keys { ctrl: mods.cmd || mods.ctrl, shift: mods.shift }
}

/// A hidden overlay must not delete an invisible guide or pass Delete to the layer tool.
fn guarded_delete(overlay_visible: bool, remove: impl FnOnce() -> bool) -> bool {
    if !overlay_visible {
        return true;
    }
    remove()
}

/// The pointer's document point and the hit distance in document units.
fn at(pointer: &ToolPointer, cx: &ToolContext<'_>) -> Option<([f64; 2], f64)> {
    let view = cx.space()?.view;
    let p = view.to_doc(pointer.pos);
    Some(([p.x, p.y], view.doc_len(HIT_POINTS as f32)))
}

impl CanvasTool for GlobalWarpTool<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        // Consume the press without handing it to a paint/transform tool when the
        // current monitor cannot draw the guide overlay.
        if !self.preview_visible() {
            return ToolAction::Done;
        }
        let Some((p, tol)) = at(pointer, cx) else { return ToolAction::Ignored };
        let began = self.scope.with_app(|app| session::pointer_down(app, p, tol, keys_of(pointer.mods)));
        // A press that starts no drag (a click on a guide, on the result, past the photo) is still the session's.
        if began { ToolAction::Claimed } else { ToolAction::Done }
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if !self.preview_visible() {
            self.scope.with_app(session::pointer_cancel);
            return;
        }
        if let Some((p, _)) = at(pointer, cx) {
            self.scope.with_app(|app| session::pointer_move(app, p, keys_of(pointer.mods)));
        }
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if !self.preview_visible() {
            self.scope.with_app(session::pointer_cancel);
            return;
        }
        match at(pointer, cx) {
            Some((p, _)) => self.scope.with_app(|app| session::pointer_up(app, p, keys_of(pointer.mods))),
            None => self.scope.with_app(session::pointer_cancel),
        }
    }

    /// Escape during a drag, a lost pointer, a navigation gesture or `ToolHost::cancel`: the guides keep the last accepted
    /// coordinates and the grab is released.
    fn cancel(&mut self, _reason: CancelReason, _cx: &mut ToolContext<'_>) {
        self.scope.with_app(session::pointer_cancel);
    }

    /// The primary button only: the secondary and middle buttons stay the canvas's (menu, pan).
    fn takes(&self, button: PointerButton, _mods: Mods) -> bool {
        button == PointerButton::Primary
    }

    /// A guide follows every move from the press, so a move of a pixel or two moves it.
    fn takes_every_move(&self) -> bool {
        self.scope.with_app(|app| session::dragging(app))
    }

    fn view_only(&self) -> bool {
        false
    }

    /// Enter commits, Escape cancels and Delete removes the selected guide, as the options bar's buttons do.
    fn claims(&self) -> ToolClaims {
        ToolClaims::NONE.keyboard().deletes()
    }

    fn key(&mut self, key: &ToolKey, _cx: &mut ToolContext<'_>) -> bool {
        match key {
            ToolKey::Submit => {
                self.scope.with_app(|app| {
                    if let Err(error) = session::commit(app) {
                        app.ui.status = error;
                        app.ui.status_error = true;
                    }
                });
                true
            }
            ToolKey::Cancel => {
                self.scope.with_app(session::cancel);
                true
            }
            ToolKey::Delete { .. } => guarded_delete(self.preview_visible(), || self.scope.with_app(session::delete_selected)),
            // An arrow key would move the layer under the guides and make the session stale.
            ToolKey::Nudge { .. } => true,
            _ => false,
        }
    }

    fn paint(&self, _draw: &Draw<'_>, _cx: &ToolContext<'_>) {}

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        if !self.preview_visible() {
            return Cursor::System(CursorIcon::Default);
        }
        let over = cx.pointer.as_ref().and_then(|pointer| at(pointer, cx));
        let hover = over.map_or(Hover::Idle, |(p, tol)| self.scope.with_app(|app| session::hover(app, p, tol)));
        match hover {
            Hover::Idle => Cursor::System(CursorIcon::Default),
            Hover::Grabbing | Hover::Handle => Cursor::System(CursorIcon::Move),
            Hover::Draw => Cursor::Painted(PaintedCursor::Crosshair),
        }
    }

    fn autoscrolls(&self) -> bool {
        false
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the edit.perspectiveWarpGlobal command")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hidden_overlay_consumes_delete_without_mutating_guides() {
        use std::cell::Cell;
        let calls = Cell::new(0);
        assert!(guarded_delete(false, || {
            calls.set(calls.get() + 1);
            true
        }));
        assert_eq!(calls.get(), 0, "invisible guide was deleted");
        assert!(guarded_delete(true, || {
            calls.set(calls.get() + 1);
            true
        }));
        assert_eq!(calls.get(), 1, "visible guide was not deleted");
        assert!(!guarded_delete(true, || false), "preserve the old no-selection result");
    }

    /// Ctrl (⌘ where the host reports Ctrl as ⌘) is the image-axis lock; Shift alone is the guide-axis lock.
    #[test]
    fn ctrl_or_command_lock_the_image_axes_and_shift_is_separate() {
        assert_eq!(keys_of(Mods::NONE), Keys::default());
        assert_eq!(keys_of(Mods { cmd: true, ..Mods::NONE }), Keys { ctrl: true, shift: false });
        assert_eq!(keys_of(Mods { ctrl: true, ..Mods::NONE }), Keys { ctrl: true, shift: false });
        assert_eq!(keys_of(Mods { shift: true, ..Mods::NONE }), Keys { ctrl: false, shift: true });
        assert_eq!(keys_of(Mods { cmd: true, shift: true, ..Mods::NONE }), Keys { ctrl: true, shift: true });
    }
}
