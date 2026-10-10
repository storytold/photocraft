//! `photocraft-jxl`: the optional JPEG XL decoder.
//!
//! A thin wrapper around `jxl-oxide`, a pure-Rust decoder of the whole JPEG XL specification:
//! Modular and VarDCT images, bare codestreams and the container, integer and float samples,
//! grayscale and RGB with alpha, orientation, ICC profiles, animation (the first frame), spot
//! colours, EXIF and XMP boxes. Its API is plain data ([`Info`], [`Decoded`], [`Error`]) so the
//! crate knows nothing about the rest of PhotoCraft; `photocraft-codecs` adapts it behind its
//! `jxl` feature.
//!
//! JPEG XL records orientation in the codestream header, not in EXIF. The pixels come back
//! upright, with [`Decoded::orientation`] saying what was applied, so a caller that wants them as
//! coded can turn them back.
//!
//! Never panics: every call into jxl-oxide runs under `catch_unwind` and a panic inside it
//! becomes [`Error::Malformed`].

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use jxl_oxide::color::{ColourEncoding, ColourSpace, Primaries, TransferFunction, WhitePoint};
use jxl_oxide::image::BitDepth;
use jxl_oxide::{AllocTracker, AuxBoxData, InitializeResult, JxlImage, PixelFormat, UninitializedJxlImage};

/// Channel layout of the decoded samples (interleaved, alpha last and straight).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Gray,
    GrayA,
    Rgb,
    Rgba,
}

impl Layout {
    pub const fn channels(self) -> usize {
        match self {
            Layout::Gray => 1,
            Layout::GrayA => 2,
            Layout::Rgb => 3,
            Layout::Rgba => 4,
        }
    }

    pub const fn has_alpha(self) -> bool {
        matches!(self, Layout::GrayA | Layout::Rgba)
    }
}

/// Storage type of one decoded sample, native-endian.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sample {
    /// Integer images of up to 8 bits.
    U8,
    /// Integer images of 9 to 32 bits, scaled to the full 16-bit range.
    U16,
    /// Float images (16 or 32-bit in the file, see [`Info::bits_per_sample`]); the nominal range
    /// is `[0, 1]` but HDR values above it are kept.
    F32,
}

impl Sample {
    pub const fn bytes(self) -> usize {
        match self {
            Sample::U8 => 1,
            Sample::U16 => 2,
            Sample::F32 => 4,
        }
    }
}

/// What a JPEG XL file declares, read from its header alone (no pixel is decoded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    /// Size with the orientation applied: what a viewer shows and what [`decode`] returns.
    pub width: u32,
    pub height: u32,
    /// Size as coded, before the orientation.
    pub coded_width: u32,
    pub coded_height: u32,
    /// The header's orientation, 1–8 with EXIF's meaning (1 = as coded).
    pub orientation: u8,
    pub layout: Layout,
    pub sample: Sample,
    /// Bits per sample of the colour channels as stored (an alpha channel may differ).
    pub bits_per_sample: u32,
    /// The samples are floats (16 or 32-bit) rather than integers.
    pub float: bool,
    /// The alpha channel is stored premultiplied (associated): it is divided out on decode,
    /// which goes through a 32-bit float working buffer whatever `sample` is.
    pub premultiplied: bool,
    /// The file declares an animation; [`decode`] returns its first frame.
    pub animated: bool,
}

/// Decode settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Budget in bytes for the decoder's own buffers (frames, groups, modular trees). Exceeding
    /// it is [`Error::Limit`]; the output buffer, and the 32-bit float working buffer of float
    /// and premultiplied-alpha images, are on top of it.
    pub alloc_limit: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options { alloc_limit: 1 << 31 }
    }
}

/// A decoded image: interleaved samples, row-major, no padding, upright.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub layout: Layout,
    pub sample: Sample,
    /// Bits per sample of the colour channels as stored.
    pub bits_per_sample: u32,
    /// The stored samples were floats.
    pub float: bool,
    /// `width * height * layout.channels() * sample.bytes()` bytes, native-endian samples.
    pub data: Vec<u8>,
    /// The orientation that was applied (1–8); the pixels are already upright.
    pub orientation: u8,
    /// The profile the samples are in: the embedded ICC profile, or one built from the
    /// header's colour encoding when it is not plain sRGB (`None` for sRGB and sRGB-curve gray).
    pub icc: Option<Vec<u8>>,
    /// EXIF as a TIFF structure (without the container box's offset header), verbatim.
    pub exif: Option<Vec<u8>>,
    /// The XMP packet.
    pub xmp: Option<String>,
    /// Keyframes loaded from the file; above 1 the image is an animation and `data` is its first
    /// frame.
    pub frames: u32,
    /// The header declares an animation (a truncated one may have loaded a single frame).
    pub animated: bool,
    /// The whole image was read: no frame is missing or cut short.
    pub complete: bool,
}

/// Why a file could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A valid file using something this decoder does not hand out (CMYK images).
    Unsupported(String),
    /// The decoder's allocations passed [`Options::alloc_limit`].
    Limit(String),
    /// Broken or truncated data (or a decoder bug, reported the same way).
    Malformed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unsupported(m) => write!(f, "unsupported JPEG XL: {m}"),
            Error::Limit(m) => write!(f, "limit exceeded: {m}"),
            Error::Malformed(m) => write!(f, "malformed JPEG XL: {m}"),
        }
    }
}

impl std::error::Error for Error {}

const CMYK: &str = "CMYK JPEG XL images are not supported yet";

/// jxl-oxide reports everything as a boxed error; its allocation tracker's message is the one
/// kind worth telling apart.
fn err(e: Box<dyn std::error::Error + Send + Sync>) -> Error {
    let m = e.to_string();
    if m.contains("failed to allocate") { Error::Limit(m) } else { Error::Malformed(m) }
}

/// Runs `f`, turning a jxl-oxide panic into an error: a decoder written for valid files can still
/// trip an assertion on a hostile one, and PhotoCraft must not crash.
fn guarded<T>(f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err(Error::Malformed("the JPEG XL decoder failed on this file".into())))
}

/// A file whose header has been parsed: [`Decoder::info`] says what it holds, so callers can
/// check their limits, and [`Decoder::decode`] then reads the pixels without parsing the header
/// (and any embedded ICC profile) a second time.
pub struct Decoder<'a> {
    bytes: &'a [u8],
    image: JxlImage,
    /// Bytes the container parser has consumed; the rest is still to be fed.
    fed: usize,
    info: Info,
}

impl<'a> Decoder<'a> {
    /// Parses the header, feeding `bytes` in a growing window. `alloc_limit` bounds the decoder's
    /// own buffers (an embedded ICC profile is decompressed here already).
    ///
    /// The parser may consume nothing from a window it cannot yet make sense of (a one-byte
    /// prefix, a box cut in half), so the window is extended rather than re-fed: feeding the same
    /// bytes again would loop forever, and a window that reaches the end of the file without a
    /// header is a truncated file.
    pub fn open(bytes: &'a [u8], alloc_limit: usize) -> Result<Self, Error> {
        guarded(|| Self::open_unguarded(bytes, alloc_limit))
    }

    fn open_unguarded(bytes: &'a [u8], alloc_limit: usize) -> Result<Self, Error> {
        let mut uninit: UninitializedJxlImage = JxlImage::builder().alloc_tracker(AllocTracker::with_limit(alloc_limit)).build_uninit();
        let mut fed = 0usize;
        let mut end = 0usize;
        loop {
            if end >= bytes.len() {
                return Err(Error::Malformed("the file ends before the image header is complete".into()));
            }
            end = end.saturating_add(4096).min(bytes.len());
            let window = bytes.get(fed..end).unwrap_or(&[]);
            let consumed = uninit.feed_bytes(window).map_err(err)?;
            fed = fed.saturating_add(consumed).min(end);
            match uninit.try_init().map_err(err)? {
                InitializeResult::Initialized(image) => {
                    let info = info_of(&image)?;
                    return Ok(Decoder { bytes, image, fed, info });
                }
                InitializeResult::NeedMoreData(u) => uninit = u,
            }
        }
    }

    /// What the header declares.
    pub fn info(&self) -> Info {
        self.info
    }

    /// Decodes the first frame, upright, with its metadata.
    pub fn decode(self) -> Result<Decoded, Error> {
        guarded(|| self.decode_unguarded())
    }

    fn decode_unguarded(mut self) -> Result<Decoded, Error> {
        let (bytes, info) = (self.bytes, self.info);
        let mut at = self.fed;
        while at < bytes.len() {
            let chunk = bytes.get(at..).unwrap_or(&[]);
            let consumed = self.image.feed_bytes(chunk).map_err(err)?;
            if consumed == 0 {
                break;
            }
            at = at.saturating_add(consumed);
        }
        self.image.finalize().map_err(err)?;
        let image = &self.image;
        let frames = u32::try_from(image.num_loaded_keyframes()).unwrap_or(u32::MAX);
        if frames == 0 {
            return Err(Error::Malformed("the file ends before its first frame is complete".into()));
        }

        let render = image.render_frame(0).map_err(err)?;
        let mut stream = render.stream();
        let (width, height) = (stream.width(), stream.height());
        let channels = usize::try_from(stream.channels()).unwrap_or(usize::MAX);
        if (width, height) != (info.width, info.height) || channels != info.layout.channels() {
            return Err(Error::Malformed(format!(
                "the rendered frame is {width}x{height} with {channels} channels, the header says {}x{} with {}",
                info.width,
                info.height,
                info.layout.channels()
            )));
        }
        let count =
            (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(channels)).ok_or_else(|| Error::Limit("image size overflows".into()))?;

        // A premultiplied (associated) alpha channel is turned straight on float samples, then
        // quantized; everything else is handed out as the decoder converts it.
        let data = match (info.sample, info.premultiplied) {
            (Sample::U8, false) => {
                let mut buf = vec![0u8; count];
                stream.write_to_buffer(&mut buf);
                buf
            }
            (Sample::U16, false) => {
                let mut buf = vec![0u16; count];
                stream.write_to_buffer(&mut buf);
                buf.iter().flat_map(|v| v.to_ne_bytes()).collect()
            }
            (sample, premultiplied) => {
                let mut buf = vec![0f32; count];
                stream.write_to_buffer(&mut buf);
                if premultiplied {
                    unpremultiply(&mut buf, channels);
                }
                quantize(&buf, sample)
            }
        };

        let (exif, xmp) = metadata(image);
        Ok(Decoded {
            width,
            height,
            layout: info.layout,
            sample: info.sample,
            bits_per_sample: info.bits_per_sample,
            float: info.float,
            data,
            orientation: info.orientation,
            icc: icc(image),
            exif,
            xmp,
            frames,
            animated: info.animated,
            complete: image.is_loading_done(),
        })
    }
}

fn layout_of(format: PixelFormat) -> Result<Layout, Error> {
    match format {
        PixelFormat::Gray => Ok(Layout::Gray),
        PixelFormat::Graya => Ok(Layout::GrayA),
        PixelFormat::Rgb => Ok(Layout::Rgb),
        PixelFormat::Rgba => Ok(Layout::Rgba),
        PixelFormat::Cmyk | PixelFormat::Cmyka => Err(Error::Unsupported(CMYK.into())),
    }
}

fn sample_of(depth: BitDepth) -> (Sample, u32, bool) {
    match depth {
        BitDepth::IntegerSample { bits_per_sample } if bits_per_sample <= 8 => (Sample::U8, bits_per_sample, false),
        BitDepth::IntegerSample { bits_per_sample } => (Sample::U16, bits_per_sample, false),
        BitDepth::FloatSample { bits_per_sample, .. } => (Sample::F32, bits_per_sample, true),
    }
}

fn info_of(image: &JxlImage) -> Result<Info, Error> {
    let header = image.image_header();
    let layout = layout_of(image.pixel_format())?;
    let (sample, bits_per_sample, float) = sample_of(header.metadata.bit_depth);
    let (coded_width, coded_height) = (header.size.width, header.size.height);
    if coded_width == 0 || coded_height == 0 {
        return Err(Error::Malformed(format!("zero-sized image {coded_width}x{coded_height}")));
    }
    let orientation = u8::try_from(header.metadata.orientation).unwrap_or(1).clamp(1, 8);
    let premultiplied = layout.has_alpha() && header.metadata.ec_info.iter().any(|ec| ec.alpha_associated() == Some(true));
    Ok(Info {
        width: image.width(),
        height: image.height(),
        coded_width,
        coded_height,
        orientation,
        layout,
        sample,
        bits_per_sample,
        float,
        premultiplied,
        animated: header.metadata.animation.is_some(),
    })
}

/// Reads the header's size, orientation, layout and depth without decoding pixels, so callers
/// can check their limits first. `alloc_limit` bounds the header parse (an embedded ICC profile
/// is decompressed here).
pub fn probe(bytes: &[u8], alloc_limit: usize) -> Result<Info, Error> {
    Decoder::open(bytes, alloc_limit).map(|d| d.info())
}

/// Decodes the first frame, upright, with its metadata.
pub fn decode(bytes: &[u8], options: &Options) -> Result<Decoded, Error> {
    Decoder::open(bytes, options.alloc_limit)?.decode()
}

/// Straight alpha from associated alpha: colour divided by alpha where there is any.
fn unpremultiply(samples: &mut [f32], channels: usize) {
    if channels < 2 {
        return;
    }
    for px in samples.chunks_exact_mut(channels) {
        let a = px.get(channels - 1).copied().unwrap_or(1.0);
        if a > 0.0 && a < 1.0 {
            for c in px.iter_mut().take(channels - 1) {
                *c /= a;
            }
        }
    }
}

fn quantize(values: &[f32], sample: Sample) -> Vec<u8> {
    match sample {
        Sample::U8 => values.iter().map(|&v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect(),
        Sample::U16 => values.iter().flat_map(|&v| ((v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).to_ne_bytes()).collect(),
        Sample::F32 => values.iter().flat_map(|&v| v.to_ne_bytes()).collect(),
    }
}

/// The header's colour encoding is plain sRGB (or gray with the sRGB curve): the default that
/// needs no profile. Anything else gets the profile jxl-oxide renders in.
fn plain_srgb(encoding: &ColourEncoding) -> bool {
    match encoding {
        ColourEncoding::Enum(e) => {
            matches!(e.white_point, WhitePoint::D65)
                && matches!(e.tf, TransferFunction::Srgb)
                && match e.colour_space {
                    ColourSpace::Rgb => matches!(e.primaries, Primaries::Srgb),
                    ColourSpace::Grey => true,
                    _ => false,
                }
        }
        _ => false,
    }
}

fn icc(image: &JxlImage) -> Option<Vec<u8>> {
    if image.original_icc().is_none() && plain_srgb(&image.image_header().metadata.colour_encoding) {
        return None;
    }
    let icc = image.rendered_icc();
    (!icc.is_empty()).then_some(icc)
}

/// EXIF (TIFF structure) and XMP from the container's boxes. Metadata is a courtesy: a broken box
/// never fails a decode whose pixels came out fine.
fn metadata(image: &JxlImage) -> (Option<Vec<u8>>, Option<String>) {
    let boxes = image.aux_boxes();
    let exif = match boxes.first_exif() {
        Ok(AuxBoxData::Data(raw)) => {
            let offset = usize::try_from(raw.tiff_header_offset()).unwrap_or(usize::MAX);
            raw.payload().get(offset..).filter(|t| t.starts_with(b"II*\0") || t.starts_with(b"MM\0*")).map(<[u8]>::to_vec)
        }
        _ => None,
    };
    let xmp = match boxes.first_xml() {
        AuxBoxData::Data(b) => String::from_utf8(b.to_vec()).ok().map(|x| x.trim_end_matches('\0').to_string()).filter(|x| !x.is_empty()),
        _ => None,
    };
    (exif, xmp)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A complete 240x135 bare codestream whose header fits in its first 6 bytes (the example in
    /// jxl-oxide's own documentation).
    const TINY: [u8; 42] = [
        0xff, 0x0a, 0x30, 0x54, 0x10, 0x09, 0x08, 0x06, 0x01, 0x00, 0x78, 0x00, 0x4b, 0x38, 0x41, 0x3c, 0xb6, 0x3a, 0x51, 0xfe, 0x00, 0x47, 0x1e, 0xa0, 0x85,
        0xb8, 0x27, 0x1a, 0x48, 0x45, 0x84, 0x1b, 0x71, 0x4f, 0xa8, 0x3e, 0x8e, 0x30, 0x03, 0x92, 0x84, 0x01,
    ];

    #[test]
    fn a_tiny_codestream_decodes() {
        let info = probe(&TINY, 1 << 20).unwrap();
        assert_eq!((info.width, info.height, info.orientation), (240, 135, 1));
        let d = decode(&TINY, &Options { alloc_limit: 1 << 20 }).unwrap();
        assert_eq!((d.width, d.height, d.frames, d.animated, d.complete), (240, 135, 1, false, true));
        let dec = Decoder::open(&TINY, 1 << 20).unwrap();
        assert_eq!(dec.info(), info, "one parse serves both");
        assert_eq!(dec.decode().unwrap(), d);
        assert_eq!(d.data.len(), 240 * 135 * d.layout.channels() * d.sample.bytes());
        assert!(d.icc.is_none() && d.exif.is_none() && d.xmp.is_none());
    }

    /// Every strict prefix fails to decode, promptly: a one-byte prefix used to spin forever,
    /// the parser consuming nothing from a window that was then fed again unchanged. The header
    /// alone is complete from 6 bytes on, so `probe` may succeed before `decode` can.
    #[test]
    fn every_prefix_fails_to_decode_and_returns() {
        for n in 0..TINY.len() {
            let started = std::time::Instant::now();
            let p = probe(&TINY[..n], 1 << 20);
            assert!(n >= 6 || matches!(p, Err(Error::Malformed(_))), "{n}: {p:?}");
            assert!(decode(&TINY[..n], &Options { alloc_limit: 1 << 20 }).is_err(), "{n}");
            assert!(started.elapsed().as_secs() < 5, "prefix {n} took {:?}", started.elapsed());
        }
    }

    #[test]
    fn garbage_is_an_error() {
        for bytes in [&b""[..], &[0xFF, 0x0A][..], b"\0\0\0\x0cJXL \r\n\x87\n", &[0xFF; 64], b"\xff\x0a\xff\xff\xff\xff\xff\xff\xff\xff"] {
            assert!(probe(bytes, 1 << 20).is_err(), "{bytes:?}");
            assert!(decode(bytes, &Options { alloc_limit: 1 << 20 }).is_err(), "{bytes:?}");
        }
    }

    #[test]
    fn errors_display_their_kind() {
        assert!(Error::Unsupported("x".into()).to_string().contains("unsupported"));
        assert!(Error::Limit("x".into()).to_string().contains("limit"));
        assert!(Error::Malformed("x".into()).to_string().contains("malformed"));
        assert!(matches!(err("failed to allocate 10 byte(s)".into()), Error::Limit(_)));
        assert!(matches!(err("bad".into()), Error::Malformed(_)));
    }

    #[test]
    fn unpremultiply_divides_colour_by_alpha_where_there_is_any() {
        let mut px = [0.25, 0.5, 0.0, 0.5, 0.1, 0.1, 0.1, 0.0, 0.3, 0.3, 0.3, 1.0];
        unpremultiply(&mut px, 4);
        assert_eq!(px, [0.5, 1.0, 0.0, 0.5, 0.1, 0.1, 0.1, 0.0, 0.3, 0.3, 0.3, 1.0]);
        let mut ga = [0.2, 0.4];
        unpremultiply(&mut ga, 2);
        assert_eq!(ga, [0.5, 0.4]);
        let mut gray = [0.2];
        unpremultiply(&mut gray, 1);
        assert_eq!(gray, [0.2]);
    }

    #[test]
    fn quantize_rounds_and_clamps() {
        assert_eq!(quantize(&[0.0, 0.5, 1.0, 1.5, -1.0], Sample::U8), [0, 128, 255, 255, 0]);
        let u16s: Vec<u16> = quantize(&[0.5, 2.0], Sample::U16).chunks_exact(2).map(|b| u16::from_ne_bytes([b[0], b[1]])).collect();
        assert_eq!(u16s, [32768, 65535]);
        assert_eq!(quantize(&[1.5], Sample::F32), 1.5f32.to_ne_bytes());
    }

    #[test]
    fn plain_srgb_is_the_default_that_needs_no_profile() {
        use jxl_oxide::{EnumColourEncoding, RenderingIntent};
        assert!(plain_srgb(&ColourEncoding::Enum(EnumColourEncoding::srgb(RenderingIntent::Perceptual))));
        assert!(!plain_srgb(&ColourEncoding::Enum(EnumColourEncoding::srgb_linear(RenderingIntent::Perceptual))));
        assert!(!plain_srgb(&ColourEncoding::Enum(EnumColourEncoding::display_p3(RenderingIntent::Perceptual))));
    }
}
