//! `photocraft-heif`: the optional HEIF / HEIC (iPhone, Mac and camera photo) decoder.
//!
//! A thin wrapper around `heic-decoder`, a pure-Rust HEVC still-picture decoder whose output
//! is bit-exact against HM, the HEVC reference decoder: single pictures and grid-tiled photos,
//! 8/10/12-bit, 4:2:0, 4:2:2, 4:4:4 and monochrome, alpha auxiliary images, ICC, EXIF and XMP.
//! Its API is plain data ([`Info`], [`Decoded`], [`Error`]) so the crate knows nothing about the
//! rest of PhotoCraft; `photocraft-codecs` adapts it behind its `heif` feature.
//!
//! HEIF records orientation in the container (`irot`/`imir`, plus a `clap` crop), not in EXIF.
//! [`Options::apply_transforms`] applies them; with it off, the pixels come back as coded.
//!
//! Never panics: the decoder returns errors on malformed input, and every call into it also runs
//! under `catch_unwind` as a last resort, so a panic would become [`Error::Malformed`].

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use heic_decoder::{DecodeOptions, HeifError};

/// What a HEIF file declares, read from the container alone (no pixel is decoded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    /// Size after the container transforms (rotation, mirror, crop): what a viewer shows.
    pub width: u32,
    pub height: u32,
    /// Size as coded, before the transforms.
    pub coded_width: u32,
    pub coded_height: u32,
    /// The primary image has an alpha auxiliary image.
    pub has_alpha: bool,
    /// More than 8 bits per sample (colour or alpha): it decodes to 16-bit.
    pub sixteen_bit: bool,
}

/// Decode settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Refuse images with more pixels than this (checked before decoding).
    pub max_pixels: u64,
    /// Apply the container's rotation, mirror and crop (`irot`/`imir`/`clap`).
    pub apply_transforms: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { max_pixels: 1 << 28, apply_transforms: true }
    }
}

/// A decoded image: interleaved RGB or RGBA samples, row-major, no padding. 16-bit samples are
/// native-endian `u16`s spanning `0..=65535`. Alpha is straight (not premultiplied).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    /// Four channels (RGBA) instead of three (RGB).
    pub has_alpha: bool,
    /// Two bytes per sample instead of one.
    pub sixteen_bit: bool,
    pub data: Vec<u8>,
    /// The primary image's ICC profile.
    pub icc: Option<Vec<u8>>,
    /// EXIF as a TIFF structure (without HEIF's offset header). Its Orientation tag only mirrors
    /// what the container says; it is returned verbatim.
    pub exif: Option<Vec<u8>>,
    /// The XMP packet.
    pub xmp: Option<String>,
}

/// Why a file could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A valid file using something this decoder does not handle (AVC, layered HEVC…).
    Unsupported(String),
    /// The image is larger than [`Options::max_pixels`] or the decoder's own limit.
    Limit(String),
    /// Broken or truncated data (or a decoder bug, reported the same way).
    Malformed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unsupported(m) => write!(f, "unsupported HEIF: {m}"),
            Error::Limit(m) => write!(f, "limit exceeded: {m}"),
            Error::Malformed(m) => write!(f, "malformed HEIF: {m}"),
        }
    }
}

impl std::error::Error for Error {}

fn err(e: HeifError) -> Error {
    match e {
        HeifError::Unsupported(what) => Error::Unsupported(what.to_string()),
        HeifError::LimitExceeded(_) => Error::Limit(e.to_string()),
        e => Error::Malformed(e.to_string()),
    }
}

/// Runs `f`, turning a decoder panic into an error. heic-decoder is written not to panic
/// (and is fuzzed for it); this is the last-resort net for a file nobody has tried yet.
fn guarded<T>(f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err(Error::Malformed("the HEIF decoder failed on this file".into())))
}

fn size(v: usize) -> Result<u32, Error> {
    u32::try_from(v).map_err(|_| Error::Limit(format!("dimension {v} does not fit in 32 bits")))
}

/// Reads the container's declared size, depth and alpha without decoding pixels, so callers can
/// check their limits first.
pub fn probe(bytes: &[u8]) -> Result<Info, Error> {
    guarded(|| probe_unguarded(bytes))
}

fn probe_unguarded(bytes: &[u8]) -> Result<Info, Error> {
    let info = heic_decoder::probe(bytes).map_err(err)?;
    let deepest = info.bit_depth_luma.max(info.bit_depth_chroma).max(info.alpha_bit_depth.unwrap_or(0));
    Ok(Info {
        width: size(info.width)?,
        height: size(info.height)?,
        coded_width: size(info.coded_width)?,
        coded_height: size(info.coded_height)?,
        has_alpha: info.alpha_bit_depth.is_some(),
        sixteen_bit: deepest > 8,
    })
}

/// Decodes the primary image, with its metadata.
pub fn decode(bytes: &[u8], options: &Options) -> Result<Decoded, Error> {
    guarded(|| decode_unguarded(bytes, options))
}

fn decode_unguarded(bytes: &[u8], options: &Options) -> Result<Decoded, Error> {
    let info = probe_unguarded(bytes)?;
    let (w, h) = if options.apply_transforms { (info.width, info.height) } else { (info.coded_width, info.coded_height) };
    let pixels = u64::from(w) * u64::from(h);
    if pixels > options.max_pixels {
        return Err(Error::Limit(format!("{w}x{h} is {pixels} pixels, more than the {} allowed", options.max_pixels)));
    }
    let image = heic_decoder::decode_with(bytes, &DecodeOptions { apply_transforms: options.apply_transforms }).map_err(err)?;
    let has_alpha = image.alpha.is_some();
    let deepest = image.bit_depth_luma.max(image.bit_depth_chroma).max(image.alpha.as_ref().map_or(0, |a| a.bit_depth));
    let sixteen_bit = deepest > 8;
    let data = match (has_alpha, sixteen_bit) {
        (true, true) => {
            let mut rgba = image.to_rgba16().map_err(err)?;
            if rgba.premultiplied {
                unpremultiply(&mut rgba.data, u16::MAX);
            }
            rgba.data.iter().flat_map(|v| v.to_ne_bytes()).collect()
        }
        (true, false) => {
            let mut rgba = image.to_rgba8().map_err(err)?;
            if rgba.premultiplied {
                unpremultiply(&mut rgba.data, u8::MAX);
            }
            rgba.data
        }
        (false, true) => image.to_rgb16().map_err(err)?.data.iter().flat_map(|v| v.to_ne_bytes()).collect(),
        (false, false) => image.to_rgb8().map_err(err)?.data,
    };
    let metadata = image.metadata;
    let xmp = metadata.xmp.and_then(|x| String::from_utf8(x).ok()).map(|x| x.trim_end_matches('\0').to_string());
    Ok(Decoded {
        width: size(image.width)?,
        height: size(image.height)?,
        has_alpha,
        sixteen_bit,
        data,
        icc: metadata.icc_profile,
        exif: metadata.exif,
        xmp,
    })
}

/// Turns premultiplied RGBA (a `prem` reference in the file) into straight alpha, in place.
/// Fully transparent pixels become black; colour is clamped to the alpha it was scaled by.
fn unpremultiply<T: Copy + Into<u32> + TryFrom<u32>>(rgba: &mut [T], max: T) {
    let max: u32 = max.into();
    for [r, g, b, a] in rgba.as_chunks_mut::<4>().0 {
        let alpha: u32 = (*a).into();
        for c in [r, g, b] {
            let straight = ((*c).into() * max + alpha / 2).checked_div(alpha).unwrap_or(0);
            if let Ok(v) = T::try_from(straight.min(max)) {
                *c = v;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made heic-rs 0.1.1, the previous decoder, slice out of range in its box parser. Kept as a
    /// regression input: it must be an error, not a crash.
    const HEIC_RS_BOX_PANIC: [u8; 72] = [
        0x00, 0x00, 0x00, 0x24, 0x66, 0x74, 0x79, 0x70, 0x68, 0x65, 0x69, 0x63, 0x00, 0x00, 0x00, 0x00, 0x6d, 0x69, 0x66, 0x31, 0x4d, 0x69, 0x50, 0x72, 0x6d,
        0x69, 0x61, 0x66, 0x4d, 0x69, 0x48, 0x42, 0x72, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x6d, 0x65, 0x74, 0x61, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x10, 0x75, 0x75, 0x69, 0x64, 0x00, 0x00, 0x00, 0x00, 0x03, 0x08, 0x08, 0x08, 0x00, 0x03, 0x01, 0x03, 0x70, 0x00, 0xa8, 0x00,
    ];

    #[test]
    fn a_malformed_box_is_an_error() {
        for apply_transforms in [true, false] {
            let r = decode(&HEIC_RS_BOX_PANIC, &Options { apply_transforms, ..Default::default() });
            assert!(matches!(r, Err(Error::Malformed(_))), "{r:?}");
        }
    }

    #[test]
    fn a_sequence_brand_without_a_track_is_an_error() {
        // An ftyp and nothing else: a HEIF image sequence would have a moov here instead of a meta.
        let bytes = b"\0\0\0\x18ftypmsf1\0\0\0\0msf1hevc";
        for r in [probe(bytes).map(|_| ()), decode(bytes, &Options::default()).map(|_| ())] {
            assert!(r.is_err(), "{r:?}");
        }
    }

    #[test]
    fn garbage_is_an_error() {
        for bytes in [&b""[..], b"\0\0\0\x18ftypheic\0\0\0\0mif1heic", &[0xFF; 64]] {
            assert!(probe(bytes).is_err());
            assert!(decode(bytes, &Options::default()).is_err());
        }
    }

    #[test]
    fn errors_display_their_kind() {
        assert!(Error::Unsupported("x".into()).to_string().contains("unsupported"));
        assert!(Error::Limit("x".into()).to_string().contains("limit"));
        assert!(Error::Malformed("x".into()).to_string().contains("malformed"));
    }

    #[test]
    fn decoder_errors_map_to_their_kind() {
        assert!(matches!(err(HeifError::Unsupported("AVC")), Error::Unsupported(m) if m == "AVC"));
        assert!(matches!(err(HeifError::LimitExceeded("big")), Error::Limit(_)));
        assert!(matches!(err(HeifError::CabacDesync("x")), Error::Malformed(_)));
        assert!(matches!(err(HeifError::Truncated { at: 1, needed: 2 }), Error::Malformed(_)));
    }

    #[test]
    fn unpremultiply_restores_straight_colour() {
        let mut px: [u8; 12] = [64, 32, 0, 128, 10, 10, 10, 0, 255, 255, 255, 255];
        unpremultiply(&mut px, u8::MAX);
        assert_eq!(px, [128, 64, 0, 128, 0, 0, 0, 0, 255, 255, 255, 255]);
        // Colour above its alpha (invalid premultiplication) clamps instead of overflowing.
        let mut px: [u16; 4] = [65535, 1000, 0, 1];
        unpremultiply(&mut px, u16::MAX);
        assert_eq!(px, [65535, 65535, 0, 1]);
    }
}
