//! Movie files (MP4, MOV, MKV, WebM) as footage: the first video track's frames, in presentation
//! order, read from the file in place and decoded one at a time.
//!
//! Containers come from `craft-media` (ISO BMFF and Matroska demuxers) and pictures from
//! `craft-video`'s `TrackDecoder` (H.264, HEVC, AV1, VP9, VP8, ProRes, DNxHD/HR, MPEG-2). Only the
//! file's index is read when it is opened; a frame costs the decode from the keyframe before it,
//! and playing forward continues the decode rather than starting again.
//!
//! Y'CbCr pictures become R'G'B' with craft-color's matrices, the chroma repeated across the
//! subsampled pixels, limited-range code values stretched to full range at the picture's bit
//! depth. Colour tags the bitstream leaves unspecified are taken from the container (`colr`,
//! Matroska `Colour`); the primaries and transfer travel with the image (`Image::cicp`), as for
//! AVIF and HEIF stills. A display rotation the container asks for (an MP4 `tkhd` matrix, a
//! Matroska projection roll) turns the picture, so a phone's portrait clip opens upright.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use craft_media::isobmff::{self, ColorInfo, Mp4File};
use craft_media::matroska::{self, MkvFile};
use craft_media::{ByteSource, CodecId, FileReader};
use craft_video::frame::{ColorFamily, ColorTags, Plane, Samples, VideoFrame};
use craft_video::{TrackDecoder, TrackDecoderOptions};

use crate::IoError;
use photocraft_codecs as codecs;

/// What a movie file holds, as it is displayed (after its rotation).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovieInfo {
    pub width: u32,
    pub height: u32,
    pub frames: usize,
    pub fps: f32,
}

pub use crate::{MOVIE_EXTENSIONS as EXTENSIONS, is_movie_name};

/// Largest picture a movie frame may have, as for still images.
const MAX_FRAME_PIXELS: u64 = 1 << 28;
/// Samples past the wanted one that a decode may already have been sent (B-frame reordering)
/// and still continue rather than start again at the keyframe.
const REORDER_SLACK: usize = 16;
/// Decoded pictures held for later frames.
const MAX_READY: usize = 32;

/// The colour description a container gives a track, H.273 code points; `None` where it says
/// nothing. Used only for what the bitstream itself leaves unspecified.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContainerColor {
    pub primaries: Option<u16>,
    pub transfer: Option<u16>,
    pub matrix: Option<u16>,
    pub full_range: Option<bool>,
}

impl ContainerColor {
    fn of_mp4(c: Option<&ColorInfo>) -> Self {
        match c {
            Some(ColorInfo::Nclx { primaries, transfer, matrix, full_range }) => {
                ContainerColor { primaries: Some(*primaries), transfer: Some(*transfer), matrix: Some(*matrix), full_range: Some(*full_range) }
            }
            // QuickTime's indices are the H.273 code points for every value it defines.
            Some(ColorInfo::Nclc { primaries, transfer, matrix }) => {
                ContainerColor { primaries: Some(*primaries), transfer: Some(*transfer), matrix: Some(*matrix), full_range: None }
            }
            _ => ContainerColor::default(),
        }
    }

    fn of_mkv(c: Option<&matroska::Color>) -> Self {
        let code = |v: Option<u64>| v.and_then(|v| u16::try_from(v).ok());
        match c {
            // Matroska Range: 1 broadcast (limited), 2 full, 0 unspecified, 3 defined by the codec.
            Some(c) => ContainerColor {
                primaries: code(c.primaries),
                transfer: code(c.transfer_characteristics),
                matrix: code(c.matrix_coefficients),
                full_range: match c.range {
                    Some(1) => Some(false),
                    Some(2) => Some(true),
                    _ => None,
                },
            },
            None => ContainerColor::default(),
        }
    }

    /// `tags` with each field the bitstream leaves unspecified (code point 2) taken from the
    /// container. The range is full when either says so: a bitstream that doesn't signal it reads
    /// as limited.
    pub fn fill(&self, tags: ColorTags) -> ColorTags {
        let pick = |own: u8, container: Option<u16>| match (own, container.and_then(|c| u8::try_from(c).ok())) {
            (2, Some(c)) => c,
            (own, _) => own,
        };
        ColorTags {
            primaries: pick(tags.primaries, self.primaries),
            transfer: pick(tags.transfer, self.transfer),
            matrix: pick(tags.matrix, self.matrix),
            full_range: tags.full_range || self.full_range == Some(true),
        }
    }
}

enum Container {
    Mp4(Box<Mp4File>),
    Mkv(Box<MkvFile>),
}

pub struct Movie {
    src: Arc<FileReader>,
    container: Container,
    track: usize,
    /// For each frame in presentation order, its sample index (decode order). Samples that show
    /// no picture (VP8's hidden frames) are decoded but have no frame number.
    order: Vec<usize>,
    /// For each sample (decode order), its presentation time in the track's units.
    pts: Vec<i64>,
    /// Samples that are keyframes (decode order), ascending.
    keys: Vec<usize>,
    info: MovieInfo,
    /// Clockwise quarter turns (0-3) the container asks for at display.
    turns: u8,
    color: ContainerColor,
    decoder: Option<TrackDecoder>,
    /// The next sample to send to `decoder`.
    next: usize,
    /// Decoded pictures not handed out yet, by presentation time.
    ready: BTreeMap<i64, VideoFrame>,
    /// The presentation time last handed out: a decode in progress has passed everything before it.
    last: Option<i64>,
    params: craft_media::CodecParams,
}

fn bad(name: &str, why: impl std::fmt::Display) -> IoError {
    IoError::Unsupported(format!("`{name}`: {why}"))
}

/// One sample of the video track, as the index lists it.
struct Indexed {
    pts: i64,
    key: bool,
}

/// What the container says about the video track.
struct Track {
    container: Container,
    index: usize,
    samples: Vec<Indexed>,
    /// Ticks per second of the sample times.
    timescale: f64,
    /// The container's frame duration in ticks, when it declares one.
    frame_ticks: Option<f64>,
    /// The coded picture size.
    size: (u32, u32),
    turns: Option<u8>,
    color: ContainerColor,
}

fn mkv_track(name: &str, src: &Arc<FileReader>) -> Result<Track, IoError> {
    let f = matroska::open(src.clone()).map_err(|e| bad(name, e))?;
    if !f.indexed {
        return Err(bad(name, "the file's frames could not all be found (a live or cut-off recording)"));
    }
    let index = f.track_of_kind(matroska::TrackKind::Video).ok_or_else(|| bad(name, "the file has no video track"))?;
    let tr = &f.tracks[index];
    let samples = tr.samples.iter().map(|s| Indexed { pts: s.pts, key: s.keyframe }).collect();
    let video = tr.video.as_ref();
    // Matroska times are in nanoseconds once scaled by the segment's TimestampScale.
    let scale = f.info.timestamp_scale.max(1) as f64;
    Ok(Track {
        samples,
        timescale: 1e9 / scale,
        frame_ticks: tr.default_duration_ns.map(|ns| ns as f64 / scale),
        size: video.map_or((0, 0), |v| (v.pixel_width, v.pixel_height)),
        turns: video.map_or(Some(0), matroska::VideoInfo::display_rotation),
        color: ContainerColor::of_mkv(video.and_then(|v| v.color.as_ref())),
        index,
        container: Container::Mkv(Box::new(f)),
    })
}

fn mp4_track(name: &str, src: &Arc<FileReader>) -> Result<Track, IoError> {
    let f = isobmff::open(src.clone()).map_err(|e| bad(name, e))?;
    let index = f.track_of_kind(isobmff::TrackKind::Video).ok_or_else(|| bad(name, "the file has no video track"))?;
    let tr = &f.tracks[index];
    let samples = tr.samples.iter().map(|s| Indexed { pts: s.pts, key: s.is_sync }).collect();
    let video = tr.video();
    Ok(Track {
        samples,
        timescale: f64::from(tr.timescale.max(1)),
        frame_ticks: None,
        // The sample entry holds the coded size; `tkhd`'s is the size after the matrix.
        size: video.map_or((tr.width, tr.height), |v| (u32::from(v.width), u32::from(v.height))),
        turns: tr.display_rotation(),
        color: ContainerColor::of_mp4(video.and_then(|v| v.color.as_ref())),
        index,
        container: Container::Mp4(Box::new(f)),
    })
}

/// Whether a VP8 frame is shown: the `show_frame` bit of its frame tag (RFC 6386 §9.1). A hidden
/// frame (an alternate reference) only updates the decoder's references.
fn vp8_shown(first: u8) -> bool {
    first & 0x10 != 0
}

impl Movie {
    /// Open the movie at `path`, reading only its index (and, for VP8, each frame's first byte).
    pub fn open(path: &Path) -> Result<Movie, IoError> {
        let name = path.display().to_string();
        let src = Arc::new(FileReader::open(path).map_err(|e| bad(&name, e))?);
        let mut head = [0u8; 4];
        let n = head.len().min(usize::try_from(src.len()).unwrap_or(usize::MAX));
        src.read_at(0, &mut head[..n]).map_err(|e| bad(&name, e))?;
        let track = if head == [0x1A, 0x45, 0xDF, 0xA3] { mkv_track(&name, &src)? } else { mp4_track(&name, &src)? };
        if track.samples.is_empty() {
            return Err(bad(&name, "the video track has no frames"));
        }
        let (w, h) = track.size;
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_FRAME_PIXELS {
            return Err(bad(&name, format!("a {w} × {h} picture is not one PhotoCraft can open")));
        }
        let params = match &track.container {
            Container::Mp4(f) => f.tracks[track.index].codec_params(),
            Container::Mkv(f) => f.tracks[track.index].codec_params(),
        };
        // Refuse a codec we can't decode now, not at the first frame.
        TrackDecoder::new(&params).map_err(|e| bad(&name, e))?;
        // A mirrored or skewed display matrix is shown as coded, and said so nowhere else: the
        // frames are still the right ones, only not turned.
        let turns = track.turns.unwrap_or(0);

        let pts: Vec<i64> = track.samples.iter().map(|s| s.pts).collect();
        let keys: Vec<usize> = track.samples.iter().enumerate().filter(|(_, s)| s.key).map(|(i, _)| i).collect();
        let mut movie = Movie {
            src,
            container: track.container,
            track: track.index,
            order: Vec::new(),
            pts,
            keys,
            info: MovieInfo { width: w, height: h, frames: 0, fps: 30.0 },
            turns,
            color: track.color,
            decoder: None,
            next: 0,
            ready: BTreeMap::new(),
            last: None,
            params,
        };
        let shown: Vec<usize> = if movie.params.codec == CodecId::Vp8 {
            let mut first = [0u8; 1];
            (0..movie.pts.len())
                .filter(|&i| {
                    // A frame whose first byte can't be read shows (and fails when decoded).
                    movie.read_prefix(i, &mut first).is_err() || vp8_shown(first[0])
                })
                .collect()
        } else {
            (0..movie.pts.len()).collect()
        };
        let mut order = shown;
        order.sort_by_key(|&i| (movie.pts[i], i));
        if order.is_empty() {
            return Err(bad(&name, "the video track shows no frames"));
        }
        // The rate from the container's frame duration, else the most common step between frames.
        let step = track.frame_ticks.unwrap_or_else(|| {
            let mut steps: Vec<i64> = order.windows(2).map(|p| movie.pts[p[1]] - movie.pts[p[0]]).filter(|d| *d > 0).collect();
            steps.sort_unstable();
            steps.get(steps.len() / 2).copied().unwrap_or(1) as f64
        });
        let fps = if step > 0.0 { (track.timescale / step) as f32 } else { 30.0 };
        let fps = if fps.is_finite() && fps > 0.0 && fps <= 1000.0 { fps } else { 30.0 };
        let (width, height) = if turns % 2 == 1 { (h, w) } else { (w, h) };
        movie.info = MovieInfo { width, height, frames: order.len(), fps };
        movie.order = order;
        Ok(movie)
    }

    pub fn info(&self) -> MovieInfo {
        self.info
    }

    /// The colour description the container gives the track.
    pub fn container_color(&self) -> ContainerColor {
        self.color
    }

    /// Frame `n` (presentation order) as a one-layer document, as a still image opens: turned as
    /// the container asks, coloured as the bitstream and the container describe it.
    pub fn frame_document(&mut self, n: usize) -> Result<photocraft_doc::Document, String> {
        let picture = self.picture(n)?;
        let image = picture_image(&picture, self.color, self.turns)?;
        crate::image_to_document(&format!("frame {n}"), &image).map(|r| r.document).map_err(|e| e.to_string())
    }

    fn read(&self, sample: usize) -> Result<Vec<u8>, String> {
        match &self.container {
            Container::Mp4(f) => f.read_sample(&*self.src, self.track, sample).map_err(|e| e.to_string()),
            Container::Mkv(f) => f.read_sample(&*self.src, self.track, sample).map_err(|e| e.to_string()),
        }
    }

    /// The first bytes of sample `i`, without reading the rest of it.
    fn read_prefix(&self, i: usize, buf: &mut [u8]) -> Result<(), String> {
        let (offset, size) = match &self.container {
            Container::Mp4(f) => f.tracks[self.track].samples.get(i).map(|s| (s.offset, u64::from(s.size))),
            // A Matroska frame whose stored bytes omit a header-stripped prefix starts elsewhere:
            // read it whole then.
            Container::Mkv(f) if f.tracks[self.track].frame_prefix_len() == 0 => f.tracks[self.track].samples.get(i).map(|s| (s.offset, u64::from(s.size))),
            Container::Mkv(_) => {
                let data = self.read(i)?;
                let n = buf.len().min(data.len());
                buf[..n].copy_from_slice(&data[..n]);
                return if n == buf.len() { Ok(()) } else { Err("the frame is shorter than its header".into()) };
            }
        }
        .ok_or_else(|| format!("the movie has no sample {i}"))?;
        if size < buf.len() as u64 {
            return Err("the frame is shorter than its header".into());
        }
        self.src.read_at(offset, buf).map_err(|e| e.to_string())
    }

    /// Frame `n` (presentation order) as decoded.
    pub fn picture(&mut self, n: usize) -> Result<VideoFrame, String> {
        let &sample = self.order.get(n).ok_or_else(|| format!("the movie has no frame {n}"))?;
        let want = self.pts[sample];
        if let Some(f) = self.take(want) {
            return Ok(f);
        }
        let key = match self.keys.partition_point(|&k| k <= sample) {
            0 => 0,
            i => self.keys[i - 1],
        };
        // Continue the current decode when it is between that keyframe and the frame; otherwise
        // start again at the keyframe.
        let behind = self.last.is_some_and(|l| want <= l);
        if self.decoder.is_none() || behind || self.next < key || self.next > sample + REORDER_SLACK {
            let decoder = TrackDecoder::with_options(&self.params, TrackDecoderOptions::default()).map_err(|e| e.to_string())?;
            self.decoder = Some(decoder);
            self.next = key;
            self.ready.clear();
        }
        while self.next < self.pts.len() {
            let data = self.read(self.next)?;
            let pts = self.pts[self.next];
            self.next += 1;
            let decoded = self.decoder.as_mut().map(|d| d.decode(&data, pts)).transpose().map_err(|e| e.to_string())?.unwrap_or_default();
            self.keep(decoded, want);
            if let Some(f) = self.take(want) {
                return Ok(f);
            }
            // Pictures come out in presentation order, however late a threaded decoder hands them
            // over: once a later one is out and the reordering depth is passed, this one won't be.
            if self.next > sample.saturating_add(REORDER_SLACK) && !self.ready.is_empty() {
                return Err(format!("frame {n} could not be decoded"));
            }
        }
        let rest = self.decoder.as_mut().map(TrackDecoder::flush).transpose().map_err(|e| e.to_string())?.unwrap_or_default();
        self.keep(rest, want);
        self.take(want).ok_or_else(|| format!("frame {n} could not be decoded"))
    }

    /// Hold decoded pictures at or after `want`; earlier ones were for frames already passed.
    fn keep(&mut self, frames: Vec<VideoFrame>, want: i64) {
        for f in frames {
            if let Some(p) = f.pts
                && p >= want
            {
                self.ready.insert(p, f);
            }
        }
        // A stream that never shows the wanted time must not grow this without bound.
        while self.ready.len() > MAX_READY {
            self.ready.pop_last();
        }
    }

    fn take(&mut self, want: i64) -> Option<VideoFrame> {
        let f = self.ready.remove(&want)?;
        self.ready.retain(|p, _| *p > want);
        self.last = Some(want);
        Some(f)
    }
}

/// A plane's code value at (x, y).
fn code(p: &Plane, x: usize, y: usize) -> f32 {
    let i = y * p.stride() + x;
    match p.samples() {
        Samples::U8(v) => f32::from(v.get(i).copied().unwrap_or(0)),
        Samples::U16(v) => f32::from(v.get(i).copied().unwrap_or(0)),
    }
}

/// How a plane's code values map to 0..1 (luma, R'G'B', alpha) or -0.5..0.5 (chroma), at its
/// own bit depth (ITU-T H.273 §8.3): limited range puts black at 16 and white at 235 (chroma
/// 16..240 around 128), scaled by 2^(depth - 8); full range spans 0..2^depth - 1.
#[derive(Clone, Copy)]
struct Levels {
    offset: f32,
    span: f32,
}

impl Levels {
    fn luma(depth: u8, full: bool) -> Self {
        let max = ((1u32 << depth) - 1) as f32;
        if full { Levels { offset: 0.0, span: max } } else { Levels { offset: 16.0 * scale(depth), span: 219.0 * scale(depth) } }
    }

    fn chroma(depth: u8, full: bool) -> Self {
        let max = ((1u32 << depth) - 1) as f32;
        if full { Levels { offset: (1u32 << (depth - 1)) as f32, span: max } } else { Levels { offset: 128.0 * scale(depth), span: 224.0 * scale(depth) } }
    }

    fn of(self, code: f32) -> f32 {
        (code - self.offset) / self.span
    }
}

/// 2^(depth - 8), the limited-range code values' scale at `depth` bits.
fn scale(depth: u8) -> f32 {
    2f32.powi(i32::from(depth) - 8)
}

/// A decoded picture as an RGB(A) image, 8-bit for 8-bit pictures and 16-bit for deeper ones,
/// turned `turns` quarter turns clockwise. Colour tags the picture leaves unspecified come from
/// `container`.
pub fn picture_image(f: &VideoFrame, container: ContainerColor, turns: u8) -> Result<codecs::Image, String> {
    let (w, h) = (f.width() as usize, f.height() as usize);
    let planes = f.planes();
    let tags = container.fill(f.color);
    let rgb_family = matches!(f.family(), ColorFamily::Rgb);
    let full = tags.full_range || rgb_family;
    let matrix = match tags.matrix {
        1 => craft_color::Matrix::Bt709,
        9 | 10 => craft_color::Matrix::Bt2020Ncl,
        5 | 6 => craft_color::Matrix::Bt601,
        // Unspecified: HD and larger pictures are BT.709, smaller ones BT.601.
        _ if w >= 1280 || h >= 720 => craft_color::Matrix::Bt709,
        _ => craft_color::Matrix::Bt601,
    };
    let depth_of = |i: usize| planes.get(i).map_or(8, Plane::bit_depth);
    let (y_levels, c_levels) = (Levels::luma(depth_of(0), full), Levels::chroma(depth_of(1).max(1), full));
    let alpha = f.alpha();
    let a_levels = alpha.map(|a| Levels::luma(a.bit_depth(), true));
    let (cw, ch) = f.subsampling().chroma_size(f.width(), f.height());
    let (sx, sy) = ((w as u32).div_ceil(cw.max(1)) as usize, (h as u32).div_ceil(ch.max(1)) as usize);
    let turns = turns % 4;
    let (ow, oh) = if turns % 2 == 1 { (h, w) } else { (w, h) };
    // Where source pixel (x, y) lands once turned clockwise.
    let land = |x: usize, y: usize| match turns {
        1 => (h - 1 - y, x),
        2 => (w - 1 - x, h - 1 - y),
        3 => (y, w - 1 - x),
        _ => (x, y),
    };
    let chans = if alpha.is_some() { 4 } else { 3 };
    let mut out = vec![0f32; w * h * chans];
    for y in 0..h {
        for x in 0..w {
            let rgb = match f.family() {
                ColorFamily::Rgb => [0, 1, 2].map(|i| y_levels.of(code(&planes[i], x, y))),
                ColorFamily::Gray => [y_levels.of(code(&planes[0], x, y)); 3],
                ColorFamily::YCbCr => {
                    let luma = y_levels.of(code(&planes[0], x, y));
                    let (cb, cr) = (c_levels.of(code(&planes[1], x / sx, y / sy)), c_levels.of(code(&planes[2], x / sx, y / sy)));
                    craft_color::ycbcr_to_rgb(luma, cb, cr, matrix)
                }
            };
            let (tx, ty) = land(x, y);
            let o = (ty * ow + tx) * chans;
            for c in 0..3 {
                out[o + c] = rgb[c].clamp(0.0, 1.0);
            }
            if let (Some(a), Some(levels)) = (alpha, a_levels) {
                out[o + 3] = levels.of(code(a, x, y)).clamp(0.0, 1.0);
            }
        }
    }
    let layout = if alpha.is_some() { codecs::ChannelLayout::Rgba } else { codecs::ChannelLayout::Rgb };
    let st = if f.bit_depth() > 8 { codecs::SampleType::U16 } else { codecs::SampleType::U8 };
    let mut img = codecs::Image::from_normalized(ow as u32, oh as u32, layout, st, &out).map_err(|e| e.to_string())?;
    img.cicp = Some(codecs::Cicp { primaries: u16::from(tags.primaries), transfer: u16::from(tags.transfer), matrix: 0, full_range: true });
    Ok(img)
}

/// Movie files made from synthetic frames, for tests in this and other crates.
#[cfg(any(test, feature = "testgen"))]
pub mod testgen {
    use std::io::Cursor;

    use craft_media::isobmff::{AvcConfig, Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
    use craft_media::matroska::{MkvWriter, MuxOptions, TrackKind, TrackSpec};
    use craft_video::h264::{Encoder, EncoderConfig, PacketFormat, Preset, RateControl, YuvFrame};

    /// The container a synthetic movie is written in.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Container {
        Mp4,
        Mkv,
    }

    /// The grey level (0..1, full range) frame `n` of a synthetic movie shows: distinct for every
    /// frame, so a decoded frame says which one it is.
    pub fn level(n: usize) -> f32 {
        ((n * 37) % 200 + 20) as f32 / 255.0
    }

    /// A synthetic H.264 movie's settings.
    #[derive(Clone, Debug)]
    pub struct Synth {
        pub container: Container,
        pub width: u32,
        pub height: u32,
        pub frames: usize,
        pub fps: u32,
        /// Frame `n` shows `level(n + first)`.
        pub first: usize,
        /// Black left half and white right half instead of a flat grey, to tell orientations apart.
        pub halves: bool,
        /// A Matroska `Colour` element to write (the bitstream signals no colour of its own).
        pub mkv_color: Option<craft_media::matroska::Color>,
    }

    impl Synth {
        pub fn new(container: Container, width: u32, height: u32, frames: usize, fps: u32) -> Self {
            Synth { container, width, height, frames, fps, first: 0, halves: false, mkv_color: None }
        }
    }

    /// A movie of `frames` flat grey H.264 frames (`level(n)`, limited range), `w` × `h` at `fps`,
    /// with B-frames and a keyframe every 8 frames, so frames come out of the decoder in a
    /// different order than they are stored.
    pub fn h264(container: Container, w: u32, h: u32, frames: usize, fps: u32) -> Result<Vec<u8>, String> {
        h264_with(&Synth::new(container, w, h, frames, fps))
    }

    /// [`h264`] whose frame `n` shows `level(n + first)`.
    pub fn h264_from(container: Container, w: u32, h: u32, frames: usize, fps: u32, first: usize) -> Result<Vec<u8>, String> {
        h264_with(&Synth { first, ..Synth::new(container, w, h, frames, fps) })
    }

    /// The movie `s` describes.
    pub fn h264_with(s: &Synth) -> Result<Vec<u8>, String> {
        let err = |x: &dyn std::fmt::Display| x.to_string();
        let mut cfg = EncoderConfig::new(s.width, s.height, s.fps, 1);
        cfg.preset = Preset::Speed;
        cfg.rate = RateControl::Qp(12);
        cfg.keyint = 8;
        cfg.bframes = 2;
        cfg.threads = 1;
        cfg.format = PacketFormat::LengthPrefixed;
        let mut enc = Encoder::new(cfg).map_err(|x| err(&x))?;
        let (wu, hu) = (s.width as usize, s.height as usize);
        let mut packets = Vec::new();
        for n in 0..s.frames {
            // Full-range level to limited-range luma code value.
            let grey = (16.0 + level(n + s.first) * 219.0).round() as u8;
            let y: Vec<u8> = (0..wu * hu)
                .map(|i| {
                    if !s.halves {
                        grey
                    } else if i % wu < wu / 2 {
                        16
                    } else {
                        235
                    }
                })
                .collect();
            let c = vec![128u8; wu.div_ceil(2) * hu.div_ceil(2)];
            let f = YuvFrame { y: &y, u: &c, v: &c, y_stride: wu, uv_stride: wu.div_ceil(2) };
            packets.extend(enc.encode(&f, n as i64).map_err(|x| err(&x))?);
        }
        packets.extend(enc.flush());
        let (sps, pps) = enc.sps_pps();
        match s.container {
            Container::Mp4 => {
                let entry = SampleEntry::avc(AvcConfig::new(vec![sps], vec![pps], 4), s.width as u16, s.height as u16);
                let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).map_err(|x| err(&x))?;
                let t = mux.add_track(TrackConfig::new(entry, s.fps)).map_err(|x| err(&x))?;
                for p in &packets {
                    let sample = WriteSample { data: &p.data, duration: 1, composition_offset: (p.pts - p.dts) as i32, is_sync: p.keyframe };
                    mux.write_sample(t, sample).map_err(|x| err(&x))?;
                }
                Ok(mux.finish().map_err(|x| err(&x))?.into_inner())
            }
            Container::Mkv => {
                let mut spec = TrackSpec::new(TrackKind::Video, "V_MPEG4/ISO/AVC");
                spec.codec_private = enc.avcc();
                spec.video_size = Some((s.width, s.height));
                spec.color = s.mkv_color.clone();
                let ns = 1_000_000_000 / u64::from(s.fps);
                spec.default_duration_ns = Some(ns);
                let mut mux = MkvWriter::new(Cursor::new(Vec::new()), vec![spec], MuxOptions::default()).map_err(|x| err(&x))?;
                for p in &packets {
                    mux.write_frame(0, p.pts * ns as i64, p.keyframe, &p.data, Some(ns)).map_err(|x| err(&x))?;
                }
                Ok(mux.finish().map_err(|x| err(&x))?.into_inner())
            }
        }
    }

    /// `mp4` with its video track's `tkhd` matrix turning it `turns` quarter turns clockwise, as a
    /// phone writes a portrait clip.
    pub fn turned_mp4(mut mp4: Vec<u8>, turns: u8) -> Result<Vec<u8>, String> {
        const ONE: i32 = 0x0001_0000;
        let (a, b, c, d) = match turns % 4 {
            1 => (0, ONE, -ONE, 0),
            2 => (-ONE, 0, 0, -ONE),
            3 => (0, -ONE, ONE, 0),
            _ => (ONE, 0, 0, ONE),
        };
        let at = mp4.windows(4).position(|w| w == b"tkhd").ok_or("no tkhd box")?;
        // Version 0 `tkhd`: the matrix follows the type at 4 + 20 + 8 + 8 bytes.
        let matrix = at + 4 + 4 + 20 + 8 + 8;
        for (i, v) in [a, b, 0, c, d, 0, 0, 0, 0x4000_0000].into_iter().enumerate() {
            mp4.get_mut(matrix + i * 4..matrix + i * 4 + 4).ok_or("tkhd cut short")?.copy_from_slice(&v.to_be_bytes());
        }
        Ok(mp4)
    }

    /// A WebM of three VP8 keyframes in which the middle one is hidden (`show_frame` 0, as an
    /// alternate reference is): two frames are shown, `w` × `h`, the first black, the last white.
    pub fn vp8_with_hidden(w: u32, h: u32) -> Result<Vec<u8>, String> {
        let err = |x: &dyn std::fmt::Display| x.to_string();
        let frame = |v: u8| -> Result<Vec<u8>, String> {
            let rgba = [v, v, v, 255].repeat((w * h) as usize);
            craft_video::vp8::encode_rgba(&rgba, w, h, &craft_video::vp8::Vp8Params::default()).map(|f| f.data).map_err(|x| err(&x))
        };
        let mut hidden = frame(128)?;
        if let Some(b) = hidden.first_mut() {
            *b &= !0x10;
        }
        let mut spec = TrackSpec::new(TrackKind::Video, "V_VP8");
        spec.video_size = Some((w, h));
        let ms = 40_000_000i64;
        let mut mux =
            MkvWriter::new(Cursor::new(Vec::new()), vec![spec], MuxOptions { doc_type: "webm".into(), ..MuxOptions::default() }).map_err(|x| err(&x))?;
        for (i, data) in [frame(0)?, hidden, frame(255)?].iter().enumerate() {
            mux.write_frame(0, i as i64 * ms, true, data, Some(ms as u64)).map_err(|x| err(&x))?;
        }
        Ok(mux.finish().map_err(|x| err(&x))?.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::testgen::{Container, Synth, h264, h264_with, level, turned_mp4, vp8_with_hidden};
    use super::*;

    fn temp(name: &str, ext: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("pc-movie-{}-{name}.{ext}", std::process::id()));
        std::fs::write(&p, bytes).expect("write movie");
        p
    }

    fn written(name: &str, container: Container, frames: usize) -> std::path::PathBuf {
        let ext = if container == Container::Mp4 { "mp4" } else { "mkv" };
        temp(name, ext, &h264(container, 48, 32, frames, 25).expect("movie"))
    }

    /// The colour at (x, y) of frame `n`, decoded.
    fn pixel(m: &mut Movie, n: usize, x: i32, y: i32) -> Vec<f32> {
        let doc = m.frame_document(n).expect("frame");
        let s = photocraft_compose::flatten_to_surface(&doc, doc.pixel_format(), None);
        s.pixel(x, y)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 4.0 / 255.0
    }

    #[test]
    fn frames_come_back_in_any_order_from_both_containers() {
        for container in [Container::Mp4, Container::Mkv] {
            let path = written(&format!("{container:?}-order"), container, 30);
            let mut m = Movie::open(&path).expect("open");
            let info = m.info();
            assert_eq!((info.width, info.height, info.frames), (48, 32, 30), "{container:?}");
            assert!((info.fps - 25.0).abs() < 0.01, "{container:?}: {} fps", info.fps);
            // Forward, backward across keyframes, within a group of pictures, and to the end.
            for n in [0, 1, 2, 3, 9, 5, 4, 29, 17, 0, 16] {
                assert!(close(pixel(&mut m, n, 24, 16)[0], level(n)), "{container:?}: frame {n} decoded as another frame");
            }
            let _ = std::fs::remove_file(&path);
        }
    }

    #[test]
    fn playing_forward_continues_the_decode() {
        let path = written("forward", Container::Mp4, 24);
        let mut m = Movie::open(&path).expect("open");
        for n in 0..24 {
            assert!(close(pixel(&mut m, n, 24, 16)[0], level(n)), "frame {n}");
        }
        // Past the last frame is an error, not a panic or a stale frame.
        assert!(m.picture(24).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn files_that_are_not_movies_are_refused_with_the_reason() {
        let p = temp("junk", "mp4", b"not a movie at all, just some bytes");
        let err = Movie::open(&p).err().expect("junk is refused").to_string();
        assert!(err.contains("pc-movie"), "the error names the file: {err}");
        let _ = std::fs::remove_file(&p);
        assert!(Movie::open(Path::new("Z:/no/such/movie.mp4")).is_err());
        // A cut-off file: the index is gone, so it is refused rather than half read.
        let whole = h264(Container::Mp4, 48, 32, 10, 25).expect("movie");
        let p = temp("cut", "mp4", &whole[..whole.len() / 3]);
        if let Ok(mut m) = Movie::open(&p) {
            // Some cuts keep the index (it may come first): reading must then fail cleanly.
            for n in 0..m.info().frames {
                let _ = m.picture(n);
            }
        }
        let _ = std::fs::remove_file(&p);
    }

    /// A phone's portrait clip (an MP4 whose `tkhd` turns it a quarter clockwise) opens upright:
    /// its width and height swap, and what was the left of the coded picture is at the top.
    #[test]
    fn a_turned_mp4_opens_upright() {
        let coded = h264_with(&Synth { halves: true, ..Synth::new(Container::Mp4, 48, 32, 2, 25) }).expect("movie");
        let p = temp("turned", "mp4", &turned_mp4(coded, 1).expect("turned"));
        let mut m = Movie::open(&p).expect("open");
        assert_eq!((m.info().width, m.info().height), (32, 48));
        let top = pixel(&mut m, 0, 16, 4)[0];
        let bottom = pixel(&mut m, 0, 16, 43)[0];
        assert!(top < 0.1 && bottom > 0.9, "the coded left half (black) is at the top: top {top}, bottom {bottom}");
        let _ = std::fs::remove_file(&p);
    }

    /// A hidden VP8 frame (an alternate reference) is decoded but has no frame number: the clip
    /// has two frames, and both decode.
    #[test]
    fn hidden_vp8_frames_have_no_frame_number() {
        let p = temp("vp8-hidden", "webm", &vp8_with_hidden(32, 16).expect("webm"));
        let mut m = Movie::open(&p).expect("open");
        assert_eq!(m.info().frames, 2);
        assert!(pixel(&mut m, 0, 8, 8)[0] < 0.1, "frame 0 is the black one");
        assert!(pixel(&mut m, 1, 8, 8)[0] > 0.9, "frame 1 is the white one, after the hidden grey");
        let _ = std::fs::remove_file(&p);
    }

    /// Colour the bitstream leaves unspecified comes from the container's `Colour` element; colour
    /// the bitstream gives is kept.
    #[test]
    fn container_colour_fills_what_the_bitstream_leaves_out() {
        let color = craft_media::matroska::Color {
            primaries: Some(9),
            transfer_characteristics: Some(16),
            matrix_coefficients: Some(9),
            range: Some(2),
            ..Default::default()
        };
        let bytes = h264_with(&Synth { mkv_color: Some(color), ..Synth::new(Container::Mkv, 48, 32, 2, 25) }).expect("movie");
        let p = temp("tagged", "mkv", &bytes);
        let m = Movie::open(&p).expect("open");
        let c = m.container_color();
        assert_eq!((c.primaries, c.transfer, c.matrix, c.full_range), (Some(9), Some(16), Some(9), Some(true)));
        let filled = c.fill(ColorTags::UNSPECIFIED);
        assert_eq!((filled.primaries, filled.transfer, filled.matrix, filled.full_range), (9, 16, 9, true));
        let own = c.fill(ColorTags { primaries: 1, transfer: 1, matrix: 1, full_range: false });
        assert_eq!((own.primaries, own.transfer, own.matrix), (1, 1, 1), "what the bitstream says stays");
        let _ = std::fs::remove_file(&p);
    }

    /// 10-bit limited-range white and neutral chroma become exactly white, as 8-bit does: the
    /// offsets and spans scale with the bit depth.
    #[test]
    fn deep_limited_range_pictures_keep_their_white_and_grey() {
        let plane = |v: u16, w: u32, h: u32| Plane::packed(w, h, 10, Samples::U16(vec![v; (w * h) as usize])).expect("plane");
        let mut f =
            VideoFrame::new(4, 4, ColorFamily::YCbCr, craft_video::frame::Subsampling::S420, vec![plane(940, 4, 4), plane(512, 2, 2), plane(512, 2, 2)], None)
                .expect("frame");
        f.color = ColorTags { primaries: 1, transfer: 1, matrix: 1, full_range: false };
        let img = picture_image(&f, ContainerColor::default(), 0).expect("image");
        let rgba = img.to_rgba16();
        assert!(rgba.chunks(4).all(|p| p[..3].iter().all(|&c| c >= 65_500)), "white stays white: {:?}", &rgba[..4]);
        let plane8 = |v: u8, w: u32, h: u32| Plane::packed(w, h, 8, Samples::U8(vec![v; (w * h) as usize])).expect("plane");
        let mut g = VideoFrame::new(
            4,
            4,
            ColorFamily::YCbCr,
            craft_video::frame::Subsampling::S420,
            vec![plane8(126, 4, 4), plane8(128, 2, 2), plane8(128, 2, 2)],
            None,
        )
        .expect("frame");
        g.color = f.color;
        let mut deep =
            VideoFrame::new(4, 4, ColorFamily::YCbCr, craft_video::frame::Subsampling::S420, vec![plane(504, 4, 4), plane(512, 2, 2), plane(512, 2, 2)], None)
                .expect("frame");
        deep.color = f.color;
        let (a, b) = (
            picture_image(&g, ContainerColor::default(), 0).expect("8-bit").to_rgba16(),
            picture_image(&deep, ContainerColor::default(), 0).expect("10-bit").to_rgba16(),
        );
        assert!(a.iter().zip(&b).all(|(x, y)| x.abs_diff(*y) <= 300), "the same grey at 8 and 10 bits: {:?} vs {:?}", &a[..4], &b[..4]);
    }
}
