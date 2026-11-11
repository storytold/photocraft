//! A baseline JPEG opened band by band into the tiles ([`import_jpeg_streamed`]) gives the
//! document the whole-image path gives: the same everything but the pixels, which `craft-jpeg`
//! decodes where the whole-image path uses `zune-jpeg`, and which agree within the difference
//! between their inverse transforms and chroma upsampling. Files it doesn't take open as before.

use std::sync::atomic::{AtomicU32, Ordering};

use jpeg_encoder::{ColorType, Encoder, SamplingFactor};

use super::*;

fn whole(bytes: &[u8]) -> Result<ImportResult, IoError> {
    import_flat_impl("t", bytes, &Interrupt::NONE, None)
}

/// `import_flat` with the stream forced on whatever the size.
fn forced(bytes: &[u8]) -> Result<ImportResult, IoError> {
    import_flat_impl("t", bytes, &Interrupt::NONE, Some(0))
}

fn streamed(bytes: &[u8]) -> Result<Option<ImportResult>, IoError> {
    import_jpeg_streamed("t", bytes, &crate::ImportOptions::default(), &Interrupt::NONE, 0)
}

/// Gradients, edges and noise in every channel.
fn picture(w: u16, h: u16, channels: usize) -> Vec<u8> {
    let mut seed = 0x9E37_79B9u32 ^ u32::from(w) << 16 ^ u32::from(h);
    let mut out = Vec::with_capacity(usize::from(w) * usize::from(h) * channels);
    for y in 0..usize::from(h) {
        for x in 0..usize::from(w) {
            for c in 0..channels {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let base = [x * 255 / usize::from(w), y * 255 / usize::from(h), if (x / 9 + y / 5) % 2 == 0 { 50 } else { 210 }, (x + 2 * y) % 256][c % 4];
                out.push((base as i32 + (seed >> 24) as i32 % 24 - 12).clamp(0, 255) as u8);
            }
        }
    }
    out
}

struct Jpeg {
    w: u16,
    h: u16,
    color: ColorType,
    sampling: SamplingFactor,
    restart: u16,
    progressive: bool,
    app: Vec<(u8, Vec<u8>)>,
    icc: Option<Vec<u8>>,
}

impl Jpeg {
    fn new(w: u16, h: u16, color: ColorType) -> Self {
        Jpeg { w, h, color, sampling: SamplingFactor::R_4_2_0, restart: 0, progressive: false, app: Vec::new(), icc: None }
    }

    fn bytes(&self) -> Vec<u8> {
        let channels = match self.color {
            ColorType::Luma => 1,
            ColorType::Rgb => 3,
            _ => 4,
        };
        let mut out = Vec::new();
        let mut enc = Encoder::new(&mut out, 88);
        enc.set_sampling_factor(self.sampling);
        enc.set_progressive(self.progressive);
        if self.restart > 0 {
            enc.set_restart_interval(self.restart);
        }
        for (n, data) in &self.app {
            enc.add_app_segment(*n, data).unwrap();
        }
        if let Some(icc) = &self.icc {
            enc.add_icc_profile(icc).unwrap();
        }
        enc.encode(&picture(self.w, self.h, channels), self.w, self.h, self.color).unwrap();
        out
    }
}

/// An EXIF block (APP1 body) with only an orientation.
fn exif(orientation: u8) -> Vec<u8> {
    let mut e = b"Exif\0\0MM\x00\x2a\x00\x00\x00\x08\x00\x01\x01\x12\x00\x03\x00\x00\x00\x01\x00".to_vec();
    e.extend_from_slice(&[orientation, 0, 0, 0, 0, 0, 0]);
    e
}

fn raster(r: &ImportResult) -> Vec<u8> {
    let d = &r.document;
    let LayerContent::Raster(s) = &d.layers[0].content else { panic!("not raster") };
    s.to_interleaved(d.bounds())
}

/// Everything but the pixels as `assert_same` in the PNG tests compares it, and the pixels
/// within (`mean`, `max`) of each other's, sample for sample. Returns the pixels' mean and
/// maximum difference.
fn assert_close(a: &ImportResult, b: &ImportResult, (mean, max): (f64, u8), what: &str) -> (f64, u8) {
    assert_eq!(a.warnings, b.warnings, "{what}: warnings");
    assert_eq!((a.source_read_only, a.preview_only), (b.source_read_only, b.preview_only), "{what}");
    let (da, db) = (&a.document, &b.document);
    assert_eq!((da.size, da.mode, da.depth, &da.color_table), (db.size, db.mode, db.depth, &db.color_table), "{what}: shape");
    assert_eq!(
        (&da.icc_profile, &da.metadata, da.resolution_dpi, &da.file_encoding),
        (&db.icc_profile, &db.metadata, db.resolution_dpi, &db.file_encoding),
        "{what}: metadata"
    );
    assert_eq!(da.layers.len(), 1, "{what}");
    assert_eq!(db.layers.len(), 1, "{what}");
    let (la, lb) = (&da.layers[0], &db.layers[0]);
    assert_eq!((&la.name, la.locks, la.opacity, la.visible), (&lb.name, lb.locks, lb.opacity, lb.visible), "{what}: layer");
    let (LayerContent::Raster(sa), LayerContent::Raster(sb)) = (&la.content, &lb.content) else { panic!("{what}: not raster") };
    assert_eq!((sa.format(), sa.tile_count()), (sb.format(), sb.tile_count()), "{what}: format and tiles");
    let (pa, pb) = (raster(a), raster(b));
    assert_eq!(pa.len(), pb.len(), "{what}");
    let diffs: Vec<u8> = pa.iter().zip(&pb).map(|(x, y)| x.abs_diff(*y)).collect();
    let m = diffs.iter().map(|&d| f64::from(d)).sum::<f64>() / diffs.len().max(1) as f64;
    let x = diffs.iter().copied().max().unwrap_or(0);
    assert!(m <= mean && x <= max, "{what}: pixels differ by {m:.3} on average, {x} at most");
    // Everything else, but for the ids every document and layer gets fresh and the pixels.
    let mut db = db.clone();
    db.id = da.id;
    db.layers.iter_mut().zip(&da.layers).for_each(|(l, k)| {
        l.id = k.id;
        l.content = k.content.clone();
    });
    assert!(da == &db, "{what}: documents differ");
    (m, x)
}

/// The decoders' difference: their inverse transforms (float against integer) and YCbCr
/// conversion, and for subsampled chroma their interpolation, which differs most at edges.
/// `None`: the whole-image path is wrong there, so no bound holds. `zune-jpeg` 0.5.15 misreads
/// a file in which a component after the first has the largest sampling factor, as
/// `jpeg-encoder` gives K in a subsampled CMYK file: its samples are some 40 from the picture
/// encoded, where `craft-jpeg`'s are within a JPEG's loss of it
/// ([`subsampled_cmyk_matches_the_picture_encoded`]); 0.5.16-rc3 reads it right. YCCK, whose
/// first component has the largest factor, is unaffected.
fn tolerance(color: ColorType, sampling: SamplingFactor) -> Option<(f64, u8)> {
    match (color, sampling) {
        (ColorType::Luma, _) | (_, SamplingFactor::R_4_4_4) => Some((0.75, 6)),
        (ColorType::Cmyk, _) => None,
        _ => Some((1.5, 64)),
    }
}

/// The samples `craft-jpeg` decodes from the whole file, in the layer's layout (an opaque alpha
/// after each pixel).
fn craft_samples(bytes: &[u8]) -> Vec<u8> {
    let adobe = bytes.windows(9).position(|w| w[..2] == [0xFF, 0xEE] && &w[4..9] == b"Adobe").map(|at| bytes[at + 15]);
    let d = craft_jpeg::decode(bytes, &craft_jpeg::Limits::default(), adobe).unwrap();
    let craft_jpeg::Samples::U8(px) = d.samples else { panic!("16-bit") };
    px.chunks_exact(usize::from(d.components)).flat_map(|p| p.iter().copied().chain([255])).collect()
}

/// Every layout `jpeg-encoder` writes in one scan, with and without a restart interval at every
/// line of MCUs (which decodes on all cores), streams to exactly `craft-jpeg`'s samples of the
/// whole file and to the whole-image import's document, pixels within the decoders' difference.
#[test]
fn streamed_import_matches_the_whole_image_for_every_layout() {
    let mut worst = (0.0f64, 0u8);
    for color in [ColorType::Luma, ColorType::Rgb, ColorType::Cmyk, ColorType::CmykAsYcck] {
        for sampling in [SamplingFactor::R_4_4_4, SamplingFactor::R_4_2_2, SamplingFactor::R_4_2_0, SamplingFactor::R_4_4_0] {
            for (w, h) in [(1u16, 1u16), (513, 520), (300, 7), (9, 300)] {
                let hmax = if matches!(color, ColorType::Luma) || matches!(sampling, SamplingFactor::R_4_4_4 | SamplingFactor::R_4_4_0) { 1 } else { 2 };
                for restart in [0, w.div_ceil(8 * hmax)] {
                    let bytes = Jpeg { sampling, restart, ..Jpeg::new(w, h, color) }.bytes();
                    let what = format!("{color:?} {sampling:?} {w}x{h} restart {restart}");
                    let want = whole(&bytes).unwrap();
                    assert!(streamed(&bytes).unwrap().is_some(), "{what}: did not stream");
                    let got = forced(&bytes).unwrap();
                    assert!(raster(&got) == craft_samples(&bytes), "{what}: not craft-jpeg's samples");
                    let bound = tolerance(color, sampling);
                    let (m, x) = assert_close(&got, &want, bound.unwrap_or((255.0, 255)), &what);
                    if bound.is_some() {
                        worst = (worst.0.max(m), worst.1.max(x));
                    }
                }
            }
        }
    }
    eprintln!("against zune-jpeg: largest mean difference {:.3}, largest difference {}", worst.0, worst.1);
}

/// Subsampled CMYK and YCCK stream to within a JPEG's loss of the (smooth) picture encoded.
#[test]
fn subsampled_cmyk_matches_the_picture_encoded() {
    let (w, h) = (128u16, 96u16);
    let src: Vec<u8> = (0..usize::from(w) * usize::from(h)).flat_map(|i| [(i % 128 * 2) as u8, (i / 128 * 2) as u8, 200, (i % 128 + i / 128) as u8]).collect();
    for color in [ColorType::Cmyk, ColorType::CmykAsYcck] {
        for sampling in [SamplingFactor::R_4_2_2, SamplingFactor::R_4_2_0] {
            let mut bytes = Vec::new();
            let mut enc = Encoder::new(&mut bytes, 95);
            enc.set_sampling_factor(sampling);
            enc.encode(&src, w, h, color).unwrap();
            let got = raster(&forced(&bytes).unwrap());
            let colour = got.as_chunks::<5>().0.iter().flat_map(|p| p[..4].to_vec());
            let mean = colour.zip(&src).map(|(a, &b)| f64::from(a.abs_diff(b))).sum::<f64>() / src.len() as f64;
            assert!(mean < 2.0, "{color:?} {sampling:?}: {mean:.2} from the picture");
        }
    }
}

/// The profile, EXIF, XMP, Photoshop resources and resolution come through as in the whole
/// image.
#[test]
fn metadata_is_kept() {
    let mut xmp = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
    xmp.extend_from_slice(br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:tiff="http://ns.adobe.com/tiff/1.0/" tiff:XResolution="300/1" tiff:YResolution="300/1" tiff:ResolutionUnit="2"/></rdf:RDF></x:xmpmeta>"#);
    let icc = photocraft_cms::Builtin::DisplayP3.profile().to_bytes();
    let jpeg = Jpeg { app: vec![(1, exif(1)), (1, xmp)], icc: Some(icc.to_vec()), ..Jpeg::new(400, 300, ColorType::Rgb) };
    let bytes = jpeg.bytes();
    assert!(streamed(&bytes).unwrap().is_some());
    let got = forced(&bytes).unwrap();
    assert_close(&got, &whole(&bytes).unwrap(), tolerance(ColorType::Rgb, SamplingFactor::R_4_2_0).unwrap(), "metadata");
    let d = &got.document;
    assert!(d.icc_profile.as_deref().is_some_and(|p| *p == *icc));
    assert!(d.metadata.exif.is_some() && d.metadata.xmp.is_some());
    assert!((d.resolution_dpi - 300.0).abs() < 0.1, "{}", d.resolution_dpi);
}

/// Progressive files and those an EXIF orientation turns open exactly as before.
#[test]
fn other_files_open_through_the_whole_image() {
    let progressive = Jpeg { progressive: true, ..Jpeg::new(300, 200, ColorType::Rgb) }.bytes();
    let turned = Jpeg { app: vec![(1, exif(6))], ..Jpeg::new(300, 200, ColorType::Rgb) }.bytes();
    for (bytes, what) in [(progressive, "progressive"), (turned, "orientation 6")] {
        assert!(streamed(&bytes).unwrap().is_none(), "{what}");
        let (f, w) = (forced(&bytes).unwrap(), whole(&bytes).unwrap());
        assert!(raster(&f) == raster(&w), "{what}");
        assert_close(&f, &w, (0.0, 0), what);
    }
    let upright = Jpeg { app: vec![(1, exif(1))], ..Jpeg::new(300, 200, ColorType::Rgb) }.bytes();
    assert!(streamed(&upright).unwrap().is_some());
}

/// A file cut short opens with the truncation warning the whole image gives it; one cut before
/// it holds image data is refused by both.
#[test]
fn truncated_files_open_or_fail_as_the_whole_image_does() {
    let bytes = Jpeg { restart: 33, ..Jpeg::new(513, 520, ColorType::Rgb) }.bytes();
    let sos = bytes.windows(2).rposition(|w| w == [0xFF, 0xDA]).unwrap();
    let mut opened = 0;
    for cut in [sos + 2, sos + 16, sos + 400, bytes.len() / 2, bytes.len() - 2] {
        let t = &bytes[..cut];
        match (forced(t), whole(t)) {
            (Ok(f), Ok(w)) => {
                assert_eq!(f.warnings, w.warnings, "cut at {cut}");
                assert_eq!(f.document.size, w.document.size, "cut at {cut}");
                opened += 1;
            }
            (Err(_), Err(_)) => {}
            (f, w) => panic!("cut at {cut}: streamed {:?}, whole {:?}", f.map(|r| r.warnings), w.map(|r| r.warnings)),
        }
    }
    assert!(opened >= 3, "{opened}");
}

/// A short scan under a large frame is refused before a tile exists, as the whole image
/// refuses it (#8597).
#[test]
fn a_scan_too_short_for_its_frame_is_refused() {
    let mut bytes = Jpeg::new(64, 64, ColorType::Luma).bytes();
    let sof = bytes.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    bytes[sof + 5..sof + 9].copy_from_slice(&[0x7F, 0xFF, 0x7F, 0xFF]);
    let e = streamed(&bytes).unwrap_err().to_string();
    assert!(e.contains("too short for the frame it declares"), "{e}");
    assert!(whole(&bytes).is_err());
}

#[test]
fn a_cancelled_stream_stops_with_cancelled() {
    let bytes = Jpeg { restart: 33, ..Jpeg::new(513, 520, ColorType::Rgb) }.bytes();
    for n in [0, 1, 2, 3] {
        let calls = AtomicU32::new(0);
        let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= n;
        let r = import_flat_impl("t", &bytes, &Interrupt::cancel_only(&cancel), Some(0));
        assert!(matches!(r, Err(IoError::Cancelled)), "after {n} checks: {:?}", r.map(|_| ()));
    }
}

#[test]
fn progress_is_monotonic() {
    let bytes = Jpeg::new(513, 520, ColorType::Rgb).bytes();
    let seen = std::sync::Mutex::new(Vec::new());
    let no = || false;
    let progress = |f: f32| seen.lock().unwrap().push(f);
    import_flat_impl("t", &bytes, &Interrupt::new(&no, &progress), Some(0)).unwrap();
    let seen = seen.into_inner().unwrap();
    assert!(seen.len() >= 2 && seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
    assert!(seen.last().is_some_and(|&l| l >= 0.95), "{seen:?}");
}

/// Only images of at least the minimum decoded size stream; smaller ones open whole, exactly as
/// before.
#[test]
fn only_images_of_the_minimum_decoded_size_stream() {
    let bytes = Jpeg::new(300, 270, ColorType::Rgb).bytes();
    let at = |min: u64| import_jpeg_streamed("t", &bytes, &crate::ImportOptions::default(), &Interrupt::NONE, min).unwrap().is_some();
    assert!(at(243_000) && !at(243_001));
    let (small, w) = (import_flat("t", &bytes).unwrap(), whole(&bytes).unwrap());
    assert!(raster(&small) == raster(&w));
}

/// The tiles and the decoder's buffers are checked against free memory before a tile exists.
#[test]
fn the_tiles_are_admitted_against_free_memory_first() {
    let bytes = Jpeg::new(513, 520, ColorType::Rgb).bytes();
    let opts = crate::ImportOptions::default();
    let e = import_jpeg_streamed_with("t", &bytes, &opts, &Interrupt::NONE, 0, || Some(1 << 20)).unwrap_err().to_string();
    assert!(e.contains("not enough memory"), "{e}");
    assert!(import_jpeg_streamed_with("t", &bytes, &opts, &Interrupt::NONE, 0, || None).unwrap().is_some(), "unknown free memory is not a refusal");
}

/// Mutated files (cut, bits flipped, bytes overwritten, spans deleted or repeated) never make
/// the stream panic, and it opens every file the whole-image path opens, at the same size and with
/// at least its warnings. It opens some the whole-image path refuses (`zune-jpeg` gives up on
/// stray bytes between marker segments, which libjpeg skips with a warning), and warns about
/// damage `zune-jpeg` passes over.
#[test]
fn mutated_files_open_whenever_the_whole_image_does() {
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
    }
    let inputs = [
        Jpeg { restart: 33, ..Jpeg::new(257, 130, ColorType::Rgb) }.bytes(),
        Jpeg { sampling: SamplingFactor::R_4_4_4, ..Jpeg::new(200, 90, ColorType::Rgb) }.bytes(),
        Jpeg { restart: 5, ..Jpeg::new(120, 64, ColorType::CmykAsYcck) }.bytes(),
        Jpeg { restart: 15, ..Jpeg::new(120, 64, ColorType::Luma) }.bytes(),
    ];
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let (mut both, mut neither, mut only_streamed) = (0, 0, 0);
    for (n, original) in inputs.iter().enumerate() {
        for case in 0..250 {
            let mut b = original.clone();
            match rng.below(5) {
                0 => b.truncate(rng.below(b.len())),
                1 => {
                    for _ in 0..1 + rng.below(6) {
                        let at = rng.below(b.len());
                        b[at] ^= 1 << rng.below(8);
                    }
                }
                2 => {
                    let at = rng.below(b.len());
                    b[at] = rng.next() as u8;
                }
                3 => {
                    let at = rng.below(b.len());
                    let len = rng.below(64).min(b.len() - at);
                    b.drain(at..at + len);
                }
                _ => {
                    let at = rng.below(b.len());
                    let len = rng.below(256).min(b.len() - at);
                    let span = b[at..at + len].to_vec();
                    let to = rng.below(b.len());
                    b.splice(to..to, span);
                }
            }
            let what = format!("input {n} case {case}");
            match (forced(&b), whole(&b)) {
                (Ok(s), Ok(w)) => {
                    assert_eq!(s.document.size, w.document.size, "{what}");
                    assert!(w.warnings.iter().all(|x| s.warnings.contains(x)), "{what}: {:?} against {:?}", s.warnings, w.warnings);
                    both += 1;
                }
                (Err(_), Err(_)) => neither += 1,
                (Ok(_), Err(_)) => only_streamed += 1,
                (Err(e), Ok(_)) => panic!("{what}: the whole image opens, the stream refuses: {e}"),
            }
        }
    }
    eprintln!("{both} open both ways, {neither} neither, {only_streamed} only streamed");
    assert!(both > 500 && neither > 30, "{both} {neither} {only_streamed}");
}
