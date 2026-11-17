//! Photoshop's on-canvas brush resize (#231, #297): with a painting tool, Alt+right-drag
//! (Windows, Linux) or Control+Option+drag (macOS; Control+Alt+drag works everywhere) changes the
//! brush instead of painting. Left and right set the diameter, up and down the hardness (the opacity
//! with "Vary Round Brush Hardness on HUD" off). The gesture, its rates and its readout rows are the
//! framework's `BrushResize`; this file applies its steps to the brush and shows its falloff preview.
//!
//! The change is `tools.setBrush` calls sharing one `coalesce` key, so the whole drag is one
//! journal entry (Rule 1) and, the brush being tool state, no history step. Krita's ⇧+drag (the
//! request in #231) is not used: ⇧ is Photoshop's straight-line modifier for painting tools.

use craft::behavior::{GesturePhase, PointerButton};
use craft::keymap::Mods;
use craft::ui::canvas::{ToolContext, ToolPointer};
use craft::ui::canvas_controls::CanvasControlWords;
use craft::ui::canvas_controls::brush::{BrushDimensions, BrushLevel, BrushResize, BrushResizeOptions, ResizeStep, TipFalloff};
use craft::ui::contract::Show as _;
use craft::ui::style::DataColor;
use photocraft_engine::paint::MAX_BRUSH_SIZE;

use crate::PhotocraftApp;
use crate::state::Tool;

/// Screen points of vertical drag that take the hardness from 0 % to 100 %: Photoshop's rate,
/// where 50 points down from 0 % give 13 % (#11891).
const HARDNESS_TRAVEL: f32 = 385.0;
/// Screen points of vertical drag that take the opacity from 0 % to 100 % when "Vary Round Brush
/// Hardness on HUD" is off.
const OPACITY_TRAVEL: f32 = 200.0;
/// The overlay alpha of the preview tint.
const TINT_ALPHA: u8 = 110;

/// The brush resize drag of the painting tools, kept in `PhotocraftApp::brush_resize`. The painting tool takes it
/// out of the app for each call, so its methods get the app as an argument.
#[derive(Default)]
pub(crate) struct PaintResize {
    resize: BrushResize,
    /// Numbers the drags: the `coalesce` key of one drag's `tools.setBrush` calls.
    key: u64,
}

/// Tools the gesture resizes: every painting tool, and Quick Selection (it has a brush too).
fn applies(tool: Tool) -> bool {
    tool.is_brushlike() || tool == Tool::QuickSelection
}

/// Whether "Vary Round Brush Hardness on HUD" is on: the vertical movement takes the hardness, else the opacity.
fn varies_hardness(app: &PhotocraftApp) -> bool {
    app.session.prefs().tools.vary_round_brush_hardness_on_hud
}

/// How a resize behaves for the current preferences.
fn options(app: &PhotocraftApp) -> BrushResizeOptions {
    let options = BrushResizeOptions::new(f64::from(MAX_BRUSH_SIZE));
    let options = if varies_hardness(app) {
        options.level(BrushLevel::Hardness).travel(HARDNESS_TRAVEL)
    } else {
        options.level(BrushLevel::Opacity).travel(OPACITY_TRAVEL)
    };
    // Preferences ▸ Cursors ▸ Brush Preview colour; red, as in Photoshop, when it is unreadable.
    let [r, g, b] =
        crate::color_picker_ui::parse_hex(&app.session.prefs().cursors.brush_preview_color).map_or([255, 0, 0], |c| c.map(|v| (v * 255.0).round() as u8));
    options.tint(DataColor::new(egui::Color32::from_rgba_unmultiplied(r, g, b, TINT_ALPHA)))
}

impl PaintResize {
    /// Whether the painting `tool` takes a press of `button` with `mods` held: the gesture's buttons and chords, on a
    /// tool the gesture resizes.
    pub(crate) fn takes(tool: Tool, button: PointerButton, mods: Mods) -> bool {
        applies(tool) && BrushResize::takes(button, mods)
    }

    /// Whether a press of `button` with `mods` on the painting `tool` is the resize gesture's chord (Ctrl+Alt with the
    /// primary button): the tool takes that press even where it would hand the press to the ⌘ stand-in.
    pub(crate) fn chord(tool: Tool, button: PointerButton, mods: Mods) -> bool {
        applies(tool) && BrushResize::chord(button, mods)
    }

    /// Whether a resize runs.
    pub(crate) fn active(&self) -> bool {
        self.resize.active()
    }

    /// A press: begins a resize when it is the gesture's chord on a tool it resizes, and says whether it did, so the
    /// tool paints nothing.
    pub(crate) fn press(&mut self, app: &mut PhotocraftApp, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> bool {
        if !applies(app.ui.tool) || !BrushResize::chord(pointer.button, pointer.mods) {
            return false;
        }
        let Some(space) = cx.space() else { return false };
        let brush = &app.session.tools.brush;
        let level = if varies_hardness(app) { brush.hardness } else { brush.opacity };
        let start = BrushDimensions::new(f64::from(brush.size), f64::from(level));
        if self.resize.begin(&space.view, pointer.pos, start, &options(app)).is_none() {
            return false;
        }
        self.key += 1;
        self.hud(cx);
        true
    }

    /// The pointer moved during a resize. Applies only the movement this call carries: a pass without pointer input
    /// does not call it, so a second pass in a frame sets nothing again.
    pub(crate) fn drag(&mut self, app: &mut PhotocraftApp, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(space) = cx.space() else { return };
        let step = self.resize.track(&space.view, pointer.pos);
        self.apply(app, step);
        self.hud(cx);
    }

    /// The button came up: the last values stay.
    pub(crate) fn release(&mut self, app: &mut PhotocraftApp, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(space) = cx.space() else { return };
        let step = self.resize.finish(&space.view, pointer.pos);
        self.apply(app, step);
    }

    /// Escape or a lost pointer: the values the resize started from go back.
    pub(crate) fn cancel(&mut self, app: &mut PhotocraftApp, _cx: &mut ToolContext<'_>) {
        let step = self.resize.cancel();
        self.apply(app, step);
    }

    /// Asks for the diameter and level readout beside the pointer while a resize runs. `hud`, not `readout`: Show
    /// Transformation Values never hides it. The request lasts one pass, so the tool repeats it every pass.
    pub(crate) fn hud(&self, cx: &mut ToolContext<'_>) {
        if !self.resize.active() {
            return;
        }

        let words = cx.words::<CanvasControlWords>();
        let rows = self.resize.readout_rows(&cx.units(), &words);
        cx.hud(rows);
    }

    /// The falloff preview at the press point, shown in the canvas body (`brush_feedback::show`). Nothing while no
    /// resize runs.
    pub(crate) fn show_preview(&self, ui: &mut egui::Ui, app: &PhotocraftApp) {
        // The brush already holds the resize's current values: each step is applied as it comes.
        let brush = &app.session.tools.brush;
        let (radius, hardness) = (brush.size / 2.0, brush.hardness);
        let (core, _) = photocraft_paint::tip_shape(radius, hardness);
        let reach = photocraft_paint::tip_reach(radius, hardness).max(core);
        let falloff = TipFalloff::sampled(f64::from(core), f64::from(reach), |d| photocraft_paint::tip_falloff(d as f32, radius, hardness));
        if let Some(preview) = self.resize.preview(&falloff) {
            preview.show(ui);
        }
    }

    /// Sets the brush to the step's values: an `Update` or `Commit` that changed them, or a `Cancel`'s start values.
    fn apply(&self, app: &mut PhotocraftApp, step: Option<ResizeStep>) {
        let Some(step) = step else { return };
        if !step.changed && step.phase != GesturePhase::Cancel {
            return;
        }
        let vary = varies_hardness(app);
        let size = step.dimensions.diameter.round() as f32;
        // Whole percent, like the readout and the options bar.
        let level = ((step.dimensions.level * 100.0).round() / 100.0) as f32;
        let before = app.session.tools.brush.clone();
        let mut after = before.clone();
        after.size = size;
        if vary {
            after.hardness = level;
        } else {
            after.opacity = level;
        }
        crate::panels::brush_panel::commit_keyed(app, &before, &after, Some(format!("brush-resize:{}", self.key)));
    }
}

#[cfg(test)]
mod tests;
