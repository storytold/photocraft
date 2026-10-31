//! Turn a scanner file (JPEG, PNG or TIFF) into 8-bit RGBA, then apply the requested mode.

use crate::{Error, Mode, ScanImage};

/// Largest scan kept in memory: 128 megapixels, which is 512MB of RGBA.
pub(crate) const MAX_PIXELS: u64 = 128 * 1024 * 1024;

pub(crate) fn decode_scan(bytes: &[u8]) -> Result<ScanImage, Error> {
    let img = image::load_from_memory(bytes).map_err(|e| Error::Platform(format!("the scan could not be read ({e})")))?;
    let rgba = img.into_rgba8();
    let (width, height) = rgba.dimensions();
    let pixels = u64::from(width).checked_mul(u64::from(height)).ok_or_else(|| Error::Platform("the scan is too large".into()))?;
    if pixels > MAX_PIXELS {
        return Err(Error::Platform("the scan is too large".into()));
    }
    Ok(ScanImage { width, height, rgba: rgba.into_raw(), dpi: 0, pages: 1 })
}

/// Rec. 709-ish luma in 8.8 fixed point (54 + 183 + 19 = 256).
fn luma(r: u8, g: u8, b: u8) -> u8 {
    let y = 54 * u32::from(r) + 183 * u32::from(g) + 19 * u32::from(b);
    (y >> 8).min(255) as u8
}

pub(crate) fn apply_mode(rgba: &mut [u8], mode: Mode) {
    if mode == Mode::Color {
        return;
    }
    for px in rgba.as_chunks_mut::<4>().0 {
        let y = luma(px[0], px[1], px[2]);
        let v = if mode == Mode::BlackAndWhite { if y >= 128 { 255 } else { 0 } } else { y };
        px[0] = v;
        px[1] = v;
        px[2] = v;
        px[3] = 255;
    }
}

#[cfg(test)]
mod tests {
    use image::ImageEncoder;

    use super::*;

    fn png(pixels: &[u8], w: u32, h: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_raw(w, h, pixels.to_vec()).unwrap();
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        bytes
    }

    #[test]
    fn a_png_round_trips_exact_pixels() {
        let decoded = decode_scan(&png(&[10, 20, 30, 255], 1, 1)).unwrap();
        assert_eq!((decoded.width, decoded.height, decoded.rgba.as_slice()), (1, 1, &[10, 20, 30, 255][..]));
    }

    #[test]
    fn gray_and_black_and_white_collapse_the_colour() {
        let mut px = [0, 255, 0, 128];
        apply_mode(&mut px, Mode::Gray);
        assert_eq!(px[0], px[1]);
        assert_eq!(px[1], px[2]);
        assert_eq!(px[3], 255);
        assert!(px[0] > 128);
        let mut bw = [10, 10, 10, 255, 200, 200, 200, 255];
        apply_mode(&mut bw, Mode::BlackAndWhite);
        assert_eq!(&bw[..4], &[0, 0, 0, 255]);
        assert_eq!(&bw[4..], &[255, 255, 255, 255]);
    }

    #[test]
    fn a_jpeg_decodes() {
        let image = image::RgbImage::from_pixel(2, 1, image::Rgb([40, 80, 120]));
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 90).write_image(image.as_raw(), 2, 1, image::ExtendedColorType::Rgb8).unwrap();
        let decoded = decode_scan(&bytes).unwrap();
        assert_eq!((decoded.width, decoded.height, decoded.rgba.len()), (2, 1, 8));
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(decode_scan(b"not an image").is_err());
    }
}
