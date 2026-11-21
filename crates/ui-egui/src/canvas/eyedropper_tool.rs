//! The Eyedropper tool and the Alt stand-in that samples with a painting tool: PhotoCraft's two
//! uses of the framework's `EyedropperTool`.
//!
//! As the Eyedropper tool, a press and each drag sample the foreground (Alt: the background). As
//! the Alt stand-in, every sample goes to the foreground, because Alt is the key that called the
//! stand-in, and Shift+Alt adds a colour sampler at the clicked pixel instead (#11890). As the
//! Triangle's stand-in, a sample also becomes the tool's solid Fill, and Shift+Alt samples as Alt
//! does (#13364).

use std::cell::Cell;
use std::rc::Rc;

use craft::behavior::{CancelReason, Cursor, PointerButton};
use craft::color_ui::ui::{EyedropperTool, SampleSize, SampleSource};
use craft::keymap::Mods;
use craft::theme::Srgba;
use craft::ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolHostResponse, ToolPath, ToolPointer};
use craft::ui::primitives::Draw;
use serde_json::json;

use super::document_display::CanvasHost;
use super::sampling;
use super::tools::{FamilyTool, ToolScope};
use crate::PhotocraftApp;
use crate::state::Tool;

/// The gesture's flags, kept in `CanvasTools::eyedropper`.
#[derive(Debug, Default)]
pub(crate) struct EyedropperState {
    /// The running press added a colour sampler, so its drags sample nothing.
    sampler_added: bool,
}

/// The Eyedropper over PhotoCraft's sampling, options and colours. A pick applies only in the
/// `press` or `drag` call that took it, so a second pass in a frame, which gets no new pointer
/// input, picks nothing again.
pub(crate) struct PcEyedropper<'a> {
    scope: &'a ToolScope<'a>,
    state: &'a mut EyedropperState,
    inner: EyedropperTool<'a>,
    /// Runs as the Alt stand-in of a painting tool.
    stand_in: bool,
    /// Runs as the Triangle's Alt stand-in: a pick also sets the tool's Fill.
    shape_fill: bool,
    /// Whether the sample being taken may read the pasteboard off the image ([`Self::arm_pasteboard`]).
    on_pasteboard: Rc<Cell<bool>>,
}

/// The Eyedropper tool, for `Tool::Eyedropper`.
pub(crate) fn tool<'a>(scope: &'a ToolScope<'a>, state: &'a mut EyedropperState, _host: CanvasHost) -> FamilyTool<'a> {
    Box::new(PcEyedropper::new(scope, state, false))
}

/// The Alt stand-in of the tools where `cursor_policy::alt_eyedropper` holds, registered as
/// `stand_in(StandInKey::Alt, ..)`.
pub(crate) fn stand_in_tool<'a>(scope: &'a ToolScope<'a>, state: &'a mut EyedropperState, _host: CanvasHost) -> FamilyTool<'a> {
    Box::new(PcEyedropper::new(scope, state, true))
}

/// Nothing to drain: `press`, `release` and `cancel` clear the gesture's flags themselves. Kept for
/// the family shape of `CanvasTools`.
pub(crate) fn drain(_app: &mut PhotocraftApp, _state: &mut EyedropperState, _response: &ToolHostResponse) {}

impl<'a> PcEyedropper<'a> {
    fn new(scope: &'a ToolScope<'a>, state: &'a mut EyedropperState, stand_in: bool) -> Self {
        let (mut options, foreground, precise, triangle) = scope.with_app(|app| {
            let [r, g, b, _] = app.session.tools.foreground;
            let precise = app.session.prefs().cursors.other == photocraft_engine::prefs::OtherCursor::Precise;
            (app.ui.tool_options.eyedropper, Srgba::rgb(r, g, b), precise, app.ui.tool == Tool::Triangle)
        });
        // Preferences › Cursors › Other Cursors = Precise wants the plain crosshair, so no ring.
        options.show_ring &= !precise;
        let on_pasteboard = Rc::new(Cell::new(false));
        let pasteboard = Rc::clone(&on_pasteboard);
        let sample = move |at: [f64; 2], size: SampleSize, source: SampleSource| {
            scope
                .with_app(|app| {
                    // Off the image, the Eyedropper takes the pasteboard as drawn, whatever the Sample mode (#11755).
                    // A pick from a targeted layer mask is not measured there and keeps picking nothing.
                    if pasteboard.get() && sampling::off_image(app, at) && !sampling::samples_mask(app, source) {
                        return Some(sampling::pasteboard_rgb(app));
                    }
                    sampling::pick(app, at, size, source)
                })
                .map(|c| Srgba::rgb(c[0], c[1], c[2]))
        };
        let inner = EyedropperTool::new(options, sample).foreground(foreground);
        PcEyedropper { scope, state, inner, stand_in, shape_fill: stand_in && triangle, on_pasteboard }
    }

    /// Whether a sample at `pointer` may take the pasteboard's colour: the Eyedropper tool itself, without Alt, over
    /// the canvas. Alt's background pick and the painting tools' Alt stand-in outside the image are not measured, so
    /// they keep picking nothing there; past the canvas (another panel) the window's pixels are not read here either.
    fn arm_pasteboard(&self, pointer: &ToolPointer, cx: &ToolContext<'_>) {
        self.on_pasteboard.set(!self.stand_in && !pointer.mods.alt && cx.rect.contains(pointer.pos));
    }

    /// Writes the newest pick to its swatch; as the Triangle's stand-in, also makes the tool's Fill
    /// solid, so it follows the foreground (`vector_ui::shapes::tool_paint`).
    fn apply(&mut self) {
        let Some(pick) = self.inner.take() else { return };
        let key = if !self.stand_in && pick.background { "background" } else { "foreground" };
        let c = pick.color;
        let shape_fill = self.shape_fill;
        self.scope.with_app(|app| {
            if app.run("tools.setColors", json!({ key: [c.r, c.g, c.b, 1.0] })).is_ok() && shape_fill {
                app.ui.tool_options.shape_fill = true;
                app.ui.tool_options.shape_fill_gradient = None;
            }
        });
    }

    /// Shift+Alt adds a colour sampler: the painting tools' stand-in, not the Triangle's.
    fn shift_adds_samplers(&self) -> bool {
        self.stand_in && !self.shape_fill
    }

    /// Shift+Alt: adds a colour sampler at the clicked pixel, as the Color Sampler tool places one.
    /// Returns whether the press did; a press on an existing point is not measured again and samples
    /// instead.
    fn add_sampler(&mut self, pointer: &ToolPointer, cx: &ToolContext<'_>) -> bool {
        let Some(space) = cx.space() else { return false };
        let p = space.view.to_doc(pointer.pos);
        let (x, y) = (p.x, p.y);
        self.scope.with_app(|app| {
            if on_sampler(app, x, y) {
                return false;
            }
            let size = app.session.active().map(|d| d.doc.size);
            let inside = size.is_some_and(|s| (0.0..f64::from(s.width)).contains(&x) && (0.0..f64::from(s.height)).contains(&y));
            // The Color Sampler tool places no point off the canvas and says nothing.
            if inside && let Err(e) = app.run("view.colorSamplers.add", json!({"x": x, "y": y})) {
                // Only the limit is a plain report; any other refusal (a failed Pen draft left in
                // another document) stays an error, also when the document is full.
                let limit = e == photocraft_engine::sampler_cmds::limit_message();
                app.ui.status = e;
                app.ui.status_error = !limit;
            }
            true
        })
    }
}

/// Whether document point (x, y) is on a colour sampler, within the Color Sampler tool's reach of one.
fn on_sampler(app: &PhotocraftApp, x: f64, y: f64) -> bool {
    let tol = app.tool_reach();
    app.session.active().is_some_and(|d| d.color_samplers.iter().any(|q| (q[0] - x).hypot(q[1] - y) <= tol))
}

impl CanvasTool for PcEyedropper<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        self.state.sampler_added = false;
        if self.shift_adds_samplers() && pointer.mods.shift && self.add_sampler(pointer, cx) {
            self.state.sampler_added = true;
            return ToolAction::Claimed;
        }
        self.arm_pasteboard(pointer, cx);
        let action = self.inner.press(pointer, cx);
        self.apply();
        action
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        // While Shift+Alt stay held after a press that added a sampler, moves sample nothing.
        if self.state.sampler_added || (self.shift_adds_samplers() && pointer.mods.shift) {
            return;
        }
        self.arm_pasteboard(pointer, cx);
        self.inner.drag(pointer, cx);
        self.apply();
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.state.sampler_added = false;
        self.inner.release(pointer, cx);
    }

    fn cancel(&mut self, reason: CancelReason, cx: &mut ToolContext<'_>) {
        self.state.sampler_added = false;
        self.inner.cancel(reason, cx);
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        self.inner.paint(draw, cx);
    }

    /// `EyedropperTool::cursor`: the pipette, the precise crosshair with Caps Lock or the preference.
    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        self.inner.cursor(cx)
    }

    fn takes(&self, button: PointerButton, mods: Mods) -> bool {
        // ⌘ is the Move stand-in's, here as with the painting tools.
        button == PointerButton::Primary && (self.stand_in || !mods.cmd)
    }

    fn autoscrolls(&self) -> bool {
        false
    }

    fn command_form(&self) -> ToolPath {
        self.inner.command_form()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "../brush_shift_alt_sampler_tests.rs"]
mod brush_shift_alt_sampler_tests;

#[cfg(test)]
#[path = "../eyedropper_outside_document_tests.rs"]
mod eyedropper_outside_document_tests;
