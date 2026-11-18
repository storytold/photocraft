//! The Perspective Crop tool (C, in the Crop flyout), as in Photoshop:
//!
//! - drag a box, or click four corners, to make a quad (the framework's `PerspectiveCropTool`: TL, TR, BR, BL in
//!   document px, clicked corners put in that order);
//! - drag a corner to move it alone, or drag inside the quad to move it (the framework's quad control);
//! - the quad shows its perspective grid (Show Grid) and turns red while it isn't a convex quad, which can't be
//!   applied; one flipped on one axis (wound the other way) applies mirrored (#12489);
//! - ↵ (or ✓) applies: `image.perspectiveCrop` maps the quad onto an upright rectangle, the whole document in one
//!   undo step. Its size comes from the options bar's W × H and resolution (`CropSizeFields`), or with them empty
//!   from the rectangle the quad shows in perspective. Esc (or ⊘) cancels.
//!
//! The quad lives in the tool, not in the document: its edits are no history steps. A pending quad belongs to the
//! document it was drawn on, like the Crop tool's frame.

use craft::behavior::CancelReason;
use craft::behavior::kurbo::Size;
use craft::draw::ui::crop::{CropDocKey, CropDocument, PerspectiveCropBar, PerspectiveCropEvent, PerspectiveCropTool, PerspectiveInput};
use craft::ui::bars::BarOutput;
use craft::ui::canvas::{CropShield, ToolHostResponse};
use craft::ui::contract::Bind;
use craft::ui::runtime::edit::cancel_all;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::document_display::CanvasHost;
use crate::canvas::tools::{FamilyTool, ToolScope};
use crate::state::Tool;

/// The Perspective Crop family's state between passes (`CanvasTools::perspective`).
#[derive(Default)]
pub struct PerspectiveCrop {
    /// The framework tool: the quad, the clicked corners and the press.
    pub(crate) tool: PerspectiveCropTool,
}

/// A quad or corners clicked: what the options bar's Commit and Cancel end.
pub fn pending(app: &PhotocraftApp) -> bool {
    app.canvas_tools.perspective.tool.pending()
}

/// The pending quad's corners (TL, TR, BR, BL; document px), for the control route.
pub(crate) fn quad(app: &PhotocraftApp) -> Option<[[f64; 2]; 4]> {
    let quad = *app.canvas_tools.perspective.tool.quad();
    quad.map(|q| q.map(|p| [p.x, p.y]))
}

/// Drops the quad and the clicked corners.
pub fn cancel(app: &mut PhotocraftApp) {
    app.canvas_tools.perspective.tool.clear();
}

/// The Perspective Crop tool for one pass: the framework's pass over the family's state, with the options bar's Show
/// Grid and the Perspective Crop shield. Its idle cursor, box and corner rules, clicks and preview are the pass's.
pub(crate) fn tool<'a>(scope: &'a ToolScope<'a>, state: &'a mut PerspectiveCrop, _host: CanvasHost) -> FamilyTool<'a> {
    Box::new(state.tool.pass(scope.with_app(|app| input(app))))
}

/// What the pass is given each frame: the active document, whose pending quad the pass keeps and drops when another
/// document becomes active, and the options bar's Show Grid.
fn input<'a>(app: &PhotocraftApp) -> PerspectiveInput<'a> {
    let mut input = PerspectiveInput::default();
    input.document = app.session.active().map(|st| CropDocKey::new(st.doc.id.0));
    input.show_grid = app.ui.tool_options.perspective_crop_grid;
    input.shield = Some(&CropShield::PERSPECTIVE);
    input
}

/// What the app does with a finished pass: a new quad's first press drops an active selection as its own "Deselect"
/// step, as in Photoshop (#11975), so it is gone after the commit and after Esc too. The pass reports the session's
/// first press as `Started`; refused (a job locks the document, the reason in the status bar), the press is cancelled
/// and the tool cleared, so it starts no quad.
///
/// A second pass of a frame has no events of the first (they are taken here), so it runs no second Deselect.
pub(crate) fn drain(app: &mut PhotocraftApp, state: &mut PerspectiveCrop, _response: &ToolHostResponse) {
    let events = state.tool.take_events();
    if app.ui.tool != Tool::PerspectiveCrop {
        return;
    }
    // The pass reports a session's first press once, so the Deselect runs once per session.
    if !events.iter().any(|event| matches!(event, PerspectiveCropEvent::Started)) {
        return;
    }
    let selected = app.session.active().is_some_and(|st| st.doc.selection.is_some());
    if selected && app.run("select.deselect", json!({})).is_err() {
        cancel_all(app.ctx(), CancelReason::HostChanged);
        state.tool.clear();
    }
}

/// The canvas body's quad: its handles, grid and shield. Enter applies and Esc cancels through the control.
///
/// The control reads the frame's keys itself and an applied quad is gone from the tool, so a second pass of the frame
/// applies nothing of the first.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: egui::Rect) {
    if app.ui.tool != Tool::PerspectiveCrop || app.session.active().is_none() {
        return;
    }
    let input = input(app);
    let out = app.canvas_tools.perspective.tool.pass(input).show_quad(ui, rect);
    if out.apply {
        apply(app);
    } else if out.discard {
        cancel(app);
    }
}

/// The active document as the options fields and the crop size see it.
fn document(app: &PhotocraftApp) -> Option<CropDocument> {
    let st = app.session.active()?;
    let size = Size::new(f64::from(st.doc.size.width), f64::from(st.doc.size.height));
    Some(CropDocument::new(CropDocKey::new(st.doc.id.0), size, f64::from(st.doc.resolution_dpi)))
}

/// ↵: crops to the pending quad. W and H come from the options fields (inches at the typed resolution; an empty side
/// takes the perspective size), each at least 1 px. An invalid quad stays for fixing, with the engine's reason.
pub fn apply(app: &mut PhotocraftApp) {
    let Some(corners) = quad(app) else { return };
    let mut params = json!({"corners": corners});
    if let Some(document) = document(app) {
        let target = app.ui.tool_options.perspective_crop.target(&document, &crate::crop_ui::units(app));
        let side = |px: Option<f64>| px.map(|v| v.round().max(1.0));
        if let Some(width) = side(target.width) {
            params["width"] = json!(width);
        }
        if let Some(height) = side(target.height) {
            params["height"] = json!(height);
        }
        if let Some(resolution) = target.resolution {
            params["resolution"] = json!(resolution);
        }
    }
    match app.run("image.perspectiveCrop", params) {
        Ok(_) => cancel(app),
        Err(err) => {
            app.ui.status = err.to_string();
            app.ui.status_error = true;
        }
    }
}

/// Whether the options bar shows Commit and Cancel: during a session, a quad or clicked corners (#11978).
pub fn commit_shown(app: &PhotocraftApp) -> bool {
    pending(app)
}

/// The bar's Commit or Cancel, when its button was pressed this pass.
pub fn committed(app: &mut PhotocraftApp, out: &BarOutput) {
    if out.committed {
        apply(app);
    } else if out.cancelled {
        cancel(app);
    }
}

/// Options bar: W, ⇄, H, the resolution, Front Image, Clear and Show Grid. A value typed out of range raises the
/// range alert once.
pub fn options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let document = document(app).unwrap_or_else(|| CropDocument::new(CropDocKey::new(0), Size::ZERO, 72.0));
    let options = &mut app.ui.tool_options;
    let shown = PerspectiveCropBar::new(&mut options.perspective_crop, &document, Bind::bound(&mut options.perspective_crop_grid)).show(ui);
    if let Some(alert) = shown.out_of_range {
        crate::option_cells::range_alert(app, ui, &alert);
    }
}

#[cfg(test)]
#[path = "perspective_crop_fields_tests.rs"]
mod perspective_crop_fields_tests;

#[cfg(test)]
#[path = "perspective_crop_bar_buttons_tests.rs"]
mod perspective_crop_bar_buttons_tests;

#[cfg(test)]
#[path = "perspective_crop_deselect_tests.rs"]
mod perspective_crop_deselect_tests;

#[cfg(test)]
#[path = "perspective_crop_mirrored_tests.rs"]
mod perspective_crop_mirrored_tests;

#[cfg(test)]
mod tests {
    use craft::behavior::kurbo::Point;
    use craft::draw::ui::CropLength;
    use craft::ui::runtime::env::UnitKind;
    use egui::Modifiers;
    use photocraft_doc::{Color, ColorMode, Document, SampleType, Size as DocSize};

    use super::*;
    use crate::canvas::testing::{ToolEvent, tool_event};

    fn app_with(documents: &[(&str, u32, u32)]) -> PhotocraftApp {
        let mut s = photocraft_engine::Session::new();
        for (name, w, h) in documents {
            s.add_document(Document::with_background(*name, DocSize::new(*w, *h), ColorMode::Rgb, SampleType::U8, Color::WHITE), None);
        }
        s.set_active(0);
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.sync_views();
        app.ui.tool = Tool::PerspectiveCrop;
        app
    }

    fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
        tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Modifiers::NONE);
        tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, Modifiers::NONE);
        tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, Modifiers::NONE);
    }

    fn size(app: &PhotocraftApp) -> (u32, u32) {
        let size = app.session.active().unwrap().doc.size;
        (size.width, size.height)
    }

    /// Apply sends `image.perspectiveCrop` with the quad drawn on the canvas and the size from the fields: typed
    /// lengths at the typed resolution, empty W and H the perspective size; one undo step, and the tool is cleared.
    #[test]
    fn apply_maps_the_quad_and_size_to_image_perspective_crop() {
        // A box drag, W and H typed in pixels.
        let mut app = app_with(&[("p", 200, 150)]);
        drag(&mut app, [20.0, 10.0], [120.0, 90.0]);
        let corners = [[20.0, 10.0], [120.0, 10.0], [120.0, 90.0], [20.0, 90.0]];
        assert_eq!(quad(&app), Some(corners), "the drag drew the box");
        let fields = &mut app.ui.tool_options.perspective_crop;
        fields.width = Some(CropLength::new(60.0, UnitKind::Pixels));
        fields.height = Some(CropLength::new(40.0, UnitKind::Pixels));
        let steps = app.session.active().unwrap().history.past_len();
        apply(&mut app);
        assert_eq!(size(&app), (60, 40));
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "one undo step");
        assert_eq!(quad(&app), None, "the tool is cleared");

        // Inches at the typed resolution.
        let mut app = app_with(&[("p", 800, 500)]);
        app.canvas_tools.perspective.tool.set(Some([Point::new(100.0, 50.0), Point::new(700.0, 80.0), Point::new(760.0, 450.0), Point::new(40.0, 420.0)]));
        let fields = &mut app.ui.tool_options.perspective_crop;
        fields.width = Some(CropLength::new(2.0, UnitKind::Inches));
        fields.height = Some(CropLength::new(1.0, UnitKind::Inches));
        fields.resolution = "100".into();
        apply(&mut app);
        assert_eq!(size(&app), (200, 100));
        assert!((app.session.active().unwrap().doc.resolution_dpi - 100.0).abs() < 0.01);

        // W and H empty: the perspective size.
        let mut app = app_with(&[("p", 800, 500)]);
        app.canvas_tools.perspective.tool.set(Some([Point::new(100.0, 50.0), Point::new(700.0, 80.0), Point::new(760.0, 450.0), Point::new(40.0, 420.0)]));
        apply(&mut app);
        let (w, h) = size(&app);
        assert!(w.abs_diff(731) <= 1 && h.abs_diff(588) <= 1, "{w} x {h}");
        assert_eq!(quad(&app), None);
    }

    /// The pending quad and clicked corners belong to their document (#1918): switching documents drops them, so they
    /// are never drawn on the other one; on their own document they stay.
    #[test]
    fn switching_documents_drops_the_pending_quad() {
        let mut app = app_with(&[("a", 200, 150), ("b", 200, 150)]);
        drag(&mut app, [20.0, 10.0], [120.0, 90.0]);
        app.sync_views();
        assert!(quad(&app).is_some(), "same document: the quad stays");
        app.session.set_active(1);
        app.sync_views();
        // The tool host drops what belongs to the other document in its next pass.
        crate::canvas::testing::frame(&mut app);
        assert_eq!(quad(&app), None);
        tool_event(&mut app, ToolEvent::Down { x: 30.0, y: 30.0, pressure: 1.0 }, Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 30.0 }, Modifiers::NONE);
        assert_eq!(app.canvas_tools.perspective.tool.placing(), 1, "a corner clicked on the second document");
        app.session.set_active(0);
        app.sync_views();
        crate::canvas::testing::frame(&mut app);
        assert_eq!(app.canvas_tools.perspective.tool.placing(), 0, "the clicks are dropped too");
    }
}
