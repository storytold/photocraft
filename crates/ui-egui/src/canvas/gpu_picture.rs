//! [`GpuPicture`]: what the GPU canvas shows of a document this pass (a live preview's texture, viewport pages, or the
//! document's texture set), and its paint for a canvas view.

use std::sync::Arc;

use craft::behavior::CanvasView;
use craft::ui::canvas::AppPaint;

use super::display::{LiveProposal, hdr_preview, sync_display_lut};
use super::filter_preview::{ensure_adjust_proxy, ensure_filter_preview, ensure_proxy_preview, filter_preview_display_doc};
use super::textures::{ensure_gpu, preview_bounds, redraw_focus};
use super::{Document, PhotocraftApp, viewport_pages};
use crate::gpu_canvas::ViewParams;
use photocraft_geom::Rect as DRect;

#[cfg(test)]
mod tests;

/// What [`gpu_picture`] chooses from: document `idx` as a canvas shows it.
pub(crate) struct GpuRequest<'a> {
    pub ctx: &'a egui::Context,
    pub idx: usize,
    pub doc: &'a Arc<Document>,
    /// The display the canvas is on (`None`: the main window's).
    pub output: Option<u32>,
    /// The view on screen and the one it is animating to: redraws start with the blocks either shows.
    pub view: &'a CanvasView,
    pub target: &'a CanvasView,
    /// The gradient a tool proposes this pass, shown over the document.
    pub live: LiveProposal<'a>,
    /// Physical pixels per document pixel on screen, and the zoom live previews are reduced for.
    pub zoom: f32,
    pub render_zoom: f32,
}

/// Which texture a [`GpuPicture`] draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shows {
    /// A live adjustment, filter or proxy preview, `k` document pixels per texel.
    Preview,
    /// Viewport pages of a document too large for its texture set.
    Pages,
    /// The document's own texture set.
    Document,
}

/// The GPU canvas's picture of one document this pass: everything of its [`ViewParams`] but the view, which a canvas
/// gives when it paints. The caller checks `gpu_canvas::gpu_view_ok` first: the shader draws no mirrored or
/// non-square-pixel view.
#[derive(Clone)]
pub(crate) struct GpuPicture {
    params: ViewParams,
    shows: Shows,
    /// Document pixels per texel of a reduced preview (1 otherwise).
    k: u32,
    /// How far a size-changing preview grows past the reduced document on each side, in its texels: it grows around
    /// the old image centre, so the view keeps its pan.
    growth: [f32; 2],
    /// Where the picture lies in the document, in document pixels: a preview's own area (`preview_bounds`), else the
    /// document's bounds.
    bounds: DRect,
}

/// The GPU canvas's picture of document `req.idx`, after bringing its texture up to date this pass: a live preview
/// when one is shown, else the viewport pages when the document's texture set does not fit, else its texture set.
/// `None` when the GPU canvas cannot show it (no GPU canvas, a device fault), and the CPU canvas draws it.
pub(crate) fn gpu_picture(app: &mut PhotocraftApp, req: &GpuRequest<'_>) -> Option<GpuPicture> {
    let (ctx, idx, doc, output) = (req.ctx, req.idx, req.doc, req.output);
    let doc_size = [doc.size.width, doc.size.height];
    let base = |doc_key: u64, size: [u32; 2], view_key: u64, display: u8, hdr: Option<[f32; 2]>| ViewParams {
        doc: doc_key,
        doc_size: size,
        zoom: 1.0,
        center: [0.0, 0.0],
        rotation: 0.0,
        view_key,
        display,
        output: output.unwrap_or(0),
        hdr,
    };
    // Live adjustment previews on big documents use a downsampled proxy (see proxy.rs), unless Preferences ›
    // Performance › Low Resolution Previews is off.
    if app.gpu.is_some()
        && let Some((k, key, preview_size)) = ensure_adjust_proxy(app, idx, req.render_zoom)
            .or_else(|| ensure_filter_preview(app, idx, ctx))
            .or_else(|| ensure_proxy_preview(app, idx, req.render_zoom))
    {
        app.navigator_gpu.remove(&doc.id);
        let k = k.max(1);
        let reduced = [doc.size.width.div_ceil(k), doc.size.height.div_ceil(k)];
        let shown = filter_preview_display_doc(app, doc, key);
        let display = sync_display_lut(app, &shown, key, output);
        let view_key = egui::Id::unique(("pc-canvas-proxy", ctx.viewport_id(), idx)).value();
        let params = base(key, preview_size, view_key, display, hdr_preview(app, doc));
        let growth = [(preview_size[0] as f32 - reduced[0] as f32) / 2.0, (preview_size[1] as f32 - reduced[1] as f32) / 2.0];
        let bounds = preview_bounds(doc.size, photocraft_geom::Size::new(preview_size[0], preview_size[1]), k);
        return Some(GpuPicture { params, shows: Shows::Preview, k, growth, bounds });
    }
    let plain = |params: ViewParams, shows: Shows| GpuPicture { params, shows, k: 1, growth: [0.0, 0.0], bounds: doc.bounds() };
    if viewport_pages::show(app, ctx, idx, req.view, req.zoom, req.live) {
        app.perf.gpu = true;
        let key = viewport_pages::stream_key(doc.id.0);
        let view_key = egui::Id::unique(("pc-canvas-pages", ctx.viewport_id(), doc.id.0)).value();
        let display = sync_display_lut(app, doc, key, output);
        return Some(plain(base(key, doc_size, view_key, display, hdr_preview(app, doc)), Shows::Pages));
    }
    if ensure_gpu(app, ctx, idx, redraw_focus(req.target, req.view), req.live) {
        app.perf.gpu = true;
        let view_key = egui::Id::unique(("pc-canvas", ctx.viewport_id(), idx)).value();
        let display = sync_display_lut(app, doc, doc.id.0, output);
        return Some(plain(base(doc.id.0, doc_size, view_key, display, hdr_preview(app, doc)), Shows::Document));
    }
    None
}

impl GpuPicture {
    /// Where the picture lies in the document, in document pixels: a reduced or size-changing preview's own area, else
    /// the document's bounds.
    pub(crate) fn bounds(&self) -> DRect {
        self.bounds
    }

    /// Whether it draws the document's own texture set, which the Navigator can draw again at its size.
    pub(crate) fn shows_document(&self) -> bool {
        self.shows == Shows::Document
    }

    /// The shader's parameters for `view`: its zoom and centre in the texture's texels, and its rotation.
    pub(crate) fn params(&self, view: &CanvasView) -> ViewParams {
        let k = self.k as f32;
        let rotation = if view.rotation.is_finite() { (view.rotation as f32).to_radians() } else { 0.0 };
        ViewParams {
            zoom: view.zoom as f32 * k,
            center: [view.center.x as f32 / k + self.growth[0], view.center.y as f32 / k + self.growth[1]],
            rotation,
            ..self.params
        }
    }

    /// The picture as the app's paint of a framework `RasterDisplay`, which calls it once for the document and once
    /// for each View › Pattern Preview copy, with the view shifted to the copy. Each call is its own draw, keyed by
    /// its place in the pass, so the copies reuse the same few uniform buffers as the view pans.
    pub(crate) fn into_paint<'a>(self) -> AppPaint<'a, CanvasView> {
        let mut n = 0u64;
        AppPaint::new(move |cx, view: CanvasView| {
            let key = if n == 0 { self.params.view_key } else { egui::Id::unique((self.params.view_key, "copy", n)).value() };
            n += 1;
            crate::gpu_canvas::GpuCanvas::paint(&cx.painter, view.rect, ViewParams { view_key: key, ..self.params(&view) });
        })
    }
}
