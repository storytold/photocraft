//! Movie footage behind video layers: frames decoded when they are shown, not held in the
//! document.
//!
//! A video layer linked to a movie file ([`photocraft_doc::FootageFrames`]) keeps only the frames
//! painted on; the others are decoded from the file on demand. The document holds plain data (the
//! file's path, what it holds and where its frames land), so the decoders and their frame cache
//! live here, process-wide and keyed by the file's path, the way an opened file stays open for
//! every document that uses it.
//!
//! Decoding itself comes from an opener: [`set_opener`] installs one that reads movie files, and
//! tests [`register`] their own decoders. Without one, a linked file is offline: its frames show
//! empty and [`status`] says why, so no caller presents missing footage as rendered.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use photocraft_color::PixelFormat;
use photocraft_doc::{Document, FootageInfo, FootageLink};
use photocraft_geom::{Affine, Rect};
use photocraft_raster::Surface;

/// Reads the frames of one movie file.
pub trait FootageDecoder: Send {
    /// What the file holds.
    fn info(&self) -> FootageInfo;
    /// Footage frame `n` (0-based, in presentation order) as a flat document with its own colour
    /// mode, depth and profile; the engine flattens it into the document's pixel format.
    fn decode(&mut self, n: usize) -> Result<Document, String>;
}

/// Opens a movie file's decoder, given the file's path.
pub type Opener = fn(&str) -> Result<Box<dyn FootageDecoder>, String>;

static OPENER: OnceLock<Opener> = OnceLock::new();

/// Install the decoder opener. The first one installed stays.
pub fn set_opener(opener: Opener) {
    let _ = OPENER.set(opener);
}

/// Decoded frames kept across every file: past this, the least recently shown frames are dropped
/// and decoded again when shown.
const DEFAULT_CACHE_BYTES: u64 = 1 << 30;

struct Source {
    decoder: Mutex<Box<dyn FootageDecoder>>,
    info: FootageInfo,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    path: String,
    frame: usize,
    format: PixelFormat,
}

#[derive(Default)]
struct Cache {
    frames: HashMap<Key, (Surface, u64)>,
    order: VecDeque<Key>,
    bytes: u64,
    limit: u64,
}

impl Cache {
    fn get(&mut self, k: &Key) -> Option<Surface> {
        let hit = self.frames.get(k).map(|(s, _)| s.clone())?;
        if let Some(i) = self.order.iter().position(|o| o == k) {
            self.order.remove(i);
        }
        self.order.push_back(k.clone());
        Some(hit)
    }

    fn put(&mut self, k: Key, s: Surface) {
        let size = surface_bytes(&s);
        if let Some((_, old)) = self.frames.insert(k.clone(), (s, size)) {
            self.bytes = self.bytes.saturating_sub(old);
            self.order.retain(|o| o != &k);
        }
        self.bytes = self.bytes.saturating_add(size);
        self.order.push_back(k);
        let limit = if self.limit == 0 { DEFAULT_CACHE_BYTES } else { self.limit };
        // Keep the frame just added even when it alone is over the limit.
        while self.bytes > limit && self.order.len() > 1 {
            let Some(old) = self.order.pop_front() else { break };
            if let Some((_, b)) = self.frames.remove(&old) {
                self.bytes = self.bytes.saturating_sub(b);
            }
        }
    }

    fn forget(&mut self, path: &str) {
        self.frames.retain(|k, _| k.path != path);
        self.order.retain(|k| k.path != path);
        self.bytes = self.frames.values().map(|(_, b)| *b).sum();
    }
}

fn surface_bytes(s: &Surface) -> u64 {
    let tile = u64::from(photocraft_geom::TILE_SIZE.unsigned_abs()).pow(2);
    tile.saturating_mul(s.format().bytes_per_pixel() as u64).saturating_mul(s.tile_count() as u64)
}

#[derive(Default)]
struct Registry {
    sources: HashMap<String, Result<Arc<Source>, String>>,
    cache: Cache,
}

fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(Mutex::default)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Use `decoder` for the file at `path` from now on, dropping any decoder and frames held for it.
pub fn register(path: &str, decoder: Box<dyn FootageDecoder>) -> FootageInfo {
    let info = decoder.info();
    let mut r = lock(registry());
    r.cache.forget(path);
    r.sources.insert(path.to_string(), Ok(Arc::new(Source { decoder: Mutex::new(decoder), info })));
    info
}

/// Close the file at `path`: its decoder and cached frames go, and the next use opens it again
/// (Reload Frame, a relinked file).
pub fn forget(path: &str) {
    let mut r = lock(registry());
    r.sources.remove(path);
    r.cache.forget(path);
}

/// The limit on decoded frames kept in memory, in bytes (0 = the default, 1 GiB).
pub fn set_cache_limit(bytes: u64) {
    lock(registry()).cache.limit = bytes;
}

/// The decoder for `path`, opening the file the first time. A failure is kept, so a missing file
/// is not looked for again on every frame; [`forget`] retries.
fn source(path: &str) -> Result<Arc<Source>, String> {
    if let Some(s) = lock(registry()).sources.get(path) {
        return s.clone();
    }
    let opened = match OPENER.get() {
        Some(open) => open(path).map(|decoder| {
            let info = decoder.info();
            Arc::new(Source { decoder: Mutex::new(decoder), info })
        }),
        None => Err("this build of PhotoCraft can't read movie files".to_string()),
    };
    lock(registry()).sources.entry(path.to_string()).or_insert(opened).clone()
}

/// Open the movie at `path` and say what it holds.
pub fn probe(path: &str) -> Result<FootageInfo, String> {
    source(path).map(|s| s.info)
}

/// Whether the file a layer links to can be read: `Err` says why not (missing, unreadable, or
/// shorter than the frames the layer shows).
pub fn status(link: &FootageLink, frames_used: usize) -> Result<(), String> {
    let s = source(&link.path).map_err(|e| format!("`{}` is offline: {e}", link.path))?;
    if s.info.frames < frames_used {
        return Err(format!("`{}` has {} frames, but the layer shows {frames_used}", link.path, s.info.frames));
    }
    Ok(())
}

/// Footage frame `n` of `link`'s file in `format`, cropped by the link's clip and moved where the
/// link places it in the document.
pub fn frame(link: &FootageLink, n: usize, format: PixelFormat) -> Result<Surface, String> {
    let decoded = decoded(&link.path, n, format)?;
    Ok(place(decoded, link))
}

fn decoded(path: &str, n: usize, format: PixelFormat) -> Result<Surface, String> {
    let key = Key { path: path.to_string(), frame: n, format };
    if let Some(hit) = lock(registry()).cache.get(&key) {
        return Ok(hit);
    }
    let src = source(path)?;
    if n >= src.info.frames {
        return Err(format!("`{path}` has no frame {n} (it has {})", src.info.frames));
    }
    let doc = lock(&src.decoder).decode(n)?;
    let surface = crate::file_cmds::flattened(&doc, format);
    lock(registry()).cache.put(key, surface.clone());
    Ok(surface)
}

/// Crop `s` (footage pixels) by `link.clip` and map it into the document by `link.placement`:
/// exactly for whole-pixel moves and quarter turns, resampled otherwise.
fn place(s: Surface, link: &FootageLink) -> Surface {
    let frame = Rect::from_xywh(0, 0, link.info.width, link.info.height);
    let keep = link.clip.map_or(frame, |c| c.intersect(&frame));
    let s = if keep == frame { s } else { photocraft_algo::resample::crop_surface(&s, keep) };
    let a = link.placement;
    if a == Affine::IDENTITY {
        return s;
    }
    let whole = |v: f64| (v - v.round()).abs() < 1e-9;
    let [m0, m1, m2, m3, dx, dy] = a.m;
    if m0 == 1.0 && m1 == 0.0 && m2 == 0.0 && m3 == 1.0 && whole(dx) && whole(dy) {
        return photocraft_algo::resample::translate_surface(&s, dx.round() as i32, dy.round() as i32);
    }
    // A quarter turn or flip with whole-pixel offsets maps pixels exactly: nearest keeps them.
    let unit = |v: f64| v == 0.0 || v.abs() == 1.0;
    let exact = [m0, m1, m2, m3].iter().all(|v| unit(*v)) && whole(dx) && whole(dy);
    let interp = if exact { photocraft_algo::transform::Interp::Nearest } else { photocraft_algo::transform::Interp::Bicubic };
    let h = photocraft_algo::transform::Homography([m0, m2, dx, m1, m3, dy, 0.0, 0.0, 1.0]);
    photocraft_algo::transform::warp_surface(&s, keep, &h, interp).unwrap_or_else(|_| Surface::new(s.format()))
}

/// Change `link` as a whole-image map `a` (document pixel → new document pixel) changes the
/// layer's stored pixels.
pub(crate) fn transform(link: &mut FootageLink, a: &Affine) {
    link.placement = a.mul(&link.placement);
}

/// Keep only the part of the footage that lands inside `r` (document pixels): Delete Cropped
/// Pixels on a layer whose frames are decoded.
pub(crate) fn clip(link: &mut FootageLink, r: Rect) {
    let Some(inv) = link.placement.inverse() else { return };
    let corners = [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)].map(|(x, y)| inv.apply(photocraft_geom::Point::new(f64::from(x), f64::from(y))));
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in corners {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
        return;
    }
    let c = Rect::new(x0.floor() as i32, y0.floor() as i32, x1.ceil() as i32, y1.ceil() as i32);
    link.clip = Some(link.clip.map_or(c, |old| old.intersect(&c)));
}

#[cfg(test)]
pub(crate) mod testing {
    //! A decoder that draws each footage frame from its number, for tests.

    use super::*;

    /// Frames `width` × `height`, `frames` of them at `fps`: frame `n` is filled with the grey
    /// value `n / frames` (sRGB, opaque). Counts its decodes.
    pub struct Synthetic {
        pub info: FootageInfo,
        pub decodes: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Synthetic {
        pub fn new(width: u32, height: u32, frames: usize, fps: f32) -> Self {
            Synthetic { info: FootageInfo { width, height, frames, fps }, decodes: Arc::default() }
        }

        /// The grey value footage frame `n` is drawn with.
        pub fn level(&self, n: usize) -> f32 {
            (n as f32 + 1.0) / (self.info.frames as f32 + 1.0)
        }
    }

    impl FootageDecoder for Synthetic {
        fn info(&self) -> FootageInfo {
            self.info
        }

        fn decode(&mut self, n: usize) -> Result<Document, String> {
            self.decodes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let v = self.level(n);
            let fmt = PixelFormat { mode: photocraft_color::ColorMode::Rgb, sample: photocraft_color::SampleType::U8, alpha: true };
            let mut doc = Document::new(
                "frame",
                photocraft_doc::Size::new(self.info.width, self.info.height),
                photocraft_color::ColorMode::Rgb,
                photocraft_color::SampleType::U8,
            );
            let mut s = Surface::new(fmt);
            s.fill_rect(Rect::from_xywh(0, 0, self.info.width, self.info.height), &[v, v, v, 1.0]);
            let mut l = photocraft_doc::Layer::raster("Layer 0", fmt);
            l.content = photocraft_doc::LayerContent::Raster(s);
            doc.layers.push(l);
            Ok(doc)
        }
    }
}
