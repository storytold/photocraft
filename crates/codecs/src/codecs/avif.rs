//! AVIF decoding for previews of files this crate wrote: Save for Web's Optimized pane shows the
//! compression artefacts by decoding the file it just encoded.
//!
//! Decoding uses rav1d (the Rust port of dav1d, pinned to a revision of the official repository
//! and built without its assembly, so it is pure Rust) and avif-parse for the container, with the
//! `avif` feature. It is not File › Open's AVIF support (#463): container transforms
//! (`irot`/`imir`/`clap`), grid images and the `colr` ICC profile are not applied, which our own
//! writer never produces. Without the feature this is a [`CodecError::Unsupported`] error.

use crate::Format;
use crate::error::CodecError;
use crate::image::Image;

const F: Format = Format::Avif;

#[cfg(not(feature = "avif"))]
pub(crate) fn decode_preview(_bytes: &[u8]) -> Result<Image, CodecError> {
    Err(CodecError::unsupported(F, "AVIF isn't included in this build of PhotoCraft"))
}

/// One decoded AV1 frame: full-resolution luma, chroma at the layout's subsampling.
#[cfg(feature = "avif")]
struct Frame {
    width: u32,
    height: u32,
    /// Chroma subsampling shifts (x, y); `None` for monochrome.
    sub: Option<(u32, u32)>,
    chroma_width: u32,
    bits: u32,
    full_range: bool,
    matrix: rav1d::pixel::MatrixCoefficients,
    y: Vec<u16>,
    u: Vec<u16>,
    v: Vec<u16>,
}

#[cfg(feature = "avif")]
fn decode_av1(obu: &[u8]) -> Result<Frame, CodecError> {
    use rav1d::{Decoder, PixelLayout, PlanarImageComponent as P, Rav1dError, Settings};
    let bad = |e: Rav1dError| CodecError::malformed(F, format!("AV1: {e:?}"));
    // A still image is one frame: a frame delay of 1 hands it back as soon as it is decoded.
    let mut settings = Settings::new();
    settings.set_max_frame_delay(1);
    let mut decoder = Decoder::with_settings(&settings).map_err(bad)?;
    let mut sent = decoder.send_data(obu.to_vec().into_boxed_slice(), None, None, None);
    while matches!(sent, Err(Rav1dError::TryAgain)) {
        sent = decoder.send_pending_data();
    }
    sent.map_err(bad)?;
    let pic = decoder.get_picture().map_err(bad)?;
    let (width, height) = (pic.width(), pic.height());
    let sub = match pic.pixel_layout() {
        PixelLayout::I400 => None,
        PixelLayout::I420 => Some((1, 1)),
        PixelLayout::I422 => Some((1, 0)),
        PixelLayout::I444 => Some((0, 0)),
    };
    let (sx, sy) = sub.unwrap_or((0, 0));
    let (chroma_width, chroma_height) = ((width + sx) >> sx, (height + sy) >> sy);
    // 10- and 12-bit samples are stored as little-endian u16.
    let wide = pic.bit_depth() == 16;
    let plane = |c: P, w: u32, h: u32| -> Result<Vec<u16>, CodecError> {
        let (stride, data) = (pic.stride(c) as usize, pic.plane(c));
        let mut out = Vec::with_capacity(w as usize * h as usize);
        for y in 0..h as usize {
            let row = data.get(y * stride..).ok_or_else(|| CodecError::malformed(F, "AV1 plane is shorter than its size"))?;
            if wide {
                out.extend(row.as_chunks::<2>().0.iter().take(w as usize).map(|b| u16::from_le_bytes(*b)));
            } else {
                out.extend(row.iter().take(w as usize).map(|v| u16::from(*v)));
            }
        }
        if out.len() != w as usize * h as usize {
            return Err(CodecError::malformed(F, "AV1 plane is shorter than its size"));
        }
        Ok(out)
    };
    let y = plane(P::Y, width, height)?;
    let (u, v) = if sub.is_some() { (plane(P::U, chroma_width, chroma_height)?, plane(P::V, chroma_width, chroma_height)?) } else { Default::default() };
    Ok(Frame {
        width,
        height,
        sub,
        chroma_width,
        bits: pic.bits_per_component().map_or(8, |b| u32::from(b.0)),
        full_range: pic.color_range() == rav1d::pixel::YUVRange::Full,
        matrix: pic.matrix_coefficients(),
        y,
        u,
        v,
    })
}

#[cfg(feature = "avif")]
impl Frame {
    /// Luma (or a full-scale sample) 0..=1 for its range.
    fn luma(&self, v: u16) -> f32 {
        let scale = (1u32 << (self.bits - 8)) as f32;
        if self.full_range { f32::from(v) / ((1u32 << self.bits) - 1) as f32 } else { (f32::from(v) - 16.0 * scale) / (219.0 * scale) }
    }
    /// Chroma -0.5..=0.5 for its range.
    fn chroma(&self, v: u16) -> f32 {
        let scale = (1u32 << (self.bits - 8)) as f32;
        if self.full_range {
            (f32::from(v) - (1u32 << (self.bits - 1)) as f32) / ((1u32 << self.bits) - 1) as f32
        } else {
            (f32::from(v) - 128.0 * scale) / (224.0 * scale)
        }
    }
}

/// Decodes `bytes` to straight RGBA8, nearest-sample chroma upsampling.
#[cfg(feature = "avif")]
pub(crate) fn decode_preview(bytes: &[u8]) -> Result<Image, CodecError> {
    use rav1d::pixel::MatrixCoefficients as M;
    let avif = avif_parse::read_avif(&mut &bytes[..]).map_err(|e| CodecError::malformed(F, e))?;
    let color = decode_av1(&avif.primary_item)?;
    let alpha = avif.alpha_item.as_deref().map(decode_av1).transpose()?;
    if alpha.as_ref().is_some_and(|a| (a.width, a.height) != (color.width, color.height)) {
        return Err(CodecError::malformed(F, "the alpha image's size differs from the colour image's"));
    }
    // Luma weights (Kr, Kb) by matrix; unspecified and the BT.601 family use BT.601's.
    let (kr, kb) = match color.matrix {
        M::BT709 => (0.2126, 0.0722),
        M::BT2020NonConstantLuminance => (0.2627, 0.0593),
        M::BT470M => (0.30, 0.11),
        M::ST240M => (0.212, 0.087),
        M::Identity | M::Unspecified | M::BT470BG | M::ST170M => (0.299, 0.114),
        other => return Err(CodecError::unsupported(F, format!("matrix coefficients {other:?}"))),
    };
    let kg = 1.0 - kr - kb;
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (w, h) = (color.width as usize, color.height as usize);
    let mut data = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let luma = color.luma(color.y[i]);
            let rgb = match color.sub {
                None => [luma; 3],
                Some((sx, sy)) => {
                    let c = (y >> sy) * color.chroma_width as usize + (x >> sx);
                    if color.matrix == M::Identity {
                        // GBR: Y holds green, U blue and V red, each a full-scale sample.
                        [color.luma(color.v[c]), luma, color.luma(color.u[c])]
                    } else {
                        let (cb, cr) = (color.chroma(color.u[c]), color.chroma(color.v[c]));
                        let r = luma + 2.0 * (1.0 - kr) * cr;
                        let b = luma + 2.0 * (1.0 - kb) * cb;
                        [r, (luma - kr * r - kb * b) / kg, b]
                    }
                }
            };
            let a = alpha.as_ref().map_or(1.0, |a| a.luma(a.y[i]).clamp(0.0, 1.0));
            // A premultiplied file (`prem`) divides colour back out by alpha.
            let rgb = if avif.premultiplied_alpha && a > 0.0 { rgb.map(|v| v / a) } else { rgb };
            data.extend_from_slice(&[to8(rgb[0]), to8(rgb[1]), to8(rgb[2]), to8(a)]);
        }
    }
    Image::from_u8(color.width, color.height, crate::image::ChannelLayout::Rgba, data)
}

#[cfg(all(test, feature = "avif"))]
mod tests {
    use crate::{ChannelLayout, EncodeOptions, Format, Image, encode};

    /// A gradient with a translucent band, through our writer and back: close at high quality,
    /// visibly lossy at low quality, alpha kept.
    #[test]
    fn our_avif_round_trips_with_its_artefacts_and_alpha() {
        let (w, h) = (96u32, 64u32);
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let a = if (16..32).contains(&y) { 128 } else { 255 };
                data.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, ((x ^ y) * 3) as u8, a]);
            }
        }
        let src = Image::from_u8(w, h, ChannelLayout::Rgba, data.clone()).unwrap();
        let mse = |q: u8| {
            let bytes = encode(&src, Format::Avif, &EncodeOptions { jpeg_quality: q, ..Default::default() }).unwrap();
            let out = super::decode_preview(&bytes).unwrap();
            assert_eq!((out.width(), out.height(), out.layout()), (w, h, ChannelLayout::Rgba));
            let px = out.data();
            let alpha_ok = px.as_chunks::<4>().0.iter().zip(data.as_chunks::<4>().0).all(|(a, b)| a[3].abs_diff(b[3]) <= 8);
            assert!(alpha_ok, "alpha survives at quality {q}");
            px.iter().zip(&data).map(|(a, b)| f64::from(a.abs_diff(*b)).powi(2)).sum::<f64>() / px.len() as f64
        };
        let (hi, lo) = (mse(95), mse(5));
        assert!(hi < 10.0, "quality 95 is close to the source: MSE {hi}");
        assert!(lo > hi * 4.0, "quality 5 shows its artefacts: MSE {lo} vs {hi}");
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(super::decode_preview(b"\0\0\0\x18ftypavif").is_err());
        assert!(super::decode_preview(&[]).is_err());
    }
}
