//! The worker threads' side of [`super`]: everything here reads the file system and may block, so
//! none of it runs on the frame thread.

use std::path::Path;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use photocraft_codecs::{ChannelLayout, DecodeOptions, EncodeOptions, Format, Image, Limits, SampleType};
use photocraft_doc::Document;
use photocraft_raster::Rgba8Image;

use super::{Done, FileStat, Job, MAX_CACHE_FILES, MAX_DECODE_BYTES, MISSING_KEY, Outcome, Probe, THUMB_SIDE, lock};

/// Pixel memory a PSD may take when it is decoded for its thumbnail.
const PSD_PIXEL_BUDGET: u64 = 512 << 20;

/// A decode for a thumbnail takes at most this many pixels, ...
const DECODE_MAX_PIXELS: u64 = 100_000_000;
/// ... this many bytes for them, ...
const DECODE_MAX_ALLOC: u64 = 512 << 20;
/// ... and is at most this wide or tall.
const DECODE_MAX_SIDE: u32 = 1 << 16;

/// Largest cached thumbnail file read.
const MAX_CACHED_BYTES: u64 = 4 << 20;

/// Most directory entries looked at in the cache folder.
const MAX_SCAN: usize = 10_000;

/// Longest side of a preview taken from a file (an embedded one is smaller than this).
const MAX_PREVIEW_SIDE: u32 = 4096;

/// A worker thread: answers jobs until the window is gone. A panic while working on one file
/// answers "no preview" for it and the thread carries on.
pub(super) fn serve(jobs: &Arc<Mutex<Receiver<Job>>>, answers: &Sender<Done>, cache: Option<&Path>) {
    loop {
        // The lock is held while waiting, so the other worker waits its turn here.
        let job = lock(jobs).recv();
        let Ok(job) = job else { return };
        match job {
            Job::Probe(p) => {
                let ctx = p.ctx.clone();
                let failed = Done { path: p.path.clone(), stat: None, key: None, outcome: Outcome::NoPreview };
                let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| probe(&p, cache))).unwrap_or(failed);
                if answers.send(done).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
            Job::Seed { path, doc } => {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| seed(&path, &doc, cache)));
            }
        }
    }
}

/// `stat` of `path`: what it found, and whether it is a folder.
pub(super) fn stat(path: &str) -> (FileStat, bool) {
    match std::fs::metadata(path) {
        Ok(m) => {
            let modified = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
            (FileStat::Present { size: if m.is_file() { m.len() } else { 0 }, modified }, m.is_dir())
        }
        Err(_) => (FileStat::Missing, false),
    }
}

/// Look at the file in `p`: its status and, when asked, its thumbnail.
pub(super) fn probe(p: &Probe, cache: Option<&Path>) -> Done {
    let (stat, is_dir) = stat(&p.path);
    let done = |key, outcome| Done { path: p.path.clone(), stat: Some(stat), key, outcome };
    if !p.want_thumb {
        return done(None, Outcome::Skipped);
    }
    match stat {
        FileStat::Missing => {
            if p.known == Some(MISSING_KEY) {
                return done(Some(MISSING_KEY), Outcome::Unchanged);
            }
            // A file that is gone keeps the thumbnail it had, if one was ever cached.
            let kept = cache.and_then(|dir| load_any(dir, &p.path));
            done(Some(MISSING_KEY), kept.map_or(Outcome::NoPreview, Outcome::Image))
        }
        FileStat::Present { size, modified } => {
            let key = cache_key(&p.path, size, modified);
            if p.known == Some(key) {
                return done(Some(key), Outcome::Unchanged);
            }
            if let Some(img) = cache.and_then(|dir| load(dir, &p.path, size, modified)) {
                return done(Some(key), Outcome::Image(img));
            }
            match make(&p.path, size, is_dir) {
                Some(img) => {
                    if let Some(dir) = cache {
                        store(dir, &p.path, size, modified, &img);
                    }
                    done(Some(key), Outcome::Image(img))
                }
                None => done(Some(key), Outcome::NoPreview),
            }
        }
    }
}

/// Cache the thumbnail of `doc` for the file at `path`, which `doc` was just opened from or saved to.
pub(super) fn seed(path: &str, doc: &Document, cache: Option<&Path>) {
    let Some(dir) = cache else { return };
    let (FileStat::Present { size, modified }, false) = stat(path) else { return };
    if dir.join(file_name(path, size, modified)).exists() {
        return;
    }
    if let Some(img) = valid(photocraft_compose::thumbnail(doc, THUMB_SIDE)) {
        store(dir, path, size, modified, &img);
    }
}

// ------------------------------------------------------------------ making a thumbnail

/// A thumbnail for the file at `path`: the preview a `.pcraft`, PSD/PSB or camera raw file embeds,
/// else (under [`MAX_DECODE_BYTES`]) a decode. Flat pictures are checked against [`decode_limits`]
/// first, so a small file that expands to a huge image is refused before it is allocated; only
/// PSD/PSB (with its pixel budget) are imported without that check. Camera raws are never
/// demosaiced. `None` shows "No preview".
pub(super) fn make(path: &str, size: u64, is_dir: bool) -> Option<Rgba8Image> {
    if is_dir {
        return None;
    }
    let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "pcraft" => pcraft_preview(path, size),
        "psd" | "psb" | "psdt" => psd_preview(path).or_else(|| decode(path, size, Kind::Layered)),
        "svg" => decode(path, size, Kind::Svg),
        _ => decode(path, size, Kind::Flat),
    }
}

/// How [`decode`] may open a file.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A picture a codec reads: its size is checked before the import.
    Flat,
    /// PSD/PSB: the import honours its pixel budget.
    Layered,
    /// SVG, which the import bounds itself.
    Svg,
}

/// What a decode for a thumbnail may take: about 100 megapixels, and 512 MiB for the pixels.
pub(super) fn decode_limits() -> Limits {
    Limits { max_width: DECODE_MAX_SIDE, max_height: DECODE_MAX_SIDE, max_pixels: DECODE_MAX_PIXELS, max_alloc: DECODE_MAX_ALLOC }
}

/// Only one full decode runs at a time, whatever the number of workers.
static FULL_DECODE: Mutex<()> = Mutex::new(());

/// The PSD/PSB's thumbnail resource, read without reading the document.
fn psd_preview(path: &str) -> Option<Rgba8Image> {
    let img = photocraft_io::psd_thumbnail::read_path(Path::new(path)).ok().flatten()?;
    valid(img).map(|img| fit(img, THUMB_SIDE))
}

/// The thumbnail inside a `.pcraft` (a zip, read whole, so only under the size cap).
fn pcraft_preview(path: &str, size: u64) -> Option<Rgba8Image> {
    if size > MAX_DECODE_BYTES {
        return None;
    }
    let bytes = photocraft_format::read_file(Path::new(path)).ok()?;
    let png = photocraft_format::read_thumbnail(&bytes).ok().flatten()?;
    drop(bytes);
    decode_small(&png)
}

/// Open the file as a document and render its thumbnail, one file at a time across the workers.
fn decode(path: &str, size: u64, kind: Kind) -> Option<Rgba8Image> {
    if size > MAX_DECODE_BYTES {
        return None;
    }
    let bytes = photocraft_format::read_file(Path::new(path)).ok()?;
    let _one = lock(&FULL_DECODE);
    if kind == Kind::Flat {
        // A camera raw shows the preview inside it; developing it is far too much for a thumbnail.
        if photocraft_io::raw::is_raw(&bytes) {
            return raw_preview(&bytes);
        }
        photocraft_codecs::detect(&bytes)?;
        // The codec checks the declared size against the limits before it allocates anything.
        let opts = DecodeOptions { limits: decode_limits(), ..Default::default() };
        if photocraft_codecs::decode_with(&bytes, &opts).is_err() {
            return None;
        }
    }
    let name = Path::new(path).file_name()?.to_string_lossy().into_owned();
    let opts = photocraft_io::ImportOptions { psd_max_pixel_bytes: PSD_PIXEL_BUDGET, ..Default::default() };
    let imported = photocraft_io::import_with_options(&name, &bytes, &opts).ok()?;
    drop(bytes);
    valid(photocraft_compose::thumbnail(&imported.document, THUMB_SIDE))
}

/// The largest JPEG preview embedded in a camera raw file, if it has one.
fn raw_preview(bytes: &[u8]) -> Option<Rgba8Image> {
    let preview = photocraft_raw::embedded_preview(bytes)?;
    let opts = DecodeOptions { limits: decode_limits(), ..Default::default() };
    let img = photocraft_codecs::decode_with(preview.jpeg, &opts).ok()?;
    let (width, height) = img.dimensions();
    valid(Rgba8Image { width, height, pixels: img.to_rgba8() }).map(|img| fit(img, THUMB_SIDE))
}

/// `img` if its pixels are as many as its size says.
fn valid(img: Rgba8Image) -> Option<Rgba8Image> {
    let n = (img.width as usize).checked_mul(img.height as usize)?.checked_mul(4)?;
    (img.width > 0 && img.height > 0 && img.pixels.len() == n).then_some(img)
}

/// Decode a small picture (a cached or embedded thumbnail) with tight limits, reduced to
/// [`THUMB_SIDE`] if larger.
fn decode_small(bytes: &[u8]) -> Option<Rgba8Image> {
    let limits = Limits {
        max_width: MAX_PREVIEW_SIDE,
        max_height: MAX_PREVIEW_SIDE,
        max_pixels: u64::from(MAX_PREVIEW_SIDE) * u64::from(MAX_PREVIEW_SIDE),
        max_alloc: 128 << 20,
    };
    let img = photocraft_codecs::decode_with(bytes, &DecodeOptions { limits, ..Default::default() }).ok()?;
    let (width, height) = img.dimensions();
    valid(Rgba8Image { width, height, pixels: img.to_rgba8() }).map(|img| fit(img, THUMB_SIDE))
}

/// `img` reduced, by averaging, so its longer side is at most `side`.
pub(super) fn fit(img: Rgba8Image, side: u32) -> Rgba8Image {
    let longest = img.width.max(img.height);
    if longest <= side || side == 0 {
        return img;
    }
    let scale = f64::from(side) / f64::from(longest);
    let w = ((f64::from(img.width) * scale).round() as u32).max(1);
    let h = ((f64::from(img.height) * scale).round() as u32).max(1);
    let (sw, sh) = (img.width as usize, img.height as usize);
    let (ow, oh) = (w as usize, h as usize);
    let mut pixels = Vec::with_capacity(ow * oh * 4);
    for y in 0..oh {
        let y0 = y * sh / oh;
        let y1 = ((y + 1) * sh / oh).max(y0 + 1).min(sh);
        for x in 0..ow {
            let x0 = x * sw / ow;
            let x1 = ((x + 1) * sw / ow).max(x0 + 1).min(sw);
            let mut sum = [0u32; 4];
            let mut n = 0u32;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    if let Some(px) = img.pixels.get((sy * sw + sx) * 4..(sy * sw + sx) * 4 + 4) {
                        for (s, v) in sum.iter_mut().zip(px) {
                            *s += u32::from(*v);
                        }
                        n += 1;
                    }
                }
            }
            let n = n.max(1);
            pixels.extend(sum.iter().map(|s| (s / n).min(255) as u8));
        }
    }
    Rgba8Image { width: w, height: h, pixels }
}

// ------------------------------------------------------------------ the on-disk cache

const FNV_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

fn fnv(bytes: &[u8], mut h: u64) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Identity of a file's contents as far as a thumbnail goes: its path, size and modified time.
pub(super) fn cache_key(path: &str, size: u64, modified: Option<u64>) -> u64 {
    let h = fnv(&size.to_le_bytes(), fnv(&modified.unwrap_or(0).to_le_bytes(), fnv(path.as_bytes(), FNV_BASIS)));
    h.min(MISSING_KEY - 1)
}

/// Every cached thumbnail of `path` starts with this, so a file that is gone can still be found.
fn prefix(path: &str) -> String {
    format!("{:016x}_", fnv(path.as_bytes(), FNV_BASIS))
}

/// The cache file for the file at `path` with this size and modified time.
pub(super) fn file_name(path: &str, size: u64, modified: Option<u64>) -> String {
    format!("{}{}_{}.png", prefix(path), modified.unwrap_or(0), size)
}

fn read_cached(file: &Path) -> Option<Rgba8Image> {
    if std::fs::metadata(file).ok()?.len() > MAX_CACHED_BYTES {
        return None;
    }
    decode_small(&std::fs::read(file).ok()?)
}

/// The cached thumbnail of this exact version of the file.
pub(super) fn load(dir: &Path, path: &str, size: u64, modified: Option<u64>) -> Option<Rgba8Image> {
    read_cached(&dir.join(file_name(path, size, modified)))
}

/// The newest cached thumbnail of `path`, whatever version of the file it was made from.
pub(super) fn load_any(dir: &Path, path: &str) -> Option<Rgba8Image> {
    let newest = entries_of(dir, &prefix(path)).into_iter().max_by_key(|(modified, _)| *modified)?;
    read_cached(&newest.1)
}

/// The cache files in `dir` whose names start with `prefix`, with their modified times.
fn entries_of(dir: &Path, prefix: &str) -> Vec<(std::time::SystemTime, std::path::PathBuf)> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    read.flatten()
        .take(MAX_SCAN)
        .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect()
}

/// Cache `img` as the thumbnail of this version of the file, replacing older ones of the same
/// path, then delete the oldest files past [`MAX_CACHE_FILES`]. Failures (a full or read-only
/// disk) only mean the thumbnail is made again next time.
pub(super) fn store(dir: &Path, path: &str, size: u64, modified: Option<u64>, img: &Rgba8Image) {
    let Ok(image) = Image::from_raw(img.width, img.height, ChannelLayout::Rgba, SampleType::U8, img.pixels.clone()) else { return };
    let opts = EncodeOptions { embed_icc: false, embed_metadata: false, ..EncodeOptions::default() };
    let Ok(png) = photocraft_codecs::encode(&image, Format::Png, &opts) else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let name = file_name(path, size, modified);
    for (_, old) in entries_of(dir, &prefix(path)) {
        if old.file_name().is_some_and(|n| n.to_string_lossy() != name) {
            let _ = std::fs::remove_file(old);
        }
    }
    let temp = dir.join(format!("{name}.tmp"));
    if std::fs::write(&temp, &png).is_ok() && std::fs::rename(&temp, dir.join(&name)).is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    evict(dir, MAX_CACHE_FILES);
}

/// Delete the oldest files in `dir` so that at most `max` remain (with a tenth of room to spare, so
/// the next store does not have to scan again).
pub(super) fn evict(dir: &Path, max: usize) {
    let mut files = entries_of(dir, "");
    if files.len() <= max {
        return;
    }
    files.sort();
    let excess = files.len() - max + max / 10;
    for (_, file) in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(file);
    }
}
