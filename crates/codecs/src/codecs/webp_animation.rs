//! Animated WebP containers, implemented from the public RIFF specification:
//! <https://developers.google.com/speed/webp/docs/riff_container>.
//! Pixel coding/compositing stays with the existing Rust WebP codecs.

use std::io::Cursor;

use crate::options::alloc_zeroed;
use crate::{AnimationFrame, ChannelLayout, CodecError, DecodeOptions, EncodeOptions, Format, Image, Limits, Metadata, SampleType};

pub const MAX_WEBP_FRAMES: usize = 100_000;
pub const MAX_WEBP_DELAY_MS: u32 = 0x00ff_ffff;
const F: Format = Format::WebP;

/// Container facts without allocating decoded frames. Zero loops means forever.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WebpAnimationInfo {
    pub width: u32,
    pub height: u32,
    pub frames: usize,
    pub loop_count: u16,
}

/// Fully composited frames in chronological order, with exact file durations.
#[derive(Debug, Clone, PartialEq)]
pub struct WebpAnimation {
    pub frames: Vec<AnimationFrame>,
    /// None for a still image; Some(0) is an animation that loops forever.
    pub loop_count: Option<u16>,
}

fn malformed(reason: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, reason)
}

struct Chunks<'a>(&'a [u8]);

impl<'a> Iterator for Chunks<'a> {
    type Item = Result<(&'a [u8], &'a [u8]), CodecError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_empty() {
            return None;
        }
        let bytes = std::mem::take(&mut self.0);
        Some((|| {
            let tag = bytes.get(..4).ok_or_else(|| malformed("truncated chunk tag"))?;
            let size = bytes.get(4..8).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).ok_or_else(|| malformed("truncated chunk size"))?;
            let size = usize::try_from(size).map_err(malformed)?;
            let end = 8usize.checked_add(size).ok_or_else(|| malformed("chunk size overflow"))?;
            let padded = end.checked_add(size % 2).ok_or_else(|| malformed("chunk padding overflow"))?;
            let payload = bytes.get(8..end).ok_or_else(|| malformed("truncated chunk"))?;
            self.0 = bytes.get(padded..).ok_or_else(|| malformed("missing chunk padding"))?;
            Ok((tag, payload))
        })())
    }
}

fn riff(bytes: &[u8]) -> Result<Chunks<'_>, CodecError> {
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WEBP") {
        return Err(malformed("not a WebP RIFF container"));
    }
    let size = bytes.get(4..8).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).ok_or_else(|| malformed("missing RIFF size"))?;
    let end = usize::try_from(size).ok().and_then(|n| n.checked_add(8)).ok_or_else(|| malformed("RIFF size overflow"))?;
    Ok(Chunks(bytes.get(12..end).ok_or_else(|| malformed("truncated RIFF container"))?))
}

fn u24(bytes: &[u8], at: usize) -> Result<u32, CodecError> {
    let b = bytes.get(at..at.saturating_add(3)).ok_or_else(|| malformed("truncated 24-bit field"))?;
    let mut value = 0;
    for (i, byte) in b.iter().enumerate() {
        value |= u32::from(*byte) << (i * 8);
    }
    Ok(value)
}

/// Validate frame dimensions before entering image-webp: its ALPH/VP8 path assumes that the
/// bitstream and ANMF dimensions agree when allocating and indexing its alpha plane.
fn validate_frame(data: &[u8], w: u32, h: u32) -> Result<(), CodecError> {
    let x = u24(data, 0)? * 2;
    let y = u24(data, 3)? * 2;
    let fw = u24(data, 6)? + 1;
    let fh = u24(data, 9)? + 1;
    if x.saturating_add(fw) > w || y.saturating_add(fh) > h {
        return Err(malformed("animation frame outside canvas"));
    }
    let mut found = false;
    for entry in Chunks(data.get(16..).ok_or_else(|| malformed("short ANMF header"))?) {
        let (tag, payload) = entry?;
        let dims = match tag {
            b"VP8 " => {
                if payload.get(3..6) != Some(&[0x9d, 0x01, 0x2a]) {
                    return Err(malformed("invalid VP8 frame header"));
                }
                let a = payload.get(6..8).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes).ok_or_else(|| malformed("short VP8 width"))?;
                let b = payload.get(8..10).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes).ok_or_else(|| malformed("short VP8 height"))?;
                Some((u32::from(a & 0x3fff), u32::from(b & 0x3fff)))
            }
            b"VP8L" => {
                if payload.first() != Some(&0x2f) {
                    return Err(malformed("invalid VP8L signature"));
                }
                let bits = payload.get(1..5).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).ok_or_else(|| malformed("short VP8L header"))?;
                Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
            }
            _ => None,
        };
        if let Some(dims) = dims {
            if found || dims != (fw, fh) {
                return Err(malformed("ANMF and bitstream dimensions disagree"));
            }
            found = true;
        }
    }
    if !found {
        return Err(malformed("animation frame has no image"));
    }
    Ok(())
}

/// Inspect an animation, checking container bounds and frame headers. A still yields None.
pub fn webp_animation_info(bytes: &[u8]) -> Result<Option<WebpAnimationInfo>, CodecError> {
    let mut info = None;
    let mut loops = None;
    for chunk in riff(bytes)? {
        let (tag, data) = chunk?;
        match tag {
            b"VP8X" if data.first().is_some_and(|b| b & 2 != 0) => {
                if info.is_some() {
                    return Err(malformed("duplicate VP8X header"));
                }
                info = Some(WebpAnimationInfo { width: u24(data, 4)? + 1, height: u24(data, 7)? + 1, frames: 0, loop_count: 0 });
            }
            b"ANIM" => {
                if loops.is_some() {
                    return Err(malformed("duplicate ANIM header"));
                }
                loops = Some(data.get(4..6).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes).ok_or_else(|| malformed("short ANIM header"))?);
            }
            b"ANMF" => {
                let i = info.as_mut().ok_or_else(|| malformed("ANMF without animation header"))?;
                if loops.is_none() {
                    return Err(malformed("ANMF before ANIM"));
                }
                i.frames += 1;
                if i.frames > MAX_WEBP_FRAMES {
                    return Err(CodecError::LimitExceeded("too many WebP frames".into()));
                }
                validate_frame(data, i.width, i.height)?;
            }
            _ => {}
        }
    }
    if let Some(i) = &mut info {
        i.loop_count = loops.ok_or_else(|| malformed("animation has no ANIM header"))?;
        if i.frames == 0 {
            return Err(malformed("animation has no frames"));
        }
    }
    Ok(info)
}

/// Decode one complete loop, applying frame blending/disposal and EXIF orientation. Every
/// frame must decode; no partially decoded animation is returned. Limits include retained
/// pixels/metadata and headroom for the decoder's canvas and working frame.
pub fn decode_webp_frames(bytes: &[u8], opts: &DecodeOptions) -> Result<WebpAnimation, CodecError> {
    let Some(info) = webp_animation_info(bytes)? else {
        let image = crate::decode_as_with(F, bytes, opts)?;
        opts.limits.check(image.width(), image.height(), ChannelLayout::Rgba, SampleType::U8)?;
        return Ok(WebpAnimation { frames: vec![AnimationFrame { image: image.convert(ChannelLayout::Rgba, SampleType::U8), delay_ms: 0 }], loop_count: None });
    };
    let l = &opts.limits;
    l.check(info.width, info.height, ChannelLayout::Rgba, SampleType::U8)?;
    let frame_bytes = u64::from(info.width) * u64::from(info.height) * 4;
    // image-webp currently computes its internal canvas byte count as u32.
    if frame_bytes > u64::from(u32::MAX) {
        return Err(CodecError::LimitExceeded("WebP animation canvas exceeds decoder capacity".into()));
    }
    let mut dec = image_webp::WebPDecoder::new(Cursor::new(bytes)).map_err(malformed)?;
    dec.set_memory_limit(l.alloc_usize());
    if !dec.is_animated() || dec.num_frames() as usize != info.frames || dec.dimensions() != (info.width, info.height) {
        return Err(malformed("inconsistent animation header"));
    }
    // Explicit transparency also makes disposal clear the previous rectangle. With None,
    // image-webp leaves disposed pixels in its canvas instead of clearing them.
    dec.set_background_color([0; 4]).map_err(malformed)?;
    let icc = dec.icc_profile().map_err(malformed)?;
    let exif = dec.exif_metadata().map_err(malformed)?;
    let xmp = dec.xmp_metadata().map_err(malformed)?.and_then(|b| String::from_utf8(b).ok());
    let meta_bytes = icc.as_ref().map_or(0, Vec::len).saturating_add(exif.as_ref().map_or(0, Vec::len)).saturating_add(xmp.as_ref().map_or(0, String::len));
    let retained = frame_bytes.saturating_add(meta_bytes as u64).saturating_mul(info.frames as u64);
    if retained.saturating_add(frame_bytes.saturating_mul(3)) > l.max_alloc {
        return Err(CodecError::LimitExceeded("WebP animation exceeds max_alloc (frames and decoder buffers)".into()));
    }
    Limits::check_plausible(frame_bytes.saturating_mul(info.frames as u64), bytes.len(), 1 << 16)?;
    let size = dec.output_buffer_size().ok_or_else(|| malformed("image too large"))?;
    let mut buf = alloc_zeroed::<u8>(size)?;
    let layout = if dec.has_alpha() { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
    let mut frames = Vec::new();
    frames.try_reserve_exact(info.frames).map_err(|e| CodecError::LimitExceeded(e.to_string()))?;
    for _ in 0..info.frames {
        let delay_ms = dec.read_frame(&mut buf).map_err(malformed)?;
        let mut data = alloc_zeroed::<u8>(buf.len())?;
        data.copy_from_slice(&buf);
        let mut image = Image::from_u8(info.width, info.height, layout, data)?;
        image.icc = icc.clone();
        image.meta = Metadata { exif: exif.clone(), xmp: xmp.clone(), ..Default::default() };
        image.meta.dpi = crate::resolution::from_metadata(image.meta.exif.as_deref(), image.meta.xmp.as_deref());
        let orientation = image.meta.exif.as_deref().map_or(1, crate::exif_orientation);
        let image = crate::finish_decode(image, orientation, opts)?.convert(ChannelLayout::Rgba, SampleType::U8);
        frames.push(AnimationFrame { image, delay_ms });
    }
    Ok(WebpAnimation { frames, loop_count: Some(info.loop_count) })
}

fn append_chunk(out: &mut Vec<u8>, tag: &[u8; 4], payload: &[u8]) -> Result<(), CodecError> {
    let size = u32::try_from(payload.len()).map_err(|_| CodecError::LimitExceeded("WebP chunk exceeds 4 GiB".into()))?;
    let extra = payload.len().checked_add(8 + payload.len() % 2).ok_or_else(|| CodecError::LimitExceeded("WebP size overflow".into()))?;
    let max = (u64::from(u32::MAX) - 9).min(Limits::default().max_alloc / 2);
    if (out.len() as u64).saturating_add(extra as u64) > max {
        return Err(CodecError::LimitExceeded("WebP RIFF exceeds size or allocation limit".into()));
    }
    out.try_reserve(extra).map_err(|e| CodecError::LimitExceeded(e.to_string()))?;
    out.extend_from_slice(tag);
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(payload);
    if !payload.len().is_multiple_of(2) {
        out.push(0);
    }
    Ok(())
}

fn put_u24(out: &mut Vec<u8>, n: u32) {
    out.extend(n.to_le_bytes().into_iter().take(3));
}

/// Encode full-canvas replacement frames, independent of prior frame alpha. Input sample
/// types and color layouts follow the same conversion policy as the still WebP encoder.
pub fn encode_webp_animation(frames: &[AnimationFrame], loop_count: u16, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let first = frames.first().ok_or_else(|| CodecError::InvalidImage("WebP animation needs a frame".into()))?;
    if frames.len() > MAX_WEBP_FRAMES {
        return Err(CodecError::LimitExceeded("too many WebP frames".into()));
    }
    let (w, h) = first.image.dimensions();
    Limits::default().check(w, h, ChannelLayout::Rgba, SampleType::U8)?;
    if w > 16383 || h > 16383 {
        return Err(CodecError::InvalidImage("WebP frames exceed 16383 pixels per side".into()));
    }
    // Reuse the still encoder's metadata policy, including dropping an ICC profile when its
    // native color model was converted (for example CMYK to RGB) and normalizing EXIF.
    let first_still = crate::encode(&first.image, F, opts)?;
    let mut metadata = Vec::new();
    let mut flags = 0x12; // animation and alpha
    for chunk in riff(&first_still)? {
        let (tag, payload) = chunk?;
        let flag = match tag {
            b"ICCP" => 0x20,
            b"EXIF" => 0x08,
            b"XMP " => 0x04,
            _ => continue,
        };
        flags |= flag;
        metadata.push((tag, payload));
    }
    let mut body = b"WEBP".to_vec();
    let mut header = vec![flags, 0, 0, 0];
    put_u24(&mut header, w - 1);
    put_u24(&mut header, h - 1);
    append_chunk(&mut body, b"VP8X", &header)?;
    for &(tag, data) in &metadata {
        if tag == b"ICCP" {
            append_chunk(&mut body, b"ICCP", data)?;
        }
    }
    let mut anim = vec![0; 4]; // transparent background
    anim.extend_from_slice(&loop_count.to_le_bytes());
    append_chunk(&mut body, b"ANIM", &anim)?;
    let frame_opts = EncodeOptions { embed_icc: false, embed_metadata: false, ..opts.clone() };
    for (i, frame) in frames.iter().enumerate() {
        if frame.image.dimensions() != (w, h) || frame.delay_ms > MAX_WEBP_DELAY_MS {
            return Err(CodecError::InvalidImage("WebP frame dimensions or duration are invalid".into()));
        }
        let later;
        let still = if i == 0 {
            &first_still
        } else {
            later = crate::encode(&frame.image, F, &frame_opts)?;
            &later
        };
        let mut data = vec![0; 6]; // x = y = 0
        put_u24(&mut data, w - 1);
        put_u24(&mut data, h - 1);
        put_u24(&mut data, frame.delay_ms);
        data.push(2); // replacement blending, keep canvas
        for chunk in riff(still)? {
            let (tag, payload) = chunk?;
            if matches!(tag, b"VP8 " | b"VP8L" | b"ALPH") {
                let tag: &[u8; 4] = tag.try_into().map_err(malformed)?;
                append_chunk(&mut data, tag, payload)?;
            }
        }
        append_chunk(&mut body, b"ANMF", &data)?;
    }
    for &(tag, data) in &metadata {
        if matches!(tag, b"EXIF" | b"XMP ") {
            append_chunk(&mut body, tag.try_into().map_err(malformed)?, data)?;
        }
    }
    let size = u32::try_from(body.len()).map_err(malformed)?;
    let mut out = Vec::new();
    out.try_reserve_exact(body.len().saturating_add(8)).map_err(|e| CodecError::LimitExceeded(e.to_string()))?;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend(body);
    Ok(out)
}
