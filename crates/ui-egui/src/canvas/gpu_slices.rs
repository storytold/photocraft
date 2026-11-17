//! A full GPU refresh of a very large canvas, a slice per frame. Compositing a 1.1 GP storyboard
//! in one frame took 1.0–1.9 s (on opening it, on switching back to its tab, after moving layers
//! or undoing), and the desktop's compositor closes a window that stops answering for about
//! 1.5–2 s: the app was closed while it reopened the file after a crash. From [`SLICE_FROM_PX`] up,
//! the canvas is composited in bands of about [`BAND_PX`], the bands the view shows first, until a
//! frame has spent [`FRAME_MS`]; the rest follow on the next frame while the window keeps
//! answering. The budget is large on purpose: a frame's bands cost about 100 ms before their pixels,
//! and small bands at 120 ms a frame took 4–6 s to fill the storyboard the compositor drew in one
//! 1.0–1.6 s frame. Now a refresh the compositor finished in one frame under the budget still does,
//! and a longer one takes two frames of about a second, close to its old speed.
//! An edit whose damage is that large (undoing a move of a board, say) is drawn the same way.

use std::collections::VecDeque;

use photocraft_doc::DocId;
use photocraft_geom::Rect;

use super::{Document, PhotocraftApp};
use crate::gpu_canvas::GpuCanvas;

/// Canvases of this many pixels or more refresh in slices (about 134 MP).
pub(crate) const SLICE_FROM_PX: u64 = 1 << 27;
/// About this many pixels a band (about 134 MP, some 150 ms on the storyboard).
const BAND_PX: u64 = 1 << 27;
/// Bands are composited until a frame has spent this long on them (at least one band a frame): with
/// the band that crosses it, a frame stays near a second, short of the 1.5–2 s after which the
/// desktop closes a window that isn't answering.
const FRAME_MS: f64 = 800.0;

/// The bands of `bounds`, full rows of about [`BAND_PX`] each: those meeting `visible` first, the
/// nearest to its middle first, then the rest by their distance from it.
pub(crate) fn bands(bounds: Rect, visible: Rect) -> VecDeque<Rect> {
    if bounds.is_empty() {
        return VecDeque::new();
    }
    let rows = (BAND_PX / u64::from(bounds.width())).clamp(1, u64::from(bounds.height())) as i32;
    let mut out = Vec::new();
    let mut y = bounds.y0;
    while y < bounds.y1 {
        let y1 = y.saturating_add(rows).min(bounds.y1);
        out.push(Rect::new(bounds.x0, y, bounds.x1, y1));
        y = y1;
    }
    let middle = i64::from(visible.y0) + i64::from(visible.y1);
    out.sort_by_key(|b| (b.intersect(&visible).is_empty(), (i64::from(b.y0) + i64::from(b.y1) - middle).abs()));
    out.into()
}

/// Starts refreshing all of `doc` in slices, in place of one full refresh. The bands start on the
/// next frame, so this one shows the window as it now is (a switched tab, the zoom it shows) without
/// waiting for them. `None` (nothing started) when the canvas is small enough to refresh at once,
/// or the wgpu compositor doesn't draw it (still being built, say: the CPU compositor then redraws
/// in the background), or its texture can't be made: the ordinary refresh decides. A refresh in
/// slices still running for document `id` starts over.
pub(crate) fn start(app: &mut PhotocraftApp, ctx: &egui::Context, gpu: &GpuCanvas, (id, doc): (DocId, &Document), visible: Rect) -> Option<()> {
    app.gpu_slices.remove(&id);
    if !sliced(gpu, (id, doc), doc.bounds()) {
        return None;
    }
    app.gpu_slices.insert(id, bands(doc.bounds(), visible.intersect(&doc.bounds())));
    ctx.request_repaint();
    Some(())
}

/// Starts refreshing an edit's `damage` in slices, in place of one partial refresh, when it covers
/// [`SLICE_FROM_PX`] or more: undoing a move of a storyboard's board redrew 550–690 MP in one frame
/// of 1.0–1.5 s, three undos in a row, and the desktop closed the window. Its bands go first, the
/// view's first, then those of a refresh in slices already running that it doesn't cover. Like
/// [`start`], from the next frame; `None` when the ordinary refresh draws it.
pub(crate) fn start_damage(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    gpu: &GpuCanvas,
    (id, doc): (DocId, &Document),
    damage: Rect,
    visible: Rect,
) -> Option<()> {
    let damage = damage.intersect(&doc.bounds());
    if !sliced(gpu, (id, doc), damage) {
        return None;
    }
    let old = app.gpu_slices.remove(&id);
    app.gpu_slices.insert(id, queue_damage(old, damage, visible));
    ctx.request_repaint();
    Some(())
}

/// Whether a refresh of `area` of `doc` goes in slices: [`SLICE_FROM_PX`] or more, and drawn by
/// the wgpu compositor into a texture it could make.
fn sliced(gpu: &GpuCanvas, (id, doc): (DocId, &Document), area: Rect) -> bool {
    let px = u64::from(area.width()) * u64::from(area.height());
    px >= SLICE_FROM_PX && doc.id == id && !gpu.compositor_building() && gpu.supports(doc) && gpu.prepare_texture(id.0, doc).is_some()
}

/// The bands to draw after an edit's `damage`: its own, those meeting `visible` first, then the
/// bands of the running refresh (`old`) not wholly inside it.
pub(crate) fn queue_damage(old: Option<VecDeque<Rect>>, damage: Rect, visible: Rect) -> VecDeque<Rect> {
    let mut queue = bands(damage, visible.intersect(&damage));
    queue.extend(old.into_iter().flatten().filter(|b| !damage.contains_rect(b)));
    queue
}

/// Whether document `id` still has bands to draw.
pub(crate) fn pending(app: &PhotocraftApp, id: DocId) -> bool {
    app.gpu_slices.get(&id).is_some_and(|q| !q.is_empty())
}

/// Draws the next bands of document `id`'s refresh in slices, as `doc` is now, for about
/// [`FRAME_MS`] and asks for another frame while some are left. False when the GPU canvas can't
/// draw them (lost, or over its budget): the refresh in slices ends and this frame goes to the
/// CPU canvas.
pub(crate) fn step(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    gpu: &GpuCanvas,
    (id, doc): (DocId, &Document),
    display: Option<&photocraft_engine::display_color::CanvasDisplay>,
) -> bool {
    let start = crate::gpu_canvas::now_ms();
    while let Some(band) = app.gpu_slices.get_mut(&id).and_then(VecDeque::pop_front) {
        let r = gpu.refresh(id.0, doc, Some(band), display);
        if r.kind == "lost" || r.kind == crate::gpu_canvas::REFRESH_OVER_BUDGET {
            app.gpu_slices.remove(&id);
            if let Some(reason) = r.fallback {
                crate::render_path::note(app, id.0, crate::render_path::Path::Cpu(reason));
            }
            return false;
        }
        app.perf.record_refresh(&r);
        if crate::gpu_canvas::now_ms() - start >= FRAME_MS {
            break;
        }
    }
    if pending(app, id) {
        ctx.request_repaint();
    } else {
        app.gpu_slices.remove(&id);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_cover_the_canvas_once_the_visible_ones_first() {
        let bounds = Rect::new(0, 0, 40_000, 30_000);
        let visible = Rect::new(10_000, 20_000, 12_000, 21_000);
        let bands = bands(bounds, visible);
        let rows: u64 = bands.iter().map(|b| u64::from(b.height())).sum();
        assert_eq!(rows, 30_000, "every row once");
        assert!(bands.iter().all(|b| b.x0 == 0 && b.x1 == 40_000 && u64::from(b.width()) * u64::from(b.height()) <= BAND_PX));
        assert!(!bands[0].intersect(&visible).is_empty(), "the view's band first: {:?}", bands[0]);
        let first_hidden = bands.iter().position(|b| b.intersect(&visible).is_empty()).unwrap();
        assert!(bands.iter().skip(first_hidden).all(|b| b.intersect(&visible).is_empty()), "then the rest");
    }

    #[test]
    fn a_narrow_tall_or_empty_canvas_still_bands() {
        assert!(bands(Rect::new(0, 0, 0, 0), Rect::new(0, 0, 0, 0)).is_empty());
        let one = bands(Rect::new(0, 0, 100, 100), Rect::new(0, 0, 100, 100));
        assert_eq!(Vec::from(one), [Rect::new(0, 0, 100, 100)]);
        // Wider than a band: one row at a time.
        let wide = bands(Rect::new(0, 0, 200_000_000, 3), Rect::new(0, 0, 10, 1));
        assert_eq!(wide.len(), 3);
    }

    #[test]
    fn an_edit_s_bands_go_before_the_rest_of_a_running_refresh() {
        let damage = Rect::new(0, 0, 20_000, 20_000);
        let old: VecDeque<Rect> = [Rect::new(0, 0, 40_000, 3_000), Rect::new(0, 30_000, 40_000, 33_000), Rect::new(0, 0, 10_000, 10_000)].into();
        let q = queue_damage(Some(old), damage, Rect::new(0, 15_000, 1_000, 16_000));
        let own = q.iter().take_while(|b| damage.contains_rect(b)).count();
        let rows: u64 = q.iter().take(own).map(|b| u64::from(b.height())).sum();
        assert_eq!(rows, 20_000, "the edit's bands first, every row once");
        assert!(q[0].y0 <= 15_000 && q[0].y1 > 15_000, "the view's band first: {:?}", q[0]);
        // Of the running refresh: the bands the edit doesn't cover, still in order.
        assert_eq!(Vec::from(q).split_off(own), [Rect::new(0, 0, 40_000, 3_000), Rect::new(0, 30_000, 40_000, 33_000)]);
        assert!(queue_damage(None, Rect::new(0, 0, 0, 0), Rect::new(0, 0, 0, 0)).is_empty());
    }
}
