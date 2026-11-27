//! The Navigator panel: the framework's `NavigatorPanel` bound to the active document's main canvas view,
//! with the document's picture in its preview box.

use craft::behavior::kurbo::Size;
use craft::theme::variant::Tone;
use craft::ui::canvas::{AppPaint, NavigatorPreview};
use craft::ui::contract::{Bind, Show as _};
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::canvas::NavigatorPanel;
use craft::ui::primitives::ContentPainterCx;
use egui::{Pos2, Rect, pos2};

use super::PhotocraftApp;
use crate::canvas::document_display;

pub(super) fn navigator(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let Some(idx) = app.session.active_index() else {
        let _ = Notice::new(Tone::Neutral, tl!("No document")).layout(NoticeLayout::Centred).show(ui);
        return;
    };
    // The canvas as its last pass showed it, the eased camera of an Animated Zoom included.
    let Some(mut view) = document_display::active_view(app) else { return };
    let size = app.session.documents()[idx].doc.size;
    let document = Size::new(f64::from(size.width), f64::from(size.height));
    let navigation = document_display::navigation(app);
    let animated_zoom = app.session.prefs().general.animated_zoom;
    let out = {
        let paint = AppPaint::new(|painter: &ContentPainterCx<'_>, rect: Rect| picture(app, &ctx, idx, painter, rect));
        NavigatorPanel::new(tl!("Navigator"), Bind::bound(&mut view), document).navigation(navigation).preview(NavigatorPreview::Paint(paint)).show(ui)
    };
    // The panel edits the bound view in place and reports an edit of this pass; a pass with no input
    // writes nothing back.
    if !out.view.is_empty() {
        let ppp = app.canvas_ppp();
        let target = app.ui.views.get_mut(idx);
        if let Some(target) = target {
            // The ease starts from the zoom the view has now, so it is requested before the zoom changes.
            let zoom = (view.zoom * f64::from(ppp)) as f32;
            if (zoom - target.zoom).abs() > target.zoom.abs() * 1e-5 {
                target.request_zoom_animation(animated_zoom);
            }
            target.set_from(&view, ppp);
            target.clear_pending();
        }
    }
    crate::chrome_ui::zoom_alert(app, out.invalid_zoom);
}

/// The document's picture in the preview box `rect`: the GPU canvas's own texture at this size while the
/// canvas is on the GPU (#4518), the checkerboard while that texture is not ready, else the CPU thumbnail.
fn picture(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize, painter: &ContentPainterCx<'_>, rect: Rect) {
    if let Some(params) = crate::canvas::navigator_gpu_view(app, idx, rect, ctx) {
        crate::gpu_canvas::GpuCanvas::paint(&painter.painter, rect, params);
        return;
    }
    painter.checker(rect);
    if crate::canvas::navigator_waits_for_gpu(app, idx, ctx) {
        return;
    }
    if let Some(texture) = crate::canvas::navigator_texture(app, ctx, idx) {
        painter.picture(texture, rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), 0.0);
    }
}

#[cfg(test)]
mod tests;
