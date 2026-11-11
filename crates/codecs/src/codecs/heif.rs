//! HEIF / HEIC (the iPhone and Mac photo format), read-only: see [`crate::ASYMMETRIC_EXCEPTIONS`].
//!
//! Decoding lives in the optional `craft-heif` crate from craft-libs (heic-rs, a pure-Rust HEVC
//! still-picture decoder), enabled by this crate's `heif` feature. Without it, HEIF files are
//! still detected and opening one is an [`CodecError::Unsupported`] error, never a panic.
//!
//! HEIF records orientation in the container (`irot`/`imir`, plus a `clap` crop), not in EXIF,
//! so those are applied by the decoder and the EXIF Orientation is rewritten to 1. Its EXIF tag
//! only mirrors what the container already says, and applying it as well would turn the photo
//! twice.

use crate::Format;
use crate::error::CodecError;
use crate::image::Image;
use crate::options::Limits;

const F: Format = Format::Heif;

/// The reason given when this build has no HEIF decoder.
#[cfg(not(feature = "heif"))]
pub(crate) const NOT_IN_BUILD: &str = "HEIC/HEIF support isn't included in this build of PhotoCraft";

#[cfg(not(feature = "heif"))]
pub(crate) fn decode(_bytes: &[u8], _limits: &Limits, _keep_orientation: bool) -> Result<Image, CodecError> {
    Err(CodecError::unsupported(F, NOT_IN_BUILD))
}

#[cfg(feature = "heif")]
fn err(e: craft_heif::Error) -> CodecError {
    match e {
        craft_heif::Error::Unsupported(what) => CodecError::unsupported(F, what),
        craft_heif::Error::Limit(what) => CodecError::LimitExceeded(what),
        craft_heif::Error::Malformed(what) => CodecError::malformed(F, what),
    }
}

#[cfg(feature = "heif")]
pub(crate) fn decode(bytes: &[u8], limits: &Limits, keep_orientation: bool) -> Result<Image, CodecError> {
    use crate::cicp::Cicp;
    use crate::image::{ChannelLayout, Metadata, SampleType};

    // The container alone: the declared size is checked before any pixel is decoded.
    let info = craft_heif::probe(bytes).map_err(err)?;
    let layout = if info.has_alpha { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
    let sample = if info.bit_depth > 8 { SampleType::U16 } else { SampleType::U8 };
    let (w, h) = if keep_orientation { (info.coded_width, info.coded_height) } else { (info.width, info.height) };
    // craft-heif hands back 16-bit samples at any depth, and that buffer is still live while the
    // output copy below is built, so the peak is both of them, not the output alone.
    let peak = (layout.channels() * (2 + sample.bytes())) as u64;
    limits.check_bytes(w, h, peak)?;
    // The delegated decoder allocates the full coded image before applying any clean-aperture
    // crop or orientation transform, so the coded dimensions must pass the limit policy even
    // when the displayed (transformed) size is small. Checking only the transformed size let a
    // tiny visible image hide a large coded allocation.
    if !keep_orientation && (info.coded_width, info.coded_height) != (w, h) {
        limits.check_bytes(info.coded_width, info.coded_height, peak)?;
    }
    // Photoshop on macOS opens HEIC through Apple's decoder, so match its chroma upsampling.
    // A grid's payload can declare a larger canvas than the `ispe` probed above. craft-heif holds
    // the canvas it really decodes to `max_pixels`, so `max_alloc` is passed down as pixels too.
    let max_pixels = limits.max_pixels.min(limits.max_alloc / peak);
    let options = craft_heif::Options { max_pixels, apply_transforms: !keep_orientation, chroma: craft_heif::Chroma::Apple };
    let mut decoded = craft_heif::decode(bytes, &options).map_err(err)?;
    // The decoder re-reads the container; trust its output's shape, not the probe's.
    let layout = if decoded.has_alpha { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
    // The decoder returns full-scale 16-bit samples at any coded depth; an 8-bit file stays 8-bit.
    // Taken, so the decoder's buffer is freed here and not held through the HDR conversion below.
    let raw = std::mem::take(&mut decoded.data);
    let (sample, data) = if decoded.bit_depth > 8 {
        (SampleType::U16, raw.iter().flat_map(|v| v.to_ne_bytes()).collect())
    } else {
        (SampleType::U8, raw.iter().map(|&v| ((u32::from(v) * 255 + 32767) / 65535) as u8).collect())
    };
    drop(raw);
    let mut img = Image::from_raw(decoded.width, decoded.height, layout, sample, data)?;
    img.icc = decoded.icc;
    // `colour` is always filled in (nclx, else the HEVC VUI, else H.273's "unspecified" 2s); only a
    // specified one tags the image, so a file that says nothing still opens untagged.
    let c = decoded.colour;
    img.cicp =
        (c.primaries != 2 || c.transfer != 2).then_some(Cicp { primaries: c.primaries, transfer: c.transfer, matrix: c.matrix, full_range: c.full_range });
    img.meta = Metadata {
        exif: decoded.exif.map(|e| if keep_orientation { e } else { crate::orientation::upright_exif(&e).into_owned() }),
        xmp: decoded.xmp.map(|x| if keep_orientation { x } else { crate::orientation::upright_xmp(&x).into_owned() }),
        ..Default::default()
    };
    // A 10-bit HDR HEIC (BT.2100 HLG or PQ) opens as linear light in 32-bit float (#6752): no
    // ICC curve can describe those transfers, and an 8/16-bit document would clip the highlights.
    match img.cicp.filter(|c| c.is_hdr()) {
        Some(c) => {
            // `linearize` holds the 16-bit image, its float copy and the float output at once.
            let hdr_peak = (layout.channels() * (SampleType::U16.bytes() + 2 * SampleType::F32.bytes())) as u64;
            limits.check_bytes(decoded.width, decoded.height, hdr_peak)?;
            crate::cicp::linearize(img, c)
        }
        None => Ok(img),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(feature = "heif"))]
    #[test]
    fn without_the_feature_heif_is_a_clear_unsupported_error() {
        let header = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic";
        assert_eq!(crate::detect(header), Some(F), "still recognised");
        let r = crate::decode(header);
        assert!(matches!(&r, Err(CodecError::Unsupported { format: Format::Heif, reason }) if reason.contains("isn't included in this build")), "{r:?}");
        assert!(!crate::caps(F).read);
    }

    #[cfg(feature = "heif")]
    #[test]
    fn with_the_feature_heif_is_readable_and_errors_map() {
        assert!(crate::caps(F).read);
        let r = crate::decode(b"\0\0\0\x18ftypmsf1\0\0\0\0msf1hevc");
        assert!(matches!(&r, Err(CodecError::Unsupported { format: Format::Heif, reason }) if reason.contains("sequence")), "{r:?}");
        assert!(matches!(crate::decode_as(F, b""), Err(CodecError::Malformed { .. } | CodecError::Unsupported { .. })));
    }

    /// Found by the `decode_heif` fuzz target: a malformed box makes heic-rs 0.1.1 slice out of range
    /// (`boxes.rs:130`, "slice index starts at 24 but ends at 16"). It must be an error, not a crash.
    #[cfg(feature = "heif")]
    const HEIC_RS_BOX_PANIC: [u8; 72] = [
        0x00, 0x00, 0x00, 0x24, 0x66, 0x74, 0x79, 0x70, 0x68, 0x65, 0x69, 0x63, 0x00, 0x00, 0x00, 0x00, 0x6d, 0x69, 0x66, 0x31, 0x4d, 0x69, 0x50, 0x72, 0x6d,
        0x69, 0x61, 0x66, 0x4d, 0x69, 0x48, 0x42, 0x72, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x6d, 0x65, 0x74, 0x61, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x10, 0x75, 0x75, 0x69, 0x64, 0x00, 0x00, 0x00, 0x00, 0x03, 0x08, 0x08, 0x08, 0x00, 0x03, 0x01, 0x03, 0x70, 0x00, 0xa8, 0x00,
    ];

    /// A 10-bit HLG file opens as float: the conversion holds the 16-bit image, its float copy and
    /// the float output at once, ten bytes a sample.
    #[cfg(feature = "heif")]
    #[test]
    fn an_hdr_heic_is_checked_against_its_conversion_peak() {
        use craft_heif::testdata::{Prop, Spec, build};
        let grey = |_, _| [512u16, 512, 512];
        let mut spec = Spec::new(32, 32, &grey);
        spec.bit_depth = 10;
        spec.props = vec![Prop::Nclx(9, 18, 9, false)];
        let bytes = build(&spec);
        let peak = 32 * 32 * 3 * 10;
        let with = |max_alloc| crate::decode_with(&bytes, &crate::DecodeOptions { limits: Limits { max_alloc, ..Default::default() }, ..Default::default() });
        assert!(matches!(with(peak - 1), Err(CodecError::LimitExceeded(_))));
        assert_eq!(with(peak).unwrap().sample_type(), crate::image::SampleType::F32);
    }

    /// A grid whose payload declares a larger canvas (64 x 64) than its `ispe` (33 x 33): the
    /// byte budget applies to the canvas that is decoded, not the one the container declares.
    #[cfg(feature = "heif")]
    #[test]
    fn a_grid_canvas_larger_than_its_declared_size_is_held_to_max_alloc() {
        use craft_heif::testdata::{Spec, build};
        let grey = |_, _| [128u16, 128, 128];
        let mut bytes = build(&Spec::new(33, 33, &grey));
        // The grid item's payload: version, flags, rows - 1, columns - 1, output width and height.
        let at = bytes.windows(8).position(|w| w == [0, 0, 1, 1, 0, 33, 0, 33]).unwrap();
        bytes[at + 5] = 64;
        bytes[at + 7] = 64;
        let with = |max_alloc| crate::decode_with(&bytes, &crate::DecodeOptions { limits: Limits { max_alloc, ..Default::default() }, ..Default::default() });
        // Enough for the declared 33 x 33 (three bytes a sample at the peak), not for 64 x 64.
        // Refused by the decoder before it allocates the canvas, not by the size check on its output.
        match with(33 * 33 * 3 * 3) {
            Err(CodecError::LimitExceeded(m)) => assert!(m.contains("pixel limit"), "{m}"),
            r => panic!("{:?}", r.map(|i| i.dimensions())),
        }
    }

    #[cfg(feature = "heif")]
    #[test]
    fn a_heic_rs_panic_is_a_malformed_file_error() {
        assert_eq!(crate::detect(&HEIC_RS_BOX_PANIC), Some(Format::Heif));
        for opts in [crate::DecodeOptions::default(), crate::DecodeOptions { keep_orientation: true, ..Default::default() }] {
            let r = crate::decode_with(&HEIC_RS_BOX_PANIC, &opts);
            assert!(matches!(r, Err(CodecError::Malformed { format: Format::Heif, .. })), "{r:?}");
        }
    }
}
