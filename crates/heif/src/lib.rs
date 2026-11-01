//! `photocraft-heif`: the optional HEIF / HEIC (iPhone, Mac and camera photo) decoder.
//!
//! A thin wrapper around `heic-decoder`, a pure-Rust HEVC still-picture decoder whose output
//! is bit-exact against HM, the HEVC reference decoder: single pictures and grid-tiled photos,
//! 8/10/12-bit, 4:2:0, 4:2:2, 4:4:4 and monochrome, alpha auxiliary images, ICC, EXIF and XMP.
//! Its API is plain data ([`Info`], [`Decoded`], [`Error`]) so the crate knows nothing about the
//! rest of PhotoCraft; `photocraft-codecs` adapts it behind its `heif` feature.
//!
//! A grid's ICC profile and `nclx` come from its tiles when the grid item itself carries no `colr`,
//! which is how phone cameras write them; heic-decoder resolves both.
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
    /// Refuse images with more pixels than this, as coded (checked before decoding).
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
    /// The primary image's `nclx` colour description (inherited from its tiles like [`Self::icc`]).
    /// The decoder has already applied its matrix and range; its primaries and transfer
    /// characteristics describe the RGB samples returned.
    pub nclx: Option<Nclx>,
    /// EXIF as a TIFF structure (without HEIF's offset header). Its Orientation tag only mirrors
    /// what the container says; it is returned verbatim.
    pub exif: Option<Vec<u8>>,
    /// The XMP packet.
    pub xmp: Option<String>,
}

/// A `colr` box of type `nclx`: ITU-T H.273 code points (CICP), as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nclx {
    /// Colour primaries: 1 BT.709, 9 BT.2020, 12 P3-D65…
    pub primaries: u16,
    /// Transfer characteristics: 13 sRGB, 1/6/14/15 BT.709, 16 PQ, 18 HLG…
    pub transfer: u16,
    /// Matrix coefficients (already applied by the decoder).
    pub matrix: u16,
    /// Full-range samples (already applied by the decoder).
    pub full_range: bool,
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
    // The decoder allocates the coded picture before cropping it, and the transforms never make
    // it larger, so the coded size bounds the work either way.
    let (w, h) = (info.coded_width, info.coded_height);
    let pixels = u64::from(w) * u64::from(h);
    if pixels > options.max_pixels {
        return Err(Error::Limit(format!("{w}x{h} as coded is {pixels} pixels, more than the {} allowed", options.max_pixels)));
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
    // The container's `nclx`, inherited from the tiles like the profile. Not `image.color_info`,
    // which falls back to the HEVC stream's VUI.
    let nclx = metadata.color_info.map(|c| Nclx {
        primaries: c.color_primaries,
        transfer: c.transfer_characteristics,
        matrix: c.matrix_coefficients,
        full_range: c.full_range,
    });
    let xmp = metadata.xmp.and_then(|x| String::from_utf8(x).ok()).map(|x| x.trim_end_matches('\0').to_string());
    Ok(Decoded {
        width: size(image.width)?,
        height: size(image.height)?,
        has_alpha,
        sixteen_bit,
        data,
        icc: metadata.icc_profile,
        nclx,
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

    /// Made heic-rs 0.1.1 slice out of range in its box parser (fixed in 0.1.2). Kept as a
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
    fn a_sequence_brand_without_a_track_is_malformed() {
        // An ftyp and nothing else: no still image (`meta`) and no image sequence track (`moov`).
        // A real sequence opens at its first sync frame, so this is a broken file, not an
        // unsupported one.
        let bytes = b"\0\0\0\x18ftypmsf1\0\0\0\0msf1hevc";
        for r in [probe(bytes).map(|_| ()), decode(bytes, &Options::default()).map(|_| ())] {
            assert!(matches!(&r, Err(Error::Malformed(m)) if m.contains("meta")), "{r:?}");
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

    // Synthetic containers around one real coded picture, for container behaviour no corpus file
    // pins: the `imir` direction and where a grid's ICC profile comes from. Each test first checks
    // the fixture is the shape it claims to be, so a mistake in the builder cannot look like a pass.

    /// `hvcC` of a 64x64 4:2:0 picture: red top left, green top right, blue bottom left, white
    /// bottom right. Encoded by macOS `sips` from a PNG generated for this test.
    const QUAD_HVCC: [u8; 104] = [
        0x01, 0x03, 0x70, 0x00, 0x00, 0x00, 0xb0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1e, 0xf0, 0x00, 0xfc, 0xfd, 0xf8, 0xf8, 0x00, 0x00, 0x0b, 0x03, 0xa0, 0x00,
        0x01, 0x00, 0x17, 0x40, 0x01, 0x0c, 0x01, 0xff, 0xff, 0x03, 0x70, 0x00, 0x00, 0x03, 0x00, 0xb0, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03, 0x00, 0x1e, 0x70,
        0x24, 0xa1, 0x00, 0x01, 0x00, 0x22, 0x42, 0x01, 0x01, 0x03, 0x70, 0x00, 0x00, 0x03, 0x00, 0xb0, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03, 0x00, 0x1e, 0xa0,
        0x14, 0x20, 0x41, 0xc1, 0x8f, 0x88, 0x7b, 0x91, 0x65, 0x53, 0x70, 0x20, 0x20, 0x60, 0x08, 0xa2, 0x00, 0x01, 0x00, 0x09, 0x44, 0x01, 0xc0, 0x60, 0x84,
        0xb2, 0x10, 0x14, 0xc9,
    ];
    /// Its coded slice, length-prefixed as HEIF stores it.
    const QUAD_HEVC: [u8; 107] = [
        0x00, 0x00, 0x00, 0x67, 0x28, 0x01, 0xaf, 0xa3, 0x57, 0x62, 0x9a, 0xec, 0x27, 0xf8, 0x03, 0x13, 0xff, 0xfc, 0xad, 0xfe, 0x87, 0xf5, 0xa9, 0xbc, 0x31,
        0x41, 0xfe, 0x0d, 0x57, 0xff, 0xf8, 0x11, 0x5f, 0xff, 0xd1, 0x7e, 0xa3, 0xa9, 0x0f, 0xd8, 0x2a, 0x6e, 0x3e, 0x90, 0x0f, 0x2f, 0xf1, 0xc0, 0x0c, 0x3b,
        0x8f, 0xff, 0x40, 0x51, 0xda, 0x67, 0x8f, 0xe6, 0x09, 0x34, 0x7e, 0xaf, 0x61, 0x13, 0xf7, 0xd0, 0xe2, 0x58, 0xbb, 0xa1, 0xfd, 0xfd, 0xb1, 0xc1, 0x5a,
        0x32, 0xe1, 0xe4, 0xf9, 0xa1, 0xc6, 0x73, 0xf3, 0x9f, 0xd5, 0x8e, 0xbb, 0xe1, 0x65, 0xb9, 0x4b, 0x7f, 0xc0, 0xdf, 0x58, 0xfe, 0x29, 0x3a, 0x7f, 0x79,
        0x8e, 0x9f, 0x42, 0xfd, 0x30, 0xff, 0xc0,
    ];

    /// Stands in for a profile. `colr` hands its payload back without parsing it.
    const TILE_ICC: &[u8] = b"tile-profile";
    const GRID_ICC: &[u8] = b"grid-profile";

    const GRID_ID: u16 = 1;
    const PICTURE_ID: u16 = 2;

    fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = u32::try_from(payload.len() + 8).unwrap().to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(payload);
        v
    }

    /// A box whose first four payload bytes are a version and 24 bits of flags.
    fn full(kind: &[u8; 4], version: u8, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![version, 0, 0, 0];
        p.extend_from_slice(payload);
        bx(kind, &p)
    }

    fn hdlr() -> Vec<u8> {
        let mut p = vec![0; 4];
        p.extend_from_slice(b"pict");
        p.extend_from_slice(&[0; 12]);
        p.push(0);
        full(b"hdlr", 0, &p)
    }

    fn infe(id: u16, item_type: &[u8; 4]) -> Vec<u8> {
        let mut p = id.to_be_bytes().to_vec();
        p.extend_from_slice(&0u16.to_be_bytes());
        p.extend_from_slice(item_type);
        p.push(0);
        full(b"infe", 2, &p)
    }

    fn colr_prof(icc: &[u8]) -> Vec<u8> {
        let mut p = b"prof".to_vec();
        p.extend_from_slice(icc);
        bx(b"colr", &p)
    }

    /// A `colr` box of type `nclx`, laid out as ISO/IEC 23008-12 says: three big-endian `u16`
    /// code points, then the full-range flag in the top bit of a byte.
    fn colr_nclx(primaries: u16, transfer: u16, matrix: u16, full_range: bool) -> Vec<u8> {
        let mut p = b"nclx".to_vec();
        for v in [primaries, transfer, matrix] {
            p.extend_from_slice(&v.to_be_bytes());
        }
        p.push(if full_range { 0x80 } else { 0 });
        bx(b"colr", &p)
    }

    fn ispe(w: u32, h: u32) -> Vec<u8> {
        let mut p = w.to_be_bytes().to_vec();
        p.extend_from_slice(&h.to_be_bytes());
        full(b"ispe", 0, &p)
    }

    fn ipma(entries: &[(u16, &[u8])]) -> Vec<u8> {
        let mut p = u32::try_from(entries.len()).unwrap().to_be_bytes().to_vec();
        for (item, assoc) in entries {
            p.extend_from_slice(&item.to_be_bytes());
            p.push(u8::try_from(assoc.len()).unwrap());
            p.extend_from_slice(assoc);
        }
        full(b"ipma", 0, &p)
    }

    /// Each `(item, bytes)` lives in `idat` (construction method 1), one after the other.
    fn idat_and_iloc(items: &[(u16, &[u8])]) -> Vec<u8> {
        let mut iloc = vec![0x44, 0x00];
        iloc.extend_from_slice(&u16::try_from(items.len()).unwrap().to_be_bytes());
        let mut idat = Vec::new();
        for (item, bytes) in items {
            iloc.extend_from_slice(&item.to_be_bytes());
            iloc.extend_from_slice(&1u16.to_be_bytes());
            iloc.extend_from_slice(&0u16.to_be_bytes());
            iloc.extend_from_slice(&1u16.to_be_bytes());
            iloc.extend_from_slice(&u32::try_from(idat.len()).unwrap().to_be_bytes());
            iloc.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_be_bytes());
            idat.extend_from_slice(bytes);
        }
        let mut out = bx(b"idat", &idat);
        out.extend_from_slice(&full(b"iloc", 1, &iloc));
        out
    }

    /// `ftyp`, then a `meta` whose primary item is `primary` and whose items are `infes`.
    /// `ipco` children are numbered from 1 in file order; an association byte's top bit marks the
    /// property essential, as macOS writes it.
    fn file(primary: u16, infes: &[Vec<u8>], ipco: &[u8], assoc: &[(u16, &[u8])], data: &[(u16, &[u8])]) -> Vec<u8> {
        let mut ftyp = b"heic".to_vec();
        ftyp.extend_from_slice(&0u32.to_be_bytes());
        ftyp.extend_from_slice(b"mif1heic");
        let mut items = u16::try_from(infes.len()).unwrap().to_be_bytes().to_vec();
        items.extend(infes.iter().flatten());
        let mut iprp = bx(b"ipco", ipco);
        iprp.extend_from_slice(&ipma(assoc));

        let mut meta = hdlr();
        meta.extend_from_slice(&full(b"pitm", 0, &primary.to_be_bytes()));
        meta.extend_from_slice(&full(b"iinf", 0, &items));
        if primary == GRID_ID {
            let mut p = GRID_ID.to_be_bytes().to_vec();
            p.extend_from_slice(&1u16.to_be_bytes());
            p.extend_from_slice(&PICTURE_ID.to_be_bytes());
            meta.extend_from_slice(&full(b"iref", 0, &bx(b"dimg", &p)));
        }
        meta.extend_from_slice(&bx(b"iprp", &iprp));
        meta.extend_from_slice(&idat_and_iloc(data));
        let mut out = bx(b"ftyp", &ftyp);
        out.extend_from_slice(&full(b"meta", 0, &meta));
        out
    }

    /// The quadrant picture as the primary item, with an `imir` of `mirror_axis` and a `prof`
    /// of `icc` when given.
    fn picture_file(mirror_axis: Option<u8>, icc: Option<&[u8]>) -> Vec<u8> {
        picture_file_colr(mirror_axis, icc.map(colr_prof))
    }

    /// [`picture_file`] with a whole `colr` box.
    fn picture_file_colr(mirror_axis: Option<u8>, colr: Option<Vec<u8>>) -> Vec<u8> {
        let mut ipco = ispe(64, 64);
        ipco.extend_from_slice(&bx(b"hvcC", &QUAD_HVCC));
        let mut assoc = vec![0x81, 0x82];
        if let Some(axis) = mirror_axis {
            ipco.extend_from_slice(&bx(b"imir", &[axis]));
            assoc.push(0x80 | u8::try_from(assoc.len() + 1).unwrap());
        }
        if let Some(colr) = colr {
            ipco.extend_from_slice(&colr);
            assoc.push(0x80 | u8::try_from(assoc.len() + 1).unwrap());
        }
        file(PICTURE_ID, &[infe(PICTURE_ID, b"hvc1")], &ipco, &[(PICTURE_ID, assoc.as_slice())], &[(PICTURE_ID, &QUAD_HEVC)])
    }

    /// A 1x1 64x64 grid of the quadrant picture, with `grid_icc` on the grid item and `tile_icc`
    /// on its single tile.
    fn grid_file(grid_icc: Option<&[u8]>, tile_icc: Option<&[u8]>) -> Vec<u8> {
        grid_file_colr(grid_icc.map(colr_prof), tile_icc.map(colr_prof))
    }

    /// [`grid_file`] with whole `colr` boxes.
    fn grid_file_colr(grid_colr: Option<Vec<u8>>, tile_colr: Option<Vec<u8>>) -> Vec<u8> {
        let mut ipco = ispe(64, 64);
        ipco.extend_from_slice(&bx(b"hvcC", &QUAD_HVCC));
        let (mut grid_assoc, mut tile_assoc, mut index) = (vec![0x81], vec![0x81, 0x82], 2u8);
        if let Some(colr) = grid_colr {
            index += 1;
            ipco.extend_from_slice(&colr);
            grid_assoc.push(0x80 | index);
        }
        if let Some(colr) = tile_colr {
            index += 1;
            ipco.extend_from_slice(&colr);
            tile_assoc.push(0x80 | index);
        }
        // Version 0, flags 0, one row, one column, 64x64 output.
        const DESCRIPTOR: &[u8] = &[0, 0, 0, 0, 0, 64, 0, 64];
        file(
            GRID_ID,
            &[infe(GRID_ID, b"grid"), infe(PICTURE_ID, b"hvc1")],
            &ipco,
            &[(GRID_ID, grid_assoc.as_slice()), (PICTURE_ID, tile_assoc.as_slice())],
            &[(GRID_ID, DESCRIPTOR), (PICTURE_ID, &QUAD_HEVC)],
        )
    }

    /// A 1x1 grid of the quadrant picture declaring a `size` x `size` canvas, with a centred
    /// 64x64 `clap` on the grid item: the probe reports the canvas as the coded size.
    fn cropped_grid_file(size: u16) -> Vec<u8> {
        let mut ipco = ispe(64, 64);
        ipco.extend_from_slice(&bx(b"hvcC", &QUAD_HVCC));
        ipco.extend_from_slice(&ispe(size.into(), size.into()));
        let clap: Vec<u8> = [64, 1, 64, 1, 0, 1, 0, 1].iter().flat_map(|v: &u32| v.to_be_bytes()).collect();
        ipco.extend_from_slice(&bx(b"clap", &clap));
        let mut descriptor = vec![0, 0, 0, 0];
        descriptor.extend_from_slice(&size.to_be_bytes());
        descriptor.extend_from_slice(&size.to_be_bytes());
        file(
            GRID_ID,
            &[infe(GRID_ID, b"grid"), infe(PICTURE_ID, b"hvc1")],
            &ipco,
            &[(GRID_ID, &[0x83, 0x84]), (PICTURE_ID, &[0x81, 0x82])],
            &[(GRID_ID, descriptor.as_slice()), (PICTURE_ID, &QUAD_HEVC)],
        )
    }

    #[test]
    fn the_pixel_limit_applies_to_the_coded_size_not_the_crop() {
        let bytes = cropped_grid_file(16384);
        let info = probe(&bytes).unwrap();
        assert_eq!((info.width, info.height, info.coded_width, info.coded_height), (64, 64, 16384, 16384));
        for apply_transforms in [true, false] {
            let r = decode(&bytes, &Options { max_pixels: 1 << 26, apply_transforms });
            assert!(matches!(&r, Err(Error::Limit(m)) if m.contains("16384x16384")), "transforms {apply_transforms}: {r:?}");
        }
    }

    /// The colour at the centre of each quadrant, as the nearest of the four the picture uses:
    /// top left, top right, bottom left, bottom right.
    fn quadrants(d: &Decoded) -> [&'static str; 4] {
        assert_eq!((d.width, d.height, d.has_alpha, d.sixteen_bit), (64, 64, false, false));
        [(16, 16), (48, 16), (16, 48), (48, 48)].map(|(x, y)| {
            let i = (y * 64 + x) * 3;
            let px = [d.data[i], d.data[i + 1], d.data[i + 2]].map(i32::from);
            let named = [("red", [255, 0, 0]), ("green", [0, 255, 0]), ("blue", [0, 0, 255]), ("white", [255, 255, 255])];
            named.iter().min_by_key(|(_, c)| (0..3).map(|k| (px[k] - c[k]).pow(2)).sum::<i32>()).unwrap().0
        })
    }

    fn decode_quadrants(bytes: &[u8], apply_transforms: bool) -> [&'static str; 4] {
        quadrants(&decode(bytes, &Options { apply_transforms, ..Default::default() }).unwrap())
    }

    #[test]
    fn the_quadrant_picture_decodes_as_coded() {
        assert_eq!(decode_quadrants(&picture_file(None, None), true), ["red", "green", "blue", "white"]);
    }

    #[test]
    fn imir_axis_0_flips_top_and_bottom() {
        // As libheif reads it ("flip image vertically") and the Nokia suite describes C042
        // ("vertically mirrored"). heic-rs, the previous decoder, flipped left-right here.
        let bytes = picture_file(Some(0), None);
        assert_eq!(decode_quadrants(&bytes, true), ["blue", "white", "red", "green"]);
        assert_eq!(decode_quadrants(&bytes, false), ["red", "green", "blue", "white"], "as coded without transforms");
    }

    #[test]
    fn imir_axis_1_flips_left_and_right() {
        let bytes = picture_file(Some(1), None);
        assert_eq!(decode_quadrants(&bytes, true), ["green", "red", "white", "blue"]);
        assert_eq!(decode_quadrants(&bytes, false), ["red", "green", "blue", "white"], "as coded without transforms");
    }

    fn icc_of(bytes: &[u8]) -> Option<Vec<u8>> {
        decode(bytes, &Options::default()).unwrap().icc
    }

    #[test]
    fn a_grid_inherits_its_tiles_icc_profile() {
        // `colr` is optional on a `grid` derivation, and phone cameras write it on the tiles
        // instead: a OnePlus 11 photograph carries one `prof` (Display P3) on each of its 48 tiles
        // and nothing on the grid. Losing it would open the photo as if it were sRGB.
        let bytes = grid_file(None, Some(TILE_ICC));
        assert_eq!(decode_quadrants(&bytes, true), ["red", "green", "blue", "white"], "a 1x1 grid of the picture");
        assert_eq!(icc_of(&bytes).as_deref(), Some(TILE_ICC));
    }

    #[test]
    fn a_grids_own_profile_wins_over_its_tiles() {
        assert_eq!(icc_of(&grid_file(Some(GRID_ICC), Some(TILE_ICC))).as_deref(), Some(GRID_ICC));
    }

    #[test]
    fn an_unlabelled_grid_has_no_profile() {
        assert_eq!(icc_of(&grid_file(None, None)), None);
    }

    #[test]
    fn a_coded_picture_resolves_its_own_profile() {
        assert_eq!(icc_of(&picture_file(None, Some(GRID_ICC))).as_deref(), Some(GRID_ICC));
        assert_eq!(icc_of(&picture_file(None, None)), None);
    }

    /// An iPhone 10-bit HDR HEIC's description (#6752): BT.2020 primaries, ARIB STD-B67 (HLG)
    /// transfer, BT.2020 non-constant-luminance matrix, full range.
    const HLG: Nclx = Nclx { primaries: 9, transfer: 18, matrix: 9, full_range: true };

    fn nclx_of(bytes: &[u8]) -> Option<Nclx> {
        decode(bytes, &Options::default()).unwrap().nclx
    }

    #[test]
    fn a_coded_pictures_nclx_is_read() {
        assert_eq!(nclx_of(&picture_file_colr(None, Some(colr_nclx(9, 18, 9, true)))), Some(HLG));
        let sdr = Nclx { primaries: 1, transfer: 13, matrix: 6, full_range: false };
        assert_eq!(nclx_of(&picture_file_colr(None, Some(colr_nclx(1, 13, 6, false)))), Some(sdr));
        assert_eq!(nclx_of(&picture_file(None, Some(TILE_ICC))), None, "an ICC profile is not an nclx");
    }

    #[test]
    fn a_grid_inherits_its_tiles_nclx() {
        assert_eq!(nclx_of(&grid_file_colr(None, Some(colr_nclx(9, 18, 9, true)))), Some(HLG));
    }
}
