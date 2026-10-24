//! Animated GIF output for a document with a frame animation (Window › Timeline): each frame is
//! the layers it shows, flattened and quantised to its own palette, held for its own delay.

use photocraft_codecs::web::{self, GifFrame, GifLoops};
use photocraft_codecs::{ChannelLayout, Format, Image};
use photocraft_doc::{Document, LoopCount};
use photocraft_raster::Interrupt;

use crate::{ExportOptions, IoError};

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

/// An image's pixels as straight-alpha RGBA, gray repeated across the three colour channels.
fn rgba_pixels(img: &Image) -> Vec<[f32; 4]> {
    let n = img.layout().channels();
    let gray = matches!(img.layout(), ChannelLayout::Gray | ChannelLayout::GrayA);
    let alpha = img.layout().has_alpha();
    img.to_normalized()
        .chunks_exact(n.max(1))
        .map(|c| {
            let at = |i: usize| c.get(i).copied().unwrap_or(0.0);
            let (r, g, b) = if gray { (at(0), at(0), at(0)) } else { (at(0), at(1), at(2)) };
            [r, g, b, if alpha { at(n - 1) } else { 1.0 }]
        })
        .collect()
}

/// A frame delay in the hundredths of a second a GIF stores.
pub fn delay_cs(delay_ms: u32) -> u16 {
    ((f64::from(delay_ms) / 10.0).round() as u64).min(u64::from(u16::MAX)) as u16
}

/// The loop count as the encoder takes it.
pub fn loops(l: LoopCount) -> GifLoops {
    // `Times(0)` is forever and `Times(1)` once, as everywhere else.
    match l.normalized() {
        LoopCount::Once => GifLoops::Once,
        LoopCount::Forever => GifLoops::Forever,
        LoopCount::Times(n) => GifLoops::Times(u16::try_from(n).unwrap_or(u16::MAX)),
    }
}

/// Most bytes an export holds at once: every frame's indexed pixels, which are kept until the GIF is
/// encoded, and the working set of the frame being rendered (see [`WORKING_BYTES_PER_PIXEL`]).
pub const MAX_STAGED_BYTES: u64 = 1 << 30;

/// What rendering one frame holds per pixel while it is quantised: the document copy's composite, the
/// converted image, its `f32` RGBA, the palette's copy of the opaque pixels and the indices. Measured
/// at about 43 bytes (peak resident memory less one byte a pixel for each frame kept, on 3840 × 2160 and
/// 2000 × 2000 exports), so this leaves some room for busier pictures.
pub const WORKING_BYTES_PER_PIXEL: u64 = 64;

/// The document's frame animation as an animated GIF, or `None` when it has none. Reports progress
/// through `ctl` and stops between frames when it is cancelled.
pub fn export(doc: &Document, ctl: &Interrupt) -> Result<Option<Vec<u8>>, IoError> {
    export_stepped(doc, &mut |done, total| {
        ctl.progress(done as f32 / total as f32 * 0.95);
        ctl.check().is_ok()
    })
}

/// Refuses an animation of `total` frames of `w` × `h` pixels that an export could not hold at once: see
/// [`MAX_STAGED_BYTES`].
pub fn check_size(w: usize, h: usize, total: usize) -> Result<(), IoError> {
    check_size_in(w, h, total, (w, h))
}

/// [`check_size`] for frames that are a part of a bigger canvas, such as a slice: each is rendered from the
/// whole `canvas` (width, height), which is what the working memory is for, while the frames kept are the
/// part's size.
pub fn check_size_in(w: usize, h: usize, total: usize, canvas: (usize, usize)) -> Result<(), IoError> {
    if (w as u64) * (h as u64) > MAX_FRAME_PIXELS {
        return Err(IoError::Unsupported(format!("{w} × {h} is too large to save as an animated GIF (limit {MAX_FRAME_PIXELS} pixels per frame)")));
    }
    let kept = (w as u64).saturating_mul(h as u64).saturating_mul(total as u64);
    let working = (canvas.0 as u64).saturating_mul(canvas.1 as u64).saturating_mul(WORKING_BYTES_PER_PIXEL);
    if kept.saturating_add(working) > MAX_STAGED_BYTES {
        return Err(IoError::Unsupported(format!(
            "{total} frames of {w} × {h}, rendered from a {} × {} canvas, are too large to save as an animated GIF together (limit {} MB held at once); use fewer frames or a smaller image",
            canvas.0,
            canvas.1,
            MAX_STAGED_BYTES >> 20
        )));
    }
    Ok(())
}

/// [`export`] for a caller with its own progress: `step(done, total)` is called before the first frame
/// and after each one, and the export stops with [`IoError::Cancelled`] when it returns false.
pub fn export_stepped(doc: &Document, step: &mut dyn FnMut(usize, usize) -> bool) -> Result<Option<Vec<u8>>, IoError> {
    let Some(a) = &doc.frame_animation else { return Ok(None) };
    // As a still export does: a Duotone document is written as the RGB its inks print as, not as gray.
    let shown = crate::flat::duotone_as_rgb(doc);
    let doc = shown.as_ref().unwrap_or(doc);
    let bounds = doc.bounds();
    let (w, h) = (bounds.width() as usize, bounds.height() as usize);
    let total = a.frames.len();
    check_size(w, h, total)?;
    let mut frames = Vec::with_capacity(total);
    if !step(0, total) {
        return Err(IoError::Cancelled);
    }
    for (i, f) in a.frames.iter().enumerate() {
        let mut d = doc.clone();
        a.apply(i, &mut d.layers);
        // The colours a still GIF of this frame would have: converted from the document's profile to
        // sRGB (CMYK through its own profile), since the file can't carry the profile.
        let (img, _) = crate::flat::flat_image(&d, Format::Gif, &ExportOptions::default(), &Interrupt::NONE)?;
        let px = rgba_pixels(&img);
        if px.len() != w * h {
            return Err(IoError::Unsupported("a frame did not render at the size of the canvas".into()));
        }
        frames.push(quantized_frame(px, w, delay_cs(f.delay_ms))?);
        if !step(i + 1, total) {
            return Err(IoError::Cancelled);
        }
    }
    // Compressing every frame takes a while on a long animation: the caller can stop it there too.
    let bytes = web::encode_gif_animated_stepped(w as u32, h as u32, &frames, loops(a.loops), &mut |_, n| step(n, n))?;
    bytes.map(Some).ok_or(IoError::Cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_round_to_hundredths() {
        assert_eq!((delay_cs(0), delay_cs(100), delay_cs(104), delay_cs(106), delay_cs(u32::MAX)), (0, 10, 10, 11, u16::MAX));
    }
}
