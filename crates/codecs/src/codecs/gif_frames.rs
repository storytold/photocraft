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

/// Decodes every frame of a GIF, each as a full RGBA8 image of the logical screen size.
///
/// The first frame must decode. If a later frame is damaged the frames before it are returned
/// and the first one carries [`DecodeWarning::Truncated`]. Dimensions, the frame count
/// ([`MAX_GIF_FRAMES`]) and the total decoded bytes are checked against `opts.limits` before
/// each frame is kept.
pub fn decode(bytes: &[u8], opts: &DecodeOptions) -> Result<Vec<AnimationFrame>, CodecError> {
    let (frames, more) = decode_frames(bytes, opts, MAX_GIF_FRAMES, false)?;
    if more {
        return Err(CodecError::LimitExceeded(format!("more than {MAX_GIF_FRAMES} frames")));
    }
    Ok(frames)
}

/// The first `keep` frames of a GIF, and whether it has more. Decoding stops after frame `keep`, so a
/// caller that can only hold `keep` frames never decodes (or holds) the rest. Running into the decoded
/// bytes budget (`opts.limits`) after the first frame also ends the prefix there, with `more` set,
/// rather than failing the whole GIF. Otherwise the limits and the handling of a damaged frame are
/// those of [`decode`].
pub fn decode_prefix(bytes: &[u8], opts: &DecodeOptions, keep: usize) -> Result<(Vec<AnimationFrame>, bool), CodecError> {
    decode_frames(bytes, opts, keep, true)
}

fn decode_frames(bytes: &[u8], opts: &DecodeOptions, keep: usize, prefix_on_budget: bool) -> Result<(Vec<AnimationFrame>, bool), CodecError> {
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
    let frame_bytes = u64::from(w) * u64::from(h) * 4;

    let mut out: Vec<AnimationFrame> = Vec::new();
    let mut damaged = false;
    let mut more = false;
    for frame in decoder.into_frames() {
        let frame = match frame {
            Ok(f) => f,
            Err(e) if out.is_empty() => return Err(map(e)),
            Err(_) => {
                damaged = true;
                break;
            }
        };
        if out.len() >= keep {
            more = true;
            break;
        }
        if frame_bytes.saturating_mul(out.len() as u64 + 1) > limits.max_alloc {
            if prefix_on_budget && !out.is_empty() {
                more = true;
                break;
            }
            return Err(CodecError::LimitExceeded(format!("{} frames of {w}x{h} exceed max_alloc {}", out.len() + 1, limits.max_alloc)));
        }
        let (numer, denom) = frame.delay().numer_denom_ms();
        let delay_ms = numer.checked_div(denom).unwrap_or(0);
        let rgba = frame.into_buffer();
        out.push(AnimationFrame { image: Image::from_u8(w, h, ChannelLayout::Rgba, rgba.into_raw())?, delay_ms });
    }
    if damaged && let Some(first) = out.first_mut() {
        first.image.warnings.push(DecodeWarning::Truncated { format: Format::Gif });
    }
    Ok((out, more))
}

#[cfg(test)]
mod prefix_tests {
    use super::*;
    use crate::web::{GifFrame, encode_gif_animated};

    fn gif(n: usize) -> Vec<u8> {
        let frames: Vec<GifFrame> = (0..n).map(|i| GifFrame { indices: vec![0], palette: vec![[i as u8, 0, 0]], transparent: None, delay_cs: 2 }).collect();
        encode_gif_animated(1, 1, &frames, true).unwrap()
    }

    #[test]
    fn a_prefix_stops_decoding_and_says_whether_there_was_more() {
        let opts = DecodeOptions::default();
        let (f, more) = decode_prefix(&gif(5), &opts, 3).unwrap();
        assert_eq!((f.len(), more), (3, true));
        let (f, more) = decode_prefix(&gif(5), &opts, 5).unwrap();
        assert_eq!((f.len(), more), (5, false));
        let (f, more) = decode_prefix(&gif(5), &opts, 50).unwrap();
        assert_eq!((f.len(), more), (5, false));
        // A budget that holds three 1x1 RGBA frames: the prefix ends there; a full decode still refuses.
        let mut tight = DecodeOptions::default();
        tight.limits.max_alloc = 12;
        let (f, more) = decode_prefix(&gif(5), &tight, 50).unwrap();
        assert_eq!((f.len(), more), (3, true));
        assert!(decode(&gif(5), &tight).is_err(), "decode keeps refusing a GIF that doesn't fit");
        assert_eq!(decode(&gif(5), &opts).unwrap().len(), 5, "decode still returns every frame");
    }
}
