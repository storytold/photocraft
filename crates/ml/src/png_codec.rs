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

/// Decode any PNG (grey, grey+alpha, RGB, RGBA, palette, 8 or 16 bit) to 8-bit RGBA.
pub fn decode_png_rgba(bytes: &[u8]) -> Result<RgbaImage, Error> {
    let mut dec = png::Decoder::new(Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().map_err(|e| Error::Png(e.to_string()))?;
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
}
