//! Web-optimised writers for Save for Web: indexed GIF (our own LZW encoder, GIF89a spec),
//! WBMP (WAP Wireless Bitmap, type 0) and baseline/progressive JPEG from 8-bit RGB.
//!
//! Indexed PNG is [`crate::encode_png_indexed`]. Every writer here has a matching reader (GIF
//! through the regular decoder, WBMP through [`decode_wbmp`]), keeping the crate symmetric.

use std::collections::HashMap;

use crate::error::CodecError;
use crate::format::Format;

fn gif_err(msg: impl Into<String>) -> CodecError {
    CodecError::InvalidImage(msg.into())
}

/// Packs variable-length LZW codes LSB-first into GIF data sub-blocks.
struct BitSink {
    bytes: Vec<u8>,
    acc: u32,
    nbits: u32,
}

impl BitSink {
    fn put(&mut self, code: u16, size: u32) {
        self.acc |= u32::from(code) << self.nbits;
        self.nbits += size;
        while self.nbits >= 8 {
            self.bytes.push(self.acc as u8);
            self.acc >>= 8;
            self.nbits -= 8;
        }
    }
    fn finish(mut self) -> Vec<u8> {
        if self.nbits > 0 {
            self.bytes.push(self.acc as u8);
        }
        self.bytes
    }
}

/// GIF variable-code-length LZW compression of `indices` (each < 2^`min_code_size`).
pub fn lzw_encode(indices: &[u8], min_code_size: u8) -> Vec<u8> {
    let min = u32::from(min_code_size.clamp(2, 8));
    let clear = 1u16 << min;
    let eoi = clear + 1;
    let mut size = min + 1;
    let mut next = eoi + 1;
    let mut dict: HashMap<u32, u16> = HashMap::with_capacity(4096);
    let mut out = BitSink { bytes: Vec::with_capacity(indices.len() / 2 + 16), acc: 0, nbits: 0 };
    out.put(clear, size);
    let Some((&first, rest)) = indices.split_first() else {
        out.put(eoi, size);
        return out.finish();
    };
    let mut prefix = u16::from(first);
    for &k in rest {
        let key = u32::from(prefix) << 8 | u32::from(k);
        if let Some(&c) = dict.get(&key) {
            prefix = c;
            continue;
        }
        out.put(prefix, size);
        // The decoder adds its entries one code late, so widen once the next free code no
        // longer fits (before assigning it), exactly like the classic compress-based encoders.
        if u32::from(next) >= 1 << size && size < 12 {
            size += 1;
        }
        if next < 4096 {
            dict.insert(key, next);
            next += 1;
        } else {
            out.put(clear, size);
            dict.clear();
            size = min + 1;
            next = eoi + 1;
        }
        prefix = u16::from(k);
    }
    out.put(prefix, size);
    if u32::from(next) >= 1 << size && size < 12 {
        size += 1;
    }
    out.put(eoi, size);
    out.finish()
}

/// Rows in GIF interlaced order (passes every 8th from 0, every 8th from 4, every 4th from 2,
/// every 2nd from 1).
fn interlaced_rows(h: usize) -> Vec<usize> {
    let mut v = Vec::with_capacity(h);
    for (start, step) in [(0, 8), (4, 8), (2, 4), (1, 2)] {
        v.extend((start..h).step_by(step));
    }
    v
}

/// An indexed GIF89a: `palette` (1..=256 entries), one index per pixel, optional transparent
/// index (Graphic Control Extension) and interlacing.
pub fn encode_gif_indexed(
    width: u32,
    height: u32,
    indices: &[u8],
    palette: &[[u8; 3]],
    transparent: Option<u8>,
    interlaced: bool,
) -> Result<Vec<u8>, CodecError> {
    let (w, h) = (width as usize, height as usize);
    if width == 0 || height == 0 || width > 65535 || height > 65535 || indices.len() != w * h {
        return Err(gif_err("GIF size must be 1..=65535 and match the index data"));
    }
    if palette.is_empty() || palette.len() > 256 || indices.iter().any(|&i| usize::from(i) >= palette.len()) {
        return Err(gif_err("palette must have 1..=256 entries covering every index"));
    }
    // Colour table size: 2^(bits) entries, bits ≥ 1.
    let bits = (1..=8u8).find(|b| (1usize << b) >= palette.len()).unwrap_or(8);
    let mut out = b"GIF89a".to_vec();
    out.extend((width as u16).to_le_bytes());
    out.extend((height as u16).to_le_bytes());
    out.push(0x80 | ((bits - 1) << 4) | (bits - 1));
    out.push(transparent.unwrap_or(0));
    out.push(0);
    for i in 0..(1usize << bits) {
        out.extend(palette.get(i).copied().unwrap_or([0, 0, 0]));
    }
    if let Some(t) = transparent {
        out.extend([0x21, 0xF9, 0x04, 0x01, 0, 0, t, 0]);
    }
    out.push(0x2C);
    out.extend([0, 0, 0, 0]);
    out.extend((width as u16).to_le_bytes());
    out.extend((height as u16).to_le_bytes());
    out.push(if interlaced { 0x40 } else { 0 });
    let min = bits.max(2);
    out.push(min);
    let data = if interlaced {
        let mut v = Vec::with_capacity(indices.len());
        for y in interlaced_rows(h) {
            v.extend_from_slice(&indices[y * w..(y + 1) * w]);
        }
        lzw_encode(&v, min)
    } else {
        lzw_encode(indices, min)
    };
    for chunk in data.chunks(255) {
        out.push(chunk.len() as u8);
        out.extend_from_slice(chunk);
    }
    out.push(0);
    out.push(0x3B);
    Ok(out)
}

/// One frame of an animated GIF: palette indices, its (local) palette, an optional transparent
/// index, and the on-screen delay in centiseconds.
#[derive(Clone, Debug)]
pub struct GifFrame {
    pub indices: Vec<u8>,
    pub palette: Vec<[u8; 3]>,
    pub transparent: Option<u8>,
    pub delay_cs: u16,
}

/// How many times an animated GIF plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GifLoops {
    /// Plays through once: no looping extension.
    Once,
    Forever,
    /// Plays this many times in all (two or more; the file stores the repeats after the first).
    Times(u16),
}

/// Encode an animated GIF (GIF89a). Each frame carries its own local colour table; `loop_forever`
/// adds the NETSCAPE2.0 looping extension.
pub fn encode_gif_animated(width: u32, height: u32, frames: &[GifFrame], loop_forever: bool) -> Result<Vec<u8>, CodecError> {
    encode_gif_animated_loops(width, height, frames, if loop_forever { GifLoops::Forever } else { GifLoops::Once })
}

/// [`encode_gif_animated`] with a play count.
pub fn encode_gif_animated_loops(width: u32, height: u32, frames: &[GifFrame], loops: GifLoops) -> Result<Vec<u8>, CodecError> {
    encode_gif_animated_stepped(width, height, frames, loops, &mut |_, _| true)?.ok_or_else(|| gif_err("the encoder stopped without being asked to"))
}

/// [`encode_gif_animated_loops`] that calls `step(done, total)` after each frame is written and stops
/// when it returns false, giving `None`: compressing a long animation takes a while.
pub fn encode_gif_animated_stepped(
    width: u32,
    height: u32,
    frames: &[GifFrame],
    loops: GifLoops,
    step: &mut dyn FnMut(usize, usize) -> bool,
) -> Result<Option<Vec<u8>>, CodecError> {
    let (w, h) = (width as usize, height as usize);
    if width == 0 || height == 0 || width > 65535 || height > 65535 {
        return Err(gif_err("GIF size must be 1..=65535"));
    }
    if frames.is_empty() {
        return Err(gif_err("an animated GIF needs at least one frame"));
    }
    let mut out = b"GIF89a".to_vec();
    out.extend((width as u16).to_le_bytes());
    out.extend((height as u16).to_le_bytes());
    out.push(0x70); // no global colour table, 8-bit colour resolution
    out.push(0); // background colour index
    out.push(0); // pixel aspect ratio
    let repeats = match loops {
        GifLoops::Once => None,
        GifLoops::Forever => Some(0u16), // loop count 0 = forever
        GifLoops::Times(n) => Some(n.max(2) - 1),
    };
    if let Some(repeats) = repeats {
        out.extend([0x21, 0xFF, 0x0B]);
        out.extend_from_slice(b"NETSCAPE2.0");
        out.extend([0x03, 0x01]);
        out.extend(repeats.to_le_bytes());
        out.push(0x00);
    }
    for (i, f) in frames.iter().enumerate() {
        if f.palette.is_empty() || f.palette.len() > 256 || f.indices.len() != w * h || f.indices.iter().any(|&idx| usize::from(idx) >= f.palette.len()) {
            return Err(gif_err("every frame's palette must have 1..=256 entries covering its indices"));
        }
        let bits = (1..=8u8).find(|b| (1usize << b) >= f.palette.len()).unwrap_or(8);
        // Graphic Control Extension: delay and optional transparency. Every frame covers the whole
        // image, so one is normally left in place (disposal 1). A frame with a transparent index, and
        // the frame before one, are cleared afterwards (disposal 2): otherwise what was drawn before
        // shows through the holes.
        let clears = f.transparent.is_some() || frames.get(i + 1).is_some_and(|next| next.transparent.is_some());
        let packed = (if clears { 2 << 2 } else { 1 << 2 }) | u8::from(f.transparent.is_some());
        out.extend([0x21, 0xF9, 0x04, packed]);
        out.extend(f.delay_cs.to_le_bytes());
        out.push(f.transparent.unwrap_or(0));
        out.push(0);
        // Image Descriptor with a Local Colour Table.
        out.push(0x2C);
        out.extend([0, 0, 0, 0]);
        out.extend((width as u16).to_le_bytes());
        out.extend((height as u16).to_le_bytes());
        out.push(0x80 | (bits - 1));
        for i in 0..(1usize << bits) {
            out.extend(f.palette.get(i).copied().unwrap_or([0, 0, 0]));
        }
        let min = bits.max(2);
        out.push(min);
        for chunk in lzw_encode(&f.indices, min).chunks(255) {
            out.push(chunk.len() as u8);
            out.extend_from_slice(chunk);
        }
        out.push(0);
        if !step(i + 1, frames.len()) {
            return Ok(None);
        }
    }
    out.push(0x3B);
    Ok(Some(out))
}

/// One frame of an animated WebP: straight RGBA8 pixels of the whole canvas and how long it
/// shows, in milliseconds (WebP stores up to 2^24 − 1).
pub struct WebpFrame {
    pub rgba: Vec<u8>,
    pub delay_ms: u32,
}

/// Encode a lossless animated WebP (`VP8X` + `ANIM` + one `ANMF` per frame). Every frame covers
/// the whole canvas and replaces the one before (no blending, no disposal). `loops` 0 plays
/// forever.
pub fn encode_webp_animated(width: u32, height: u32, frames: &[WebpFrame], loops: u16) -> Result<Vec<u8>, CodecError> {
    let err = |m: &str| CodecError::encode(Format::WebP, m.to_string());
    if width == 0 || height == 0 || width > 1 << 14 || height > 1 << 14 {
        return Err(err("WebP size must be 1..=16384"));
    }
    if frames.is_empty() {
        return Err(err("an animated WebP needs at least one frame"));
    }
    let u24 = |out: &mut Vec<u8>, v: u32| out.extend(&v.to_le_bytes()[..3]);
    let chunk = |out: &mut Vec<u8>, fourcc: &[u8; 4], payload: &[u8]| {
        out.extend_from_slice(fourcc);
        out.extend((payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            out.push(0);
        }
    };
    let mut body = b"WEBP".to_vec();
    let mut vp8x = vec![0x10 | 0x02, 0, 0, 0]; // alpha, animation
    u24(&mut vp8x, width - 1);
    u24(&mut vp8x, height - 1);
    chunk(&mut body, b"VP8X", &vp8x);
    let mut anim = vec![0u8; 4]; // background colour: transparent
    anim.extend(loops.to_le_bytes());
    chunk(&mut body, b"ANIM", &anim);
    for f in frames {
        if f.rgba.len() != width as usize * height as usize * 4 {
            return Err(err("every frame must be width × height RGBA8"));
        }
        let mut still = Vec::new();
        image_webp::WebPEncoder::new(&mut still)
            .encode(&f.rgba, width, height, image_webp::ColorType::Rgba8)
            .map_err(|e| CodecError::encode(Format::WebP, e))?;
        // The encoder writes RIFF/WEBP then the VP8L chunk; the frame keeps that chunk whole.
        let bitstream = still.get(12..).ok_or_else(|| err("the still encoder wrote no chunk"))?;
        let mut anmf = Vec::new();
        u24(&mut anmf, 0);
        u24(&mut anmf, 0);
        u24(&mut anmf, width - 1);
        u24(&mut anmf, height - 1);
        u24(&mut anmf, f.delay_ms.min((1 << 24) - 1));
        anmf.push(0x02); // do not blend, do not dispose
        anmf.extend_from_slice(bitstream);
        chunk(&mut body, b"ANMF", &anmf);
    }
    let mut out = b"RIFF".to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    Ok(out)
}

fn put_multibyte(out: &mut Vec<u8>, v: u32) {
    let mut groups = vec![(v & 0x7f) as u8];
    let mut v = v >> 7;
    while v > 0 {
        groups.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
    out.extend(groups.iter().rev());
}

/// WBMP type 0 (1 bit per pixel, rows padded to bytes, 1 = white). `white[i]` per pixel.
pub fn encode_wbmp(width: u32, height: u32, white: &[bool]) -> Result<Vec<u8>, CodecError> {
    let (w, h) = (width as usize, height as usize);
    if width == 0 || height == 0 || white.len() != w * h {
        return Err(CodecError::InvalidImage("WBMP size must match the pixel data".into()));
    }
    let mut out = vec![0u8, 0u8];
    put_multibyte(&mut out, width);
    put_multibyte(&mut out, height);
    let stride = w.div_ceil(8);
    for y in 0..h {
        let mut row = vec![0u8; stride];
        for x in 0..w {
            if white[y * w + x] {
                row[x / 8] |= 0x80 >> (x % 8);
            }
        }
        out.extend(row);
    }
    Ok(out)
}

/// Reads a WBMP type 0 image: (width, height, white per pixel).
pub fn decode_wbmp(bytes: &[u8]) -> Result<(u32, u32, Vec<bool>), CodecError> {
    let bad = || CodecError::InvalidImage("not a type 0 WBMP".into());
    let mut i = 0usize;
    let mut next = || -> Result<u8, CodecError> {
        let b = *bytes.get(i).ok_or_else(bad)?;
        i += 1;
        Ok(b)
    };
    if next()? != 0 || next()? & 0x9f != 0 {
        return Err(bad());
    }
    let mut mb = || -> Result<u32, CodecError> {
        let mut v: u32 = 0;
        for _ in 0..5 {
            let b = next()?;
            v = v.checked_mul(128).ok_or_else(bad)? | u32::from(b & 0x7f);
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(bad())
    };
    let (w, h) = (mb()?, mb()?);
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 1 << 28 {
        return Err(bad());
    }
    let stride = (w as usize).div_ceil(8);
    let data = bytes.get(i..i + stride * h as usize).ok_or_else(bad)?;
    let mut px = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h as usize {
        for x in 0..w as usize {
            px.push(data[y * stride + x / 8] & (0x80 >> (x % 8)) != 0);
        }
    }
    Ok((w, h, px))
}

/// JPEG from 8-bit RGB (`rgb.len() == 3·w·h`): quality 1..=100, optional progressive scans and
/// optimised Huffman tables (Save for Web's "Optimized"), optional ICC profile, density, EXIF and
/// XMP. EXIF that doesn't fit in one segment is left out (see [`jpeg_exif_fits`]).
#[allow(clippy::too_many_arguments)]
pub fn encode_jpeg_rgb8(
    width: u32,
    height: u32,
    rgb: &[u8],
    quality: u8,
    progressive: bool,
    optimized: bool,
    icc: Option<&[u8]>,
    dpi: Option<f32>,
    exif: Option<&[u8]>,
    xmp: Option<&str>,
) -> Result<Vec<u8>, CodecError> {
    let e = |e: jpeg_encoder::EncodingError| CodecError::encode(Format::Jpeg, e);
    let (Ok(w16), Ok(h16)) = (u16::try_from(width), u16::try_from(height)) else {
        return Err(CodecError::encode(Format::Jpeg, "JPEG dimensions are limited to 65535"));
    };
    if rgb.len() != width as usize * height as usize * 3 {
        return Err(CodecError::InvalidImage("RGB data does not match the size".into()));
    }
    let mut out = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut out, quality.clamp(1, 100));
    // Like Photoshop's Save for Web: 4:2:0 chroma up to quality 50, 4:4:4 from 51 (measured on
    // Photoshop 2025's output; its Save As keeps 4:2:0 at every quality).
    let subsampled = quality <= 50;
    enc.set_progressive(progressive);
    // jpeg-encoder 0.6 writes corrupt baseline scans when optimised Huffman tables meet 4:2:0
    // (both zune-jpeg and image-rs decode them green); progressive output is fine.
    enc.set_optimized_huffman_tables(progressive || (optimized && !subsampled));
    enc.set_sampling_factor(if subsampled { jpeg_encoder::SamplingFactor::R_4_2_0 } else { jpeg_encoder::SamplingFactor::R_4_4_4 });
    if let Some(d) = dpi.filter(|d| *d >= 1.0) {
        let v = d.round().min(65535.0) as u16;
        enc.set_density(jpeg_encoder::Density::Inch { x: v, y: v });
    }
    let ppi = dpi.filter(|d| *d >= 1.0).map(|d| (d, d));
    // Upright, without the source's thumbnail, at the JFIF resolution, as Save As writes it.
    if let Some(seg) = exif.and_then(|x| crate::codecs::jpeg::exif_segment(x, ppi)) {
        enc.add_app_segment(1, &seg).map_err(e)?;
    }
    if let Some(x) = xmp {
        // Its `tiff:` resolution, if any, says what the JFIF density says (#1691).
        let x = crate::resolution::export_xmp(x, ppi);
        let mut seg = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
        seg.extend_from_slice(x.as_bytes());
        enc.add_app_segment(1, &seg).map_err(e)?;
    }
    if let Some(icc) = icc {
        enc.add_icc_profile(icc).map_err(e)?;
    }
    enc.encode(rgb, w16, h16, jpeg_encoder::ColorType::Rgb).map_err(e)?;
    Ok(out)
}

/// Whether [`encode_jpeg_rgb8`] can write `exif` at `dpi` (it must fit in one APP1 segment).
pub fn jpeg_exif_fits(exif: &[u8], dpi: Option<f32>) -> bool {
    crate::codecs::jpeg::exif_segment(exif, dpi.filter(|d| *d >= 1.0).map(|d| (d, d))).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_rgba(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let img = crate::decode(bytes).unwrap();
        let (w, h) = img.dimensions();
        (w, h, img.convert(crate::ChannelLayout::Rgba, crate::SampleType::U8).data().to_vec())
    }

    fn pattern(w: usize, h: usize, n: usize) -> Vec<u8> {
        (0..w * h).map(|i| (((i % w) * 7 + (i / w) * 13 + (i * i) % 5) % n) as u8).collect()
    }

    /// The luma sampling factors (h, v) of a baseline or progressive JPEG.
    fn luma_sampling(bytes: &[u8]) -> Option<(u8, u8)> {
        let i = bytes.windows(2).position(|w| w[0] == 0xFF && matches!(w[1], 0xC0..=0xC2))?;
        let f = *bytes.get(i + 11)?;
        Some((f >> 4, f & 15))
    }

    #[test]
    fn jpeg_chroma_is_full_above_quality_50() {
        let rgb: Vec<u8> = (0..16 * 16 * 3).map(|i| (i * 37 % 256) as u8).collect();
        for (q, want) in [(1, (2, 2)), (50, (2, 2)), (51, (1, 1)), (60, (1, 1)), (89, (1, 1)), (100, (1, 1))] {
            for progressive in [false, true] {
                let bytes = encode_jpeg_rgb8(16, 16, &rgb, q, progressive, false, None, None, None, None).unwrap();
                assert_eq!(luma_sampling(&bytes), Some(want), "quality {q} progressive {progressive}");
            }
        }
    }

    #[test]
    fn gif_roundtrips_through_the_decoder() {
        let pal: Vec<[u8; 3]> = (0..200).map(|i| [i as u8, (255 - i) as u8, (i * 3 % 256) as u8]).collect();
        for (w, h, interlaced) in [(1, 1, false), (37, 23, false), (37, 23, true), (300, 200, false)] {
            let idx = pattern(w, h, pal.len());
            let bytes = encode_gif_indexed(w as u32, h as u32, &idx, &pal, None, interlaced).unwrap();
            let (dw, dh, px) = decode_rgba(&bytes);
            assert_eq!((dw, dh), (w as u32, h as u32));
            for (i, k) in idx.iter().enumerate() {
                assert_eq!(&px[i * 4..i * 4 + 3], &pal[*k as usize], "pixel {i} of {w}x{h} interlaced={interlaced}");
            }
        }
    }

    #[test]
    fn gif_small_palettes_and_transparency() {
        let pal = [[255, 255, 255], [0, 0, 0], [255, 0, 0]];
        let idx: Vec<u8> = (0..64 * 64).map(|i| (i % 3) as u8).collect();
        let bytes = encode_gif_indexed(64, 64, &idx, &pal, Some(2), false).unwrap();
        let (_, _, px) = decode_rgba(&bytes);
        assert_eq!(px[3], 255);
        assert_eq!(px[2 * 4 + 3], 0, "index 2 is transparent");
        // Two-colour image: minimum code size 2.
        let bw: Vec<u8> = (0..1000).map(|i| u8::from(i % 7 == 0)).collect();
        let bytes = encode_gif_indexed(100, 10, &bw, &pal[..2], None, false).unwrap();
        let (_, _, px) = decode_rgba(&bytes);
        assert_eq!(px[0], 0, "index 1 = black at x 0");
        assert!(encode_gif_indexed(2, 2, &[0, 1, 2, 3], &pal, None, false).is_err());
    }

    #[test]
    fn gif_long_runs_hit_the_dictionary_limit() {
        // Noise forces many codes and dictionary resets; flat areas make long strings.
        let mut seed = 12345u32;
        let idx: Vec<u8> = (0..512 * 512)
            .map(|i| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                if (i / 512) % 64 < 32 { (seed >> 16) as u8 } else { 7 }
            })
            .collect();
        let pal: Vec<[u8; 3]> = (0..256).map(|i| [i as u8, i as u8, 255 - i as u8]).collect();
        let bytes = encode_gif_indexed(512, 512, &idx, &pal, None, false).unwrap();
        let (_, _, px) = decode_rgba(&bytes);
        for (i, k) in idx.iter().enumerate().step_by(97) {
            assert_eq!(px[i * 4], *k);
        }
    }

    #[test]
    fn wbmp_roundtrips() {
        let px: Vec<bool> = (0..13 * 5).map(|i| i % 3 == 0).collect();
        let bytes = encode_wbmp(13, 5, &px).unwrap();
        assert_eq!(&bytes[..4], &[0, 0, 13, 5]);
        assert_eq!(decode_wbmp(&bytes).unwrap(), (13, 5, px));
        let big = encode_wbmp(300, 1, &[true; 300]).unwrap();
        assert_eq!(&big[2..4], &[0x82, 0x2c], "300 as a multi-byte integer");
        assert!(decode_wbmp(&big[..10]).is_err());
        assert!(decode_wbmp(&[1, 0, 1, 1, 0]).is_err());
    }

    #[test]
    fn progressive_jpeg_decodes() {
        let rgb: Vec<u8> = (0..64 * 48 * 3).map(|i| (i % 251) as u8).collect();
        let base = encode_jpeg_rgb8(64, 48, &rgb, 80, false, false, None, Some(72.0), None, Some("<x:xmpmeta/>")).unwrap();
        let prog = encode_jpeg_rgb8(64, 48, &rgb, 80, true, true, None, None, None, None).unwrap();
        assert_ne!(base, prog);
        assert_eq!(crate::decode(&base).unwrap().meta.xmp.as_deref(), Some("<x:xmpmeta/>"));
        // SOF2 marks a progressive frame.
        assert!(prog.windows(2).any(|w| w == [0xFF, 0xC2]));
        assert_eq!(crate::decode(&prog).unwrap().dimensions(), (64, 48));
    }

    /// #1691: the XMP resolution of a Save for Web JPEG says what its JFIF density says.
    #[test]
    fn web_jpeg_xmp_resolution_follows_the_density() {
        let rgb = vec![100u8; 8 * 8 * 3];
        let xmp = r#"<rdf:Description tiff:XResolution="300/1" tiff:YResolution="300/1"/>"#;
        let out = encode_jpeg_rgb8(8, 8, &rgb, 80, false, false, None, Some(72.0), None, Some(xmp)).unwrap();
        let img = crate::decode(&out).unwrap();
        assert_eq!(img.meta.dpi, Some((72.0, 72.0)));
        assert_eq!(img.meta.xmp.as_deref().and_then(crate::xmp_resolution), Some((72.0, 72.0)));
    }
}

#[cfg(test)]
mod subsampling_tests {
    use super::*;

    /// jpeg-encoder 0.6 writes broken scans for baseline 4:2:0 with optimised Huffman tables;
    /// `encode_jpeg_rgb8` must avoid that combination.
    #[test]
    fn every_option_combination_decodes_to_the_source() {
        let (w, h) = (64u32, 48u32);
        let rgb: Vec<u8> = (0..w * h).flat_map(|i| [((i % w) * 4) as u8, 76, ((i / w) * 5) as u8]).collect();
        for q in [30, 60, 95] {
            for (prog, opt) in [(false, false), (false, true), (true, false), (true, true)] {
                let bytes = encode_jpeg_rgb8(w, h, &rgb, q, prog, opt, None, None, None, None).unwrap();
                let ours = crate::decode(&bytes).unwrap().convert(crate::ChannelLayout::Rgb, crate::SampleType::U8);
                let err = ours.data().iter().zip(&rgb).map(|(a, b)| i32::from(*a).abs_diff(i32::from(*b))).max().unwrap();
                assert!(err < 24, "q{q} progressive={prog} optimized={opt}: max error {err}");
            }
        }
    }
}

#[cfg(test)]
mod anim_tests {
    use super::*;

    #[test]
    fn animated_gif_has_header_and_frames() {
        let frames: Vec<GifFrame> = (0..3)
            .map(|i| GifFrame { indices: vec![i as u8; 4], palette: vec![[0, 0, 0], [255, 0, 0], [0, 255, 0]], transparent: None, delay_cs: 4 })
            .collect();
        let g = encode_gif_animated(2, 2, &frames, true).unwrap();
        assert_eq!(&g[..6], b"GIF89a");
        assert_eq!(*g.last().unwrap(), 0x3B);
        // Three image descriptors (0x2C) for three frames.
        assert_eq!(g.iter().filter(|&&b| b == 0x2C).count(), 3);
        // NETSCAPE loop extension present.
        assert!(g.windows(11).any(|w| w == b"NETSCAPE2.0"));
        // First frame still decodes.
        let img = crate::decode(&g).unwrap();
        assert_eq!(img.dimensions(), (2, 2));
    }

    #[test]
    fn transparent_frames_are_cleared_before_the_next() {
        let frame = |transparent| GifFrame { indices: vec![0, 1, 1, 1], palette: vec![[0, 0, 0], [255, 0, 0]], transparent, delay_cs: 4 };
        // The Graphic Control Extension's packed byte follows its 0x21 0xF9 0x04 introducer.
        let packed = |g: &[u8]| g.windows(4).find(|w| w[..3] == [0x21, 0xF9, 0x04]).map(|w| w[3]).unwrap();
        let opaque = encode_gif_animated(2, 2, &[frame(None)], false).unwrap();
        assert_eq!(packed(&opaque), 0x04, "disposal 1, no transparency");
        let clear = encode_gif_animated(2, 2, &[frame(Some(0))], false).unwrap();
        assert_eq!(packed(&clear), 0x09, "disposal 2 with the transparency flag");
        let px = crate::decode(&clear).unwrap().to_rgba8();
        assert_eq!((px[3], &px[4..8]), (0, &[255, 0, 0, 255][..]), "index 0 transparent, index 1 red");
    }

    #[test]
    fn an_opaque_frame_before_a_transparent_one_is_cleared_too() {
        let frame = |transparent| GifFrame { indices: vec![0, 1, 1, 1], palette: vec![[0, 0, 0], [255, 0, 0]], transparent, delay_cs: 4 };
        let g = encode_gif_animated(2, 2, &[frame(None), frame(Some(0)), frame(None), frame(None)], false).unwrap();
        let packed: Vec<u8> = g.windows(4).filter(|w| w[..3] == [0x21, 0xF9, 0x04]).map(|w| w[3]).collect();
        // Opaque then transparent: cleared (the holes must not show it); transparent: cleared;
        // opaque then opaque: left in place; the last frame has nothing after it.
        assert_eq!(packed, [0x08, 0x09, 0x04, 0x04]);
    }

    #[test]
    fn rejects_bad_frames() {
        assert!(encode_gif_animated(2, 2, &[], false).is_err());
        let bad = GifFrame { indices: vec![0; 3], palette: vec![[0, 0, 0]], transparent: None, delay_cs: 1 };
        assert!(encode_gif_animated(2, 2, &[bad], false).is_err());
    }
}

#[cfg(test)]
mod webp_anim_tests {
    use super::*;

    fn solid(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
        px.repeat((w * h) as usize)
    }

    #[test]
    fn an_animated_webp_decodes_to_every_frame_with_its_delay() {
        let frames = [([255, 0, 0, 255], 40), ([0, 0, 255, 128], 250), ([0, 255, 0, 255], 0)].map(|(c, delay_ms)| WebpFrame { rgba: solid(5, 3, c), delay_ms });
        let bytes = encode_webp_animated(5, 3, &frames, 0).unwrap();
        assert_eq!(crate::detect(&bytes), Some(Format::WebP));
        let out = crate::decode_webp_frames(&bytes, &crate::DecodeOptions::default()).unwrap();
        assert_eq!(out.iter().map(|f| f.delay_ms).collect::<Vec<_>>(), [40, 250, 0]);
        for (f, want) in out.iter().zip([[255, 0, 0, 255], [0, 0, 255, 128], [0, 255, 0, 255]]) {
            let rgba = f.image.convert(crate::ChannelLayout::Rgba, crate::SampleType::U8);
            assert_eq!(f.image.dimensions(), (5, 3));
            assert!(rgba.data().chunks(4).all(|p| p == want), "every pixel {want:?}");
        }
        // The first-frame decode still says frames were left out.
        assert!(matches!(crate::decode(&bytes).unwrap().warnings.first(), Some(crate::DecodeWarning::MoreFrames { total: Some(3) })));
    }

    #[test]
    fn a_still_webp_is_one_frame() {
        let still =
            crate::encode(&crate::Image::from_u8(2, 2, crate::ChannelLayout::Rgb, vec![9; 12]).unwrap(), Format::WebP, &crate::EncodeOptions::default())
                .unwrap();
        let out = crate::decode_frames(&still, &crate::DecodeOptions::default()).unwrap().unwrap();
        assert_eq!(out.len(), 1);
        assert!(crate::decode_frames(b"neither", &crate::DecodeOptions::default()).is_none());
    }

    #[test]
    fn too_many_animated_frames_for_the_budget_are_refused() {
        let frames: Vec<WebpFrame> = (0..4).map(|_| WebpFrame { rgba: solid(64, 64, [1, 2, 3, 255]), delay_ms: 10 }).collect();
        let bytes = encode_webp_animated(64, 64, &frames, 0).unwrap();
        let mut opts = crate::DecodeOptions::default();
        opts.limits.max_alloc = 64 * 64 * 4 * 3;
        assert!(matches!(crate::decode_webp_frames(&bytes, &opts), Err(CodecError::LimitExceeded(_))));
    }

    #[test]
    fn bad_frames_are_refused() {
        assert!(encode_webp_animated(2, 2, &[], 0).is_err());
        assert!(encode_webp_animated(2, 2, &[WebpFrame { rgba: vec![0; 3], delay_ms: 0 }], 0).is_err());
    }
}
