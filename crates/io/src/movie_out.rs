//! Movie files from rendered frames: Render Video's movie output (#14165).
//!
//! A [`MovieWriter`] takes the rendered frames one at a time, as straight RGBA, and builds the
//! whole file in memory: QuickTime movies (`.mov`) of ProRes or uncompressed RGB, and H.264 MP4
//! files in builds with the `h264` feature. Transparency is matted onto black, as no codec here
//! keeps alpha yet (the Alpha Channel option is #14163).
//!
//! The encoders and containers are craft-libs'; this is only the adapter from PhotoCraft's pixels
//! to their frames and samples.

use std::io::Cursor;

use craft_media::isobmff::{Brand, CodecConfig, ColorInfo, FourCc, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use craft_video::prores;

/// Most bytes a movie may take: the file is built in memory before it is written.
pub const MAX_MOVIE_BYTES: u64 = 2 << 30;

/// The codec a movie's frames are written with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovieCodec {
    /// ProRes 4444 (`ap4h`), 12-bit 4:4:4, in a QuickTime movie.
    ProRes4444,
    /// ProRes 422 HQ (`apch`), 10-bit 4:2:2, in a QuickTime movie.
    ProRes422Hq,
    /// 24-bit RGB (`raw `) in a QuickTime movie.
    Uncompressed,
    /// H.264 High profile, 8-bit 4:2:0, BT.601 limited range, in an MP4 file. Only in builds with
    /// the `h264` feature: see [`MovieCodec::available`].
    H264,
}

impl MovieCodec {
    /// The file extension of the container this codec is written in.
    pub fn extension(self) -> &'static str {
        match self {
            MovieCodec::H264 => "mp4",
            _ => "mov",
        }
    }

    /// Whether this build can write the codec: H.264 needs the `h264` feature (#14165 asks a
    /// maintainer whether PhotoCraft may ship the encoder).
    pub fn available(self) -> bool {
        self != MovieCodec::H264 || cfg!(feature = "h264")
    }

    /// The codec's name, for messages and results.
    pub fn name(self) -> &'static str {
        match self {
            MovieCodec::ProRes4444 => "ProRes 4444",
            MovieCodec::ProRes422Hq => "ProRes 422 HQ",
            MovieCodec::Uncompressed => "Uncompressed RGB",
            MovieCodec::H264 => "H.264",
        }
    }
}

/// Why a movie could not be written.
#[derive(Debug, thiserror::Error)]
pub enum MovieError {
    /// The codec isn't in this build.
    #[error("{0} movies can't be written by this build")]
    Unavailable(&'static str),
    /// The frame size or rate is outside what the codec or container takes.
    #[error("{0}")]
    Unsupported(String),
    /// The movie grew past [`MAX_MOVIE_BYTES`].
    #[error("the movie is larger than {} MiB; render fewer frames, a smaller size, or an image sequence", MAX_MOVIE_BYTES >> 20)]
    TooLarge,
    /// The encoder or the container writer failed.
    #[error("{0}")]
    Encode(String),
}

fn encode_err(e: impl std::fmt::Display) -> MovieError {
    MovieError::Encode(e.to_string())
}

/// A frame rate as the integer ratio the container stores: whole rates as `n / 1`, the NTSC
/// rates (23.976, 29.97, 59.94) as `n·1000 / 1001`, and any other as thousandths.
pub fn rate_ratio(fps: f32) -> (u32, u32) {
    let f = f64::from(fps);
    if f >= 1.0 && (f - f.round()).abs() < 1e-3 {
        return (f.round() as u32, 1);
    }
    let ntsc = (f * 1.001).round();
    if ntsc >= 1.0 && (f - ntsc * 1000.0 / 1001.0).abs() < 1e-3 {
        return (ntsc as u32 * 1000, 1001);
    }
    let (n, d) = (((f * 1000.0).round() as u32).max(1), 1000);
    let g = gcd(n, d);
    (n / g, d / g)
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a.max(1) } else { gcd(b, a % b) }
}

enum Encoding {
    ProRes(Box<prores::Encoder>),
    Uncompressed,
    #[cfg(feature = "h264")]
    H264(Box<craft_video::h264::Encoder>),
}

/// A movie being written: [`MovieWriter::push`] each frame, then [`MovieWriter::finish`].
pub struct MovieWriter {
    codec: MovieCodec,
    mux: Mp4Writer<Cursor<Vec<u8>>>,
    track: usize,
    width: usize,
    height: usize,
    /// One frame's duration in the track's time scale (the rate's denominator).
    frame_duration: u32,
    encoding: Encoding,
    frames: i64,
    /// Sample bytes written so far, held against [`MAX_MOVIE_BYTES`].
    bytes: u64,
}

impl MovieWriter {
    /// A movie of `width` × `height` frames at `fps` frames a second.
    pub fn new(codec: MovieCodec, width: u32, height: u32, fps: f32) -> Result<MovieWriter, MovieError> {
        if !codec.available() {
            return Err(MovieError::Unavailable(codec.name()));
        }
        let (w16, h16) = match (u16::try_from(width), u16::try_from(height)) {
            (Ok(w), Ok(h)) if w > 0 && h > 0 => (w, h),
            _ => return Err(MovieError::Unsupported(format!("{width} × {height} frames can't be written to a movie"))),
        };
        if !(fps.is_finite() && fps > 0.0) {
            return Err(MovieError::Unsupported(format!("{fps} frames a second can't be written to a movie")));
        }
        let (rate, frame_duration) = rate_ratio(fps);
        let (encoding, entry, brand) = match codec {
            MovieCodec::ProRes4444 | MovieCodec::ProRes422Hq => {
                let profile = if codec == MovieCodec::ProRes4444 { prores::Profile::P4444 } else { prores::Profile::Hq };
                let mut cfg = prores::EncoderConfig::new(profile, width, height);
                cfg.encode_alpha = false;
                cfg.encoder_id = *b"pcft";
                let mut entry = SampleEntry::prores(FourCc(profile.fourcc()), w16, h16);
                if let Some(v) = entry.video.as_mut() {
                    // BT.709, as the frame headers say (QuickTime `nclc` indices are the H.273 codes).
                    v.color = Some(ColorInfo::Nclc { primaries: 1, transfer: 1, matrix: 1 });
                    // Without alpha the 4444 entry is 24-bit too.
                    v.depth = 24;
                }
                (Encoding::ProRes(Box::new(prores::Encoder::with_config(cfg))), entry, Brand::Mov)
            }
            MovieCodec::Uncompressed => {
                let fourcc = FourCc(*b"raw ");
                (Encoding::Uncompressed, SampleEntry::video(fourcc, CodecConfig::Unknown { fourcc, raw: raw_entry(w16, h16) }, w16, h16), Brand::Mov)
            }
            MovieCodec::H264 => h264_start(width, height, rate, frame_duration)?,
        };
        let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(brand)).map_err(encode_err)?;
        let track = mux.add_track(TrackConfig::new(entry, rate)).map_err(encode_err)?;
        Ok(MovieWriter { codec, mux, track, width: width as usize, height: height as usize, frame_duration, encoding, frames: 0, bytes: 0 })
    }

    /// The codec the movie is written with.
    pub fn codec(&self) -> MovieCodec {
        self.codec
    }

    /// Add the next frame: `width × height` straight RGBA pixels (0..1), row by row.
    pub fn push(&mut self, rgba: &[[f32; 4]]) -> Result<(), MovieError> {
        if rgba.len() != self.width * self.height {
            return Err(MovieError::Encode(format!("a frame of {} pixels for a {} × {} movie", rgba.len(), self.width, self.height)));
        }
        self.frames += 1;
        match &mut self.encoding {
            Encoding::ProRes(enc) => {
                let frame = prores_frame(enc.config().profile, self.width, self.height, rgba)?;
                let data = enc.encode(&frame).map_err(encode_err)?;
                Self::sample(&mut self.mux, self.track, &mut self.bytes, &data, self.frame_duration, 0, true)
            }
            Encoding::Uncompressed => {
                let data = raw_rgb(self.width, rgba);
                Self::sample(&mut self.mux, self.track, &mut self.bytes, &data, self.frame_duration, 0, true)
            }
            #[cfg(feature = "h264")]
            Encoding::H264(enc) => {
                let (y, cb, cr) = yuv420(self.width, self.height, rgba);
                let frame = craft_video::h264::YuvFrame { y: &y, u: &cb, v: &cr, y_stride: self.width, uv_stride: self.width.div_ceil(2) };
                let packets = enc.encode(&frame, self.frames - 1).map_err(encode_err)?;
                for p in packets {
                    Self::sample(&mut self.mux, self.track, &mut self.bytes, &p.data, self.frame_duration, h264_offset(&p, self.frame_duration)?, p.keyframe)?;
                }
                Ok(())
            }
        }
    }

    fn sample(
        mux: &mut Mp4Writer<Cursor<Vec<u8>>>,
        track: usize,
        bytes: &mut u64,
        data: &[u8],
        duration: u32,
        composition_offset: i32,
        is_sync: bool,
    ) -> Result<(), MovieError> {
        *bytes = bytes.saturating_add(data.len() as u64);
        if *bytes > MAX_MOVIE_BYTES {
            return Err(MovieError::TooLarge);
        }
        mux.write_sample(track, WriteSample { data, duration, composition_offset, is_sync }).map_err(encode_err)
    }

    /// The finished file.
    pub fn finish(mut self) -> Result<Vec<u8>, MovieError> {
        self.end_stream()?;
        Ok(self.mux.finish_faststart().map_err(encode_err)?.into_inner())
    }

    /// Write what the encoder still holds, and the stream's final configuration.
    fn end_stream(&mut self) -> Result<(), MovieError> {
        #[cfg(feature = "h264")]
        if let Encoding::H264(enc) = &mut self.encoding {
            for p in enc.flush() {
                Self::sample(&mut self.mux, self.track, &mut self.bytes, &p.data, self.frame_duration, h264_offset(&p, self.frame_duration)?, p.keyframe)?;
            }
            let (sps, pps) = enc.sps_pps();
            let entry = h264_entry(sps, pps, self.width as u16, self.height as u16);
            self.mux.set_sample_entry(self.track, entry).map_err(encode_err)?;
        }
        Ok(())
    }
}

/// The QuickTime visual sample entry body of 24-bit uncompressed RGB (`raw `), after the box
/// header: the fields of [`SampleEntry::video`], which writes no entry for a codec it doesn't know.
fn raw_entry(w: u16, h: u16) -> Vec<u8> {
    let mut b = Vec::with_capacity(78);
    b.extend_from_slice(&[0; 6]); // reserved
    b.extend_from_slice(&1u16.to_be_bytes()); // data reference index
    b.extend_from_slice(&[0; 4]); // version, revision
    b.extend_from_slice(&[0; 4]); // vendor
    b.extend_from_slice(&[0; 4]); // temporal quality
    b.extend_from_slice(&[0; 4]); // spatial quality
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // 72 dpi
    b.extend_from_slice(&0x0048_0000u32.to_be_bytes());
    b.extend_from_slice(&[0; 4]); // data size
    b.extend_from_slice(&1u16.to_be_bytes()); // frames per sample
    let name = b"Uncompressed";
    let mut compressor = [0u8; 32];
    compressor[0] = name.len() as u8;
    compressor[1..=name.len()].copy_from_slice(name);
    b.extend_from_slice(&compressor);
    b.extend_from_slice(&24u16.to_be_bytes()); // depth
    b.extend_from_slice(&(-1i16).to_be_bytes()); // no colour table
    b
}

/// A straight RGBA pixel matted onto black: the colour a viewer without alpha shows.
fn matted(p: [f32; 4]) -> [f32; 3] {
    let a = p[3].clamp(0.0, 1.0);
    [p[0].clamp(0.0, 1.0) * a, p[1].clamp(0.0, 1.0) * a, p[2].clamp(0.0, 1.0) * a]
}

/// `rgba` as `raw ` 24-bit samples: R, G, B per pixel, each row padded to an even byte count.
fn raw_rgb(width: usize, rgba: &[[f32; 4]]) -> Vec<u8> {
    let row = (width * 3).next_multiple_of(2);
    let mut out = Vec::with_capacity(row * rgba.len() / width.max(1));
    for line in rgba.chunks(width.max(1)) {
        for &p in line {
            out.extend(matted(p).map(|c| (c * 255.0).round() as u8));
        }
        out.resize(out.len().next_multiple_of(2), 0);
    }
    debug_assert_eq!(out.len(), row * (rgba.len() / width.max(1)));
    out
}

/// Normalised Y'CbCr (BT.709 for ProRes, BT.601 for H.264) of a matted pixel.
fn ycbcr(p: [f32; 4], m: craft_color::Matrix) -> [f32; 3] {
    let [r, g, b] = matted(p);
    craft_color::rgb_to_ycbcr(r, g, b, m)
}

/// Limited-range code values at `depth` bits for normalised luma and chroma.
fn luma_code(y: f32, depth: u8) -> u16 {
    let s = f32::from(1u16 << (depth - 8));
    ((16.0 + 219.0 * y.clamp(0.0, 1.0)) * s).round() as u16
}

fn chroma_code(c: f32, depth: u8) -> u16 {
    let s = f32::from(1u16 << (depth - 8));
    ((128.0 + 224.0 * c.clamp(-0.5, 0.5)) * s).round() as u16
}

/// A ProRes frame of `rgba` in BT.709 limited range: 12-bit 4:4:4 for 4444, 10-bit 4:2:2 for
/// 422 HQ, chroma averaged over each horizontal pair.
fn prores_frame(profile: prores::Profile, width: usize, height: usize, rgba: &[[f32; 4]]) -> Result<prores::Frame, MovieError> {
    let chroma = profile.chroma();
    let depth = if chroma == prores::ChromaFormat::Yuv444 { 12 } else { 10 };
    let mut f = prores::Frame::new(width as u32, height as u32, chroma, depth, false).map_err(encode_err)?;
    let cw = f.chroma_width() as usize;
    let step = width.div_ceil(cw.max(1));
    for (row, line) in rgba.chunks(width).enumerate() {
        let px: Vec<[f32; 3]> = line.iter().map(|&p| ycbcr(p, craft_color::Matrix::Bt709)).collect();
        for (x, v) in px.iter().enumerate() {
            f.y[row * width + x] = luma_code(v[0], depth);
        }
        for cx in 0..cw {
            let group = &px[cx * step..((cx + 1) * step).min(width)];
            let n = group.len() as f32;
            let (cb, cr) = group.iter().fold((0.0, 0.0), |(b, r), v| (b + v[1], r + v[2]));
            f.cb[row * cw + cx] = chroma_code(cb / n, depth);
            f.cr[row * cw + cx] = chroma_code(cr / n, depth);
        }
    }
    Ok(f)
}

/// 8-bit BT.601 limited-range 4:2:0 planes of `rgba`, chroma averaged over each 2 × 2 block.
#[cfg(feature = "h264")]
fn yuv420(width: usize, height: usize, rgba: &[[f32; 4]]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let px: Vec<[f32; 3]> = rgba.iter().map(|&p| ycbcr(p, craft_color::Matrix::Bt601)).collect();
    let y = px.iter().map(|v| luma_code(v[0], 8) as u8).collect();
    let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
    let (mut cb, mut cr) = (Vec::with_capacity(cw * ch), Vec::with_capacity(cw * ch));
    for cy in 0..ch {
        for cx in 0..cw {
            let (mut b, mut r, mut n) = (0.0, 0.0, 0.0);
            for yy in cy * 2..(cy * 2 + 2).min(height) {
                for xx in cx * 2..(cx * 2 + 2).min(width) {
                    let v = px[yy * width + xx];
                    (b, r, n) = (b + v[1], r + v[2], n + 1.0);
                }
            }
            cb.push(chroma_code(b / n, 8) as u8);
            cr.push(chroma_code(r / n, 8) as u8);
        }
    }
    (y, cb, cr)
}

#[cfg(feature = "h264")]
fn h264_start(width: u32, height: u32, rate: u32, frame_duration: u32) -> Result<(Encoding, SampleEntry, Brand), MovieError> {
    use craft_video::h264::{ColorConfig, Encoder, EncoderConfig, PacketFormat, Profile, RateControl};
    let mut cfg = EncoderConfig::new(width, height, rate, frame_duration);
    cfg.profile = Profile::High;
    cfg.rate = RateControl::Crf(18.0);
    // A keyframe every two seconds, or so.
    cfg.keyint = (2 * rate / frame_duration.max(1)).max(1);
    cfg.bframes = 2;
    cfg.threads = 0;
    // BT.601 (SMPTE 170M) limited range, as the reference application's H.264 output.
    cfg.color = ColorConfig { primaries: 6, transfer: 6, matrix: 6, full_range: false };
    cfg.format = PacketFormat::LengthPrefixed;
    let enc = Encoder::new(cfg).map_err(encode_err)?;
    // The parameter sets are final only once the stream is; `finish` stores them.
    let (sps, pps) = enc.sps_pps();
    Ok((Encoding::H264(Box::new(enc)), h264_entry(sps, pps, width as u16, height as u16), Brand::Mp4))
}

#[cfg(not(feature = "h264"))]
fn h264_start(_: u32, _: u32, _: u32, _: u32) -> Result<(Encoding, SampleEntry, Brand), MovieError> {
    Err(MovieError::Unavailable(MovieCodec::H264.name()))
}

#[cfg(feature = "h264")]
fn h264_entry(sps: Vec<u8>, pps: Vec<u8>, w: u16, h: u16) -> SampleEntry {
    let mut entry = SampleEntry::avc(craft_media::isobmff::AvcConfig::new(vec![sps], vec![pps], 4), w, h);
    if let Some(v) = entry.video.as_mut() {
        v.color = Some(ColorInfo::Nclx { primaries: 6, transfer: 6, matrix: 6, full_range: false });
    }
    entry
}

/// A packet's composition offset (presentation minus decode time) in track time.
#[cfg(feature = "h264")]
fn h264_offset(p: &craft_video::h264::Packet, frame_duration: u32) -> Result<i32, MovieError> {
    (p.pts - p.dts)
        .checked_mul(i64::from(frame_duration))
        .and_then(|o| i32::try_from(o).ok())
        .ok_or_else(|| MovieError::Encode("an H.264 packet's composition offset is out of range".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_media::isobmff::{self, TrackKind};

    fn flat(w: usize, h: usize, rgba: [f32; 4]) -> Vec<[f32; 4]> {
        vec![rgba; w * h]
    }

    /// The first video track's samples of `file`, with its rate.
    fn samples(file: &[u8]) -> (Vec<Vec<u8>>, f64, [u8; 4]) {
        let mp4 = isobmff::open(file).unwrap();
        let t = mp4.track_of_kind(TrackKind::Video).unwrap();
        let track = &mp4.tracks[t];
        let fourcc = track.entries[0].format.0;
        let rate = track.samples.len() as f64 * f64::from(track.timescale) / track.duration as f64;
        let data = (0..track.samples.len()).map(|i| mp4.read_sample(file, t, i).unwrap()).collect();
        (data, rate, fourcc)
    }

    #[test]
    fn rates_become_the_ratios_players_expect() {
        assert_eq!(rate_ratio(30.0), (30, 1));
        assert_eq!(rate_ratio(29.97), (30000, 1001));
        assert_eq!(rate_ratio(23.976), (24000, 1001));
        let (n, d) = rate_ratio(12.5);
        assert_eq!(f64::from(n) / f64::from(d), 12.5);
        let (n, d) = rate_ratio(0.5);
        assert_eq!(f64::from(n) / f64::from(d), 0.5);
    }

    #[test]
    fn prores_movies_decode_to_the_frames_written() {
        for codec in [MovieCodec::ProRes4444, MovieCodec::ProRes422Hq] {
            let (w, h) = (40, 24);
            let colours = [[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 1.0, 1.0, 0.5]];
            let mut m = MovieWriter::new(codec, w as u32, h as u32, 24.0).unwrap();
            for c in colours {
                m.push(&flat(w, h, c)).unwrap();
            }
            let file = m.finish().unwrap();
            let (data, rate, _) = samples(&file);
            assert_eq!(data.len(), 3, "{codec:?}");
            assert!((rate - 24.0).abs() < 1e-6, "{codec:?}: {rate}");
            for (sample, c) in data.iter().zip(colours) {
                let f = prores::decode_frame(sample).unwrap();
                let y = |i: usize| f32::from(f.y[i] >> (f.bit_depth - 8));
                let want = luma_code(ycbcr(c, craft_color::Matrix::Bt709)[0], 8) as f32;
                assert!((y(w * h / 2) - want).abs() <= 2.0, "{codec:?} {c:?}: luma {} for {want}", y(w * h / 2));
            }
        }
    }

    #[test]
    fn uncompressed_movies_hold_the_matted_pixels() {
        // An odd width, so each row is padded.
        let (w, h) = (5, 3);
        let mut m = MovieWriter::new(MovieCodec::Uncompressed, w as u32, h as u32, 30.0).unwrap();
        m.push(&flat(w, h, [1.0, 0.5, 0.0, 1.0])).unwrap();
        m.push(&flat(w, h, [1.0, 1.0, 1.0, 0.0])).unwrap();
        let file = m.finish().unwrap();
        let (data, _, fourcc) = samples(&file);
        assert_eq!(&fourcc, b"raw ");
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].len(), 16 * h, "rows of 15 bytes padded to 16");
        assert_eq!(&data[0][..3], &[255, 128, 0]);
        assert!(data[1].iter().all(|&b| b == 0), "transparent pixels are matted onto black");
    }

    #[test]
    fn frames_of_the_wrong_size_and_unusable_rates_are_refused() {
        assert!(MovieWriter::new(MovieCodec::Uncompressed, 0, 4, 30.0).is_err());
        assert!(MovieWriter::new(MovieCodec::Uncompressed, 70_000, 4, 30.0).is_err());
        assert!(MovieWriter::new(MovieCodec::Uncompressed, 4, 4, f32::NAN).is_err());
        let mut m = MovieWriter::new(MovieCodec::Uncompressed, 4, 4, 30.0).unwrap();
        assert!(m.push(&flat(4, 3, [0.0; 4])).is_err());
    }

    #[cfg(not(feature = "h264"))]
    #[test]
    fn without_the_h264_feature_h264_is_refused() {
        assert!(!MovieCodec::H264.available());
        assert!(matches!(MovieWriter::new(MovieCodec::H264, 16, 16, 30.0), Err(MovieError::Unavailable(_))));
    }

    #[cfg(feature = "h264")]
    #[test]
    fn h264_movies_decode_to_the_frames_written() {
        let (w, h) = (32, 32);
        let greys = [0.1f32, 0.5, 0.9, 0.3, 0.7];
        let mut m = MovieWriter::new(MovieCodec::H264, w as u32, h as u32, 30.0).unwrap();
        for g in greys {
            m.push(&flat(w, h, [g, g, g, 1.0])).unwrap();
        }
        let file = m.finish().unwrap();
        let mp4 = isobmff::open(&file[..]).unwrap();
        let t = mp4.track_of_kind(TrackKind::Video).unwrap();
        let track = &mp4.tracks[t];
        assert_eq!(track.samples.len(), greys.len());
        let mut dec = craft_video::TrackDecoder::new(&track.codec_params()).unwrap();
        let mut frames = Vec::new();
        for (i, s) in track.samples.iter().enumerate() {
            frames.extend(dec.decode(&mp4.read_sample(&file[..], t, i).unwrap(), s.pts).unwrap());
        }
        frames.extend(dec.flush().unwrap());
        frames.sort_by_key(|f| f.pts);
        assert_eq!(frames.len(), greys.len());
        for (f, g) in frames.iter().zip(greys) {
            let want = f32::from(luma_code(g, 8));
            let got = f32::from(f.planes()[0].row_u8((h / 2) as u32).unwrap()[w / 2]);
            assert!((got - want).abs() <= 3.0, "luma {got} for {want}");
        }
    }
}
