//! The Eyedropper tool and the Alt stand-in that samples with a painting tool: PhotoCraft's two
//! uses of the framework's `EyedropperTool`.
//!
//! As the Eyedropper tool, a press and each drag sample the foreground (Alt: the background). As
//! the Alt stand-in, every sample goes to the foreground, because Alt is the key that called the
//! stand-in, and Shift+Alt adds a colour sampler at the clicked pixel instead (#11890). As the
//! Triangle or Custom Shape stand-in, a sample also becomes the tool's solid Fill, and Shift+Alt
//! samples as Alt does (#13364, #15856).

use craft::behavior::{CancelReason, Cursor, PointerButton};
use craft::color_ui::ui::{EyedropperTool, SampleSize, SampleSource};
use craft::keymap::Mods;
use craft::theme::Srgba;
use craft::ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolHostResponse, ToolPath, ToolPointer};
use craft::ui::primitives::Draw;
use serde_json::json;

use super::cursor_policy::shape_alt_sets_fill;
use super::document_display::CanvasHost;
use super::sampling;
use super::tools::{FamilyTool, ToolScope};
use crate::PhotocraftApp;

#[cfg(test)]
#[path = "../eyedropper_cmyk_tests.rs"]
mod cmyk_tests;

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
    native: std::rc::Rc<std::cell::Cell<Option<[f32; 4]>>>,
    /// Runs as the Alt stand-in of a painting tool.
    stand_in: bool,
    /// Runs as a shape tool's Alt stand-in: a pick also sets the tool's Fill.
    shape_fill: bool,
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
        let (mut options, foreground, precise, shape_fill) = scope.with_app(|app| {
            let [r, g, b, _] = app.session.tools.foreground;
            let precise = app.session.prefs().cursors.other == photocraft_engine::prefs::OtherCursor::Precise;
            (app.ui.tool_options.eyedropper, Srgba::rgb(r, g, b), precise, shape_alt_sets_fill(app.ui.tool))
        });
        // Preferences › Cursors › Other Cursors = Precise wants the plain crosshair, so no ring.
        options.show_ring &= !precise;
        let native = std::rc::Rc::new(std::cell::Cell::new(None));
        let sampled_native = native.clone();
        let sample = move |at: [f64; 2], size: SampleSize, source: SampleSource| {
            scope
                .with_app(|app| {
                    let picked = sampling::native_pick(app, at, size, source);
                    sampled_native.set(picked.map(|(_, inks)| inks));
                    picked.map(|(rgb, _)| rgb).or_else(|| sampling::pick(app, at, size, source))
                })
                .map(|c| Srgba::rgb(c[0], c[1], c[2]))
        };
        PcEyedropper { scope, state, inner: EyedropperTool::new(options, sample).foreground(foreground), native, stand_in, shape_fill: stand_in && shape_fill }
    }

    /// Writes the newest pick to its swatch; as a shape tool's stand-in, also makes the tool's Fill
    /// solid, so it follows the foreground (`vector_ui::shapes::tool_paint`).
    fn apply(&mut self) {
        let Some(pick) = self.inner.take() else { return };
        let key = if !self.stand_in && pick.background { "background" } else { "foreground" };
        let c = pick.color;
        let shape_fill = self.shape_fill;
        let inks = self.native.get();
        self.scope.with_app(|app| {
            let mut params = json!({ key: [c.r, c.g, c.b, 1.0] });
            if let Some(inks) = inks {
                params[format!("{key}Cmyk")] = json!(inks);
            }
            if app.run("tools.setColors", params).is_ok() && shape_fill {
                app.ui.tool_options.shape_fill = true;
                app.ui.tool_options.shape_fill_gradient = None;
            }
        });
    }

    /// The painting tools' Shift+Alt stand-in adds a colour sampler.
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
        let action = self.inner.press(pointer, cx);
        self.apply();
        action
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        // While Shift+Alt stay held after a press that added a sampler, moves sample nothing.
        if self.state.sampler_added || (self.shift_adds_samplers() && pointer.mods.shift) {
            return;
        }
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
