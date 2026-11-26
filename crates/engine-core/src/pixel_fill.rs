//! Filling and blending pixels into a surface: Edit › Fill's compositing, which Edit › Clear,
//! Edit › Stroke, pastes into masks and channels, a floating selection and the painting tools share.

use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Surface, from_rgba_into, to_rgba};

use crate::{EngineError, Result};

/// The blend modes the reference application greys in its Fill, Stroke and Fade Mode lists for a document of `mode`
/// at `depth` (#13806, #14647). Combinations it wasn't measured in (Grayscale 16/32, Lab 16, CMYK 16, Multichannel,
/// Duotone…) grey none.
pub fn greyed_modes(mode: ColorMode, depth: SampleType) -> &'static [BlendMode] {
    use BlendMode::*;
    match (mode, depth) {
        (ColorMode::Grayscale, SampleType::U8) => &[DarkerColor, LighterColor, Hue, Saturation, Color, Luminosity],
        (ColorMode::Lab, SampleType::U8) => &[Darken, ColorBurn, Lighten, ColorDodge, Difference, Exclusion, Subtract, Divide],
        (ColorMode::Rgb, SampleType::F32) => {
            &[ColorBurn, LinearBurn, Screen, ColorDodge, Overlay, SoftLight, HardLight, VividLight, LinearLight, PinLight, HardMix, Exclusion]
        }
        _ => &[],
    }
}

/// An error for `cmd` when `blend` is one of [`greyed_modes`] for `doc`: the dialogs never offer it there.
pub fn check_mode_available(cmd: &str, doc: &Document, blend: BlendMode) -> Result<()> {
    if !greyed_modes(doc.mode, doc.depth).contains(&blend) {
        return Ok(());
    }
    let model = match doc.mode {
        ColorMode::Rgb => "RGB".to_string(),
        ColorMode::Cmyk => "CMYK".to_string(),
        other => format!("{other:?}"),
    };
    Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("{} is not available in {model} {}-bit documents", blend.label(), doc.depth.bits()) })
}

/// What a pixel of the filled area gets before blending.
pub enum Source {
    Color([f32; 4]),
    /// An opaque colour that was picked as CMYK inks: `rgba` is its sRGB stand-in (what other
    /// colour models and blend modes use), `cmyk` the inks a CMYK target gets as they are.
    Cmyk {
        rgba: [f32; 4],
        cmyk: [f32; 4],
    },
    /// Straight RGBA per pixel of `rect` (row-major).
    Pixels(Rect, Vec<[f32; 4]>),
}

impl Source {
    /// The straight RGBA the pixel at (`x`, `y`) gets before blending.
    pub fn at(&self, x: i32, y: i32) -> [f32; 4] {
        match self {
            Source::Color(c) => *c,
            Source::Cmyk { rgba, .. } => *rgba,
            Source::Pixels(r, px) => {
                if !r.contains(x, y) {
                    return [0.0; 4];
                }
                let i = (y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize;
                px.get(i).copied().unwrap_or([0.0; 4])
            }
        }
    }
}

/// `source`'s opaque colour as samples of `fmt` (alpha last, as in [`from_rgba_into`]), or `None`
/// for a source that is not one colour. A CMYK pick keeps its inks in a CMYK target.
pub fn encode_color(fmt: &photocraft_color::PixelFormat, source: &Source, enc: &mut [f32; 8]) -> Option<usize> {
    let (rgba, cmyk) = match source {
        Source::Color(c) if c[3] >= 1.0 => (*c, None),
        Source::Cmyk { rgba, cmyk } => (*rgba, Some(*cmyk)),
        _ => return None,
    };
    let n = from_rgba_into(fmt, rgba, enc);
    if let Some(k) = cmyk
        && fmt.mode == photocraft_color::ColorMode::Cmyk
    {
        enc[..4].copy_from_slice(&k);
    }
    Some(n)
}

/// `area` of `src` as straight RGBA, or an error naming `purpose` when it doesn't fit in memory.
pub fn read_rgba(src: &Surface, area: Rect, purpose: &str) -> Result<Vec<[f32; 4]>> {
    let len = (area.width() as usize)
        .checked_mul(area.height() as usize)
        .ok_or_else(|| EngineError::Other(format!("not enough memory for {purpose} (requested size overflowed); the document was not changed")))?;
    let mut px = crate::allocation::filled(len, [0.0f32; 4], purpose)?;
    src.read_rgba_into(area, &mut px);
    Ok(px)
}

/// `src` over `surf` across `area` (Normal, 100 %), each pixel weighted by `limit` when given:
/// a paste into a layer mask or channel, which keeps the pasted pixels' luminosity (#1035).
pub fn composite_over(surf: &mut Surface, src: &Surface, area: Rect, limit: Option<&Surface>) -> Result<()> {
    let source = Source::Pixels(area, read_rgba(src, area, "pasting pixels")?);
    blend_into(surf, area, &source, limit, Blend { mode: BlendMode::Normal, opacity: 1.0, keep_alpha: false, restore: false })
}

/// `area` of `surf` filled with an opaque `color` (Normal, 100 %), weighted by `sel` when given:
/// Clear on a targeted alpha channel or the Quick Mask (#2734).
pub fn fill_color(surf: &mut Surface, area: Rect, color: [f32; 4], sel: Option<&Surface>) -> Result<()> {
    blend_into(surf, area, &Source::Color(color), sel, Blend { mode: BlendMode::Normal, opacity: 1.0, keep_alpha: false, restore: false })
}

/// Reuse Edit › Fill's compositing for Edit › Stroke. The stroke's rasterized band
/// is already a grayscale coverage mask; opacity, blend modes and alpha locking
/// must behave identically for both commands at 8/16/32-bit depth.
#[allow(clippy::too_many_arguments)]
pub fn blend_color_mask(
    surf: &mut Surface,
    area: Rect,
    color: [f32; 4],
    picked: Option<[f32; 4]>,
    coverage: &Surface,
    mode: BlendMode,
    opacity: f32,
    preserve_transparency: bool,
) -> Result<()> {
    let source = match picked {
        Some(cmyk) if color[3] >= 1.0 => Source::Cmyk { rgba: color, cmyk },
        _ => Source::Color(color),
    };
    blend_into(surf, area, &source, Some(coverage), Blend { mode, opacity, keep_alpha: preserve_transparency, restore: false })
}

/// How the source goes onto the target.
#[derive(Clone, Copy)]
pub struct Blend {
    pub mode: BlendMode,
    pub opacity: f32,
    /// Leave every pixel's alpha as it was (Preserve Transparency, or locked transparency).
    pub keep_alpha: bool,
    /// Normal mode replaces the pixels, transparency included (History: back to that state).
    pub restore: bool,
}

/// `d` moved towards `c` by `k`, in premultiplied space (straight RGBA in and out).
pub fn lerp(d: [f32; 4], c: [f32; 4], k: f32) -> [f32; 4] {
    let a = d[3] * (1.0 - k) + c[3] * k;
    if a <= 0.0 {
        return [0.0; 4];
    }
    let ch = |i: usize| (d[i] * d[3] * (1.0 - k) + c[i] * c[3] * k) / a;
    [ch(0), ch(1), ch(2), a]
}

/// Blend `source` into `surf` over `area`: per pixel, the selection coverage times the opacity,
/// in the blend mode.
/// Works tile by tile in parallel on the encoded pixels (no full-surface f32 round trip).
pub fn blend_into(surf: &mut Surface, area: Rect, source: &Source, sel: Option<&Surface>, b: Blend) -> Result<()> {
    use rayon::prelude::*;
    let fmt = surf.format();
    // An opaque colour in Normal mode at 100 % that may replace alpha: fully covered tiles become
    // one shared solid tile (copy-on-write), which costs nothing per pixel.
    let mut enc = [0.0f32; 8];
    let solid = match encode_color(&fmt, source, &mut enc) {
        Some(n) if b.mode == BlendMode::Normal && b.opacity >= 1.0 && (!b.keep_alpha || !fmt.alpha) => Some(surf.solid_tile(enc.get(..n).unwrap_or(&[]))),
        _ => None,
    };
    let count = area.tiles().count();
    let done = crate::allocation::capacity(count, "blending pixels")?;
    let tiles = surf
        .try_take_tiles(area)
        .map_err(|e| EngineError::Other(format!("not enough memory while preparing pixel tiles ({e}); the document was not changed")))?;
    let done = std::sync::Mutex::new(done);
    tiles.into_par_iter().try_for_each(|(tc, tile)| -> Result<()> {
        let tr = tc.rect().intersect(&area);
        let mask = sel.map(|m| Mask::of(m, tc));
        if let Some(s) = &solid
            && tr == tc.rect()
            && mask.as_ref().is_none_or(|m| m.full(tr))
        {
            done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((tc, s.clone()));
            return Ok(());
        }
        // Nothing selected in this tile (or no opacity): `blend_tile` would leave every pixel as
        // it is, so keep the tile itself rather than a copy the history would hold again (#4517).
        if b.opacity <= 0.0 || mask.as_ref().is_some_and(|m| m.empty(tr)) {
            done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((tc, tile));
            return Ok(());
        }
        let mut tile = match std::sync::Arc::try_unwrap(tile) {
            Ok(tile) => tile,
            Err(tile) => tile
                .try_clone()
                .map_err(|e| EngineError::Other(format!("not enough memory while blending a pixel tile ({e}); the document was not changed")))?,
        };
        blend_tile(&mut tile.bytes_mut(), &fmt, tc.rect(), tr, source, mask.as_ref(), b);
        done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((tc, std::sync::Arc::new(tile)));
        Ok(())
    })?;
    let done = done.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
    surf.put_tiles(done);
    Ok(())
}

/// The selection's coverage over one tile (its channel 0).
pub struct Mask<'a> {
    bytes: Option<photocraft_raster::TileBytes<'a>>,
    default: &'a [u8],
    fmt: photocraft_color::PixelFormat,
    origin: Rect,
}

impl<'a> Mask<'a> {
    /// `sel` over the tile at `tc` (its default pixel where it has no tile there).
    pub fn of(sel: &'a Surface, tc: photocraft_geom::TileCoord) -> Self {
        Mask { bytes: sel.tile(tc).map(|t| t.bytes()), default: sel.default_bytes(), fmt: sel.format(), origin: tc.rect() }
    }
    pub fn at(&self, x: i32, y: i32) -> f32 {
        let bytes = self.bytes.as_deref().unwrap_or(self.default);
        let bpp = self.fmt.bytes_per_pixel();
        // A default-pixel mask is one pixel long: every position reads it.
        let i = if bytes.len() == bpp { 0 } else { ((y - self.origin.y0) as usize * TILE_SIZE as usize + (x - self.origin.x0) as usize) * bpp };
        bytes.get(i..i + bpp).map_or(0.0, |px| photocraft_color::read_sample(px, self.fmt.sample, 0))
    }
    /// Fully selected over `r`?
    pub fn full(&self, r: Rect) -> bool {
        (r.y0..r.y1).all(|y| (r.x0..r.x1).all(|x| self.at(x, y) >= 1.0))
    }
    /// Whether nothing in `r` is selected.
    pub fn empty(&self, r: Rect) -> bool {
        (r.y0..r.y1).all(|y| (r.x0..r.x1).all(|x| self.at(x, y) <= 0.0))
    }
}

/// Blend into one tile's encoded pixels (`bytes`, the tile at `origin`) over `tr`.
pub fn blend_tile(bytes: &mut [u8], fmt: &photocraft_color::PixelFormat, origin: Rect, tr: Rect, source: &Source, mask: Option<&Mask>, b: Blend) {
    let Blend { mode, opacity, keep_alpha, restore } = b;
    let n = fmt.channels();
    let bpp = fmt.bytes_per_pixel();
    let blend = if mode == BlendMode::Dissolve { BlendMode::Normal } else { mode };
    let mut px = [0.0f32; 8];
    let mut enc = [0.0f32; 8];
    // An opaque colour at full coverage in Normal mode just replaces the colour (and the alpha
    // unless it is kept): copy its encoded bytes.
    let opaque = match encode_color(fmt, source, &mut enc) {
        Some(m) if mode == BlendMode::Normal => {
            let mut bytes = vec![0u8; bpp];
            photocraft_raster::encode_pixel(fmt, enc.get(..m.min(n)).unwrap_or(&[]), &mut bytes);
            // Keeping alpha: copy the colour samples only (alpha is the last sample).
            let keep = if keep_alpha && fmt.alpha { bpp - bpp / n.max(1) } else { bpp };
            Some((bytes, keep))
        }
        _ => None,
    };
    for y in tr.y0..tr.y1 {
        let row = (y - origin.y0) as usize * TILE_SIZE as usize;
        for x in tr.x0..tr.x1 {
            let mut k = mask.map_or(1.0, |m| m.at(x, y)) * opacity;
            if k <= 0.0 {
                continue;
            }
            let i = (row + (x - origin.x0) as usize) * bpp;
            let Some(pb) = bytes.get_mut(i..i + bpp) else { continue };
            if k >= 1.0
                && let Some((enc_px, keep)) = &opaque
            {
                if let (Some(dst), Some(src)) = (pb.get_mut(..*keep), enc_px.get(..*keep)) {
                    dst.copy_from_slice(src);
                }
                continue;
            }
            for (c, v) in px.iter_mut().enumerate().take(n) {
                *v = photocraft_color::read_sample(pb, fmt.sample, c);
            }
            // Normal CMYK fills blend the chosen separation directly. Converting
            // through the display RGB would discard it at partial opacity/coverage.
            if fmt.mode == photocraft_color::ColorMode::Cmyk
                && mode == BlendMode::Normal
                && let Source::Cmyk { cmyk, .. } = source
            {
                let alpha = if fmt.alpha { px[4] } else { 1.0 };
                let out_alpha = k + alpha * (1.0 - k);
                let weight = if keep_alpha || !fmt.alpha {
                    k
                } else if out_alpha > 0.0 {
                    k / out_alpha
                } else {
                    0.0
                };
                for (channel, ink) in cmyk.iter().enumerate() {
                    enc[channel] = px[channel] * (1.0 - weight) + ink * weight;
                }
                if fmt.alpha {
                    enc[4] = if keep_alpha { alpha } else { out_alpha };
                }
                photocraft_raster::encode_pixel(fmt, &enc[..n.min(8)], pb);
                continue;
            }
            let d = to_rgba(fmt, &px[..n.min(8)]);
            let mut c = source.at(x, y);
            if mode == BlendMode::Dissolve {
                // Dissolve: each pixel is either fully painted or untouched, with the coverage as odds.
                k = if photocraft_color::dither_noise(x, y) < c[3] * k { 1.0 } else { 0.0 };
                c[3] = 1.0;
                if k <= 0.0 {
                    continue;
                }
            }
            let keep = keep_alpha || !fmt.alpha;
            let mut o = if restore && blend == BlendMode::Normal {
                lerp(d, c, k)
            } else if keep {
                // Alpha kept: source atop, so a half-transparent pixel takes the new colour by the
                // source alpha alone (#5783).
                photocraft_color::blend::composite_atop(blend, d, c, k)
            } else {
                photocraft_color::blend::composite(blend, d, c, k)
            };
            if keep {
                o[3] = d[3];
            }
            let m = from_rgba_into(fmt, o, &mut enc);
            photocraft_raster::encode_pixel(fmt, enc.get(..m.min(n)).unwrap_or(&[]), pb);
        }
    }
}
