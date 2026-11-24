//! A document's canvas pictures as the framework's raster displays read them: [`PhotoComposite`], the composite with
//! the live preview and the colour management folded in, and [`ChannelOverlay`], the Channels panel's view, which a
//! second display draws over the composite or the GPU picture.

use std::collections::VecDeque;
use std::sync::Arc;

use craft::behavior::PixelSize;
use craft::ui::canvas::{PixelRect, RasterSource};
use craft::ui::primitives::Revision;
use egui::{Color32, ColorImage};
use photocraft_engine::display_color::CanvasDisplay;

use super::channel_view::Overlay;
use super::display::{LiveProposal, cache_key, display_image};
use super::textures::{CanvasPicture, ProxyPreview, canvas_picture, damage_since};
use super::{Document, PhotocraftApp};
use photocraft_geom::Rect as DRect;

/// How many of the revisions a source handed out it can still tell damage since: a display's whole-document and
/// detail textures may each lag a few passes behind.
const LOG_LEN: usize = 8;

#[cfg(test)]
mod tests;

/// What a source's picture showed at one [`Revision`]: the state [`damage_since`] compares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Shown {
    /// The document's revision.
    revision: u64,
    /// The preview key with the source's display key folded in, as the CPU canvas caches keep it.
    key: u64,
    /// Where the picture lies in the document: the document's bounds, or a size-changing preview's own.
    bounds: DRect,
    /// The picture is a preview reduced for a zoomed-out view, in its own pixels.
    reduced: bool,
}

impl Shown {
    fn of(p: &CanvasPicture, display_key: u64) -> Self {
        Shown { revision: p.revision, key: p.preview_key ^ display_key, bounds: p.bounds, reduced: p.reduced }
    }

    fn revision(&self, id: photocraft_doc::DocId, view: u64) -> Revision {
        let b = self.bounds;
        Revision::of(&(id.0, self.revision, self.key, view, (b.x0, b.y0, b.x1, b.y1), self.reduced))
    }
}

/// What a [`ChannelOverlay`] showed: the picture it was drawn from and the channel view's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OverlayShown {
    picture: Shown,
    view: u64,
}

/// The revisions one source handed out on one display, newest last, so a display asking for the damage since the
/// revision its texture shows gets an answer.
pub(crate) struct SourceLog<S> {
    entries: VecDeque<(Revision, S)>,
}

impl<S> Default for SourceLog<S> {
    fn default() -> Self {
        SourceLog { entries: VecDeque::new() }
    }
}

impl<S: Copy> SourceLog<S> {
    fn record(&mut self, revision: Revision, shown: S) {
        if self.entries.back().is_some_and(|(r, _)| *r == revision) {
            return;
        }
        self.entries.retain(|(r, _)| *r != revision);
        if self.entries.len() == LOG_LEN {
            self.entries.pop_front();
        }
        self.entries.push_back((revision, shown));
    }

    fn find(&self, revision: Revision) -> Option<S> {
        self.entries.iter().rev().find(|(r, _)| *r == revision).map(|(_, s)| *s)
    }
}

/// One document's source logs on one display, kept per `(document, display)` in the app.
#[derive(Default)]
pub(crate) struct SourceLogs {
    composite: SourceLog<Shown>,
    overlay: SourceLog<OverlayShown>,
}

impl SourceLogs {
    /// The revision and preview key (display key folded in) of the composite the canvas showed last.
    #[cfg(test)]
    pub(crate) fn composite_seen(&self) -> Option<(u64, u64)> {
        self.composite.entries.back().map(|(_, shown)| (shown.revision, shown.key))
    }

    /// The pictures that showed a preview `was_preview` tells count as the committed document from now on, so the
    /// commit's damage refreshes only what it changed ([`super::textures::shown_as_document`]). The composite's keys
    /// carry the display's key `display_key`; the channel view's are raw.
    pub(crate) fn shown_as_document(&mut self, display_key: u64, was_preview: impl Fn(u64) -> bool) {
        for (_, shown) in &mut self.composite.entries {
            if was_preview(shown.key ^ display_key) {
                shown.key = display_key;
            }
        }
        for (_, shown) in &mut self.overlay.entries {
            if was_preview(shown.picture.key) {
                shown.picture.key = 0;
            }
        }
    }
}

/// The picture a source draws from this pass ([`canvas_picture`]) and what its damage reads.
struct Picture<'a> {
    app: &'a PhotocraftApp,
    idx: usize,
    doc: Arc<Document>,
    /// The document's revision and the raw preview key.
    now: (u64, u64),
    /// The key folded into `shown.key`: the display's for the composite, 0 for the channel view.
    display_key: u64,
    /// The document's damage span start and its last damage, grown by its layer effects' reach.
    last_damage: (u64, Option<DRect>),
    shown: Shown,
}

impl<'a> Picture<'a> {
    fn new(app: &'a PhotocraftApp, idx: usize, p: CanvasPicture, display_key: u64) -> Self {
        let shown = Shown::of(&p, display_key);
        Picture { app, idx, doc: p.doc, now: (p.revision, p.preview_key), display_key, last_damage: p.last_damage, shown }
    }

    fn size(&self) -> PixelSize {
        PixelSize::new(self.doc.size.width, self.doc.size.height)
    }

    /// What changed since a picture of this source showed `seen`.
    fn damage(&self, seen: Shown) -> Option<PixelRect> {
        // A reduced or moved picture is in other pixels than the document's damage.
        if seen.reduced || self.shown.reduced || seen.bounds != self.shown.bounds {
            return None;
        }
        let r = damage_since(self.app, self.idx, (seen.revision, seen.key), self.now, self.display_key, self.last_damage)?;
        let r = r.intersect(&self.doc.bounds());
        Some(if r.is_empty() { PixelRect::default() } else { PixelRect::new(r.x0, r.y0, r.x1, r.y1) })
    }

    /// Writes what `render` draws of `region` at `factor` into `out`. `render` gets the part of `region` inside the
    /// document, starting on a whole texel of `region`'s grid, and returns its row width in texels and its texels;
    /// texels before the document's start and past its end are left as they are.
    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage, render: impl FnOnce(DRect, u32) -> Option<(usize, Vec<Color32>)>) {
        let factor = factor.max(1);
        let f = i32::try_from(factor).unwrap_or(i32::MAX);
        let b = self.doc.bounds();
        let skip = |start: i32, lo: i32| if start >= lo { 0 } else { (lo - start + f - 1) / f };
        let (tx, ty) = (skip(region.x0, b.x0), skip(region.y0, b.y0));
        let area = DRect::new(region.x0 + tx * f, region.y0 + ty * f, region.x1.min(b.x1), region.y1.min(b.y1));
        if area.is_empty() {
            return;
        }
        let Some((w, px)) = render(area, factor) else { return };
        let h = px.len() / w.max(1);
        let (tx, ty) = (tx as usize, ty as usize);
        let [out_w, out_h] = out.size;
        for row in 0..h.min(out_h.saturating_sub(ty)) {
            let n = w.min(out_w.saturating_sub(tx));
            let at = (ty + row) * out_w + tx;
            out.pixels[at..at + n].copy_from_slice(&px[row * w..row * w + n]);
        }
    }
}

/// Document `idx`'s canvas picture on one display, for a `RasterDisplay`: the composite the CPU canvas shows (the
/// committed document, or the live adjustment, filter or tool preview), through the display's colour management.
/// Built once per pass; damage comes from the same records the CPU canvas cache reads ([`damage_since`]).
pub(crate) struct PhotoComposite<'a> {
    picture: Picture<'a>,
    display: Option<Arc<CanvasDisplay>>,
    revision: Revision,
    log: Option<&'a SourceLog<Shown>>,
}

/// A [`PhotoComposite`] prepared for this pass with the app borrowed mutably (the picture brought up to date, the
/// revision logged), to be read from once the app is borrowed shared.
pub(crate) struct CompositePrep {
    idx: usize,
    picture: CanvasPicture,
    display: Option<Arc<CanvasDisplay>>,
    revision: Revision,
    log_key: (photocraft_doc::DocId, u32),
}

impl CompositePrep {
    /// Document `idx`'s picture on `output` (`None`: the main window's display); `None` when no such document is open.
    /// `live` is the gradient a tool proposes this pass.
    pub(crate) fn new(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize, output: Option<u32>, live: LiveProposal<'_>) -> Option<Self> {
        let mut p = canvas_picture(app, ctx, idx, output, ProxyPreview::Document, live)?;
        let display = p.display.take();
        let shown = Shown::of(&p, p.display_key);
        let revision = shown.revision(p.id, 0);
        let log_key = cache_key(p.id, output);
        app.raster_logs.entry(log_key).or_default().composite.record(revision, shown);
        Some(CompositePrep { idx, picture: p, display, revision, log_key })
    }

    /// The source over `app`, the app this was prepared with.
    pub(crate) fn source(self, app: &PhotocraftApp) -> PhotoComposite<'_> {
        let log = app.raster_logs.get(&self.log_key).map(|l| &l.composite);
        let display_key = self.picture.display_key;
        PhotoComposite { picture: Picture::new(app, self.idx, self.picture, display_key), display: self.display, revision: self.revision, log }
    }
}

impl<'a> PhotoComposite<'a> {
    /// Where the picture lies in the document, in document pixels: the document's bounds, or a filter preview's own
    /// area when it is reduced or changes the canvas size (its pixels are then its own, `size` of them over this).
    pub(crate) fn bounds(&self) -> DRect {
        self.picture.shown.bounds
    }
}

impl RasterSource for PhotoComposite<'_> {
    fn size(&self) -> PixelSize {
        self.picture.size()
    }

    fn revision(&self) -> Revision {
        self.revision
    }

    fn damage(&self, since: Revision) -> Option<PixelRect> {
        if since == self.revision {
            return Some(PixelRect::default());
        }
        self.picture.damage(self.log?.find(since)?)
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let doc = &self.picture.doc;
        self.picture.read(region, factor, out, |area, factor| {
            let buf = photocraft_compose::render_reduced_by(doc, area, factor);
            let img = display_image(self.display.as_deref(), &buf);
            Some((img.size[0], img.pixels))
        });
    }
}

/// Document `idx`'s channel or mask view on one display (Quick Mask, a layer mask, alpha or single colour
/// channels), drawn by a second `RasterDisplay` over the canvas picture. It shows the live preview the canvas picture
/// shows, and also an adjustment's proxy preview, which the GPU picture draws and [`PhotoComposite`] does not.
/// Built once per pass, only while the view is not the plain composite.
pub(crate) struct ChannelOverlay<'a> {
    picture: Picture<'a>,
    overlay: Overlay,
    revision: Revision,
    log: Option<&'a SourceLog<OverlayShown>>,
}

/// A [`ChannelOverlay`] prepared for this pass with the app borrowed mutably, as [`CompositePrep`].
pub(crate) struct OverlayPrep {
    idx: usize,
    picture: CanvasPicture,
    overlay: Overlay,
    revision: Revision,
    log_key: (photocraft_doc::DocId, u32),
}

impl OverlayPrep {
    /// Document `idx`'s view on `output` (`None`: the main window's display); `None` when no such document is open or
    /// it shows the plain composite. `live` is the gradient a tool proposes this pass.
    pub(crate) fn new(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize, output: Option<u32>, live: LiveProposal<'_>) -> Option<Self> {
        let p = canvas_picture(app, ctx, idx, output, ProxyPreview::Preview, live)?;
        let overlay = Overlay::of(app, idx, &p.doc, output)?;
        let shown = OverlayShown { picture: Shown::of(&p, 0), view: overlay.key };
        let revision = shown.picture.revision(p.id, overlay.key);
        let log_key = cache_key(p.id, output);
        app.raster_logs.entry(log_key).or_default().overlay.record(revision, shown);
        Some(OverlayPrep { idx, picture: p, overlay, revision, log_key })
    }

    /// The source over `app`, the app this was prepared with.
    pub(crate) fn source(self, app: &PhotocraftApp) -> ChannelOverlay<'_> {
        let log = app.raster_logs.get(&self.log_key).map(|l| &l.overlay);
        ChannelOverlay { picture: Picture::new(app, self.idx, self.picture, 0), overlay: self.overlay, revision: self.revision, log }
    }
}

impl RasterSource for ChannelOverlay<'_> {
    fn size(&self) -> PixelSize {
        self.picture.size()
    }

    fn revision(&self) -> Revision {
        self.revision
    }

    fn damage(&self, since: Revision) -> Option<PixelRect> {
        if since == self.revision {
            return Some(PixelRect::default());
        }
        let seen = self.log?.find(since)?;
        // Another view, look or display transform draws everything differently.
        if seen.view != self.overlay.key {
            return None;
        }
        self.picture.damage(seen.picture)
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let doc = &self.picture.doc;
        self.picture.read(region, factor, out, |area, factor| {
            let w = area.width().div_ceil(factor) as usize;
            self.overlay.render(doc, area, factor).map(|px| (w, px))
        });
    }
}
