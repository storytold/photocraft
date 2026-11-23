//! The Crop family's adapter: the canvas body shows the frame (`show_frame`), the framework's `CropToolPass` over what
//! the body reported is the host's tool, and `finish` hands its output to the one `image.crop` mapping. The tool itself
//! lives at `app.canvas_tools.crop.tool` (`CropHost`); this module keeps no state.

use craft::draw::ui::crop::{CropOutput, CropShown, CropTool, CropToolPass};

use crate::PhotocraftApp;
use crate::crop_ui::CropRun;

/// The tool for one pass of the host: `tool` over this pass's `run`, with what the canvas body's frame reported in the
/// same pass (`shown`, from [`show_frame`]).
pub(crate) fn pass<'a>(tool: &'a mut CropTool, run: &'a CropRun, ctx: &'a egui::Context, shown: Option<CropShown>) -> CropToolPass<'a> {
    tool.pass(run.input(ctx), shown)
}

/// Shows the Crop tool's frame in the canvas body at `rect`. What it reported goes to the host's pass of this canvas
/// (`tools::run`), which hands its snap lines and autoscroll to the host and its events to [`finish`].
pub(crate) fn show_frame(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: egui::Rect) -> Option<CropShown> {
    let run = CropRun::of(app)?;
    let ctx = ui.ctx().clone();
    Some(app.canvas_tools.crop.tool.show_frame(ui, rect, &run.input(&ctx)))
}

/// Applies what the pass reported while the live Crop tool is still held by the canvas rig.
pub(crate) fn finish(app: &mut PhotocraftApp, tool: &mut CropTool, output: CropOutput) {
    crate::crop_ui::finish(app, tool, output);
}
