//! The Crop tool's host. The tool itself is the framework's `CropTool`: its session keeps the frame, its history,
//! Straighten and the default-frame rules, and its pass handles the gestures, keys, cursors and the overlay. This module
//! builds each pass's input from the app (`CropRun`), maps what a pass reports to the app (`finish`: `image.crop`, the
//! stored options, an upright view) and gives the other parts of the app their reads of the crop state.

use serde_json::{Value, json};

use craft::behavior::geometry::CropBox;
use craft::behavior::kurbo::{Rect, Size};
use craft::draw::ui::crop::{CropBarWords, CropCommit, CropDocKey, CropDocument, CropEvent, CropFill, CropInput, CropOutput, CropSettings, CropTool};
use craft::ui::canvas::ShieldColor;
use craft::ui::runtime::env::{Env, Units};

use crate::PhotocraftApp;
use crate::state::Tool;

pub(crate) mod bar;

/// The crop host kept at `app.canvas_tools.crop`: the framework tool and nothing else.
#[derive(Default)]
pub(crate) struct CropHost {
    /// The Crop tool, with the pending frame and its history.
    pub(crate) tool: CropTool,
}

/// What one pass of the Crop tool needs from the app, taken before the app is lent to the tool host.
pub(crate) struct CropRun {
    settings: CropSettings,
    document: CropDocument,
    nudge_pause: Option<f64>,
}

impl CropRun {
    /// The run for this pass; `None` unless the Crop tool is current on a document.
    pub(crate) fn of(app: &PhotocraftApp) -> Option<CropRun> {
        if app.ui.tool != Tool::Crop {
            return None;
        }
        Some(CropRun { settings: app.ui.tool_options.crop.clone(), document: document(app)?, nudge_pause: None })
    }

    /// The tool's input for the pass on `ctx`.
    pub(crate) fn input<'a>(&'a self, ctx: &'a egui::Context) -> CropInput<'a> {
        CropInput::new(ctx, &self.settings, &self.document).nudge_pause(self.nudge_pause)
    }
}

/// The units of the app's root scope, for code that runs outside a pass.
pub(crate) fn units(app: &PhotocraftApp) -> Units {
    Env::of_root(app.ctx()).units()
}

/// The crop bar's words, for code that runs outside a pass.
pub(crate) fn words(app: &PhotocraftApp) -> std::sync::Arc<CropBarWords> {
    Env::of_root(app.ctx()).words::<CropBarWords>()
}

/// The active document as the Crop tool sees it: identity, size, resolution and the selection's bounds on the canvas.
pub(crate) fn document(app: &PhotocraftApp) -> Option<CropDocument> {
    let st = app.session.active()?;
    let canvas = st.doc.bounds();
    let selection = st.doc.selection.as_ref().map(|s| s.content_bounds().intersect(&canvas)).filter(|b| !b.is_empty());
    let bounds = selection.map(|b| Rect::new(f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)));
    let size = Size::new(f64::from(st.doc.size.width), f64::from(st.doc.size.height));
    Some(CropDocument::new(CropDocKey::new(st.doc.id.0), size, f64::from(st.doc.resolution_dpi)).selection(bounds))
}

/// Maps what a canvas pass reported and acknowledges the crop on the tool that emitted it.
pub(crate) fn finish(app: &mut PhotocraftApp, tool: &mut CropTool, output: CropOutput) {
    for event in output.events {
        if let CropEvent::Apply(commit) = event {
            tool.applied(apply_commit(app, &commit));
        } else {
            handle_event(app, event);
        }
    }
}

/// Runs once per pass: while another tool is current, a drawn frame is cropped as leaving the tool does, and the view
/// is upright after it.
pub(crate) fn each_pass(app: &mut PhotocraftApp) {
    if app.ui.tool == Tool::Crop {
        return;
    }
    let Some(document) = document(app) else { return };
    let keeps_pending = app.stylus.tool_before_eraser == Some(Tool::Crop);
    let (settings, units) = (app.ui.tool_options.crop.clone(), units(app));
    let event = app.canvas_tools.crop.tool.leave(&settings, &document, &units, keeps_pending);
    // The tool drives the view only while it is picked: the view a default-mode crop turned goes upright with the
    // commit leaving the tool makes, accepted or refused, as Enter's does (#16330).
    let commits = matches!(event, Some(CropEvent::Apply(_)));
    handle_event(app, event);
    if commits {
        upright_view(app, document.key);
    }
}

/// The one commit: the menus, the bar, the task bar and the canvas menu call it. True when the document was cropped.
pub(crate) fn apply(app: &mut PhotocraftApp) -> bool {
    let Some(document) = document(app) else { return false };
    let (settings, units) = (app.ui.tool_options.crop.clone(), units(app));
    let event = app.canvas_tools.crop.tool.apply(&settings, &document, &units);
    handle_event(app, event)
}

/// Drops the pending frame (Esc, the cancel button).
pub(crate) fn discard(app: &mut PhotocraftApp) {
    let event = app.canvas_tools.crop.tool.discard();
    handle_event(app, event);
}

/// Reset: the default frame again.
pub(crate) fn reset(app: &mut PhotocraftApp) {
    let Some(document) = document(app) else { return };
    let (settings, units) = (app.ui.tool_options.crop.clone(), units(app));
    let event = app.canvas_tools.crop.tool.reset_frame(&settings, &document, &units);
    handle_event(app, event);
}

/// Fits the frame to the options after the ratio or W × H × Resolution changed; `editing` while a ratio field is scrubbed.
pub(crate) fn refit(app: &mut PhotocraftApp, editing: bool) {
    let Some(document) = document(app) else { return };
    let (settings, units) = (app.ui.tool_options.crop.clone(), units(app));
    let event = app.canvas_tools.crop.tool.refit(&settings, &document, &units, editing);
    handle_event(app, event);
}

/// The Straighten button.
pub(crate) fn toggle_straighten(app: &mut PhotocraftApp) {
    app.canvas_tools.crop.tool.toggle_straighten();
}

/// Runs one event of the tool. True for an `Apply` that cropped the document.
pub(super) fn handle_event(app: &mut PhotocraftApp, event: impl Into<Option<CropEvent>>) -> bool {
    match event.into() {
        Some(CropEvent::Apply(commit)) => {
            let accepted = apply_commit(app, &commit);
            app.canvas_tools.crop.tool.applied(accepted);
            accepted
        }
        Some(CropEvent::Discard(key)) => {
            upright_view(app, key);
            false
        }
        Some(CropEvent::Settings(settings)) => {
            app.ui.tool_options.crop = settings;
            false
        }
        // The tool mirrored the frame itself; Crop To has no command in the app.
        Some(CropEvent::Frame(_) | CropEvent::Straightened(_) | CropEvent::CropTo(_)) | None => false,
        Some(_) => false,
    }
}

/// Runs `image.crop` for `commit` and returns whether the document accepted it.
fn apply_commit(app: &mut PhotocraftApp, commit: &CropCommit) -> bool {
    let rect = commit.frame.rect;
    let px = |v: f64| v.round().max(1.0) as u64;
    let mut params = json!({
        "x": rect.x0.round(), "y": rect.y0.round(), "width": px(rect.width()), "height": px(rect.height()),
        "deleteCroppedPixels": commit.delete_pixels,
    });
    if commit.fill == CropFill::ContentAware {
        params["contentAware"] = json!(true);
    }
    if commit.frame.angle != 0.0 {
        params["angle"] = json!(commit.frame.angle);
    }
    if let Some(target) = &commit.target {
        params["targetWidth"] = json!(px(target.width));
        params["targetHeight"] = json!(px(target.height));
        params["resample"] = json!(crate::sizing::pref_resample(app.session.prefs().general.image_interpolation));
        if let Some(resolution) = target.resolution {
            params["resolution"] = json!(resolution);
        }
    }
    match app.run("image.crop", params) {
        Ok(_) => {
            if let Some(view) = app.session.active_index().and_then(|i| app.ui.views.get_mut(i)) {
                view.place(craft::ui::canvas::ViewCommands::FitOnScreen);
            }
            true
        }
        Err(error) => {
            app.ui.status = error;
            app.ui.status_error = true;
            false
        }
    }
}

/// The default mode turned the view of the document whose frame was dropped: it goes upright (Classic Mode never turns it).
fn upright_view(app: &mut PhotocraftApp, key: CropDocKey) {
    if !app.ui.tool_options.crop.image_mode() {
        return;
    }
    // `ui.views` is still in the order of `view_docs` while the documents are being synced.
    let synced = app.view_docs.iter().position(|d| d.0 == key.0);
    let index = synced.or_else(|| app.session.documents().iter().position(|st| st.doc.id.0 == key.0));
    if let Some(view) = index.and_then(|i| app.ui.views.get_mut(i)) {
        view.rotation = 0.0;
    }
}

/// A crop is pending: a frame was drawn, moved, resized, turned or keyed, not just the untouched default one.
pub fn pending(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop && app.canvas_tools.crop.tool.session().pending()
}

#[cfg(test)]
/// The frame is the untouched default one, not a drawn, moved, resized or turned box.
pub(crate) fn is_default_frame(app: &PhotocraftApp) -> bool {
    app.canvas_tools.crop.tool.session().is_default()
}

#[cfg(test)]
/// A drag of the frame or a Straighten line runs.
pub(crate) fn dragging(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop && (app.canvas_tools.crop.tool.dragging() || app.canvas_tools.crop.tool.session().straighten().on())
}

#[cfg(test)]
/// The Straighten line is armed or held.
pub(crate) fn straightening(app: &PhotocraftApp) -> bool {
    app.canvas_tools.crop.tool.straighten_armed() || app.canvas_tools.crop.tool.session().straighten().on()
}

/// The frame after the last pass.
pub fn frame(app: &PhotocraftApp) -> Option<CropBox> {
    app.canvas_tools.crop.tool.frame()
}

/// The canvas shows the pixels past its edges under the shield. Default mode only: in Classic Mode the frame lies over
/// the pasteboard and the layers stop at the canvas edge, as in Photoshop.
pub(crate) fn shows_beyond_canvas(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop && app.ui.tool_options.crop.image_mode() && app.canvas_tools.crop.tool.shows_beyond_canvas()
}

/// The colour the area a crop adds past the canvas previews in (`shows_beyond_canvas`), as the commit fills it: the
/// background colour on a visible Background with Delete Cropped Pixels on and Fill: Background. `None` previews
/// transparency (Delete off turns the Background into a layer; Content-Aware Fill makes its own pixels).
pub(crate) fn grown_fill(app: &PhotocraftApp, doc: &photocraft_doc::Document) -> Option<[f32; 4]> {
    let crop = &app.ui.tool_options.crop;
    let fills = crop.delete_pixels && crop.fill == CropFill::Background && doc.background().is_some_and(|layer| layer.visible);
    fills.then_some(app.session.tools.background)
}

/// Show Cropped Area is off with the Crop tool showing a frame: nothing outside it shows.
pub(crate) fn hides_canvas_edge(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop && frame(app).is_some() && !app.ui.tool_options.crop.shield.shows_cropped_area()
}

/// The gestures move and turn the image rather than the frame (default mode with the Crop tool).
pub(crate) fn moves_view(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop && app.ui.tool_options.crop.image_mode()
}

/// The Crop tool drives the view: it shows a frame on the active document.
pub(crate) fn turns_view(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop && frame(app).is_some() && app.session.active().is_some()
}

/// The Info panel's W and H while the Crop tool shows a frame: its own size before any turn.
pub fn info_size(app: &PhotocraftApp) -> Option<[f64; 2]> {
    if app.ui.tool != Tool::Crop {
        return None;
    }
    let r = frame(app)?.rect;
    let (w, h) = (r.width(), r.height());
    (w.is_finite() && h.is_finite()).then(|| [w.round().max(1.0), h.round().max(1.0)])
}

/// While the Crop tool has box history, Undo/Redo step the box, not the document. `Some(enabled)` for those commands,
/// else `None` so the caller falls back to document history.
pub fn is_enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    let session = app.canvas_tools.crop.tool.session();
    if app.ui.tool != Tool::Crop || !session.owns_undo() {
        return None;
    }
    match id {
        "edit.undo" => Some(session.can_undo()),
        "edit.redo" => Some(session.can_redo()),
        "edit.toggleLastState" => Some(false),
        _ => None,
    }
}

/// Intercepts Undo/Redo/Toggle Last State while the Crop tool owns box history, for every caller (menus, shortcuts, the
/// control channel).
pub fn intercept(app: &mut PhotocraftApp, id: &str) -> Option<Result<Value, String>> {
    if holds_history(app, id) {
        return Some(Err("Commit or cancel the crop first".into()));
    }
    if app.ui.tool != Tool::Crop || !app.canvas_tools.crop.tool.session().owns_undo() {
        return None;
    }
    match id {
        "edit.undo" => {
            app.canvas_tools.crop.tool.undo();
            Some(Ok(Value::Null))
        }
        "edit.redo" => {
            app.canvas_tools.crop.tool.redo();
            Some(Ok(Value::Null))
        }
        "edit.toggleLastState" => Some(Err("Commit or cancel the crop first".into())),
        _ => None,
    }
}

/// A History jump waits for a drawn frame: it would resize the document under the frame, which keeps its coordinates.
pub fn holds_history(app: &PhotocraftApp, id: &str) -> bool {
    pending(app) && matches!(id, "history.selectState" | "history.selectSnapshot" | "history.stepBackward" | "history.stepForward" | "history.deleteState")
}

/// Menu commands Photoshop greys out while a crop is pending, until it is committed or cancelled.
pub fn blocks(app: &PhotocraftApp, id: &str) -> bool {
    pending(app) && !available_while_pending(id)
}

/// Whether command `id` still runs while a crop is pending: everything except Close, Save, Save As, Save a Copy; Undo,
/// Redo, Toggle Last State, Search; Image › Crop; the View menu but Proof Setup, Pixel Aspect Ratio (Correction), 32-bit
/// Preview Options, Flip Horizontal and Screen Mode; the Window menu but Workspace and Adjustments; Help but System Info.
pub(crate) fn available_while_pending(id: &str) -> bool {
    if id.starts_with("workspace.") {
        return false;
    }
    if let Some(v) = id.strip_prefix("view.") {
        let greyed = ["proofSetup.", "pixelAspectRatio", "thirtyTwoBitPreviewOptions", "flipHorizontal", "screenMode."];
        return !greyed.iter().any(|g| v.starts_with(g));
    }
    if let Some(w) = id.strip_prefix("window.") {
        return !(w.starts_with("workspace.") || w == "panel.adjustments");
    }
    if id.starts_with("help.") {
        return id != "help.systemInfo";
    }
    matches!(
        id,
        "file.close"
            | "file.save"
            | "file.saveAs"
            | "file.saveACopy"
            | "file.exit"
            | "edit.undo"
            | "edit.redo"
            | "edit.toggleLastState"
            | "edit.search"
            | "image.crop"
    )
}

/// The Color Picker's OK for the custom shield colour (`"#rrggbb"`).
pub(crate) fn set_shield_color(app: &mut PhotocraftApp, hex: &str) -> Result<(), String> {
    let rgb = crate::color_picker_ui::parse_hex(hex).ok_or_else(|| format!("not a colour: {hex}"))?;
    let shield = &mut app.ui.tool_options.crop.shield;
    shield.custom_color = rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    shield.color = ShieldColor::Custom;
    Ok(())
}

#[cfg(test)]
mod tests;
