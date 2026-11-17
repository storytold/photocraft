//! Colour bitmap strikes (Apple Color Emoji and Noto Color Emoji, #6883).

use photocraft_codecs::{DecodeOptions, Format, Limits as DecodeLimits};
use photocraft_color::PixelFormat;
use skrifa::bitmap::{BitmapData, BitmapGlyph, Origin};
use skrifa::instance::{LocationRef, Size};
use skrifa::{FontRef, GlyphId, MetadataProvider};

use crate::color_glyphs::Limits;
use crate::raster::{Bounds, Xform};
use crate::warp::Warp;

// Normal emoji strikes are a few hundred pixels across. Reject oversized embedded
// pictures before allocating, even when the glyph is drawn at a small size.
const MAX_SIDE: u32 = 1024;
const PNG_BYTES: usize = 8 * 1024 * 1024;

pub(crate) fn get<'a>(font: &FontRef<'a>, id: GlyphId, size: f32) -> Option<BitmapGlyph<'a>> {
    let g = font.bitmap_strikes().glyph_for_size(Size::new(size), id)?;
    (matches!(g.data, BitmapData::Png(_) | BitmapData::Bgra(_))
        && (1..=MAX_SIDE).contains(&g.width)
        && (1..=MAX_SIDE).contains(&g.height)
        && g.ppem_x > 0.0
        && g.ppem_y > 0.0)
        .then_some(g)
}

/// Match the strike to the size of the glyph in the document, not its nominal point
/// size. The glyph matrix still controls placement: only the image source changes.
pub(crate) fn place_for_output<'a>(
    font: &FontRef<'a>,
    id: GlyphId,
    size: f32,
    nominal: BitmapGlyph<'a>,
    (glyph, bend): (Xform, Option<&Warp>),
    output: &Xform,
) -> (BitmapGlyph<'a>, Xform) {
    let scale = output.mul(&glyph).max_scale();
    let bitmap = if scale.is_finite() && scale > 0.0 && (scale - 1.0).abs() >= 1e-6 {
        let ppem = (f64::from(size) * scale).min(f64::from(f32::MAX)) as f32;
        get(font, id, ppem).unwrap_or(nominal)
    } else {
        nominal
    };
    let transform = output.mul(&placement(font, &bitmap, size, glyph, bend));
    (bitmap, transform)
}

/// Map the top-down image into text space. Path frames are already in `glyph`; for a
/// bending warp, approximate it with an affine map through three corners of the image.
fn placement(font: &FontRef, g: &BitmapGlyph, size: f32, glyph: Xform, bend: Option<&Warp>) -> Xform {
    let (kx, ky) = (f64::from(size) / f64::from(g.ppem_x), f64::from(size) / f64::from(g.ppem_y));
    let (w, h) = (f64::from(g.width), f64::from(g.height));
    let mut left = f64::from(g.inner_bearing_x);
    let mut top = f64::from(g.inner_bearing_y);
    if g.placement_origin == Origin::BottomLeft {
        let em = f64::from(font.metrics(Size::unscaled(), LocationRef::default()).units_per_em).max(1.0);
        left += f64::from(g.bearing_x) * f64::from(g.ppem_x) / em;
        top += h + f64::from(g.bearing_y) * f64::from(g.ppem_y) / em;
    }
    let m = glyph.mul(&Xform([kx, 0.0, 0.0, -ky, left * kx, top * ky]));
    let Some(bend) = bend else { return m };
    let map = |x, y| {
        let (x, y) = m.apply(x, y);
        bend.apply(x, y)
    };
    let (x, y) = map(0.0, 0.0);
    let (xx, xy) = map(w, 0.0);
    let (yx, yy) = map(0.0, h);
    Xform([(xx - x) / w, (xy - y) / w, (yx - x) / h, (yy - y) / h, x, y])
}

pub(crate) fn bounds(g: &BitmapGlyph, m: &Xform) -> Option<[f64; 4]> {
    let (w, h) = (f64::from(g.width), f64::from(g.height));
    let mut b = Bounds::default();
    for (u, v) in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)] {
        let p = m.apply(u, v);
        if !p.0.is_finite() || !p.1.is_finite() {
            return None;
        }
        b.add(p);
    }
    b.rect
}

/// Decode into premultiplied RGBA so interpolation does not bleed colour across transparent edges.
fn decode(g: &BitmapGlyph, png_alloc: u64) -> Option<Vec<f32>> {
    if !(1..=MAX_SIDE).contains(&g.width) || !(1..=MAX_SIDE).contains(&g.height) {
        return None;
    }
    let len = (g.width as usize).checked_mul(g.height as usize)?;
    let samples = len.checked_mul(4)?;
    match g.data {
        BitmapData::Png(bytes) => {
            let opts = DecodeOptions { limits: DecodeLimits::new(g.width, g.height, len as u64, png_alloc), keep_orientation: true };
            let image = photocraft_codecs::decode_as_with(Format::Png, bytes, &opts).ok()?;
            if image.dimensions() != (g.width, g.height) {
                return None;
            }
            let mut pixels = image.to_rgba_f32(&DecodeLimits { max_alloc: (len as u64).checked_mul(16)?, ..opts.limits }).ok()?;
            for [r, g, b, a] in pixels.as_chunks_mut::<4>().0 {
                *r *= *a;
                *g *= *a;
                *b *= *a;
            }
            (pixels.len() == samples).then_some(pixels)
        }
        BitmapData::Bgra(bytes) if bytes.len() == samples => {
            let mut pixels = Vec::new();
            pixels.try_reserve_exact(samples).ok()?;
            let f = |v: u8| f32::from(v) / 255.0;
            pixels.extend(bytes.as_chunks::<4>().0.iter().flat_map(|&[b, g, r, a]| [f(r.min(a)), f(g.min(a)), f(b.min(a)), f(a)]));
            Some(pixels)
        }
        _ => None,
    }
}

/// Sample a picture in its own colours into the document's premultiplied accumulator.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint(
    g: &BitmapGlyph,
    m: &Xform,
    opacity: f32,
    format: &PixelFormat,
    acc: &mut [f32],
    (w, h): (usize, usize),
    limits: &mut Limits,
) -> Option<()> {
    let (Some(inv), Some([x0, y0, x1, y1])) = (m.invert(), bounds(g, m)) else { return Some(()) };
    let clip = |x: f64, size: usize| x.clamp(0.0, size as f64) as usize;
    let (x0, y0, x1, y1) = (clip(x0.floor(), w), clip(y0.floor(), h), clip(x1.ceil(), w), clip(y1.ceil(), h));
    if x0 >= x1 || y0 >= y1 {
        return Some(());
    }
    let (sw, sh) = (g.width as usize, g.height as usize);
    let source = (sw as u64).checked_mul(sh as u64)?;
    let dest = ((x1 - x0) as u64).checked_mul((y1 - y0) as u64)?;
    let (png_alloc, encoded_bytes) = match g.data {
        BitmapData::Png(bytes) if bytes.len() <= PNG_BYTES => (source.checked_mul(8)?.max(64 * 1024), bytes.len() as u64),
        BitmapData::Png(_) => return Some(()),
        _ => (0, 0),
    };
    // The codec can hold pixels, decoder scratch and metadata separately. The RGBA
    // conversion has its own budget, and decoding/sampling work is cumulative.
    if source.checked_mul(16)?.checked_add(png_alloc.checked_mul(4)?)? > limits.bytes {
        return None;
    }
    limits.work = limits.work.checked_sub(source.checked_add(dest)?.checked_add(encoded_bytes)?)?;
    let Some(pixels) = decode(g, png_alloc) else { return Some(()) };
    let pixels = pixels.as_chunks::<4>().0;
    let n = format.mode.color_channels();
    let stride = n + 1;
    let opaque = PixelFormat { alpha: false, ..*format };
    let mut comps = [0.0; 5];
    let texel = |x: usize, y: usize| pixels.get(y * sw + x).copied().unwrap_or_default();
    for y in y0..y1 {
        for x in x0..x1 {
            let (u, v) = inv.apply(x as f64 + 0.5, y as f64 + 0.5);
            if !(u >= 0.0 && v >= 0.0 && u < sw as f64 && v < sh as f64) {
                continue;
            }
            let u = (u - 0.5).clamp(0.0, (sw - 1) as f64);
            let v = (v - 0.5).clamp(0.0, (sh - 1) as f64);
            let (sx, sy) = (u as usize, v as usize);
            let (rx, by) = ((sx + 1).min(sw - 1), (sy + 1).min(sh - 1));
            let (fx, fy) = (u.fract() as f32, v.fract() as f32);
            let [a, b, c, d] = [texel(sx, sy), texel(rx, sy), texel(sx, by), texel(rx, by)];
            let mix = |i: usize| (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy;
            let alpha = mix(3);
            if alpha <= 0.0 {
                continue;
            }
            let rgba = [mix(0) / alpha, mix(1) / alpha, mix(2) / alpha, 1.0];
            photocraft_raster::from_rgba_into(&opaque, rgba, &mut comps);
            let alpha = alpha * opacity;
            let o = (y * w + x) * stride;
            let px = acc.get_mut(o..o + stride)?;
            for (dst, c) in px.iter_mut().zip(comps).take(n) {
                *dst = c * alpha + *dst * (1.0 - alpha);
            }
            px[n] = alpha + px[n] * (1.0 - alpha);
        }
    }
    Some(())
}
