//! PNG in and out, the wire format between PhotoCraft and the helper.

use std::io::Cursor;

use crate::{Error, RgbaImage};

/// Encode 8-bit RGBA pixels as a PNG.
pub fn encode_png_rgba(img: &RgbaImage) -> Result<Vec<u8>, Error> {
    if !img.is_consistent() {
        return Err(Error::InvalidBuffer);
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        // Fast compression: this crosses a local socket, not the internet.
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc.write_header().map_err(|e| Error::Png(e.to_string()))?;
        writer.write_image_data(&img.data).map_err(|e| Error::Png(e.to_string()))?;
    }
    Ok(out)
}

/// The most pixels [`decode_png_rgba`] accepts without a size to expect (268 million, 1 GiB of
/// RGBA): a reply that claims more is refused before any buffer is allocated.
pub const MAX_PIXELS: u64 = 1 << 28;

/// Decode any PNG (grey, grey+alpha, RGB, RGBA, palette, 8 or 16 bit) to 8-bit RGBA. The
/// header's size is checked against [`MAX_PIXELS`] before the decode buffer exists.
pub fn decode_png_rgba(bytes: &[u8]) -> Result<RgbaImage, Error> {
    decode(bytes, None)
}

/// [`decode_png_rgba`] for a reply whose size is known in advance: the header must announce
/// exactly `width` × `height`, checked before anything is allocated ([`Error::WrongSize`]
/// otherwise), so a helper that misbehaves cannot make the app reserve memory for a huge image.
pub fn decode_png_rgba_expecting(bytes: &[u8], width: u32, height: u32) -> Result<RgbaImage, Error> {
    decode(bytes, Some((width, height)))
}

fn decode(bytes: &[u8], expected: Option<(u32, u32)>) -> Result<RgbaImage, Error> {
    let mut dec = png::Decoder::new(Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().map_err(|e| Error::Png(e.to_string()))?;
    let (hw, hh) = (reader.info().width, reader.info().height);
    match expected {
        Some((w, h)) if (hw, hh) != (w, h) => return Err(Error::WrongSize { want_w: w, want_h: h, got_w: hw, got_h: hh }),
        Some(_) => {}
        None if u64::from(hw) * u64::from(hh) > MAX_PIXELS => return Err(Error::Png(format!("image too large ({hw}x{hh})"))),
        None => {}
    }
    let size = reader.output_buffer_size().ok_or_else(|| Error::Png("image too large".into()))?;
    let mut buf = vec![0u8; size];
    let info = reader.next_frame(&mut buf).map_err(|e| Error::Png(e.to_string()))?;
    buf.truncate(info.buffer_size());
    let (width, height) = (info.width, info.height);
    let n = (width as usize).checked_mul(height as usize).ok_or_else(|| Error::Png("image too large".into()))?;
    let data = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Indexed => return Err(Error::Png("palette image was not expanded".into())),
    };
    if data.len() != n * 4 {
        return Err(Error::Png("decoded size does not match header".into()));
    }
    Ok(RgbaImage { width, height, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_roundtrip() -> Result<(), Error> {
        let mut img = RgbaImage::filled(3, 2, [10, 20, 30, 40]).ok_or(Error::InvalidBuffer)?;
        img.data[0..4].copy_from_slice(&[255, 0, 0, 255]);
        let bytes = encode_png_rgba(&img)?;
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        let back = decode_png_rgba(&bytes)?;
        assert_eq!(back, img);
        assert_eq!(decode_png_rgba_expecting(&bytes, 3, 2)?, img);
        Ok(())
    }

    #[test]
    fn rgb_png_gets_opaque_alpha() -> Result<(), Error> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 2, 1);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().map_err(|e| Error::Png(e.to_string()))?;
            w.write_image_data(&[1, 2, 3, 4, 5, 6]).map_err(|e| Error::Png(e.to_string()))?;
        }
        let img = decode_png_rgba(&out)?;
        assert_eq!((img.width, img.height), (2, 1));
        assert_eq!(img.data, vec![1, 2, 3, 255, 4, 5, 6, 255]);
        Ok(())
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(matches!(decode_png_rgba(b"not a png"), Err(Error::Png(_))));
        let bad = RgbaImage { width: 2, height: 2, data: vec![0; 3] };
        assert!(matches!(encode_png_rgba(&bad), Err(Error::InvalidBuffer)));
    }

    /// A real 1×1 PNG whose header is rewritten to claim `width` × `height` (checksum redone):
    /// what a misbehaving helper could send. The size check must run before any decode buffer.
    fn lying_header(width: u32, height: u32) -> Vec<u8> {
        let one = RgbaImage::filled(1, 1, [1, 2, 3, 4]).expect("1x1");
        let mut bytes = encode_png_rgba(&one).expect("encode");
        // Signature (8) + IHDR length (4) + "IHDR" (4): width and height follow.
        bytes[16..20].copy_from_slice(&width.to_be_bytes());
        bytes[20..24].copy_from_slice(&height.to_be_bytes());
        let crc = crc32(&bytes[12..29]);
        bytes[29..33].copy_from_slice(&crc.to_be_bytes());
        bytes
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }

    #[test]
    fn an_unexpected_size_is_refused_from_the_header() {
        let bytes = lying_header(60_000, 60_000);
        let err = decode_png_rgba_expecting(&bytes, 4, 3);
        assert!(matches!(err, Err(Error::WrongSize { want_w: 4, want_h: 3, got_w: 60_000, got_h: 60_000 })), "{err:?}");
        let err = decode_png_rgba(&bytes);
        assert!(matches!(err, Err(Error::Png(ref m)) if m.contains("too large")), "{err:?}");
    }
}
