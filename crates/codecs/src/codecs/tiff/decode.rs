use std::borrow::Cow;

use super::bands::{
    DECODER_BYTES_PER_CHUNK, Tally, budget_workers, convert_ab, decode_bands, extra_sample_indices, refill_zeroed, split_extra_samples, worker_threads, zeroed,
};
use super::tags::{F, TAG_IMAGE_SOURCE_DATA, TAG_PHOTOSHOP, TAG_XMP, TEXT_TAGS, photoshop_tags};
use super::ycbcr::YCbCr;
use super::{
    ChannelLayout, ChunkType, CodecError, DecodeWarning, Decoder, ExifDirs, File, Ifd, Image, Limits, Metadata, Read, SampleType, Seek, SeekFrom, Tag,
    camera_raw, err, ifd, io, map_err, tiff_exif,
};

/// Decodes the page Photoshop would open: the first full-resolution directory.
pub(crate) fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    decode_page(bytes, None, limits)
}

/// Decodes page `page` of [`ifd::tiff_info`]'s list (`None`: the default page). The image
/// carries the page's metadata, a [`DecodeWarning::MorePages`] when the file holds more
/// full-resolution pages, and a [`DecodeWarning::Truncated`] when image data is cut off.
pub(crate) fn decode_page(bytes: &[u8], page: Option<usize>, limits: &Limits) -> Result<Image, CodecError> {
    let (file, dir, more) = open_page(bytes, page)?;
    let mut img = decode_ifd(&file, &dir, limits)?;
    let extra_samples = std::mem::take(&mut img.meta.extra_samples);
    img.meta = metadata(&file, &dir, &mut img.icc);
    img.meta.extra_samples = extra_samples;
    img.warnings.extend(more);
    Ok(img)
}

/// Inspect the selected directory's sample classification without decoding any pixels.
/// Mirror document import: inspect bands first, then the default whole-image route when
/// auxiliary channels are present. Preparation reads metadata, not strips or tiles.
pub(crate) fn has_unretained_samples(bytes: &[u8], offset: u64) -> Result<bool, CodecError> {
    let file = File::parse(bytes).ok_or_else(|| err("not a TIFF or BigTIFF header"))?;
    let dir = file.ifd(offset).ok_or_else(|| err("image directory cut off"))?;
    let bands = prepare(&file, &dir, &Limits::banded(), cfg!(target_pointer_width = "32"))?;
    if bands.extras_over_limit > 0 || bands.extras_dropped > 0 {
        return Ok(true);
    }
    if bands.lay.extra_at.is_empty() {
        return Ok(false);
    }
    Ok(prepare(&file, &dir, &Limits::default(), true)?.extras_dropped > 0)
}

/// The file, page `page`'s directory (`None`: the default page) and the
/// [`DecodeWarning::MorePages`] the page opens with when the file holds more.
fn open_page(bytes: &[u8], page: Option<usize>) -> Result<(File<'_>, Ifd, Option<DecodeWarning>), CodecError> {
    if camera_raw(bytes) {
        return Err(CodecError::unsupported(F, "camera raw files (such as CR2, NEF, ARW or DNG) are not flat images; decode them with photocraft-raw"));
    }
    let file = File::parse(bytes).ok_or_else(|| err("not a TIFF or BigTIFF header"))?;
    let info = ifd::info_of(&file)?;
    let index = match page {
        Some(i) => i,
        None => info.default_page().ok_or_else(|| err("no image directory"))?,
    };
    let p = info
        .pages
        .get(index)
        .ok_or_else(|| CodecError::InvalidImage(format!("TIFF page {index} does not exist (the file has {} directories)", info.pages.len())))?;
    let dir = file.ifd(p.ifd_offset).ok_or_else(|| err("image directory cut off"))?;
    let pages = info.page_count();
    let more = (pages > 1).then(|| DecodeWarning::MorePages { total: info.complete.then_some(u32::try_from(pages).unwrap_or(u32::MAX)) });
    Ok((file, dir, more))
}

/// Output bytes [`TiffBands::decode`] aims to decode at once (at least one band per worker): small
/// next to the document the rows are written into, and to the copy a caller may convert them to.
const GROUP_BYTES: usize = 64 << 20;

/// A TIFF page opened for decoding a group of bands at a time, for images too large to hold
/// twice (#8169): [`TiffBands::decode`] hands each group to its caller, who writes it into the
/// document, so only one group exists beside the document. The pixels, layout and metadata are
/// those [`decode_page`] returns; [`TiffBands::upright`] applies the orientation as
/// [`Image::oriented`] does.
pub struct TiffBands<'a> {
    b: &'a [u8],
    p: Prepared,
    /// The page's Orientation tag (1–8).
    stored: u16,
    /// The orientation the bands are turned by (1: as stored).
    turn: u16,
    /// `Limits::max_alloc`: the most one group of bands may take.
    cap: u64,
    pub icc: Option<Vec<u8>>,
    pub meta: Metadata,
    /// What the decoder noticed: more pages, and after [`TiffBands::decode`] data cut off.
    pub warnings: Vec<DecodeWarning>,
}

impl<'a> TiffBands<'a> {
    /// Opens page `page` (`None`: the page [`decode_page`] opens by default) and checks it
    /// against `limits`; on 64-bit targets its `max_alloc` bounds each group of bands, not the
    /// whole image.
    pub fn open(bytes: &'a [u8], page: Option<usize>, limits: &Limits) -> Result<Self, CodecError> {
        let (file, dir, more) = open_page(bytes, page)?;
        // On 32-bit targets the whole image must still fit `max_alloc`, as it did before: the
        // document holding it shares the same small address space (wasm32: 4 GiB).
        let p = prepare(&file, &dir, limits, cfg!(target_pointer_width = "32"))?;
        let band = p.band_bytes()?;
        if band as u64 > limits.max_alloc {
            return Err(CodecError::LimitExceeded(format!(
                "one strip or row of tiles of this {}x{} image decodes to {band} bytes, over the {} byte limit",
                p.w, p.h, limits.max_alloc
            )));
        }
        let mut icc = None;
        let meta = metadata(&file, &dir, &mut icc);
        let stored = ifd::orientation_of(&file, &dir);
        // Extra samples are dropped here only on 32-bit targets, where the whole image counts.
        let mut warnings = prepared_warnings(&p);
        warnings.extend(more);
        Ok(TiffBands { b: bytes, p, stored, turn: 1, cap: limits.max_alloc, icc, meta, warnings })
    }

    /// The bands turned upright for the page's Orientation tag, as [`Image::oriented`] turns a
    /// whole image: the size and resolution swap for 5–8, and the EXIF and XMP orientation are
    /// rewritten to 1.
    pub fn upright(mut self) -> Self {
        let o = self.stored;
        if !(2..=8).contains(&o) || self.turn == o {
            return self;
        }
        self.turn = o;
        if let Some(e) = &mut self.meta.exif
            && let Cow::Owned(fixed) = crate::orientation::upright_exif(e)
        {
            *e = fixed;
        }
        if let Some(x) = &mut self.meta.xmp
            && let Cow::Owned(fixed) = crate::orientation::upright_xmp(x)
        {
            *x = fixed;
        }
        if o >= 5 {
            self.meta.dpi = self.meta.dpi.map(|(x, y)| (y, x));
        }
        self
    }

    /// The page's Orientation tag (1–8; 1 when absent or invalid).
    pub fn orientation(&self) -> u16 {
        self.stored
    }

    /// The size of the image the bands make up (upright when [`TiffBands::upright`] was applied).
    pub fn dimensions(&self) -> (u32, u32) {
        if self.turn >= 5 { (self.p.h, self.p.w) } else { (self.p.w, self.p.h) }
    }

    pub fn layout(&self) -> ChannelLayout {
        self.p.lay.out_layout
    }

    /// How many samples of a pixel are neither colour nor the alpha channel: alpha and spot
    /// channels that [`TiffBands::decode`] does not read (a whole-image decode does).
    pub fn extra_samples(&self) -> usize {
        self.p.lay.extra_at.len()
    }

    pub fn sample_type(&self) -> SampleType {
        self.p.lay.out
    }

    /// Bands decoded at once: about [`GROUP_BYTES`], at least one per worker, at most `max_alloc`.
    fn bands_per_group(&self) -> Result<usize, CodecError> {
        let band = self.p.band_bytes()?.max(1);
        #[cfg(not(target_arch = "wasm32"))]
        let workers = rayon::current_num_threads();
        #[cfg(target_arch = "wasm32")]
        let workers = 1;
        let cap = usize::try_from(self.cap).unwrap_or(usize::MAX);
        Ok((GROUP_BYTES / band).max(workers).min(cap / band).max(1))
    }

    /// Bytes of the largest group [`TiffBands::decode`] hands over: what decoding needs beside
    /// the document, for a caller checking that it fits in memory. A file stored as one strip
    /// is one group, the whole image.
    pub fn group_bytes(&self) -> Result<usize, CodecError> {
        let rows = self.bands_per_group()?.saturating_mul(self.p.band_rows).min(self.p.h as usize);
        self.p.lay.out_row().checked_mul(rows).ok_or_else(|| CodecError::LimitExceeded("band size".into()))
    }

    /// A conservative bound on the decoder's own scratch memory in a whole-image decode of this
    /// page (as `decode_page` does it): one strip or tile per worker that decodes a band at
    /// once. Zero when strips are decoded in place. Add the decoded image itself and any
    /// extra-sample planes.
    pub fn decode_scratch_bytes(&self) -> u64 {
        #[cfg(not(target_arch = "wasm32"))]
        let workers = rayon::current_num_threads() as u64;
        #[cfg(target_arch = "wasm32")]
        let workers = 1;
        scratch_bound(self.p.lay.direct(), self.p.scratch_bytes as u64, self.p.band_rows, self.p.h, workers)
    }

    /// Decodes the image a group of bands at a time, in order, handing each group to `sink`
    /// with the position of its top-left pixel in the image [`TiffBands::dimensions`] gives. A
    /// group is about 64 MiB (at least one strip or row of tiles per worker, at most `max_alloc`;
    /// see [`TiffBands::group_bytes`]). Data cut off by the end of the file adds a
    /// [`DecodeWarning::Truncated`] to `warnings`.
    pub fn decode<E: From<CodecError>>(&mut self, mut sink: impl FnMut(u32, u32, &Image) -> Result<(), E>) -> Result<(), E> {
        let per_group = self.bands_per_group()?;
        let p = &self.p;
        // The band workers' working memory (scratch buffers, decoder state) fits `max_alloc` as
        // a group does: no more workers run at once than that allows.
        let workers = budget_workers(0, p.per_worker, self.cap, worker_threads())?;
        let band = p.band_bytes()?.max(1);
        let bands = (p.h as usize).div_ceil(p.band_rows.max(1));
        let mut buf: Vec<u8> = Vec::new();
        let mut tally = Tally::default();
        let mut first = 0;
        while first < bands {
            let n = per_group.min(bands - first);
            let y0 = first * p.band_rows;
            let rows = (n * p.band_rows).min(p.h as usize - y0);
            let len = p.lay.out_row().checked_mul(rows).ok_or_else(|| CodecError::LimitExceeded("band size".into()))?;
            // Missing chunks leave their rows as they are: they must start empty in every group.
            buf = refill_zeroed(buf, len)?;
            tally = tally + decode_bands(p, self.b, &mut buf, band, first, &mut [], 0, workers)?;
            if let Some(c) = p.ycbcr {
                c.convert(&mut buf, p.lay.out_channels(), p.lay.out);
            }
            if let Some(signed) = p.lab {
                convert_ab(&mut buf, p.lay.out_channels(), p.lay.out, signed, false);
            }
            let rows = u32::try_from(rows).map_err(|_| err("band too tall"))?;
            let y0 = u32::try_from(y0).map_err(|_| err("band out of range"))?;
            let group = Image::from_raw(p.w, rows, p.lay.out_layout, p.lay.out, std::mem::take(&mut buf))?;
            let (x, y) = match self.turn {
                3 | 4 => (0, p.h.saturating_sub(y0 + rows)),
                5 | 8 => (y0, 0),
                6 | 7 => (p.h.saturating_sub(y0 + rows), 0),
                _ => (0, y0),
            };
            let group = group.oriented(self.turn)?;
            sink(x, y, &group)?;
            buf = group.into_data();
            first += n;
        }
        if tally.decoded == 0 && tally.missing > 0 {
            return Err(err("the image data is missing (cut off or past the end of the file)").into());
        }
        if tally.missing > 0 {
            // After the extra-sample warnings and before `MorePages`, as the whole-image decode
            // orders them.
            let at = self.warnings.iter().position(|w| matches!(w, DecodeWarning::MorePages { .. })).unwrap_or(self.warnings.len());
            self.warnings.insert(at, DecodeWarning::Truncated { format: F });
        }
        Ok(())
    }
}

/// [`TiffBands::decode_scratch_bytes`]'s bound: none when chunks are decoded in place,
/// otherwise one chunk per worker that decodes a band, and never more chunks than the image has.
fn scratch_bound(direct: bool, chunk_bytes: u64, band_rows: usize, h: u32, workers: u64) -> u64 {
    if direct {
        return 0;
    }
    let bands = u64::from(h).div_ceil(band_rows.max(1) as u64).max(1);
    chunk_bytes.saturating_mul(workers.min(bands))
}

/// The Orientation (1–8) of page `page` (`None`: the default page); 1 when absent, malformed
/// or out of range. Classic TIFF and BigTIFF.
pub(crate) fn orientation(bytes: &[u8], page: Option<usize>) -> u16 {
    let Some(file) = File::parse(bytes) else { return 1 };
    let Ok(info) = ifd::info_of(&file) else { return 1 };
    page.or_else(|| info.default_page()).and_then(|i| info.pages.get(i)).and_then(|p| file.ifd(p.ifd_offset)).map_or(1, |d| ifd::orientation_of(&file, &d))
}

/// The page's metadata; the ICC profile goes to `icc`.
fn metadata(f: &File<'_>, dir: &Ifd, icc: &mut Option<Vec<u8>>) -> Metadata {
    *icc = f.tag_bytes(dir, Tag::IccProfile.to_u16()).filter(|b| !b.is_empty()).map(<[u8]>::to_vec);
    let text = |tag: u16| {
        let b = f.tag_bytes(dir, tag)?;
        let s = std::str::from_utf8(b).ok()?.trim_end_matches('\0');
        (!s.is_empty()).then(|| s.to_owned())
    };
    let mut meta = Metadata { xmp: text(TAG_XMP), ..Default::default() };
    let unit = f.tag_uint(dir, Tag::ResolutionUnit.to_u16()).unwrap_or(2);
    if let (Some(x), Some(y)) = (f.tag_f64(dir, Tag::XResolution.to_u16()), f.tag_f64(dir, Tag::YResolution.to_u16()))
        && x > 0.0
        && y > 0.0
        && x.is_finite()
        && y.is_finite()
    {
        meta.dpi = match unit {
            2 => Some((x as f32, y as f32)),
            3 => Some(((x * 2.54) as f32, (y * 2.54) as f32)),
            _ => None,
        };
    }
    for (tag, key) in TEXT_TAGS {
        if let Some(s) = text(tag.to_u16()) {
            meta.text.push(((*key).to_owned(), s));
        }
    }
    // Photoshop keeps a layered TIFF's layers and resources in IFD0, where `photoshop_tags`
    // reads them; any other page reads its own directory's copies.
    let (resources, layers) =
        if dir.at == f.first_ifd { photoshop_tags(f.b) } else { (f.tag_bytes(dir, TAG_PHOTOSHOP), f.tag_bytes(dir, TAG_IMAGE_SOURCE_DATA)) };
    meta.photoshop_resources = resources.map(<[u8]>::to_vec);
    meta.photoshop_layers = layers.map(<[u8]>::to_vec);
    // EXIF: the Exif and GPS directories IFD0 points to, with IFD0's descriptive tags, as an
    // EXIF block (the other pages are reduced copies or layers of the same image).
    if dir.at == f.first_ifd
        && let Some(e) = tiff_exif::read(f.b).filter(ExifDirs::has_directories)
    {
        meta.exif = Some(tiff_exif::block(&e));
    }
    meta
}

// ---------------------------------------------------------------------------
// Banded pixel decoding
// ---------------------------------------------------------------------------

/// The file seen with a few byte ranges replaced: the header's first-IFD offset (so the `tiff`
/// decoder opens the chosen directory as its IFD0) and, for WhiteIsZero and palette images,
/// the PhotometricInterpretation value (so the decoder hands over the stored samples and this
/// module applies the interpretation itself). Reads past the end return no bytes.
#[derive(Clone, Copy)]
pub(super) struct Patched<'a> {
    b: &'a [u8],
    patches: &'a [(u64, Vec<u8>)],
    pos: u64,
}

impl Read for Patched<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let Some(src) = usize::try_from(self.pos).ok().and_then(|p| self.b.get(p..)) else { return Ok(0) };
        let n = out.len().min(src.len());
        let (Some(o), Some(s)) = (out.get_mut(..n), src.get(..n)) else { return Ok(0) };
        o.copy_from_slice(s);
        let end = self.pos.saturating_add(n as u64);
        for (at, bytes) in self.patches {
            let lo = (*at).max(self.pos);
            let hi = at.saturating_add(bytes.len() as u64).min(end);
            for i in lo..hi {
                let (Ok(oi), Ok(pi)) = (usize::try_from(i - self.pos), usize::try_from(i - at)) else { continue };
                if let (Some(d), Some(&v)) = (o.get_mut(oi), bytes.get(pi)) {
                    *d = v;
                }
            }
        }
        self.pos = end;
        Ok(n)
    }
}

impl Seek for Patched<'_> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let new = match to {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(d) => (self.b.len() as u64).checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        let new = new.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek out of range"))?;
        self.pos = new;
        Ok(new)
    }
}

/// How a stored sample is laid out in the `tiff` crate's chunk buffer (native endian).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Src {
    /// 1, 2 or 4-bit unsigned, packed MSB first, rows padded to a byte.
    Bits(u8),
    U8,
    U16,
    U32,
    U64,
    F16,
    F32,
    F64,
}

impl Src {
    pub(super) fn bytes(self) -> usize {
        match self {
            Src::Bits(_) | Src::U8 => 1,
            Src::U16 | Src::F16 => 2,
            Src::U32 | Src::F32 => 4,
            Src::U64 | Src::F64 => 8,
        }
    }

    /// The output sample type: integers above 16 bits keep their top 16, doubles become f32.
    fn out(self) -> SampleType {
        match self {
            Src::Bits(_) | Src::U8 => SampleType::U8,
            Src::U16 | Src::U32 | Src::U64 => SampleType::U16,
            Src::F16 => SampleType::F16,
            Src::F32 | Src::F64 => SampleType::F32,
        }
    }

    /// Stored bytes and output bytes are the same: a plain copy.
    pub(super) fn is_identity(self) -> bool {
        matches!(self, Src::U8 | Src::U16 | Src::F16 | Src::F32)
    }
}

/// Everything needed to decode and place the chunks of one directory.
pub(super) struct Layout {
    pub(super) w: usize,
    pub(super) src: Src,
    pub(super) out: SampleType,
    pub(super) out_layout: ChannelLayout,
    /// Samples per pixel in a decoded chunk (1 for planar data).
    pub(super) chunk_spp: usize,
    pub(super) planar: bool,
    /// Planes decoded for planar data: one per output channel (extra planes are skipped).
    pub(super) planes: usize,
    /// Tile width (a strip is `w` wide).
    pub(super) cw: usize,
    pub(super) across: usize,
    pub(super) chunks_per_plane: usize,
    /// WhiteIsZero: invert the gray channel.
    pub(super) invert: bool,
    /// The decoded sample (chunky) or file plane (planar) of the alpha channel (when there is one).
    alpha_sample: usize,
    /// Associated (premultiplied) alpha to turn straight.
    pub(super) premultiplied: bool,
    /// Palette images: the colour map (16-bit RGB per index).
    pub(super) palette: Option<Vec<[u16; 3]>>,
    /// Upper bound for one chunk's scratch buffer.
    pub(super) scratch_cap: usize,
    /// File sample indices that are neither colour nor the chosen alpha (spot / extra alpha).
    pub(super) extra_at: Vec<usize>,
}

impl Layout {
    pub(super) fn out_channels(&self) -> usize {
        self.out_layout.channels()
    }
    /// The decoded sample (chunky) or file plane (planar) of output channel `k`.
    pub(super) fn source_sample(&self, k: usize) -> usize {
        if self.out_layout.has_alpha() && k + 1 == self.out_channels() { self.alpha_sample } else { k }
    }
    pub(super) fn out_row(&self) -> usize {
        self.w * self.out_channels() * self.out.bytes()
    }
    pub(super) fn extra_row(&self) -> usize {
        self.w.saturating_mul(self.extra_at.len()).saturating_mul(self.out.bytes())
    }
    /// A chunky strip already in the output's layout: decoded in place.
    pub(super) fn direct(&self) -> bool {
        self.copyable() && self.across == 1
    }
    /// Chunk rows are output rows: copied without conversion.
    pub(super) fn copyable(&self) -> bool {
        !self.planar && self.palette.is_none() && self.src.is_identity() && self.chunk_spp == self.out_channels()
    }
}

/// A chunk read that hit the end of the file (the data is missing, not malformed).
pub(super) fn is_eof(e: &tiff::TiffError) -> bool {
    matches!(e, tiff::TiffError::IoError(io) if io.kind() == io::ErrorKind::UnexpectedEof)
}

/// The sample type of a (SampleFormat, BitsPerSample) pair.
fn sample_src(format: u64, bits: u64) -> Result<Src, CodecError> {
    Ok(match (format, bits) {
        (1, 1 | 2 | 4) => Src::Bits(bits as u8),
        (1, 8) => Src::U8,
        (1, 16) => Src::U16,
        (1, 32) => Src::U32,
        (1, 64) => Src::U64,
        (3, 16) => Src::F16,
        (3, 32) => Src::F32,
        (3, 64) => Src::F64,
        (2, _) => return Err(CodecError::unsupported(F, "signed integer samples")),
        _ => return Err(CodecError::unsupported(F, format!("{bits}-bit samples (sample format {format})"))),
    })
}

/// The ColorMap of a palette image as RGB entries.
fn read_palette(f: &File<'_>, dir: &Ifd, spp: u64, bits: u64, src: Src) -> Result<Vec<[u16; 3]>, CodecError> {
    if spp != 1 || !matches!(src, Src::Bits(_) | Src::U8 | Src::U16) {
        return Err(CodecError::unsupported(F, format!("palette image with {spp} samples of {bits} bits")));
    }
    let n = 1u64 << bits;
    let map = dir.get(ifd::TAG_COLOR_MAP).filter(|e| e.ty == 3 && e.count == 3 * n).ok_or_else(|| err("palette image without a valid ColorMap"))?;
    let at = |i: u64| f.uint(map, i).and_then(|v| u16::try_from(v).ok());
    let table: Option<Vec<[u16; 3]>> = (0..n).map(|i| Some([at(i)?, at(n + i)?, at(2 * n + i)?])).collect();
    table.ok_or_else(|| err("palette ColorMap cut off"))
}

/// The layout of a multiband read that was asked for every sample.
fn multiband_layout(photometric: u64, has_alpha: bool, gray_alpha: bool) -> ChannelLayout {
    match (photometric, has_alpha) {
        (2 | 8 | 9, true) => ChannelLayout::Rgba,
        (2 | 8 | 9, false) => ChannelLayout::Rgb,
        (5, true) => ChannelLayout::CmykA,
        (5, false) => ChannelLayout::Cmyk,
        _ if gray_alpha => ChannelLayout::GrayA,
        _ => ChannelLayout::Gray,
    }
}

const TAG_PREDICTOR: u16 = 317;
const TAG_JPEG_TABLES: u16 = 347;
/// Marker segments looked at before a JPEG chunk's frame header. Real chunks hold a handful
/// (SOI, DRI, SOF, DHT, DQT, SOS); the cap keeps chunks that all point at one long run of
/// segments from costing chunks x segments.
const MAX_JPEG_CHUNK_MARKERS: usize = 256;

/// Fill bytes (`0xFF` runs before a marker) all JPEG chunks of a TIFF may scan together. Each
/// distinct chunk offset rescans its own run, so this bounds the total work.
const MAX_JPEG_FILL_BYTES: usize = 1 << 24;

/// The frame header (width, height, components) of a JPEG stream: `Ok(None)` when it has none
/// before its first scan, `Err` when more than [`MAX_JPEG_CHUNK_MARKERS`] segments come first.
fn jpeg_frame(b: &[u8], fill: &mut usize) -> Result<Option<(usize, usize, usize)>, CodecError> {
    if b.get(..2) != Some(&[0xFF, 0xD8][..]) {
        return Ok(None);
    }
    jpeg_frame_from(b, 2, fill)
}

/// [`jpeg_frame`] from byte `start` on, with no SOI check. `fill` counts the fill bytes scanned.
fn jpeg_frame_from(b: &[u8], start: usize, fill: &mut usize) -> Result<Option<(usize, usize, usize)>, CodecError> {
    let be16 = |at: usize| b.get(at..at + 2).and_then(|v| <[u8; 2]>::try_from(v).ok()).map(|v| usize::from(u16::from_be_bytes(v)));
    let mut i = start;
    for _ in 0..MAX_JPEG_CHUNK_MARKERS {
        while b.get(i) == Some(&0xFF) && b.get(i + 1) == Some(&0xFF) {
            i += 1;
            *fill += 1;
            if *fill > MAX_JPEG_FILL_BYTES {
                return Err(CodecError::LimitExceeded("JPEG strips or tiles scan too many fill bytes".into()));
            }
        }
        if b.get(i) != Some(&0xFF) {
            return Ok(None);
        }
        let Some(&code) = b.get(i + 1) else { return Ok(None) };
        match code {
            0xC0..=0xCF if !matches!(code, 0xC4 | 0xC8 | 0xCC) => {
                let frame = be16(i + 5).zip(be16(i + 7)).zip(b.get(i + 9));
                return Ok(frame.map(|((h, w), &nc)| (w, h, usize::from(nc))));
            }
            0xD9 | 0xDA => return Ok(None),
            0x01 | 0xD0..=0xD8 => i += 2,
            _ => match be16(i + 2) {
                Some(len) => i += 2 + len,
                None => return Ok(None),
            },
        }
    }
    Err(err("a JPEG strip or tile has too many marker segments before its frame header"))
}

/// Refuses JPEG strips or tiles whose embedded frame is larger than the strip or tile
/// (`nominal`, unclipped) or whose decoded frame exceeds `cap` bytes. Chunks sharing an offset
/// are checked once.
fn check_jpeg_chunks(
    f: &File<'_>,
    offsets: &ifd::Entry,
    sizes: &ifd::Entry,
    expected: u64,
    tables: Option<&[u8]>,
    nominal: (usize, usize),
    cap: usize,
) -> Result<(), CodecError> {
    // The list below holds 16 bytes per chunk; bound it by the scratch cap, not only by the file size.
    if expected.saturating_mul(16) > cap as u64 {
        return Err(CodecError::LimitExceeded(format!("{expected} JPEG strips or tiles")));
    }
    // Reserve exactly the budgeted length: collecting from filter_map could over-allocate past `cap`.
    let mut chunks: Vec<(u64, u64)> = Vec::with_capacity(expected as usize);
    chunks.extend((0..expected).filter_map(|i| Some((f.uint(offsets, i)?, f.uint(sizes, i)?))));
    chunks.sort_unstable();
    // Entries sharing an offset but not a byte count are different chunks: check each one.
    chunks.dedup();
    let len = f.b.len() as u64;
    let mut fill = 0;
    for (at, n) in chunks {
        let end = at.saturating_add(n).min(len);
        let Some(data) = usize::try_from(at).ok().zip(usize::try_from(end).ok()).and_then(|(a, e)| f.b.get(a..e)) else { continue };
        let mut frame = match jpeg_frame(data, &mut fill)? {
            Some(frame) => Some(frame),
            None => tables.map(|t| jpeg_frame(t, &mut fill)).transpose()?.flatten(),
        };
        // Without SOI, the tiff crate drops the chunk's first two bytes, so a frame at offset 2 is decoded.
        if frame.is_none() && data.get(..2) != Some(&[0xFF, 0xD8][..]) {
            frame = jpeg_frame_from(data, 2, &mut fill)?;
        }
        let Some((w, h, nc)) = frame else { continue };
        if w > nominal.0 || h > nominal.1 {
            return Err(err(format!("a {w}x{h} JPEG frame in a {}x{} strip or tile", nominal.0, nominal.1)));
        }
        if w.checked_mul(h).and_then(|px| px.checked_mul(nc.max(1))).is_none_or(|bytes| bytes > cap) {
            return Err(CodecError::LimitExceeded(format!("a {w}x{h} JPEG strip or tile")));
        }
    }
    Ok(())
}

/// The chunk tables must match the geometry. Checked in 64 bits, before the `tiff` crate does
/// the same arithmetic in narrower types.
fn check_chunk_tables(f: &File<'_>, dir: &Ifd, (w, h): (u32, u32), spp: u64, planar: bool, tiled: bool) -> Result<(u64, u64, u64), CodecError> {
    let need = |tag: u16, name: &str| f.tag_uint(dir, tag).ok_or_else(|| err(format!("missing or unreadable {name}")));
    let (cw, ch) = if tiled {
        let tw = need(ifd::TAG_TILE_WIDTH, "TileWidth")?;
        let tl = need(ifd::TAG_TILE_LENGTH, "TileLength")?;
        (tw, tl)
    } else {
        (u64::from(w), f.tag_uint(dir, ifd::TAG_ROWS_PER_STRIP).unwrap_or(u64::from(h)).min(u64::from(h)))
    };
    if cw == 0 || ch == 0 {
        return Err(err("zero tile or strip size"));
    }
    // The chunk tables must match the geometry. Checked here, in 64 bits, before the `tiff`
    // crate does the same arithmetic in narrower types.
    let planes_in_file = if planar { spp } else { 1 };
    let expected = u64::from(w)
        .div_ceil(cw)
        .checked_mul(u64::from(h).div_ceil(ch))
        .and_then(|n| n.checked_mul(planes_in_file))
        .filter(|&n| n <= u64::from(u32::MAX))
        .ok_or_else(|| err("too many strips or tiles"))?;
    let (offsets_tag, counts_tag) = if tiled { (324, 325) } else { (273, 279) };
    let counts = (dir.get(offsets_tag).map(|e| e.count), dir.get(counts_tag).map(|e| e.count));
    if counts != (Some(expected), Some(expected)) {
        return Err(err(format!("{expected} strips or tiles expected, the tables list {counts:?}")));
    }
    Ok((cw, ch, expected))
}

/// Byte patches the `tiff` crate reads through: the directory is seen as the last of its chain,
/// and PhotometricInterpretation is handed over as black-is-zero (or RGB for Lab) when asked to.
fn header_patches(
    f: &File<'_>,
    dir: &Ifd,
    photometric_entry: Option<ifd::Entry>,
    patch_photometric: bool,
    lab_only: bool,
) -> Result<Vec<(u64, Vec<u8>)>, CodecError> {
    let mut patches: Vec<(u64, Vec<u8>)> = Vec::new();
    let encode_u = |v: u64, len: usize| -> Vec<u8> {
        let b = if f.le { v.to_le_bytes() } else { v.to_be_bytes() };
        if f.le { b.get(..len).map(<[u8]>::to_vec).unwrap_or_default() } else { b.get(8 - len..).map(<[u8]>::to_vec).unwrap_or_default() }
    };
    if f.big {
        patches.push((8, encode_u(dir.at, 8)));
    } else {
        let at = u32::try_from(dir.at).map_err(|_| err("directory offset out of range"))?;
        patches.push((4, encode_u(u64::from(at), 4)));
    }
    // The directory is seen as the last of its chain: the chain was walked already, and a
    // cycle through it must not fail this page.
    patches.push((dir.next_at, vec![0; if f.big { 8 } else { 4 }]));
    if patch_photometric {
        let e = photometric_entry.ok_or_else(|| err("missing PhotometricInterpretation"))?;
        let len = match e.ty {
            3 if e.count == 1 => 2,
            4 if e.count == 1 => 4,
            _ => return Err(err("malformed PhotometricInterpretation")),
        };
        patches.push((e.value_at, encode_u(if lab_only { 2 } else { 1 }, len)));
    }
    Ok(patches)
}

/// Lab and YCbCr are converted after decoding, from 8 or 16-bit samples only: Lab needs at least
/// its three colour samples, YCbCr exactly three.
fn check_converted_model(photometric: u64, spp: u64, bits: u64, src: Src) -> Result<(), CodecError> {
    let (name, samples_ok) = match photometric {
        8 | 9 => ("Lab", spp >= 3),
        6 => ("YCbCr", spp == 3),
        _ => return Ok(()),
    };
    if samples_ok && matches!(src, Src::U8 | Src::U16) {
        return Ok(());
    }
    Err(CodecError::unsupported(F, format!("{name} image with {spp} samples of {bits} bits")))
}

/// A directory checked and laid out for decoding, before any pixel is read.
pub(super) struct Prepared {
    w: u32,
    h: u32,
    pub(super) lay: Layout,
    /// Rows per band: one strip, or one row of tiles.
    band_rows: usize,
    patches: Vec<(u64, Vec<u8>)>,
    tl: tiff::decoder::Limits,
    /// Lab samples: `Some(true)` for CIELab (a* and b* stored signed), `Some(false)` for ICCLab.
    lab: Option<bool>,
    /// YCbCr samples: the conversion to RGB (#10316).
    ycbcr: Option<YCbCr>,
    /// The colour samples of a pixel in the file's colour model.
    color: usize,
    /// The ExtraSamples tag (0 unspecified, 1 associated alpha, 2 unassociated alpha).
    extra_kinds: Vec<u64>,
    /// Extra samples past `MAX_EXTRA_SAMPLES`, left out.
    extras_over_limit: usize,
    /// Extra samples left out because they did not fit the allocation budget beside the image.
    extras_dropped: u32,
    /// Bytes one strip or tile decodes to, before clipping to the image: a worker's scratch
    /// buffer. A strip is the full width; a tile keeps its declared size even at an edge.
    scratch_bytes: usize,
    /// Bytes one band worker holds while it decodes: its decoder's chunk tables, its strip or
    /// tile scratch buffer (none when strips decode in place) and what the `tiff` crate holds
    /// for one chunk (a JPEG chunk's decoder, a floating-point predictor's row).
    per_worker: u64,
}

impl Prepared {
    /// A decoder over `b` (the file the directory was read from) that opens the directory.
    pub(super) fn open<'a>(&'a self, b: &'a [u8]) -> Result<Decoder<Patched<'a>>, CodecError> {
        Decoder::new(Patched { b, patches: &self.patches, pos: 0 }).map(|d| d.with_limits(self.tl.clone())).map_err(map_err)
    }

    /// Output bytes of one full band.
    fn band_bytes(&self) -> Result<usize, CodecError> {
        self.lay.out_row().checked_mul(self.band_rows).ok_or_else(|| CodecError::LimitExceeded("band size".into()))
    }
}

/// Decodes the pixels of one directory.
fn decode_ifd(f: &File<'_>, dir: &Ifd, limits: &Limits) -> Result<Image, CodecError> {
    let p = prepare(f, dir, limits, true)?;
    let len = p.lay.out_row().checked_mul(p.h as usize).ok_or_else(|| CodecError::LimitExceeded("image size".into()))?;
    let extra_row = p.lay.extra_row();
    let extra_len = extra_row.checked_mul(p.h as usize).ok_or_else(|| CodecError::LimitExceeded("extra samples".into()))?;
    // As many band workers run at once as fit in `max_alloc` beside the output and the extra
    // samples; when not even one does, the image is refused before anything is allocated.
    let workers = budget_workers((len as u64).saturating_add(extra_len as u64), p.per_worker, limits.max_alloc, worker_threads())?;
    // The output, allocated once and fallibly.
    let mut data = zeroed(len)?;
    let mut extra_data = if p.lay.extra_at.is_empty() { Vec::new() } else { zeroed(extra_len)? };
    let extra_band = extra_row.saturating_mul(p.band_rows);
    let tally = decode_bands(&p, f.b, &mut data, p.band_bytes()?, 0, extra_data.as_mut_slice(), extra_band, workers)?;
    if tally.decoded == 0 && tally.missing > 0 {
        return Err(err("the image data is missing (cut off or past the end of the file)"));
    }
    if let Some(c) = p.ycbcr {
        c.convert(&mut data, p.lay.out_channels(), p.lay.out);
    }
    if let Some(signed) = p.lab {
        convert_ab(&mut data, p.lay.out_channels(), p.lay.out, signed, false);
    }
    let mut img = Image::from_raw(p.w, p.h, p.lay.out_layout, p.lay.out, data)?;
    if !p.lay.extra_at.is_empty() {
        // TIFF 6.0 defines ExtraSamples 0, 1 and 2 only; a reserved value reads as unspecified,
        // so a save never writes it back.
        let kind_of = |si: usize| match si.checked_sub(p.color).and_then(|k| p.extra_kinds.get(k)).copied() {
            Some(v @ 0..=2) => v as u16,
            _ => 0,
        };
        img.meta.extra_samples = split_extra_samples(&extra_data, p.w as usize, p.h as usize, &p.lay.extra_at, p.lay.out, kind_of)?;
    }
    img.warnings.extend(prepared_warnings(&p));
    if tally.missing > 0 {
        img.warnings.push(DecodeWarning::Truncated { format: F });
    }
    Ok(img)
}

/// What [`prepare`] left out of the image: extra samples past the supported count, then extra
/// samples over the allocation budget. Both decoders report them, in this order.
fn prepared_warnings(p: &Prepared) -> Vec<DecodeWarning> {
    let mut warnings = Vec::new();
    if p.extras_over_limit > 0 {
        let count = u32::try_from(p.extras_over_limit).unwrap_or(u32::MAX);
        warnings.push(DecodeWarning::ExtraSamplesOverLimit { format: F, count });
    }
    if p.extras_dropped > 0 {
        warnings.push(DecodeWarning::ExtraSamplesDropped { format: F, count: p.extras_dropped });
    }
    warnings
}

/// The structural tags the `tiff` crate reads must exist (a count past the end of the file is
/// neither allocated nor read) and appear once: this parser reads the first entry of a tag and
/// the `tiff` crate the last, so a repeated structural tag (a small SamplesPerPixel first, a
/// large one last) would be checked here with one value and sized by the crate with another.
fn check_structural_tags(f: &File<'_>, dir: &Ifd) -> Result<(), CodecError> {
    if let Some(e) = dir.entries.iter().find(|e| ifd::STRUCTURAL_TAGS.contains(&e.tag) && f.data(e).is_none()) {
        return Err(err(format!("tag {} points past the end of the file", e.tag)));
    }
    let mut structural: Vec<u16> = dir.entries.iter().map(|e| e.tag).filter(|t| ifd::STRUCTURAL_TAGS.contains(t)).collect();
    structural.sort_unstable();
    if let Some(w) = structural.windows(2).find(|w| w[0] == w[1]) {
        return Err(err(format!("tag {} appears more than once", w[0])));
    }
    Ok(())
}

/// The bytes the first `expected` strips or tiles hold inside the file, never more than the file.
fn held_bytes(f: &File<'_>, (offsets, sizes): (&ifd::Entry, &ifd::Entry), expected: u64) -> u64 {
    let len = f.b.len() as u64;
    let mut held = 0u64;
    for i in 0..expected {
        if held >= len {
            break;
        }
        if let (Some(at), Some(n)) = (f.uint(offsets, i), f.uint(sizes, i)) {
            held = held.saturating_add(n.min(len.saturating_sub(at)));
        }
    }
    held.min(len)
}

/// Checks one directory and lays it out for decoding. `whole`: the image is decoded into one
/// buffer, which `limits.max_alloc` bounds; otherwise only its size is checked here. Chunk
/// tables larger than `limits.max_alloc` are refused before the `tiff` crate reads them.
fn prepare(f: &File<'_>, dir: &Ifd, limits: &Limits, whole: bool) -> Result<Prepared, CodecError> {
    check_structural_tags(f, dir)?;
    let need = |tag: u16, name: &str| f.tag_uint(dir, tag).ok_or_else(|| err(format!("missing or unreadable {name}")));
    let w = u32::try_from(need(ifd::TAG_WIDTH, "ImageWidth")?).map_err(|_| err("ImageWidth out of range"))?;
    let h = u32::try_from(need(ifd::TAG_HEIGHT, "ImageLength")?).map_err(|_| err("ImageLength out of range"))?;
    if w == 0 || h == 0 {
        return Err(CodecError::InvalidImage(format!("zero-sized image {w}x{h}")));
    }
    let spp = f.tag_uint(dir, ifd::TAG_SAMPLES).unwrap_or(1);
    if spp == 0 || spp > 64 {
        return Err(CodecError::unsupported(F, format!("{spp} samples per pixel")));
    }
    let bits_all = f.tag_uints(dir, ifd::TAG_BITS, spp).unwrap_or_else(|| vec![1]);
    let bits = bits_all.first().copied().unwrap_or(1);
    if bits_all.iter().any(|&b| b != bits) {
        return Err(CodecError::unsupported(F, format!("mixed bits per sample {bits_all:?}")));
    }
    let format = f.tag_uint(dir, ifd::TAG_SAMPLE_FORMAT).unwrap_or(1);
    let photometric_entry = dir.get(ifd::TAG_PHOTOMETRIC).copied();
    let photometric = f.tag_uint(dir, ifd::TAG_PHOTOMETRIC).ok_or_else(|| CodecError::unsupported(F, "missing PhotometricInterpretation"))?;
    let planar = f.tag_uint(dir, ifd::TAG_PLANAR) == Some(2);
    let extra = f.tag_uints(dir, ifd::TAG_EXTRA_SAMPLES, 64).unwrap_or_default();
    let tiled = dir.get(ifd::TAG_TILE_OFFSETS).is_some();
    let (cw, ch, expected) = check_chunk_tables(f, dir, (w, h), spp, planar, tiled)?;
    let (offsets_tag, counts_tag) = if tiled { (324, 325) } else { (273, 279) };
    let src = sample_src(format, bits)?;
    // WhiteIsZero and palette images are handed to the `tiff` crate as BlackIsZero: it then
    // returns the stored samples, and the interpretation is applied here (for every depth and
    // with alpha, which the crate's own inversion does not cover).
    let invert = photometric == 0;
    let palette = if photometric == 3 { Some(read_palette(f, dir, spp, bits, src)?) } else { None };
    // CIE L*a*b*: CIELab (8; a* and b* signed) and ICCLab (9; a* and b* offset) are handed to the
    // `tiff` crate as RGB, then labelled Lab with a* and b* in PhotoCraft's encoding
    // (`convert_ab`).
    let lab = matches!(photometric, 8 | 9);
    // YCbCr (6), as scanners and cameras write it (usually JPEG-compressed with subsampled
    // chroma): the `tiff` crate decodes and upsamples the samples, and they are turned into RGB
    // after decoding (#10316).
    let ycbcr = photometric == 6;
    check_converted_model(photometric, spp, bits, src)?;
    // TIFF 6.0 lets any extra sample be the alpha (#1349). For RGB and CMYK the `tiff` crate
    // returns the alpha only when it is the first extra sample: otherwise it is asked for every
    // sample (as multiband gray) and the colour and alpha samples are picked here. An alpha
    // position past the samples the file has is ignored.
    let color: usize = match photometric {
        2 | 6 | 8 | 9 => 3,
        5 => 4,
        _ => 1,
    };
    let alpha_at = extra.iter().position(|&e| e == 1 || e == 2).filter(|&a| (color as u64).saturating_add(a as u64) < spp);
    // Samples the colour model does not keep. RGB, CMYK and Lab only hand back every sample when
    // asked (photometric patched to black-is-zero below); gray already comes back as multiband.
    let unused = !extra.is_empty() && (spp as usize) > color.saturating_add(usize::from(alpha_at.is_some()));
    let all_samples = palette.is_none() && matches!(photometric, 2 | 5 | 8 | 9) && (alpha_at.is_some_and(|a| a > 0) || (!planar && unused));
    let patches = header_patches(f, dir, photometric_entry, invert || palette.is_some() || all_samples || lab, lab && !all_samples)?;

    // Opening the directory reads both chunk tables, with the `tiff` crate's default limits
    // (`with_limits` applies only once it is open), before anything below is charged: tables
    // that alone are over `max_alloc` (as every band worker holds them, `DECODER_BYTES_PER_CHUNK`
    // per chunk) are refused before they are read.
    let table_bytes = expected.saturating_mul(DECODER_BYTES_PER_CHUNK);
    if table_bytes > limits.max_alloc {
        return Err(CodecError::LimitExceeded(format!(
            "the strip or tile tables of a {w}x{h} image ({expected} chunks) need {table_bytes} bytes, exceeding max_alloc {}",
            limits.max_alloc
        )));
    }
    let tl = {
        let mut l = tiff::decoder::Limits::default();
        l.decoding_buffer_size = limits.alloc_usize();
        // No compressed chunk can be larger than the file.
        l.intermediate_buffer_size = f.b.len().max(1);
        l
    };
    let mut dec = Decoder::new(Patched { b: f.b, patches: &patches, pos: 0 }).map(|d| d.with_limits(tl.clone())).map_err(map_err)?;
    if dec.dimensions().map_err(map_err)? != (w, h) || (dec.get_chunk_type() == ChunkType::Tile) != tiled {
        return Err(err("inconsistent image directory"));
    }
    let ct = dec.colortype().map_err(map_err)?;
    let gray_alpha = alpha_at.is_some() || (extra.is_empty() && spp == 2);
    let (out_layout, chunk_spp) = match ct {
        _ if palette.is_some() => (ChannelLayout::Rgb, 1),
        tiff::ColorType::Gray(_) => (ChannelLayout::Gray, 1),
        tiff::ColorType::Multiband { num_samples, .. } if all_samples => {
            // Lab reads as RGB here and is relabelled below.
            let layout = multiband_layout(photometric, alpha_at.is_some(), gray_alpha);
            (layout, usize::from(num_samples))
        }
        tiff::ColorType::Multiband { num_samples, .. } if photometric <= 1 => {
            (if gray_alpha { ChannelLayout::GrayA } else { ChannelLayout::Gray }, usize::from(num_samples))
        }
        tiff::ColorType::RGB(_) => (ChannelLayout::Rgb, 3),
        tiff::ColorType::YCbCr(_) if ycbcr => (ChannelLayout::Rgb, 3),
        tiff::ColorType::RGBA(_) => (ChannelLayout::Rgba, 4),
        tiff::ColorType::CMYK(_) => (ChannelLayout::Cmyk, 4),
        tiff::ColorType::CMYKA(_) => (ChannelLayout::CmykA, 5),
        other => return Err(CodecError::unsupported(F, format!("colour type {other:?}"))),
    };
    let out_layout = match out_layout {
        ChannelLayout::Rgb if lab => ChannelLayout::Lab,
        ChannelLayout::Rgba if lab => ChannelLayout::LabA,
        _ if lab => return Err(err("Lab image that does not read as three colour samples")),
        l => l,
    };
    let chunk_spp = if planar { 1 } else { chunk_spp };
    // The sample (chunky) or plane (planar) that holds the alpha: the colour samples come first.
    let alpha_sample = out_layout.color_channels() + alpha_at.unwrap_or(0);
    if out_layout.has_alpha() && (alpha_sample as u64 >= spp || (!planar && alpha_sample >= chunk_spp)) {
        return Err(err("the alpha sample is past the samples of a pixel"));
    }
    if matches!(src, Src::Bits(_)) && (out_layout.channels() > 1 || spp != 1) && palette.is_none() {
        return Err(CodecError::unsupported(F, format!("{bits}-bit samples with {spp} samples per pixel")));
    }
    if (!planar && palette.is_none() && chunk_spp < out_layout.channels()) || (planar && out_layout.channels() as u64 > spp) {
        return Err(err("fewer samples than the colour type needs"));
    }
    let out = match (&palette, src) {
        (Some(_), Src::U16) => SampleType::U16,
        (Some(_), _) => SampleType::U8,
        _ => src.out(),
    };
    if whole {
        limits.check(w, h, out_layout, out)?;
    } else {
        limits.check_size(w, h)?;
    }
    // Decompression-bomb guard: no compression method expands data by more than a few thousand
    // times (Deflate at most ~1032:1, LZW ~2730:1, PackBits 128:1), so a file far too small for
    // its declared size is refused before the image is allocated. CCITT Group 4 codes a blank row
    // of a strip or tile in one bit (a V0 code), so its bound is the chunk width (the decoder
    // reads at most 65535): a blank landscape A4 page at 600 dpi, 7016 px wide, compresses past
    // 4096:1 (Groups 3 and 2 are not decoded). Uncompressed data may be up
    // to three quarters missing (that decodes with a `Truncated` warning). Only the bytes the
    // strip or tile tables point at inside the file count (and never more than the file), so
    // padding the file while pointing the chunks past its end does not raise the bound: the
    // banded decoder has no whole-image `max_alloc` to fall back on.
    let compression = f.tag_uint(dir, ifd::TAG_COMPRESSION).unwrap_or(1);
    let stored = u64::from(w).saturating_mul(u64::from(h)).saturating_mul(spp).saturating_mul(bits).div_ceil(8);
    let ratio: u64 = match compression {
        1 => 4,
        32773 => 130,
        4 => cw.clamp(4096, 65_535),
        _ => 4096,
    };
    let (offsets, sizes) = (
        dir.get(offsets_tag).ok_or_else(|| err("missing strip or tile offsets"))?,
        dir.get(counts_tag).ok_or_else(|| err("missing strip or tile byte counts"))?,
    );
    let held = held_bytes(f, (offsets, sizes), expected);
    if stored > held.saturating_mul(ratio).saturating_add(1 << 20) {
        return Err(err(format!("the strips or tiles in this file hold {held} bytes, too few for a {w}x{h} image")));
    }

    let (wu, hu) = (w as usize, h as usize);
    let (cw, ch) = (usize::try_from(cw).map_err(|_| err("tile too wide"))?, usize::try_from(ch).map_err(|_| err("tile too tall"))?);
    let across = wu.div_ceil(cw);
    let down = hu.div_ceil(ch);
    let chunks_per_plane = across.checked_mul(down).ok_or_else(|| err("too many tiles"))?;
    let planes_in_file = if planar { spp as usize } else { 1 };
    let total = if tiled { dec.tile_count() } else { dec.strip_count() }.map_err(map_err)?;
    if Some(total as usize) != chunks_per_plane.checked_mul(planes_in_file) {
        return Err(err("strip or tile count does not match the image size"));
    }
    // The first chunk of a plane is the largest in it (the others are the same size or clipped
    // at the image's edge; the `tiff` crate clips the tiles of later planes only at the right
    // edge), so no chunk asks for a larger scratch buffer than these two.
    let first_chunk = dec.image_chunk_buffer_layout(0).map_err(map_err)?;
    let mut largest = first_chunk.len;
    if planes_in_file > 1 {
        let plane1 = u32::try_from(chunks_per_plane).map_err(|_| err("too many tiles"))?;
        largest = largest.max(dec.image_chunk_buffer_layout(plane1).map_err(map_err)?.len);
    }
    drop(dec);
    // One chunk's decoded bytes, clipped to the image: the scratch buffer may hold a little
    // more (the `tiff` crate reads whole tiles in some planar cases) but never an amount that
    // only a hostile tile size could ask for.
    let chunk_bytes = cw
        .min(wu)
        .checked_mul(ch.min(hu))
        .and_then(|px| px.checked_mul(chunk_spp))
        .and_then(|s| s.checked_mul(src.bytes()))
        .ok_or_else(|| CodecError::LimitExceeded("tile size".into()))?;
    let scratch_cap = chunk_bytes.saturating_mul(2).max(64 << 20).min(limits.alloc_usize());
    // The floating-point predictor makes the `tiff` crate read each row of a tile at its full
    // stored width (TileWidth, not clipped to the image) into a buffer of its own, outside the
    // scratch buffer: that row must fit the same cap.
    if f.tag_uint(dir, TAG_PREDICTOR) == Some(3) {
        let stored_row = cw.checked_mul(chunk_spp).and_then(|n| n.checked_mul(src.bytes()));
        if stored_row.is_none_or(|n| n > scratch_cap) {
            return Err(CodecError::LimitExceeded(format!("a {cw} sample wide predicted tile row")));
        }
    }
    // JPEG-compressed chunks (compression 7) are decoded by the `tiff` crate into a whole JPEG
    // image sized by the embedded frame header, with no limits, before the chunk's bytes are
    // taken. The frame must fit the strip or tile it stands for, as libtiff requires, and the
    // scratch cap.
    if compression == 7 {
        let tables = dir.get(TAG_JPEG_TABLES).and_then(|e| f.data(e));
        let (nominal_w, nominal_h) =
            (cw, if tiled { ch } else { f.tag_uint(dir, ifd::TAG_ROWS_PER_STRIP).and_then(|r| usize::try_from(r).ok()).unwrap_or(hu) });
        check_jpeg_chunks(f, offsets, sizes, expected, tables, (nominal_w, nominal_h), scratch_cap)?;
    }
    // One chunk as the decoder actually reads it: a tile keeps its declared size at an edge,
    // which can be more than the clipped image bytes above.
    let scratch_bytes = cw.saturating_mul(ch).saturating_mul(chunk_spp).saturating_mul(src.bytes());
    // What the `tiff` crate holds for one chunk beyond its output.
    let tiff_samples = if planar { 1 } else { spp };
    let codec_bytes = chunk_codec_bytes(f, dir, (offsets, sizes), expected, (cw as u64, ch as u64), tiff_samples, bits)?;
    let (extra_at, extras_over_limit) = extra_sample_indices(spp as usize, color, alpha_at, out_layout.has_alpha());
    let mut lay = Layout {
        w: wu,
        src,
        out,
        out_layout,
        chunk_spp,
        planar,
        planes: if planar { out_layout.channels() } else { 1 },
        cw,
        across,
        chunks_per_plane,
        invert: invert && palette.is_none(),
        alpha_sample,
        premultiplied: out_layout.has_alpha() && alpha_at.and_then(|a| extra.get(a)) == Some(&1),
        palette,
        scratch_cap,
        extra_at,
    };
    // The decode's working memory shares `max_alloc` with the output: each band worker holds an
    // open decoder (its chunk tables), one chunk's scratch buffer unless strips decode in place,
    // and what the `tiff` crate holds for a chunk. No chunk may ask for more scratch than that.
    let out_row = lay.out_row();
    let band_bytes = out_row.checked_mul(ch.min(hu)).ok_or_else(|| CodecError::LimitExceeded("band size".into()))?;
    let in_place = lay.direct() && first_chunk.len == band_bytes && first_chunk.row_stride.map(|s| s.get()) == Some(out_row);
    let scratch = if in_place { 0 } else { largest };
    if scratch > lay.scratch_cap {
        return Err(CodecError::LimitExceeded(format!("a {scratch} byte strip or tile")));
    }
    lay.scratch_cap = scratch;
    let tables = u64::from(total).saturating_mul(DECODER_BYTES_PER_CHUNK);
    let per_worker = (scratch as u64).saturating_add(tables).saturating_add(codec_bytes);

    // Extra samples share the allocation budget with the image: their interleaved buffer and the
    // planes split from it are held next to the image, and the interleaved buffer next to the
    // decode's working memory, so all of it counts against `max_alloc`. When they don't fit, the
    // image still opens (if it fits on its own) without them.
    let mut extras_dropped = 0;
    if whole && !lay.extra_at.is_empty() {
        let per_pixel = out_layout.channels().saturating_add(lay.extra_at.len().saturating_mul(2)).saturating_mul(out.bytes());
        let image = (out_row as u64).saturating_mul(hu as u64);
        let interleaved = (lay.extra_row() as u64).saturating_mul(hu as u64);
        let decoding = image.saturating_add(interleaved).saturating_add(per_worker);
        if limits.check_bytes(w, h, per_pixel as u64).is_err() || decoding > limits.max_alloc {
            extras_dropped = u32::try_from(lay.extra_at.len()).unwrap_or(u32::MAX);
            lay.extra_at.clear();
        }
    }
    Ok(Prepared {
        w,
        h,
        lay,
        band_rows: ch.min(hu),
        patches,
        tl,
        lab: lab.then_some(photometric == 8),
        ycbcr: ycbcr.then(|| YCbCr::of(f, dir, compression, if out == SampleType::U16 { 65535.0 } else { 255.0 })),
        color,
        extra_kinds: extra,
        extras_over_limit,
        extras_dropped,
        scratch_bytes,
        per_worker,
    })
}

/// Steps [`jpeg_sof`] may take per strip or tile beyond the file's length: room to read again
/// the few header segments of strips or tiles that share one stream (a sparse file pointing its
/// blank tiles at one tile).
const JPEG_HEADER_STEPS_PER_CHUNK: u64 = 64;

/// What the `tiff` crate holds for one chunk of directory `dir` beyond its output: a JPEG
/// chunk's decoder ([`jpeg_working_bytes`]) and a row of the declared strip or tile width for
/// the floating-point predictor ([`predictor_row_bytes`]). `offsets` and `sizes` are the chunk
/// tables (`count` entries each), `chunk` the strip or tile size and `samples` the samples per
/// pixel a chunk holds. The JPEG headers are read within the file's length plus
/// [`JPEG_HEADER_STEPS_PER_CHUNK`] steps per chunk, so chunks that overlap cannot make that
/// read longer than the file suggests.
fn chunk_codec_bytes(
    f: &File<'_>,
    dir: &Ifd,
    (offsets, sizes): (&ifd::Entry, &ifd::Entry),
    count: u64,
    chunk: (u64, u64),
    samples: u64,
    bits: u64,
) -> Result<u64, CodecError> {
    let jpeg = if f.tag_uint(dir, ifd::TAG_COMPRESSION) == Some(7) {
        let in_file = |at: u64, n: u64| {
            let at = usize::try_from(at).ok()?;
            let end = at.saturating_add(usize::try_from(n).unwrap_or(usize::MAX)).min(f.b.len());
            f.b.get(at..end)
        };
        let chunks = (0..count).map(|i| f.uint(offsets, i).zip(f.uint(sizes, i)).and_then(|(at, n)| in_file(at, n)));
        let steps = (f.b.len() as u64).saturating_add(count.saturating_mul(JPEG_HEADER_STEPS_PER_CHUNK));
        jpeg_working_bytes(f.tag_bytes(dir, TAG_JPEG_TABLES), chunks, chunk, steps)?
    } else {
        0
    };
    let row = predictor_row_bytes(f.tag_uint(dir, TAG_PREDICTOR).unwrap_or(1), chunk.0, samples, bits);
    Ok(jpeg.saturating_add(row))
}

/// Bytes the `tiff` crate holds for one row of a strip or tile as the file declares it
/// (`chunk_width` pixels of `samples` samples), which the floating-point predictor decodes each
/// row into when a chunk row holds more than the row it returns: a tile clipped at the image's
/// edge, a hostile TileWidth far wider than the image, or a strip with samples it does not
/// return. Charged for every floating-point-predicted chunk, also one that decodes in place
/// without that row (one row is a small overcount); zero for other predictors.
fn predictor_row_bytes(predictor: u64, chunk_width: u64, samples: u64, bits: u64) -> u64 {
    if predictor != 3 {
        return 0;
    }
    chunk_width.saturating_mul(samples).saturating_mul(bits).div_ceil(8)
}

/// The most zune-jpeg holds while the `tiff` crate decodes one of `chunks` (each JPEG strip or
/// tile in the file, `None` when it is not in the file): the compressed stream with `tables`
/// in front, which the `tiff` crate copies whole whether or not it holds a frame, and the
/// decoded frame its SOF marker declares with that frame's DCT coefficients (16 bits per
/// sample, padded to whole MCUs), which progressive and multi-scan streams hold whole. A frame
/// larger than its `chunk` size (width, height) is refused, as libtiff does: the decoder would
/// allocate it in full however small the TIFF chunk is. Reading the headers takes at most
/// `steps` steps in all ([`jpeg_sof`]), or the image is refused.
fn jpeg_working_bytes<'b>(tables: Option<&[u8]>, chunks: impl Iterator<Item = Option<&'b [u8]>>, chunk: (u64, u64), mut steps: u64) -> Result<u64, CodecError> {
    let table_sof = match tables {
        Some(t) => jpeg_sof(t, &mut steps)?,
        None => None,
    };
    let table_len = tables.map_or(0, |t| t.len() as u64);
    let mut most = 0u64;
    for data in chunks.flatten() {
        let mut need = (data.len() as u64).saturating_add(table_len);
        let sof = if table_sof.is_some() { table_sof } else { jpeg_sof(data, &mut steps)? };
        if let Some((w, h, components)) = sof {
            if w > chunk.0 || h > chunk.1 {
                return Err(CodecError::InvalidImage(format!("a {w}x{h} JPEG frame in a {}x{} strip or tile", chunk.0, chunk.1)));
            }
            let pad = |v: u64| v.div_ceil(32).saturating_mul(32);
            let frame = w.saturating_mul(h).saturating_mul(components);
            let coefficients = pad(w).saturating_mul(pad(h)).saturating_mul(components).saturating_mul(2);
            need = need.saturating_add(frame).saturating_add(coefficients);
        }
        most = most.max(need);
    }
    Ok(most)
}

/// The width, height and component count of the first SOF marker in JPEG stream `s`, or `None`
/// when the stream reaches its first scan (or its end) without one. Bytes between segments are
/// skipped, as zune-jpeg skips them. Each byte stepped over (a stray byte, a fill byte or a
/// marker) takes one of `steps` and a segment's body is jumped over, so the walk never takes
/// more steps than `s` has bytes. Running out of `steps` is an error, which bounds the walk
/// over strips or tiles that overlap.
fn jpeg_sof(s: &[u8], steps: &mut u64) -> Result<Option<(u64, u64, u64)>, CodecError> {
    let mut step = || -> Result<(), CodecError> {
        *steps = steps
            .checked_sub(1)
            .ok_or_else(|| CodecError::InvalidImage("the JPEG headers of the strips or tiles are longer than the file (they overlap)".into()))?;
        Ok(())
    };
    let mut i = if s.starts_with(&[0xFF, 0xD8]) { 2 } else { 0 };
    loop {
        while s.get(i).is_some_and(|&b| b != 0xFF) {
            step()?;
            i += 1;
        }
        while s.get(i) == Some(&0xFF) {
            step()?;
            i += 1;
        }
        let Some(&marker) = s.get(i) else { return Ok(None) };
        step()?;
        i += 1;
        match marker {
            // Markers without a length: SOI, TEM and the restart markers.
            0xD8 | 0x01 | 0xD0..=0xD7 => continue,
            // End of image, or start of scan: no frame header follows.
            0xD9 | 0xDA => return Ok(None),
            _ => {}
        }
        let byte = |k: usize| s.get(i.checked_add(k)?).copied().map(u64::from);
        let pair = |k: usize| Some((byte(k)? << 8) | byte(k.checked_add(1)?)?);
        // SOF0-SOF15, without DHT (C4), JPG (C8) and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            return Ok(pair(3).zip(pair(5)).zip(byte(7)).map(|((h, w), components)| (w, h, components)));
        }
        let Some(next) = pair(0).filter(|&len| len >= 2).and_then(|len| i.checked_add(usize::try_from(len).ok()?)) else {
            return Ok(None);
        };
        i = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EncodeOptions, ExtraSample, Format};

    /// `TiffBands::open` drops extra samples over the budget only on 32-bit targets (wasm32),
    /// so it reports them through the same function as the whole-image decode.
    #[test]
    fn extra_samples_over_the_budget_are_reported_by_both_decoders() {
        // Several strips (about 1 MB each): the budget holds the image and one strip of all four
        // samples, the scratch it decodes through, but not the extra sample's planes.
        let (w, h) = (1024u32, 1024u32);
        let n = (w * h) as usize;
        let mut img = Image::from_raw(w, h, ChannelLayout::Rgb, SampleType::U8, vec![90; 3 * n]).unwrap();
        img.meta.extra_samples = vec![ExtraSample { kind: 0, data: vec![7; n] }];
        let bytes = crate::encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap();
        let (file, dir, _) = open_page(&bytes, None).unwrap();
        let tight = Limits { max_alloc: 3 * n as u64 + (1 << 20), ..Limits::default() };
        let p = prepare(&file, &dir, &tight, true).unwrap();
        assert_eq!(prepared_warnings(&p), vec![DecodeWarning::ExtraSamplesDropped { format: F, count: 1 }]);
        assert!(prepared_warnings(&prepare(&file, &dir, &tight, false).unwrap()).is_empty(), "banded: only the band counts");
        assert!(prepared_warnings(&prepare(&file, &dir, &Limits::default(), true).unwrap()).is_empty());
        let whole = decode(&bytes, &tight).unwrap();
        assert_eq!(whole.warnings, prepared_warnings(&p));
    }

    /// The scratch bound a whole-image decode reserves: none for chunks decoded in place, one
    /// chunk per worker otherwise, never more chunks than the image has bands.
    #[test]
    fn whole_image_scratch_is_one_chunk_per_worker() {
        assert_eq!(scratch_bound(true, 1 << 20, 16, 1024, 8), 0, "strips decoded in place need no scratch");
        assert_eq!(scratch_bound(false, 1 << 20, 16, 1024, 8), 8 << 20, "one strip per worker");
        assert_eq!(scratch_bound(false, 1 << 20, 16, 32, 8), 2 << 20, "never more strips than the image has");
        assert_eq!(scratch_bound(false, 1 << 20, 0, 2, 8), 2 << 20, "a zero band height counts as one row");
        assert_eq!(scratch_bound(false, u64::MAX, 1, 1, 8), u64::MAX, "the bound saturates");
    }

    /// A JPEG stream with SOI, an APP0 segment, stray bytes, a fill run and a baseline SOF of
    /// `w`x`h` with `components` components, then a scan.
    fn jpeg_with_sof(w: u16, h: u16, components: u8) -> Vec<u8> {
        let mut s = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0xAA, 0xBB, 0x00, 0x00, 0xFF, 0xFF, 0xC0, 0x00, 0x0B, 8];
        s.extend_from_slice(&h.to_be_bytes());
        s.extend_from_slice(&w.to_be_bytes());
        s.extend_from_slice(&[components, 1, 0x11, 0, 0xFF, 0xDA, 0x00, 0x02]);
        s
    }

    /// [`jpeg_sof`] with no step limit.
    fn sof(s: &[u8]) -> Option<(u64, u64, u64)> {
        let mut steps = u64::MAX;
        jpeg_sof(s, &mut steps).unwrap()
    }

    /// The SOF walk finds the frame size past other segments and stray bytes, skips DHT, and
    /// gives up at the scan, at the end or on a malformed length, never reading out of bounds.
    #[test]
    fn jpeg_sof_reads_the_frame_header() {
        assert_eq!(sof(&jpeg_with_sof(640, 480, 3)), Some((640, 480, 3)));
        // Without SOI (a chunk after JPEGTables) and with a DHT (C4) segment first.
        let mut s = vec![0xFF, 0xC4, 0x00, 0x03, 0x00];
        s.extend_from_slice(&jpeg_with_sof(16, 8, 1)[2..]);
        assert_eq!(sof(&s), Some((16, 8, 1)));
        assert_eq!(sof(&[0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02]), None, "a scan before any frame header");
        assert_eq!(sof(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x01]), None, "a segment length below two");
        assert_eq!(sof(&[0xFF, 0xD8, 0xFF, 0xC0, 0x00]), None, "a cut-off frame header");
        assert_eq!(sof(&[]), None);
        let full = jpeg_with_sof(640, 480, 3);
        for cut in 0..full.len() {
            let _ = sof(&full[..cut]);
        }
    }

    /// A JPEG chunk is charged its compressed stream (with the shared tables), its decoded
    /// frame and the frame's coefficients, and a frame larger than its strip or tile is
    /// refused: zune-jpeg allocates the whole frame its SOF declares.
    #[test]
    fn jpeg_chunks_are_charged_their_frame_and_refused_past_their_chunk() {
        let small = jpeg_with_sof(64, 64, 3);
        let frame = 64 * 64 * 3;
        let coefficients = 64 * 64 * 3 * 2;
        let got = jpeg_working_bytes(None, [Some(small.as_slice()), None].into_iter(), (64, 64), u64::MAX).unwrap();
        assert_eq!(got, small.len() as u64 + frame + coefficients);
        let tables = [0xFF, 0xD8, 0xFF, 0xC4, 0x00, 0x03, 0x00, 0xFF, 0xD9];
        let with_tables = jpeg_working_bytes(Some(&tables), [Some(small.as_slice())].into_iter(), (64, 64), u64::MAX).unwrap();
        assert_eq!(with_tables, got + tables.len() as u64);
        // A 16384x16384 frame (zune-jpeg's default cap) in a 64x64 tile.
        let huge = jpeg_with_sof(16384, 16384, 3);
        let refused = jpeg_working_bytes(None, [Some(small.as_slice()), Some(huge.as_slice())].into_iter(), (64, 64), u64::MAX);
        assert!(matches!(refused, Err(CodecError::InvalidImage(_))), "{refused:?}");
        let no_frame = jpeg_working_bytes(None, [Some(&[0u8, 1, 2][..])].into_iter(), (64, 64), u64::MAX).unwrap();
        assert_eq!(no_frame, 3, "no frame header: only the copied stream");
    }

    /// The header walk takes one step per byte it steps over, never more than the stream has,
    /// and stops with an error when its budget runs out: strips or tiles that overlap (here
    /// many over one run of zeros) cannot make it read more than the file plus a few steps per
    /// chunk, while chunks that don't overlap fit in the file's length.
    #[test]
    fn jpeg_header_walk_is_bounded_by_its_step_budget() {
        // SOI is skipped; then a fill byte and APP0, two stray bytes, two fill bytes and SOF0.
        let s = jpeg_with_sof(640, 480, 3);
        let mut steps = 7;
        assert_eq!(jpeg_sof(&s, &mut steps).unwrap(), Some((640, 480, 3)));
        assert_eq!(steps, 0);
        let mut too_few = 6;
        assert!(matches!(jpeg_sof(&s, &mut too_few), Err(CodecError::InvalidImage(_))));
        let zeros = vec![0u8; 4096];
        let mut steps = 4096;
        assert_eq!(jpeg_sof(&zeros, &mut steps).unwrap(), None);
        assert_eq!(steps, 0, "one step per byte, none past the end");
        // 1000 chunks over the same 4096 bytes, budgeted as for a 4096-byte file: refused.
        let budget = 4096 + 1000 * JPEG_HEADER_STEPS_PER_CHUNK;
        let overlapping = jpeg_working_bytes(None, std::iter::repeat_n(Some(zeros.as_slice()), 1000), (64, 64), budget);
        assert!(matches!(overlapping, Err(CodecError::InvalidImage(_))), "{overlapping:?}");
        // 1000 separate chunks fit in the length of the file that holds them.
        let file_len = 1000 * s.len() as u64;
        let separate = jpeg_working_bytes(None, std::iter::repeat_n(Some(s.as_slice()), 1000), (640, 480), file_len);
        assert!(separate.is_ok(), "{separate:?}");
    }

    /// The floating-point predictor's row is charged at the declared strip or tile width, so a
    /// hostile TileWidth cannot ask for it unbudgeted, and a strip with samples the `tiff` crate
    /// does not return is charged its row too; other predictors hold no such row.
    #[test]
    fn the_floating_point_predictor_row_is_charged_at_the_declared_tile_width() {
        assert_eq!(predictor_row_bytes(3, 256, 3, 32), 256 * 3 * 4);
        assert_eq!(predictor_row_bytes(3, 0xFFFF_FFFF, 3, 64), 0xFFFF_FFFF * 3 * 8, "a 64x64 page with TileWidth 0xFFFFFFFF");
        assert_eq!(predictor_row_bytes(3, 1024, 4, 32), 1024 * 4 * 4, "a strip row, every sample the file holds");
        assert_eq!(predictor_row_bytes(2, 0xFFFF_FFFF, 3, 64), 0);
        assert_eq!(predictor_row_bytes(1, 0xFFFF_FFFF, 3, 64), 0);
        assert_eq!(predictor_row_bytes(3, u64::MAX, u64::MAX, 64), u64::MAX.div_ceil(8), "saturates");
    }
}
