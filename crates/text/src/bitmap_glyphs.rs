//! Colour bitmap strikes (Apple Color Emoji and Noto Color Emoji, #6883).

use photocraft_codecs::{DecodeOptions, Format, Limits as DecodeLimits};
use photocraft_color::PixelFormat;
use skrifa::bitmap::{BitmapData, BitmapFormat, BitmapGlyph, BitmapStrike, Origin};
use skrifa::instance::{LocationRef, Size};
use skrifa::raw::{TableProvider, tables::sbix::Strike};
use skrifa::{FontRef, GlyphId, MetadataProvider, Tag};

use crate::color_glyphs::Limits;
use crate::raster::{Bounds, Xform};
use crate::warp::Warp;

// Normal emoji strikes are a few hundred pixels across. Reject oversized embedded
// pictures before allocating, even when the glyph is drawn at a small size.
const MAX_SIDE: u32 = 1024;
const PNG_BYTES: usize = 8 * 1024 * 1024;
// Duplicate records are untrusted and can reference themselves or form a cycle.
const MAX_DUPE_LINKS: usize = 32;

pub(crate) fn get<'a>(font: &FontRef<'a>, id: GlyphId, size: f32) -> Option<BitmapGlyph<'a>> {
    let strikes = font.bitmap_strikes();
    let sbix = (strikes.format() == Some(BitmapFormat::Sbix)).then(|| font.sbix().ok()).flatten();
    let mut best: Option<BitmapGlyph<'a>> = None;
    for i in 0..strikes.len() {
        let Some(strike) = strikes.get(i) else { continue };
        let ppem = strike.ppem();
        // Same ordering as skrifa: exact size, nearest larger, then nearest smaller.
        if let Some(best) = &best
            && !((ppem >= size && ppem < best.ppem_y) || (best.ppem_y < size && ppem > best.ppem_y))
        {
            continue;
        }
        let bitmap = strike.get(id).or_else(|| {
            let strike_data = sbix.as_ref()?.strikes().get(i).ok()?;
            sbix_dupe(font, &strike, &strike_data, id)
        });
        if let Some(bitmap) = bitmap.filter(eligible) {
            best = Some(bitmap);
        }
    }
    best
}

fn eligible(g: &BitmapGlyph<'_>) -> bool {
    if !(1..=MAX_SIDE).contains(&g.width) || !(1..=MAX_SIDE).contains(&g.height) || g.ppem_x <= 0.0 || g.ppem_y <= 0.0 {
        return false;
    }
    match g.data {
        BitmapData::Png(bytes) => bytes.len() <= PNG_BYTES && bytes.starts_with(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"),
        // Both dimensions have already been bounded by MAX_SIDE.
        BitmapData::Bgra(bytes) => bytes.len() == (g.width * g.height * 4) as usize,
        BitmapData::Mask(_) => false,
    }
}

fn sbix_dupe<'a>(font: &FontRef<'a>, strike: &BitmapStrike<'a>, data: &Strike<'a>, id: GlyphId) -> Option<BitmapGlyph<'a>> {
    let offsets = data.glyph_data_offsets();
    let header_end = data.glyph_data_offsets_byte_range().end;
    let strike_end = offsets.last()?.get() as usize;
    // The raw strike view runs to the end of the table. Constrain each referenced
    // record to this strike's own glyph data, after its offsets and before its sentinel.
    let record = |id: GlyphId| {
        let index = id.to_u32() as usize;
        let start = offsets.get(index)?.get() as usize;
        let end = offsets.get(index.checked_add(1)?)?.get() as usize;
        if start < header_end || end > strike_end {
            return None;
        }
        data.glyph_data(id).ok().flatten()
    };
    let original = record(id)?;
    let mut current = original.clone();
    for _ in 0..MAX_DUPE_LINKS {
        if current.graphic_type() != Tag::new(b"dupe") {
            return None;
        }
        let target = GlyphId::new(u32::from(u16::from_be_bytes(current.data().try_into().ok()?)));
        current = record(target)?;
        if current.graphic_type() == Tag::new(b"png ") {
            let mut bitmap = strike.get(target)?;
            // The reference borrows the picture, not the other glyph's positioning.
            let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
            bitmap.bearing_x = metrics.left_side_bearing(id).unwrap_or_default();
            bitmap.bearing_y = metrics.bounds(id).unwrap_or_default().y_min;
            bitmap.inner_bearing_x = f32::from(original.origin_offset_x());
            bitmap.inner_bearing_y = f32::from(original.origin_offset_y());
            return Some(bitmap);
        }
    }
    None
}

/// Match the strike to the size of the glyph in the document, not its nominal point
/// size. The glyph matrix still controls placement: only the image source changes.
pub(crate) fn place_for_output<'a>(
    font: &FontRef<'a>,
    id: GlyphId,
    size: f32,
    (glyph, bend): (Xform, Option<&Warp>),
    output: &Xform,
) -> Option<(BitmapGlyph<'a>, Xform)> {
    let scale = output.mul(&glyph).max_scale();
    let ppem = if scale.is_finite() && scale > 0.0 { (f64::from(size) * scale).min(f64::from(f32::MAX)) as f32 } else { size };
    let bitmap = get(font, id, ppem)?;
    let transform = output.mul(&placement(font, &bitmap, size, glyph, bend));
    Some((bitmap, transform))
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

/// Average the source area of each reduced pixel in premultiplied RGBA. Use
/// integer grid coordinates so partial texels at either edge retain their weight.
fn reduce(pixels: &[[f32; 4]], (sw, sh): (usize, usize), (dw, dh): (usize, usize)) -> Option<Vec<[f32; 4]>> {
    let mut out = Vec::new();
    out.try_reserve_exact(dw.checked_mul(dh)?).ok()?;
    let area = (sw * sh) as f32;
    for y in 0..dh {
        let (top, bottom) = (y * sh, (y + 1) * sh);
        for x in 0..dw {
            let (left, right) = (x * sw, (x + 1) * sw);
            let mut pixel = [0.0; 4];
            for sy in top / dh..bottom.div_ceil(dh) {
                let wy = bottom.min((sy + 1) * dh) - top.max(sy * dh);
                for sx in left / dw..right.div_ceil(dw) {
                    let wx = right.min((sx + 1) * dw) - left.max(sx * dw);
                    let weight = (wx * wy) as f32 / area;
                    for (value, sample) in pixel.iter_mut().zip(pixels.get(sy * sw + sx)?) {
                        *value += *sample * weight;
                    }
                }
            }
            out.push(pixel);
        }
    }
    Some(out)
}

/// Sample a picture in its own colours into the document's premultiplied accumulator.
/// False leaves the outline available; `None` stops the optional bitmap pass.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint(
    g: &BitmapGlyph,
    m: &Xform,
    opacity: f32,
    format: &PixelFormat,
    acc: &mut [f32],
    (w, h): (usize, usize),
    limits: &mut Limits,
) -> Option<bool> {
    let (Some(inv), Some([x0, y0, x1, y1])) = (m.invert(), bounds(g, m)) else { return Some(false) };
    let clip = |x: f64, size: usize| x.clamp(0.0, size as f64) as usize;
    let (x0, y0, x1, y1) = (clip(x0.floor(), w), clip(y0.floor(), h), clip(x1.ceil(), w), clip(y1.ceil(), h));
    if x0 >= x1 || y0 >= y1 {
        return Some(false);
    }
    let (sw, sh) = (g.width as usize, g.height as usize);
    let source = (sw as u64).checked_mul(sh as u64)?;
    let dest = ((x1 - x0) as u64).checked_mul((y1 - y0) as u64)?;
    let (png_alloc, encoded_bytes) = match g.data {
        BitmapData::Png(bytes) if bytes.len() <= PNG_BYTES => (source.checked_mul(8)?.max(64 * 1024), bytes.len() as u64),
        BitmapData::Png(_) => return Some(false),
        _ => (0, 0),
    };
    // The inverse transform maps a destination pixel footprint back into the
    // source. Prefilter only axes that shrink, then keep bilinear sampling.
    let reduced_side = |side: usize, footprint: f64| ((side as f64 / footprint.max(1.0)).ceil() as usize).clamp(1, side);
    let (dw, dh) = (reduced_side(sw, inv.0[0].hypot(inv.0[2])), reduced_side(sh, inv.0[1].hypot(inv.0[3])));
    let shrinking = (sw, sh) != (dw, dh);
    // A source texel crosses at most one output boundary per axis. This
    // accounts for the area filter even for non-integer and unequal scales.
    let filter_work = if shrinking { ((sw + dw - 1) as u64).checked_mul((sh + dh - 1) as u64)? } else { 0 };
    let filter_bytes = if shrinking { (dw as u64).checked_mul(dh as u64)?.checked_mul(16)? } else { 0 };
    // The codec can hold pixels, decoder scratch and metadata separately. The RGBA
    // conversion has its own budget, and decoding/sampling work is cumulative.
    if source.checked_mul(16)?.checked_add(png_alloc.checked_mul(4)?)?.checked_add(filter_bytes)? > limits.bytes {
        return None;
    }
    limits.work = limits.work.checked_sub(source.checked_add(dest)?.checked_add(encoded_bytes)?.checked_add(filter_work)?)?;
    let Some(pixels) = decode(g, png_alloc) else { return Some(false) };
    let pixels = pixels.as_chunks::<4>().0;
    let filtered;
    let pixels = if shrinking {
        filtered = reduce(pixels, (sw, sh), (dw, dh))?;
        &filtered
    } else {
        pixels
    };
    let n = format.mode.color_channels();
    let stride = n + 1;
    let opaque = PixelFormat { alpha: false, ..*format };
    let mut comps = [0.0; 5];
    let texel = |x: usize, y: usize| pixels.get(y * dw + x).copied().unwrap_or_default();
    for y in y0..y1 {
        for x in x0..x1 {
            let (u, v) = inv.apply(x as f64 + 0.5, y as f64 + 0.5);
            if !(u >= 0.0 && v >= 0.0 && u < sw as f64 && v < sh as f64) {
                continue;
            }
            let u = (u * dw as f64 / sw as f64 - 0.5).clamp(0.0, (dw - 1) as f64);
            let v = (v * dh as f64 / sh as f64 - 0.5).clamp(0.0, (dh - 1) as f64);
            let (sx, sy) = (u as usize, v as usize);
            let (rx, by) = ((sx + 1).min(dw - 1), (sy + 1).min(dh - 1));
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
    Some(true)
}
