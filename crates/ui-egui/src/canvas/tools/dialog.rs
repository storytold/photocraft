//! The dialog family: the canvas tool that runs while a hosted dialog arms a pick on the canvas (an eyedropper,
//! a Curves point, a shadow or gradient drag, the filter locator, the Pattern Fill phase drag).
//!
//! Which pick applies, and what each does, is `dialogs::canvas_input`'s; this file only maps the host's pointer
//! calls onto those functions in document units.

use craft::behavior::cursor::{Cursor, PaintedCursor};
use craft::behavior::value::CancelReason;
use craft::keymap::Mods;
use craft::ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolPath, ToolPointer};
use craft::ui::primitives::Draw;

use super::{FamilyTool, ToolScope};
use crate::canvas::document_display::CanvasHost;
use crate::dialogs::canvas_input::{self, DialogPick};
use crate::pattern_fill_ui::{self, PhaseDrag};

/// The gesture in progress under an armed pick. Nothing else is kept between passes: the pick itself is read
/// from `canvas_input::armed` each pass.
#[derive(Default)]
pub(crate) struct DialogTools {
    /// The press point in document units.
    press: Option<[f64; 2]>,
    /// The pick the press began under.
    pick: Option<DialogPick>,
    /// The modifiers held at the press.
    mods: Mods,
    /// The Pattern Fill drag, from its press to its release.
    phase: Option<PhaseDrag>,
}

/// The pointer's document point: the one mapping from screen points to document units for the canvas tools of
/// this area.
// PLAN: assumed `CanvasView::to_doc(Pos2) -> kurbo::Point` as listed in the framework definitions.
pub(super) fn doc_point(pointer: &ToolPointer, cx: &ToolContext<'_>) -> [f64; 2] {
    cx.space().map_or([0.0, 0.0], |space| {
        let p = space.view.to_doc(pointer.pos);
        [p.x, p.y]
    })
}

/// The dialog tool for the canvas `host`, when a hosted dialog arms a pick and `host` shows the active document
/// (the dialog edits the active document). `None` leaves the selected tool in charge.
pub(crate) fn dialog_tool<'a>(scope: &'a ToolScope<'a>, state: &'a mut DialogTools, host: CanvasHost) -> Option<FamilyTool<'a>> {
    let pick = scope.with_app(|app| (app.session.active_index() == Some(host.idx)).then(|| canvas_input::armed(app)).flatten())?;
    Some(Box::new(DialogTool { scope, state, pick }))
}

/// Ends the gesture of a dialog's pick when a pass of its press runs without that pick: a dialog opened above the one
/// that armed it, or the pick disarmed or changed. Nothing of the gesture resumes when the pick comes back; what it
/// applied stays (production stops the Pattern Fill drag on a nested dialog).
pub(crate) fn end_if_disarmed(app: &crate::PhotocraftApp, state: &mut DialogTools, host: CanvasHost) {
    let armed = (app.session.active_index() == Some(host.idx)).then(|| canvas_input::armed(app)).flatten();
    if state.pick.is_some() && armed != state.pick {
        *state = DialogTools::default();
    }
}

/// Whether the pick samples a colour from the image, so the precise cursor (Caps Lock) shows a crosshair.
fn samples(pick: DialogPick) -> bool {
    matches!(pick, DialogPick::TonePicker(_) | DialogPick::Color | DialogPick::ParamDropper | DialogPick::ColorRange)
}

struct DialogTool<'a> {
    scope: &'a ToolScope<'a>,
    state: &'a mut DialogTools,
    pick: DialogPick,
}

impl CanvasTool for DialogTool<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let at = doc_point(pointer, cx);
        let (pick, mods) = (self.pick, pointer.mods);
        let state = &mut *self.state;
        // `(took the press, the press is the whole gesture)`
        let (took, click) = self.scope.with_app(|app| {
            let follows = canvas_input::follows_drag(app, pick);
            match pick {
                DialogPick::TonePicker(_) => (canvas_input::pick_tone(app, at), !follows),
                DialogPick::CurvesPoint => (canvas_input::add_curve_point(app, at, mods.shift), true),
                DialogPick::Color => (canvas_input::pick_color(app, at), !follows),
                DialogPick::ParamDropper => (canvas_input::pick_dropper(app, at, mods), !follows),
                DialogPick::ColorRange => (canvas_input::pick_color_range(app, at, mods), !follows),
                // A drag after the locator's press does nothing (#9648).
                DialogPick::Locator => (canvas_input::locate(app, at), true),
                DialogPick::ShadowDrag => (canvas_input::drag_shadow(app, at, [0.0, 0.0]), false),
                DialogPick::GradientDrag => (canvas_input::drag_gradient(app, at, [0.0, 0.0]), false),
                DialogPick::PatternPhase => {
                    state.phase = pattern_fill_ui::press(app, at);
                    (state.phase.is_some(), false)
                }
            }
        });
        if !took {
            self.state.press = None;
            self.state.pick = None;
            return ToolAction::Ignored;
        }
        if click {
            self.scope.with_app(|app| canvas_input::release(app, pick));
            self.state.press = None;
            self.state.pick = None;
            return ToolAction::Done;
        }
        self.state.press = Some(at);
        self.state.pick = Some(pick);
        self.state.mods = mods;
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(start) = self.state.press else { return };
        let at = doc_point(pointer, cx);
        let total = [at[0] - start[0], at[1] - start[1]];
        let (pick, mods) = (self.pick, self.state.mods);
        let state = &mut *self.state;
        self.scope.with_app(|app| match pick {
            DialogPick::ShadowDrag => {
                canvas_input::drag_shadow(app, start, total);
            }
            DialogPick::GradientDrag => {
                canvas_input::drag_gradient(app, start, total);
            }
            DialogPick::PatternPhase => {
                // `false`: a pick change, an unlink or an outside phase change ended the gesture.
                if let Some(mut drag) = state.phase.take()
                    && pattern_fill_ui::drag(app, &mut drag, at)
                {
                    state.phase = Some(drag);
                }
            }
            // The pipettes keep sampling while the button is held.
            _ => canvas_input::resample(app, pick, at, mods),
        });
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let at = doc_point(pointer, cx);
        let pick = self.pick;
        let state = &mut *self.state;
        // The release's own position counts, so a drag ending past the last move still lands there.
        self.scope.with_app(|app| {
            if let Some(drag) = state.phase.take() {
                pattern_fill_ui::release(app, drag, at);
            }
            canvas_input::release(app, pick);
        });
        self.state.press = None;
        self.state.pick = None;
    }

    fn cancel(&mut self, _reason: CancelReason, _cx: &mut ToolContext<'_>) {
        *self.state = DialogTools::default();
    }

    fn view_only(&self) -> bool {
        false
    }

    /// The drags that move content (the pattern's phase, a shadow's offset, a gradient's offset) follow every move from
    /// the press, so a move of a pixel or two moves it, as production's dialog drags; none of them reads a click.
    fn takes_every_move(&self) -> bool {
        self.state.press.is_some() && matches!(self.pick, DialogPick::PatternPhase | DialogPick::ShadowDrag | DialogPick::GradientDrag)
    }

    fn paint(&self, _draw: &Draw<'_>, _cx: &ToolContext<'_>) {}

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        if cx.precise() && samples(self.pick) {
            return Cursor::Painted(PaintedCursor::Crosshair);
        }
        self.scope.with_app(|app| canvas_input::cursor(app, self.pick))
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("a dialog's pick on the canvas has the dialog's own fields")
    }
}
