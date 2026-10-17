//! Animated GIF output for a document with a frame animation (Window › Timeline): each frame is
//! the layers it shows, flattened and quantised to its own palette, held for its own delay.

use photocraft_codecs::web::{self, GifFrame, GifLoops};
use photocraft_doc::{Document, LoopCount};
use photocraft_raster::Interrupt;

use crate::IoError;

/// Most pixels in one frame: each frame is staged as `f32` RGBA while it is quantised.
pub const MAX_FRAME_PIXELS: u64 = 64 << 20;

/// One frame's pixels (straight-alpha RGBA, `w` wide) as an indexed GIF frame held `delay_cs`
/// hundredths of a second. Pixels under half alpha become the transparent index.
pub fn quantized_frame(mut px: Vec<[f32; 4]>, w: usize, delay_cs: u16) -> Result<GifFrame, IoError> {
    use photocraft_algo::quantize::{self, Dither, Forced, PaletteKind};
    let transparent = px.iter().any(|p| p[3] < 0.5);
    let visible: Vec<[f32; 4]> = px.iter().filter(|p| p[3] >= 0.5).map(|p| [p[0], p[1], p[2], 1.0]).collect();
    // A fully transparent frame still needs a palette.
    let src = if visible.is_empty() { &px } else { &visible };
    let mut pal = quantize::build_palette(src, PaletteKind::Adaptive, if transparent { 255 } else { 256 }, Forced::None).map_err(IoError::Unsupported)?;
    let t = transparent.then(|| {
        pal.truncate(255);
        pal.push(pal.first().copied().unwrap_or([0, 0, 0]));
        pal.len() - 1
    });
    let indices = quantize::quantize(&mut px, w, &pal, Dither::None, 1.0, t);
    Ok(GifFrame { indices, palette: pal, transparent: t.map(|i| i as u8), delay_cs })
}

/// A frame delay in the hundredths of a second a GIF stores.
pub fn delay_cs(delay_ms: u32) -> u16 {
    ((f64::from(delay_ms) / 10.0).round() as u64).min(u64::from(u16::MAX)) as u16
}

fn loops(l: LoopCount) -> GifLoops {
    match l {
        LoopCount::Once => GifLoops::Once,
        LoopCount::Forever => GifLoops::Forever,
        LoopCount::Times(n) => GifLoops::Times(u16::try_from(n).unwrap_or(u16::MAX)),
    }
}

/// The document's frame animation as an animated GIF, or `None` when it has none. Reports progress
/// through `ctl` and stops between frames when it is cancelled.
pub fn export(doc: &Document, ctl: &Interrupt) -> Result<Option<Vec<u8>>, IoError> {
    let Some(a) = &doc.frame_animation else { return Ok(None) };
    let bounds = doc.bounds();
    let (w, h) = (bounds.width() as usize, bounds.height() as usize);
    if (w as u64) * (h as u64) > MAX_FRAME_PIXELS {
        return Err(IoError::Unsupported(format!("{w} × {h} is too large to save as an animated GIF (limit {MAX_FRAME_PIXELS} pixels per frame)")));
    }
    let fmt = doc.pixel_format();
    let mut frames = Vec::with_capacity(a.frames.len());
    for (i, f) in a.frames.iter().enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let mut d = doc.clone();
        a.apply(i, &mut d.layers);
        let surface = photocraft_compose::flatten_to_surface(&d, fmt, None);
        let mut px = vec![[0.0f32; 4]; w * h];
        surface.read_rgba_into(bounds, &mut px);
        frames.push(quantized_frame(px, w, delay_cs(f.delay_ms))?);
        ctl.progress((i + 1) as f32 / a.frames.len() as f32 * 0.95);
    }
    Ok(Some(web::encode_gif_animated_loops(w as u32, h as u32, &frames, loops(a.loops))?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_round_to_hundredths() {
        assert_eq!((delay_cs(0), delay_cs(100), delay_cs(104), delay_cs(106), delay_cs(u32::MAX)), (0, 10, 10, 11, u16::MAX));
    }
}
