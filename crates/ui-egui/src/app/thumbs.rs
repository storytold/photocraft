use std::cell::OnceCell;

// PLAN: assumed `craft::behavior::PixelSize` (behavior/src/pixel_size.rs).
use craft::behavior::PixelSize;
use craft::ui::primitives::{Revision, ThumbRender, ThumbRequest, ThumbSource, thumb_image as frame_image};

use super::{DocId, Document, PhotocraftApp};
use crate::channels_panel::Row;

/// The History panel's snapshot thumbnails (#2966): the composite of each snapshot of one document. A snapshot's
/// document never changes; a preserved-ID reopen or the refreshed first snapshot has a new `Arc` (#6087), which
/// the revision follows.
pub(crate) struct SnapshotThumbs<'a> {
    doc: DocId,
    snapshots: &'a [photocraft_engine::history_cmds::Snapshot],
}

impl<'a> SnapshotThumbs<'a> {
    /// The thumbnails of the snapshots of document `doc` (none when it is not open).
    pub(crate) fn new(app: &'a PhotocraftApp, doc: DocId) -> Self {
        let snapshots = app.session.documents().iter().find(|st| st.doc.id == doc).map_or(&[][..], |st| st.snapshots());
        Self { doc, snapshots }
    }
}

impl ThumbSource<u64> for SnapshotThumbs<'_> {
    fn revision(&self, key: &u64, _request: &ThumbRequest) -> Revision {
        let shown = self.snapshots.iter().find(|s| s.id == *key).map(|s| std::sync::Arc::as_ptr(&s.doc) as usize);
        Revision::of(&(self.doc.0, *key, shown))
    }

    fn render(&self, key: &u64, request: &ThumbRequest) -> ThumbRender {
        let Some(snap) = self.snapshots.iter().find(|s| s.id == *key) else { return ThumbRender::Blank };
        let comp = photocraft_compose::thumbnail(&snap.doc, request.side);
        ThumbRender::Ready(egui::ColorImage::from_rgba_unmultiplied([comp.width as usize, comp.height as usize], &comp.pixels))
    }
}

/// The document area a layer thumbnail frames: the canvas, or with `clip` the layer's pixels on
/// it (the canvas again when it has none there, so an empty layer still shows the page shape).
pub(crate) fn layer_thumb_frame(layer: &photocraft_doc::Layer, canvas: photocraft_geom::Rect, clip: bool) -> photocraft_geom::Rect {
    if !clip {
        return canvas;
    }
    let bounds = layer.surface().map_or(photocraft_geom::Rect::EMPTY, photocraft_compose::bounds::content_bounds).intersect(&canvas);
    if bounds.is_empty() { canvas } else { bounds }
}

impl PhotocraftApp {
    /// Cached `Surface::content_bounds` keyed by an id and the surface's generation.
    pub fn cached_bounds(&mut self, key: u64, surface: &photocraft_raster::Surface) -> photocraft_geom::Rect {
        let rev = surface.generation();
        if let Some((r, b)) = self.bounds_cache.get(&key)
            && *r == rev
        {
            return *b;
        }
        let b = surface.content_bounds();
        if self.bounds_cache.len() > 256 {
            self.bounds_cache.clear();
        }
        self.bounds_cache.insert(key, (rev, b));
        b
    }

    /// [`Self::cached_bounds`] where only a shared borrow is at hand: the cached box if `surface`
    /// hasn't changed since, else a scan (not cached).
    pub fn known_bounds(&self, key: u64, surface: &photocraft_raster::Surface) -> photocraft_geom::Rect {
        match self.bounds_cache.get(&key) {
            Some((r, b)) if *r == surface.generation() => *b,
            _ => surface.content_bounds(),
        }
    }

    /// The box Properties › Transform shows for `layer` (X, Y, W, H): its pixels' bounds, but a
    /// soft shape's path and stroke, whose Density and Feather render past them (#5846), as its
    /// transform frame does.
    pub fn layer_frame_bounds(&mut self, layer: &photocraft_doc::Layer, surface: &photocraft_raster::Surface) -> photocraft_geom::Rect {
        match &layer.content {
            photocraft_doc::LayerContent::Shape(sh) if sh.has_soft_mask() => {
                // Cached per rendering, like the pixel bounds: the frame renders strips of the shape.
                let (key, rev) = (layer.id.0 | 1 << 63, surface.generation());
                if let Some((r, b)) = self.bounds_cache.get(&key)
                    && *r == rev
                {
                    return *b;
                }
                let canvas = self.session.active().map_or_else(|| surface.content_bounds(), |st| st.doc.bounds());
                let b = photocraft_compose::soft_shape_frame(sh, surface.format(), canvas).unwrap_or(photocraft_geom::Rect::EMPTY);
                if self.bounds_cache.len() > 256 {
                    self.bounds_cache.clear();
                }
                self.bounds_cache.insert(key, (rev, b));
                b
            }
            _ => self.cached_bounds(layer.id.0, surface),
        }
    }
}

/// The document's composite reduced to thumbnail size, with the document's own inks when it is CMYK: made once per
/// pass and shared by every composite and colour row (a full composite at 56 px costs ~0.7 s on a layered 20 MP
/// document, #4518).
struct Reduced {
    pixels: Vec<u8>,
    w: usize,
    h: usize,
    inks: Option<Vec<[f32; 4]>>,
}

impl Reduced {
    fn of(doc: &Document) -> Reduced {
        // `thumbnail(&doc, 56)` with its proxy kept, so the ink thumbnails sample the same proxy instead of building another.
        let proxy = photocraft_compose::thumbnail_proxy(doc, 56);
        let src = proxy.as_ref().unwrap_or(doc);
        let (tw, th) = photocraft_compose::thumbnail_size(doc, 56);
        let comp = photocraft_compose::render_reduced(src, tw, th).to_rgba8();
        let (w, h) = (comp.width as usize, comp.height as usize);
        let inks = (doc.pixel_format().mode == photocraft_doc::ColorMode::Cmyk).then(|| match &proxy {
            Some(proxy) => ink_thumbnail(proxy, w, h),
            None => match ink_proxy(doc, 56) {
                Some(proxy) => ink_thumbnail(&proxy, w, h),
                None => ink_thumbnail(doc, w, h),
            },
        });
        Reduced { pixels: comp.pixels, w, h, inks }
    }

    /// The index of the reduced pixel under document pixel `(x, y)` of a `dw`×`dh` document.
    fn at(&self, x: u32, y: u32, dw: u32, dh: u32) -> usize {
        let rx = (x as usize * self.w / dw as usize).min(self.w - 1);
        let ry = (y as usize * self.h / dh as usize).min(self.h - 1);
        ry * self.w + rx
    }
}

/// The Channels panel's pictures of one document, made per pass: the source the framework thumbnail store asks.
pub(crate) struct ChannelThumbs<'a> {
    st: &'a photocraft_engine::DocState,
    show_color: bool,
    reduced: OnceCell<Reduced>,
}

impl<'a> ChannelThumbs<'a> {
    /// The thumbnails of `doc`, tinted as the preferences' Show Channels in Color says.
    pub(crate) fn new(app: &'a PhotocraftApp, doc: &'a photocraft_engine::DocState) -> Self {
        Self { st: doc, show_color: app.session.prefs().interface.show_channels_in_color, reduced: OnceCell::new() }
    }

    fn reduced(&self) -> &Reduced {
        self.reduced.get_or_init(|| Reduced::of(&self.st.doc))
    }

    /// The surface a mask, alpha or Quick Mask row shows.
    fn surface(&self, key: &Row) -> Option<&photocraft_raster::Surface> {
        let doc = &self.st.doc;
        match *key {
            Row::Alpha(i) => doc.channels.get(i).map(|c| &c.surface),
            Row::QuickMask => doc.quick_mask.as_ref().map(|c| &c.surface),
            Row::LayerMask => self.st.active_layer.and_then(|id| doc.layer(id)).and_then(|l| l.mask.as_ref()).map(|m| &m.surface),
            Row::Composite | Row::Color(_) => None,
        }
    }
}

/// The pixel rect `(x0, y0, x1, y1)` of a `w`×`h` frame that `region` (fractions of it) names, at least one pixel each way.
fn pixel_span(region: Option<egui::Rect>, w: u32, h: u32) -> (u32, u32, u32, u32) {
    let Some(r) = region else { return (0, 0, w, h) };
    let lo = |v: f32, n: u32| ((v.max(0.0) * n as f32).floor() as u32).min(n - 1);
    let hi = |v: f32, n: u32, from: u32| ((v.max(0.0) * n as f32).ceil() as u32).clamp(from + 1, n);
    let (x0, y0) = (lo(r.min.x, w), lo(r.min.y, h));
    (x0, y0, hi(r.max.x, w, x0), hi(r.max.y, h, y0))
}

impl ThumbSource<Row> for ChannelThumbs<'_> {
    fn revision(&self, key: &Row, _request: &ThumbRequest) -> Revision {
        let doc = &self.st.doc;
        let size = (doc.size.width, doc.size.height);
        match key {
            Row::Composite | Row::Color(_) => Revision::of(&(doc.id.0, self.st.pixel_revision, self.show_color, size)),
            // A channel-list change with empty damage still refreshes these: they follow their own surface.
            _ => Revision::of(&(doc.id.0, self.st.active_layer, self.surface(key).map(photocraft_raster::Surface::generation), size)),
        }
    }

    fn render(&self, key: &Row, request: &ThumbRequest) -> ThumbRender {
        let doc = &self.st.doc;
        let (dw, dh) = (doc.size.width.max(1), doc.size.height.max(1));
        let (x0, y0, x1, y1) = pixel_span(request.region, dw, dh);
        let frame = PixelSize::new(x1 - x0, y1 - y0);
        let image = match *key {
            Row::Composite => {
                let r = self.reduced();
                frame_image(frame, request.side, |x, y| {
                    let i = r.at(x0 + x, y0 + y, dw, dh) * 4;
                    [r.pixels[i], r.pixels[i + 1], r.pixels[i + 2], 255]
                })
            }
            Row::Color(k) => {
                let r = self.reduced();
                let fmt = doc.pixel_format();
                frame_image(frame, request.side, |x, y| {
                    let i = r.at(x0 + x, y0 + y, dw, dh);
                    let g = match &r.inks {
                        // Ink channels read dark where there is ink.
                        Some(inks) => 1.0 - inks[i][k],
                        None => {
                            let p = &r.pixels[i * 4..i * 4 + 4];
                            photocraft_raster::from_rgba(&fmt, [p[0], p[1], p[2], 255].map(|v| f32::from(v) / 255.0))[k]
                        }
                    };
                    let byte = (g.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    channel_tint(fmt.mode, k, byte, self.show_color).to_array()
                })
            }
            Row::Alpha(_) | Row::QuickMask | Row::LayerMask => {
                let Some(surface) = self.surface(key) else { return ThumbRender::Blank };
                frame_image(frame, request.side, |x, y| {
                    let v = surface.sample_channel((x0 + x) as i32, (y0 + y) as i32, 0);
                    let g = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    [g, g, g, 255]
                })
            }
        };
        ThumbRender::Ready(image)
    }
}

/// The inks of a CMYK document's `w`×`h` channel thumbnails, row by row: the document's own
/// inks, not the RGB thumbnail separated again (#8359). Sampled at the thumbnail's pixel centres
/// from `src`: the document, or on large documents the proxy its composite thumbnail is reduced
/// from ([`photocraft_compose::thumbnail_proxy`]).
fn ink_thumbnail(src: &Document, w: usize, h: usize) -> Vec<[f32; 4]> {
    let (sw, sh) = (src.size.width.max(1) as f32, src.size.height.max(1) as f32);
    let xs: Vec<i32> = (0..w).map(|x| ((x as f32 + 0.5) * sw / w as f32) as i32).collect();
    let ys: Vec<i32> = (0..h).map(|y| ((y as f32 + 0.5) * sh / h as f32) as i32).collect();
    crate::canvas::channel_view::ink_samples(src, &xs, &ys)
}

/// A proxy the ink thumbnails can sample when the composite thumbnail has none: up to
/// `PROXY_THUMBNAIL_PIXELS` the composite reduces the document itself, which its one-colour
/// areas keep cheap, but compositing the inks of the whole document in bands costs ~170 ms at
/// 16 MP on every refresh. `None` when the proxy wouldn't composite the same (layer effects,
/// vector or feathered masks, uncached pattern fills: [`crate::canvas::channel_view::proxy_matches`])
/// or the document is too small to need one.
fn ink_proxy(doc: &Document, max_side: u32) -> Option<Document> {
    let b = doc.bounds();
    let k = b.width().max(b.height()).max(1) / max_side.max(1).saturating_mul(2);
    (k >= 2 && crate::canvas::channel_view::proxy_matches(doc)).then(|| photocraft_compose::proxy::proxy_document(doc, k))
}

/// A channel thumbnail pixel whose lightness is `byte` (255 = white: full light, no ink).
/// Preferences ▸ Interface ▸ Show Channels in Color tints RGB channels from black to their
/// primary and CMYK channels from white to their ink, as Photoshop does; otherwise grey.
fn channel_tint(mode: photocraft_doc::ColorMode, k: usize, byte: u8, in_color: bool) -> egui::Color32 {
    use photocraft_doc::ColorMode::{Cmyk, Rgb};
    if !in_color {
        return egui::Color32::from_gray(byte);
    }
    match (mode, k) {
        (Rgb, 0) => egui::Color32::from_rgb(byte, 0, 0),
        (Rgb, 1) => egui::Color32::from_rgb(0, byte, 0),
        (Rgb, 2) => egui::Color32::from_rgb(0, 0, byte),
        (Cmyk, 0) => egui::Color32::from_rgb(byte, 255, 255),
        (Cmyk, 1) => egui::Color32::from_rgb(255, byte, 255),
        (Cmyk, 2) => egui::Color32::from_rgb(255, 255, byte),
        _ => egui::Color32::from_gray(byte),
    }
}

#[cfg(test)]
mod channel_tint_tests {
    use super::channel_tint;
    use egui::Color32;
    use photocraft_doc::ColorMode::{Cmyk, Grayscale, Rgb};

    #[test]
    fn channels_in_color_tint_rgb_from_black_and_cmyk_inks_from_white() {
        assert_eq!(channel_tint(Rgb, 0, 255, true), Color32::from_rgb(255, 0, 0));
        assert_eq!(channel_tint(Rgb, 2, 0, true), Color32::BLACK);
        // CMYK: no ink is white, full ink is the ink's colour.
        assert_eq!(channel_tint(Cmyk, 0, 255, true), Color32::WHITE);
        assert_eq!(channel_tint(Cmyk, 0, 0, true), Color32::from_rgb(0, 255, 255));
        assert_eq!(channel_tint(Cmyk, 1, 0, true), Color32::from_rgb(255, 0, 255));
        assert_eq!(channel_tint(Cmyk, 2, 0, true), Color32::from_rgb(255, 255, 0));
        assert_eq!(channel_tint(Cmyk, 3, 0, true), Color32::BLACK);
        assert_eq!(channel_tint(Grayscale, 0, 77, true), Color32::from_gray(77));
        assert_eq!(channel_tint(Rgb, 0, 77, false), Color32::from_gray(77));
    }
}

#[cfg(test)]
mod ink_thumbnail_tests {
    use super::ink_thumbnail;
    use photocraft_engine::Session;
    use serde_json::json;

    /// Rich black 100/100/100/0 keeps its inks in the channel thumbnails: no K (#8359).
    #[test]
    fn cmyk_channel_thumbnails_show_the_document_inks() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 120, "height": 60, "mode": "cmyk"})).unwrap();
        s.execute("tools.setColors", json!({"foregroundCmyk": [1.0, 1.0, 1.0, 0.0]})).unwrap();
        s.execute("edit.fillForeground", json!({})).unwrap();
        let doc = &s.active().unwrap().doc;
        let (w, h) = photocraft_compose::thumbnail_size(doc, 56);
        let inks = ink_thumbnail(doc, w as usize, h as usize);
        assert_eq!(inks.len(), (w * h) as usize);
        assert!(inks.iter().all(|p| p.iter().zip([1.0, 1.0, 1.0, 0.0]).all(|(a, b)| (a - b).abs() < 0.01)), "{:?}", inks[0]);
    }

    /// Below `PROXY_THUMBNAIL_PIXELS` the inks still come from a proxy when it composites the
    /// same, and give the same inks as the full document; with a feathered mask or a layer
    /// effect they don't.
    #[test]
    fn mid_size_cmyk_ink_thumbnails_use_a_proxy_only_when_faithful() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 2400, "height": 1600, "mode": "cmyk"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 1200, "height": 1600})).unwrap();
        s.execute("tools.setColors", json!({"foregroundCmyk": [1.0, 1.0, 1.0, 0.0]})).unwrap();
        s.execute("edit.fillForeground", json!({})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let doc = s.active().unwrap().doc.clone();
        assert!(photocraft_compose::thumbnail_proxy(&doc, 56).is_none(), "3.8 MP: the composite reduces the document");
        let proxy = super::ink_proxy(&doc, 56).expect("plain layers: the inks sample a proxy");
        let (w, h) = photocraft_compose::thumbnail_size(&doc, 56);
        let (w, h) = (w as usize, h as usize);
        assert_eq!(ink_thumbnail(&proxy, w, h), ink_thumbnail(&doc, w, h));
        // A feather is in document pixels, which the proxy doesn't scale.
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.execute("layer.layerMask.edit", json!({"feather": 4})).unwrap();
        assert!(super::ink_proxy(&s.active().unwrap().doc, 56).is_none(), "a feathered mask: no proxy");
        s.execute("layer.layerMask.edit", json!({"feather": 0})).unwrap();
        assert!(super::ink_proxy(&s.active().unwrap().doc, 56).is_some(), "the feather gone: the proxy again");
        s.execute("layer.layerStyle.dropShadow", json!({})).unwrap();
        assert!(super::ink_proxy(&s.active().unwrap().doc, 56).is_none(), "a layer effect: no proxy");
    }

    /// Above `PROXY_THUMBNAIL_PIXELS` the inks come from the thumbnail's proxy and still land on
    /// the right side: rich black on the left half, paper on the right.
    #[test]
    fn large_cmyk_channel_thumbnails_read_the_proxy() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4200, "height": 4200, "mode": "cmyk"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 2100, "height": 4200})).unwrap();
        s.execute("tools.setColors", json!({"foregroundCmyk": [1.0, 1.0, 1.0, 0.0]})).unwrap();
        s.execute("edit.fillForeground", json!({})).unwrap();
        let doc = &s.active().unwrap().doc;
        let proxy = photocraft_compose::thumbnail_proxy(doc, 56).expect("a 17.6 MP document thumbnails from a proxy");
        let (w, h) = photocraft_compose::thumbnail_size(doc, 56);
        let inks = ink_thumbnail(&proxy, w as usize, h as usize);
        let at = |x: u32, y: u32| inks[(y * w + x) as usize];
        let near = |p: [f32; 4], q: [f32; 4]| p.iter().zip(q).all(|(a, b)| (a - b).abs() < 0.01);
        assert!(near(at(5, h / 2), [1.0, 1.0, 1.0, 0.0]), "{:?}", at(5, h / 2));
        assert!(near(at(w - 5, h / 2), [0.0; 4]), "{:?}", at(w - 5, h / 2));
    }
}
