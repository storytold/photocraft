//! A PDF page drawn the way another application placed it as a smart object (#15650).
//!
//! Photoshop stores a placed PDF's frame (`Sz  `): the box the page was cropped to, in the
//! units the layer's corners map from. Its default crop for Illustrator artwork is the
//! artwork's bounding box, not the page. The page is drawn on transparency, cropped to the box
//! whose proportions match the frame (the artwork's bounds or the whole page; when both do, the
//! one whose size in points matches the frame at its stored resolution, `Rslt`) and scaled to
//! fill the frame exactly. Other page boxes (trim, bleed, art) are not tried.

use craft_pdf::raster::{self, RenderCache};
use craft_pdf::syntax::page::Page;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::Size;
use photocraft_raster::{Interrupt, Surface};

use super::{page_size, render_error, render_page, write_raster};
use crate::IoError;
use crate::smart_map::PlacedPdf;

/// The longer side of the render that finds the artwork's bounds, in pixels.
const PROBE_SIDE: f64 = 2048.0;

/// A box on the page in points, top-left origin as rendered.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Box2 {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Box2 {
    fn width(self) -> f64 {
        self.x1 - self.x0
    }
    fn height(self) -> f64 {
        self.y1 - self.y0
    }
}

/// The placed page of `bytes`, cropped as it was placed and drawn into a transparent frame (the
/// stored `Sz  `, rounded up to whole pixels). An error when the PDF can't be drawn or neither
/// the artwork's bounds nor the page has the frame's proportions: the caller then keeps the
/// pixels the file was saved with.
pub fn import_placed(name: &str, bytes: &[u8], placed: &PlacedPdf, ctl: &Interrupt) -> Result<Document, IoError> {
    let (frame_w, frame_h) = placed.frame;
    if !frame_w.is_finite() || !frame_h.is_finite() || frame_w < 1.0 || frame_h < 1.0 {
        return Err(IoError::Unsupported("the placed PDF's frame has no area".into()));
    }
    let (width, height) = (frame_w.ceil() as u32, frame_h.ceil() as u32);
    if width > raster::MAX_SIDE || height > raster::MAX_SIDE || u64::from(width) * u64::from(height) > raster::MAX_PIXELS {
        return Err(IoError::Unsupported("the placed PDF's frame is too large to draw".into()));
    }
    let pdf = craft_pdf::open_slice_with_options(bytes, None, &craft_pdf::OpenOptions::default()).map_err(super::open_error)?;
    let number = placed.page.max(1) as usize;
    let page = pdf.pages().get(number - 1).ok_or_else(|| IoError::Unsupported(format!("the placed PDF has no page {number}")))?;
    let cache = RenderCache::new();
    let crop = placed_box(page, &cache, number, placed, ctl)?.ok_or_else(|| {
        IoError::Unsupported("can't tell how the placed PDF was cropped: neither its artwork nor its page has the proportions it was placed with".into())
    })?;
    let scale = [frame_w / crop.width(), frame_h / crop.height()];
    let options = raster::Options {
        scale: scale.map(|s| s as f32),
        size: Some([width, height]),
        offset: [(crop.x0 * scale[0]) as f32, (crop.y0 * scale[1]) as f32],
        background: [0; 4],
        color_replacement: None,
    };
    let rendered = render_page(page, &cache, &options, ctl).map_err(|e| render_error(number, 72.0 * scale[0], e))?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    write_raster(&mut surface, 0, width, rendered.bytes(), true, "the placed PDF", ctl)?;
    surface.prune();
    let mut doc = Document::new(name, Size::new(width, height), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::new(format!("Page {number}"), LayerContent::Raster(surface)));
    Ok(doc)
}

/// Whether a `width` × `height` box (in pixels of the render it was measured in) has the
/// frame's proportions, allowing for the antialiased pixel at each edge.
fn fits(width: f64, height: f64, frame: (f64, f64)) -> bool {
    let expected = width * frame.1 / frame.0;
    (height - expected).abs() <= 2.0 + 0.01 * height
}

/// The box the page was placed with: its artwork's bounds or the whole page, whichever has the
/// frame's proportions; when both do, the one whose width in points is nearer the frame's at its
/// stored resolution (the artwork's without one). `None` when neither does.
fn placed_box<'a>(page: &'a Page<'a>, cache: &RenderCache<'a>, number: usize, placed: &PlacedPdf, ctl: &Interrupt) -> Result<Option<Box2>, IoError> {
    let frame = placed.frame;
    let (page_w, page_h) = page_size(page, number)?;
    let whole = Box2 { x0: 0.0, y0: 0.0, x1: page_w, y1: page_h };
    let probe = PROBE_SIDE / page_w.max(page_h);
    let size = [(page_w * probe).floor().max(1.0) as u32, (page_h * probe).floor().max(1.0) as u32];
    let art = match opaque_bounds(page, cache, number, probe, [0.0, 0.0], size, ctl)? {
        Some(art) if fits(art.width(), art.height(), frame) => {
            let coarse = Box2 { x0: art.x0 / probe, y0: art.y0 / probe, x1: art.x1 / probe, y1: art.y1 / probe };
            refine(page, cache, number, coarse, probe, frame, ctl)?
        }
        _ => None,
    };
    let page_fits = fits(page_w * probe, page_h * probe, frame);
    Ok(match (art, page_fits, placed.dpi) {
        (Some(art), true, Some(dpi)) => {
            let points = frame.0 * 72.0 / dpi;
            Some(if (whole.width() / points).ln().abs() < (art.width() / points).ln().abs() { whole } else { art })
        }
        (Some(art), _, _) => Some(art),
        (None, true, _) => Some(whole),
        (None, false, _) => None,
    })
}

/// `coarse` (artwork bounds measured at `probe` pixels per point) measured again with the
/// artwork at least as wide as the frame and [`PROBE_SIDE`], so the crop is exact to a fraction
/// of a pixel of the result. `None` when that render would be too large, finds nothing, or
/// measured this finely the artwork doesn't have the frame's proportions after all.
fn refine<'a>(
    page: &'a Page<'a>,
    cache: &RenderCache<'a>,
    number: usize,
    coarse: Box2,
    probe: f64,
    frame: (f64, f64),
    ctl: &Interrupt,
) -> Result<Option<Box2>, IoError> {
    let scale = frame.0.max(frame.1).clamp(PROBE_SIDE, 2.0 * PROBE_SIDE) / coarse.width().max(coarse.height());
    let margin = 2.0 / probe;
    let x0 = ((coarse.x0 - margin).max(0.0) * scale).floor();
    let y0 = ((coarse.y0 - margin).max(0.0) * scale).floor();
    let size = [((coarse.x1 + margin) * scale).ceil() - x0, ((coarse.y1 + margin) * scale).ceil() - y0];
    if !scale.is_finite() || size.iter().any(|&s| !(1.0..=f64::from(raster::MAX_SIDE)).contains(&s)) || size[0] * size[1] > raster::MAX_PIXELS as f64 {
        return Ok(None);
    }
    let found = opaque_bounds(page, cache, number, scale, [x0, y0], size.map(|s| s as u32), ctl)?;
    Ok(found.filter(|b| fits(b.width(), b.height(), frame)).map(|b| Box2 {
        x0: (x0 + b.x0) / scale,
        y0: (y0 + b.y0) / scale,
        x1: (x0 + b.x1) / scale,
        y1: (y0 + b.y1) / scale,
    }))
}

/// The pixel bounds of everything drawn on a transparent render of the page at `scale` pixels
/// per point, from `offset` over `size` pixels; `None` when nothing is drawn.
fn opaque_bounds<'a>(
    page: &'a Page<'a>,
    cache: &RenderCache<'a>,
    number: usize,
    scale: f64,
    offset: [f64; 2],
    size: [u32; 2],
    ctl: &Interrupt,
) -> Result<Option<Box2>, IoError> {
    let options = raster::Options { scale: [scale as f32; 2], size: Some(size), offset: offset.map(|o| o as f32), background: [0; 4], color_replacement: None };
    let rendered = render_page(page, cache, &options, ctl).map_err(|e| render_error(number, 72.0 * scale, e))?;
    let width = size[0] as usize;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for (y, row) in rendered.bytes().chunks_exact(width * 4).enumerate() {
        let px = row.as_chunks::<4>().0;
        let Some(first) = px.iter().position(|p| p[3] > 0) else { continue };
        let last = px.iter().rposition(|p| p[3] > 0).unwrap_or(first);
        (x0, x1) = (x0.min(first), x1.max(last + 1));
        (y0, y1) = (y0.min(y), y + 1);
    }
    Ok((x0 < x1).then_some(Box2 { x0: x0 as f64, y0: y0 as f64, x1: x1 as f64, y1: y1 as f64 }))
}

#[cfg(test)]
mod tests;
