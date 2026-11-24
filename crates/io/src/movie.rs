//! Movie files (MP4, MOV, MKV, WebM) as footage: the first video track's frames, in presentation
//! order, read from the file in place and decoded one at a time.
//!
//! Containers come from `craft-media` (ISO BMFF and Matroska demuxers) and pictures from
//! `craft-video`'s `TrackDecoder` (H.264, HEVC, AV1, VP9, VP8, ProRes, DNxHD/HR, MPEG-2). Only the
//! file's index is read when it is opened; a frame costs the decode from the keyframe before it,
//! and playing forward continues the decode rather than starting again.
//!
//! Y'CbCr pictures become R'G'B' with craft-color's matrices, the chroma repeated across the
//! subsampled pixels, limited-range code values stretched to full range. The picture's colour
//! primaries and transfer travel with the image (`Image::cicp`), as for AVIF and HEIF stills.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use craft_media::isobmff::{self, Mp4File};
use craft_media::matroska::{self, MkvFile};
use craft_media::{ByteSource, FileReader};
use craft_video::frame::{ColorFamily, Samples, VideoFrame};
use craft_video::{TrackDecoder, TrackDecoderOptions};

use crate::IoError;
use photocraft_codecs as codecs;

/// What a movie file holds.
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

enum Container {
    Mp4(Box<Mp4File>),
    Mkv(Box<MkvFile>),
}

pub struct Movie {
    src: Arc<FileReader>,
    container: Container,
    track: usize,
    /// For each frame in presentation order, its sample index (decode order).
    order: Vec<usize>,
    /// For each sample (decode order), its presentation time in the track's units.
    pts: Vec<i64>,
    /// Samples that are keyframes (decode order), ascending.
    keys: Vec<usize>,
    info: MovieInfo,
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

impl Movie {
    /// Open the movie at `path`, reading only its index.
    pub fn open(path: &Path) -> Result<Movie, IoError> {
        let name = path.display().to_string();
        let src = Arc::new(FileReader::open(path).map_err(|e| bad(&name, e))?);
        let mut head = [0u8; 12];
        let n = head.len().min(usize::try_from(src.len()).unwrap_or(usize::MAX));
        src.read_at(0, &mut head[..n]).map_err(|e| bad(&name, e))?;
        let ebml = head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]);
        let (container, track, samples, timescale, default_duration, width, height) = if ebml {
            let f = matroska::open(src.clone()).map_err(|e| bad(&name, e))?;
            if !f.indexed {
                return Err(bad(&name, "the file's frames could not all be found (a live or cut-off recording)"));
            }
            let t = f.track_of_kind(matroska::TrackKind::Video).ok_or_else(|| bad(&name, "the file has no video track"))?;
            let tr = &f.tracks[t];
            let samples: Vec<(i64, bool)> = tr.samples.iter().map(|s| (s.pts, s.keyframe)).collect();
            let (w, h) = tr.video.as_ref().map_or((0, 0), |v| (v.pixel_width, v.pixel_height));
            // Matroska times are in nanoseconds once scaled by the segment's TimestampScale.
            let scale = f.info.timestamp_scale.max(1);
            let dd = tr.default_duration_ns.map(|ns| ns as f64 / scale as f64);
            (Container::Mkv(Box::new(f)), t, samples, 1e9 / scale as f64, dd, w, h)
        } else {
            let f = isobmff::open(src.clone()).map_err(|e| bad(&name, e))?;
            let t = f.track_of_kind(isobmff::TrackKind::Video).ok_or_else(|| bad(&name, "the file has no video track"))?;
            let tr = &f.tracks[t];
            let samples: Vec<(i64, bool)> = tr.samples.iter().map(|s| (s.pts, s.is_sync)).collect();
            let (timescale, w, h) = (f64::from(tr.timescale.max(1)), tr.width, tr.height);
            (Container::Mp4(Box::new(f)), t, samples, timescale, None, w, h)
        };
        if samples.is_empty() {
            return Err(bad(&name, "the video track has no frames"));
        }
        let pts: Vec<i64> = samples.iter().map(|s| s.0).collect();
        let keys: Vec<usize> = samples.iter().enumerate().filter(|(_, s)| s.1).map(|(i, _)| i).collect();
        let mut order: Vec<usize> = (0..pts.len()).collect();
        order.sort_by_key(|&i| (pts[i], i));
        // The rate from the most common step between presentation times.
        let step = default_duration.unwrap_or_else(|| {
            let mut steps: Vec<i64> = order.windows(2).map(|w| pts[w[1]] - pts[w[0]]).filter(|d| *d > 0).collect();
            steps.sort_unstable();
            steps.get(steps.len() / 2).copied().unwrap_or(1) as f64
        });
        let fps = if step > 0.0 { (timescale / step) as f32 } else { 30.0 };
        let fps = if fps.is_finite() && fps > 0.0 && fps <= 1000.0 { fps } else { 30.0 };
        let params = match &container {
            Container::Mp4(f) => f.tracks[track].codec_params(),
            Container::Mkv(f) => f.tracks[track].codec_params(),
        };
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_FRAME_PIXELS {
            return Err(bad(&name, format!("a {width} × {height} picture is not one PhotoCraft can open")));
        }
        // Refuse a codec we can't decode now, not at the first frame.
        TrackDecoder::new(&params).map_err(|e| bad(&name, e))?;
        let info = MovieInfo { width, height, frames: order.len(), fps };
        Ok(Movie { src, container, track, order, pts, keys, info, decoder: None, next: 0, ready: BTreeMap::new(), last: None, params })
    }

    pub fn info(&self) -> MovieInfo {
        self.info
    }

    /// Frame `n` (presentation order) as a one-layer document, as a still image opens.
    pub fn frame_document(&mut self, n: usize) -> Result<photocraft_doc::Document, String> {
        let picture = self.picture(n)?;
        let image = picture_image(&picture)?;
        crate::image_to_document(&format!("frame {n}"), &image).map(|r| r.document).map_err(|e| e.to_string())
    }

    fn read(&self, sample: usize) -> Result<Vec<u8>, String> {
        match &self.container {
            Container::Mp4(f) => f.read_sample(&*self.src, self.track, sample).map_err(|e| e.to_string()),
            Container::Mkv(f) => f.read_sample(&*self.src, self.track, sample).map_err(|e| e.to_string()),
        }
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

/// Samples past the wanted one that a decode may already have been sent (B-frame reordering)
/// and still continue rather than start again at the keyframe.
const REORDER_SLACK: usize = 16;
/// Decoded pictures held for later frames.
const MAX_READY: usize = 32;

/// A decoded picture as an RGB(A) image: 8-bit for 8-bit pictures, 16-bit for deeper ones.
pub fn picture_image(f: &VideoFrame) -> Result<codecs::Image, String> {
    let (w, h) = (f.width() as usize, f.height() as usize);
    let depth = f.bit_depth();
    let alpha = f.alpha();
    let max = |bits: u8| ((1u32 << bits) - 1) as f32;
    let sample = |p: &craft_video::frame::Plane, x: usize, y: usize| -> f32 {
        let i = y * p.stride() + x;
        match p.samples() {
            Samples::U8(v) => f32::from(v.get(i).copied().unwrap_or(0)) / max(p.bit_depth()),
            Samples::U16(v) => f32::from(v.get(i).copied().unwrap_or(0)) / max(p.bit_depth()),
        }
    };
    let planes = f.planes();
    let tags = f.color;
    let full = tags.full_range || matches!(f.family(), ColorFamily::Rgb);
    let matrix = match tags.matrix {
        1 => craft_color::Matrix::Bt709,
        9 | 10 => craft_color::Matrix::Bt2020Ncl,
        5 | 6 => craft_color::Matrix::Bt601,
        // Unspecified: HD and larger pictures are BT.709, smaller ones BT.601 (ITU-R BT.1700 era).
        _ if w >= 1280 || h >= 720 => craft_color::Matrix::Bt709,
        _ => craft_color::Matrix::Bt601,
    };
    let (cw, ch) = f.subsampling().chroma_size(f.width(), f.height());
    let (sx, sy) = ((w as u32).div_ceil(cw.max(1)) as usize, (h as u32).div_ceil(ch.max(1)) as usize);
    let chans = if alpha.is_some() { 4 } else { 3 };
    let mut out = vec![0f32; w * h * chans];
    for y in 0..h {
        for x in 0..w {
            let rgb = match f.family() {
                ColorFamily::Rgb => [sample(&planes[0], x, y), sample(&planes[1], x, y), sample(&planes[2], x, y)],
                ColorFamily::Gray => {
                    let v = sample(&planes[0], x, y);
                    let v = if full { v } else { ((v * 255.0 - 16.0) / 219.0).clamp(0.0, 1.0) };
                    [v, v, v]
                }
                ColorFamily::YCbCr => {
                    let (yv, cb, cr) = (sample(&planes[0], x, y), sample(&planes[1], x / sx, y / sy), sample(&planes[2], x / sx, y / sy));
                    // Limited range: Y in 16..235, Cb/Cr in 16..240 (scaled to the bit depth).
                    let (yv, cb, cr) =
                        if full { (yv, cb - 0.5, cr - 0.5) } else { ((yv * 255.0 - 16.0) / 219.0, (cb * 255.0 - 128.0) / 224.0, (cr * 255.0 - 128.0) / 224.0) };
                    craft_color::ycbcr_to_rgb(yv, cb, cr, matrix)
                }
            };
            let o = (y * w + x) * chans;
            for c in 0..3 {
                out[o + c] = rgb[c].clamp(0.0, 1.0);
            }
            if let Some(a) = alpha {
                out[o + 3] = sample(a, x, y);
            }
        }
    }
    let layout = if alpha.is_some() { codecs::ChannelLayout::Rgba } else { codecs::ChannelLayout::Rgb };
    let st = if depth > 8 { codecs::SampleType::U16 } else { codecs::SampleType::U8 };
    let mut img = codecs::Image::from_normalized(w as u32, h as u32, layout, st, &out).map_err(|e| e.to_string())?;
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

    /// A movie of `frames` flat grey H.264 frames (`level(n)`, limited range, BT.709), `w` × `h`
    /// at `fps`, with B-frames and a keyframe every 8 frames, so frames come out of the decoder
    /// in a different order than they are stored.
    pub fn h264(container: Container, w: u32, h: u32, frames: usize, fps: u32) -> Result<Vec<u8>, String> {
        h264_from(container, w, h, frames, fps, 0)
    }

    /// [`h264`] whose frame `n` shows `level(n + first)`.
    pub fn h264_from(container: Container, w: u32, h: u32, frames: usize, fps: u32, first: usize) -> Result<Vec<u8>, String> {
        let err = |x: &dyn std::fmt::Display| x.to_string();
        let mut cfg = EncoderConfig::new(w, h, fps, 1);
        cfg.preset = Preset::Speed;
        cfg.rate = RateControl::Qp(12);
        cfg.keyint = 8;
        cfg.bframes = 2;
        cfg.threads = 1;
        cfg.format = PacketFormat::LengthPrefixed;
        let mut enc = Encoder::new(cfg).map_err(|x| err(&x))?;
        let (wu, hu) = (w as usize, h as usize);
        let mut packets = Vec::new();
        for n in 0..frames {
            // Full-range level to limited-range luma code value.
            let y = vec![(16.0 + level(n + first) * 219.0).round() as u8; wu * hu];
            let c = vec![128u8; wu.div_ceil(2) * hu.div_ceil(2)];
            let f = YuvFrame { y: &y, u: &c, v: &c, y_stride: wu, uv_stride: wu.div_ceil(2) };
            packets.extend(enc.encode(&f, n as i64).map_err(|x| err(&x))?);
        }
        packets.extend(enc.flush());
        let (sps, pps) = enc.sps_pps();
        match container {
            Container::Mp4 => {
                let entry = SampleEntry::avc(AvcConfig::new(vec![sps], vec![pps], 4), w as u16, h as u16);
                let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).map_err(|x| err(&x))?;
                let t = mux.add_track(TrackConfig::new(entry, fps)).map_err(|x| err(&x))?;
                for p in &packets {
                    let s = WriteSample { data: &p.data, duration: 1, composition_offset: (p.pts - p.dts) as i32, is_sync: p.keyframe };
                    mux.write_sample(t, s).map_err(|x| err(&x))?;
                }
                Ok(mux.finish().map_err(|x| err(&x))?.into_inner())
            }
            Container::Mkv => {
                let mut spec = TrackSpec::new(TrackKind::Video, "V_MPEG4/ISO/AVC");
                spec.codec_private = enc.avcc();
                spec.video_size = Some((w, h));
                let ns = 1_000_000_000 / u64::from(fps);
                spec.default_duration_ns = Some(ns);
                let mut mux = MkvWriter::new(Cursor::new(Vec::new()), vec![spec], MuxOptions::default()).map_err(|x| err(&x))?;
                for p in &packets {
                    mux.write_frame(0, p.pts * ns as i64, p.keyframe, &p.data, Some(ns)).map_err(|x| err(&x))?;
                }
                Ok(mux.finish().map_err(|x| err(&x))?.into_inner())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testgen::{Container, h264, level};
    use super::*;

    fn written(name: &str, container: Container, frames: usize) -> std::path::PathBuf {
        let ext = if container == Container::Mp4 { "mp4" } else { "mkv" };
        let p = std::env::temp_dir().join(format!("pc-movie-{}-{name}.{ext}", std::process::id()));
        std::fs::write(&p, h264(container, 48, 32, frames, 25).expect("movie")).expect("write movie");
        p
    }

    /// The grey value at the middle of frame `n`, decoded.
    fn grey(m: &mut Movie, n: usize) -> f32 {
        let doc = m.frame_document(n).expect("frame");
        let s = photocraft_compose::flatten_to_surface(&doc, doc.pixel_format(), None);
        s.pixel(24, 16)[0]
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
                assert!(close(grey(&mut m, n), level(n)), "{container:?}: frame {n} decoded as another frame");
            }
            let _ = std::fs::remove_file(&path);
        }
    }

    #[test]
    fn playing_forward_continues_the_decode() {
        let path = written("forward", Container::Mp4, 24);
        let mut m = Movie::open(&path).expect("open");
        for n in 0..24 {
            assert!(close(grey(&mut m, n), level(n)), "frame {n}");
        }
        // Past the last frame is an error, not a panic or a stale frame.
        assert!(m.picture(24).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn files_that_are_not_movies_are_refused_with_the_reason() {
        let p = std::env::temp_dir().join(format!("pc-movie-{}-junk.mp4", std::process::id()));
        std::fs::write(&p, b"not a movie at all, just some bytes").expect("write");
        let err = Movie::open(&p).err().expect("junk is refused").to_string();
        assert!(err.contains("pc-movie"), "the error names the file: {err}");
        let _ = std::fs::remove_file(&p);
        assert!(Movie::open(Path::new("Z:/no/such/movie.mp4")).is_err());
        // A cut-off file: the index is gone, so it is refused rather than half read.
        let whole = h264(Container::Mp4, 48, 32, 10, 25).expect("movie");
        let p = std::env::temp_dir().join(format!("pc-movie-{}-cut.mp4", std::process::id()));
        std::fs::write(&p, &whole[..whole.len() / 3]).expect("write");
        if let Ok(mut m) = Movie::open(&p) {
            // Some cuts keep the index (it may come first): reading must then fail cleanly.
            let frames = m.info().frames;
            for n in 0..frames {
                let _ = m.picture(n);
            }
        }
        let _ = std::fs::remove_file(&p);
    }
}
