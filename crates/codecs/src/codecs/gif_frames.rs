//! Every frame of an animated GIF. [`crate::decode`] keeps the first frame only; this walks the
//! whole animation for callers that can hold it (video layers).
//!
//! The `image` crate composites each frame onto the logical screen and applies the disposal
//! methods, so every returned frame is a full, self-contained picture.

use std::io::Cursor;

use image::codecs::gif::GifDecoder;
use image::{AnimationDecoder, ImageDecoder};

use crate::Format;
use crate::error::CodecError;
use crate::image::{ChannelLayout, DecodeWarning, Image};
use crate::options::DecodeOptions;

/// Most frames one GIF (or animated WebP) may yield. Frames are held in memory, and a tiny
/// canvas could otherwise ask for millions of them while staying inside the byte budget.
pub const MAX_GIF_FRAMES: usize = 100_000;

/// One composited frame of an animation and how long it stays on screen.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationFrame {
    pub image: Image,
    /// On-screen time in milliseconds. GIF counts in hundredths of a second, so there this is a
    /// multiple of 10 (0 when the file gives none); WebP counts in milliseconds.
    pub delay_ms: u32,
}

/// What a streamed decode ([`for_each`], and WebP's) reports once every frame it could read
/// went to the caller.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FramesDecoded {
    /// Frames handed to the callback.
    pub frames: usize,
    /// What the decoder noticed about the animation as a whole: [`DecodeWarning::Truncated`] when
    /// a later frame was damaged and the frames before it were all that came out. (A collected
    /// decode puts these on its first frame.)
    pub warnings: Vec<DecodeWarning>,
}

/// Decodes every frame of a GIF, each as a full RGBA8 image of the logical screen size.
///
/// The first frame must decode. If a later frame is damaged the frames before it are returned
/// and the first one carries [`DecodeWarning::Truncated`]. Dimensions, the frame count
/// ([`MAX_GIF_FRAMES`]) and the total decoded bytes are checked against `opts.limits` before
/// each frame is kept.
pub fn decode(bytes: &[u8], opts: &DecodeOptions) -> Result<Vec<AnimationFrame>, CodecError> {
    collect(|each| for_each(bytes, opts, each))
}

/// The frames of a streamed decode in one list, its warnings on the first frame.
pub(crate) fn collect(
    decode: impl FnOnce(&mut dyn FnMut(AnimationFrame) -> Result<(), CodecError>) -> Result<FramesDecoded, CodecError>,
) -> Result<Vec<AnimationFrame>, CodecError> {
    let mut out = Vec::new();
    let done = decode(&mut |frame| {
        out.push(frame);
        Ok(())
    })?;
    if let Some(first) = out.first_mut() {
        first.image.warnings.extend(done.warnings);
    }
    Ok(out)
}

/// Decodes the frames of a GIF one at a time, handing each to `each` before the next is decoded,
/// so the caller can convert it, keep what it needs and refuse the rest: the first error `each`
/// returns stops the decode and is returned as it is. Only one frame is held here at a time.
///
/// The checks are [`decode`]'s, made as each frame is decoded: dimensions, the frame count
/// ([`MAX_GIF_FRAMES`]) and the total bytes of every frame decoded so far against
/// `opts.limits.max_alloc`, which bounds the decoding work whatever the caller keeps. A damaged
/// later frame ends the decode with [`DecodeWarning::Truncated`] in the result.
pub fn for_each<E: From<CodecError>>(bytes: &[u8], opts: &DecodeOptions, each: &mut dyn FnMut(AnimationFrame) -> Result<(), E>) -> Result<FramesDecoded, E> {
    let limits = &opts.limits;
    let map = |e: image::ImageError| match e {
        image::ImageError::Limits(l) => CodecError::LimitExceeded(l.to_string()),
        image::ImageError::Unsupported(u) => CodecError::unsupported(Format::Gif, u.to_string()),
        e => CodecError::malformed(Format::Gif, e),
    };
    let mut decoder = GifDecoder::new(Cursor::new(bytes)).map_err(map)?;
    let mut il = image::Limits::default();
    il.max_image_width = Some(limits.max_width);
    il.max_image_height = Some(limits.max_height);
    il.max_alloc = Some(limits.max_alloc);
    decoder.set_limits(il).map_err(map)?;
    let (w, h) = decoder.dimensions();
    limits.check_bytes(w, h, 4)?;
    // The backend initializes the whole logical screen before it reads the first frame, so the
    // same file-size plausibility as the first-frame decode applies before iterating (#8597).
    super::via_image::gif_plausible(bytes, w, h, 4)?;
    let frame_bytes = u64::from(w) * u64::from(h) * 4;

    let mut done = FramesDecoded::default();
    for frame in decoder.into_frames() {
        let frame = match frame {
            Ok(f) => f,
            Err(e) if done.frames == 0 => return Err(map(e).into()),
            Err(_) => {
                done.warnings.push(DecodeWarning::Truncated { format: Format::Gif });
                break;
            }
        };
        if done.frames >= MAX_GIF_FRAMES {
            return Err(CodecError::LimitExceeded(format!("more than {MAX_GIF_FRAMES} frames")).into());
        }
        if frame_bytes.saturating_mul(done.frames as u64 + 1) > limits.max_alloc {
            return Err(CodecError::LimitExceeded(format!("{} frames of {w}x{h} exceed max_alloc {}", done.frames + 1, limits.max_alloc)).into());
        }
        let (numer, denom) = frame.delay().numer_denom_ms();
        let delay_ms = numer.checked_div(denom).unwrap_or(0);
        let rgba = frame.into_buffer();
        each(AnimationFrame { image: Image::from_u8(w, h, ChannelLayout::Rgba, rgba.into_raw())?, delay_ms })?;
        done.frames += 1;
    }
    Ok(done)
}
