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

/// A movie file read by `photocraft_io::movie` (MP4, MOV, MKV, WebM).
#[cfg(not(target_arch = "wasm32"))]
struct MovieDecoder(photocraft_io::movie::Movie);

#[cfg(not(target_arch = "wasm32"))]
impl FootageDecoder for MovieDecoder {
    fn info(&self) -> FootageInfo {
        let i = self.0.info();
        FootageInfo { width: i.width, height: i.height, frames: i.frames, fps: i.fps }
    }

    fn decode(&mut self, n: usize) -> Result<Document, String> {
        self.0.frame_document(n)
    }
}

/// The opener used when none is installed: movie files on native builds.
#[cfg(not(target_arch = "wasm32"))]
fn open_movie(path: &str) -> Result<Box<dyn FootageDecoder>, String> {
    let resolved = crate::doc_path::resolve(path).map_err(|why| format!("{why}"))?;
    let movie = photocraft_io::movie::Movie::open(&resolved).map_err(|e| e.to_string())?;
    Ok(Box::new(MovieDecoder(movie)))
}

#[cfg(target_arch = "wasm32")]
fn open_movie(_: &str) -> Result<Box<dyn FootageDecoder>, String> {
    Err("movie files can't be read in the web version of PhotoCraft".to_string())
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
    /// Bumped by [`register`] and [`forget`] for a path: an open that started before either
    /// sees a different number when it finishes, and its decoder is not kept.
    generations: HashMap<String, u64>,
    cache: Cache,
}

impl Registry {
    fn generation(&self, path: &str) -> u64 {
        self.generations.get(path).copied().unwrap_or(0)
    }

    /// Drop what is held for `path` and invalidate opens in progress.
    fn invalidate(&mut self, path: &str) {
        self.sources.remove(path);
        self.cache.forget(path);
        *self.generations.entry(path.to_string()).or_insert(0) += 1;
    }
}

fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(Mutex::default)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The registry's name for the file a stored footage path names: the file it resolves to from
/// the document being worked on ([`crate::doc_path`]), so two documents naming different files
/// `clip.mp4` don't share a decoder or frames, and two naming the same file do. A path that
/// can't be resolved keeps its own text.
fn key_of(path: &str) -> String {
    crate::doc_path::resolve(path).map_or_else(|_| path.to_string(), |p| p.to_string_lossy().into_owned())
}

/// Use `decoder` for the file at `path` from now on, dropping any decoder and frames held for it.
pub fn register(path: &str, decoder: Box<dyn FootageDecoder>) -> FootageInfo {
    let info = decoder.info();
    let key = key_of(path);
    let mut r = lock(registry());
    r.invalidate(&key);
    r.sources.insert(key, Ok(Arc::new(Source { decoder: Mutex::new(decoder), info })));
    info
}

/// Close the file at `path`: its decoder and cached frames go, and the next use opens it again
/// (Reload Frame, a relinked file).
pub fn forget(path: &str) {
    lock(registry()).invalidate(&key_of(path));
}

/// Forget a failure to read the file at `path`, so the next use tries it again; a file that is
/// open stays open. For commands that name the file themselves.
pub fn retry(path: &str) {
    let key = key_of(path);
    let mut r = lock(registry());
    if r.sources.get(&key).is_some_and(Result::is_err) {
        r.invalidate(&key);
    }
}

/// The limit on decoded frames kept in memory, in bytes (0 = the default, 1 GiB).
pub fn set_cache_limit(bytes: u64) {
    lock(registry()).cache.limit = bytes;
}

/// The decoder for `path`, opening the file the first time. A failure is kept, so a missing file
/// is not looked for again on every frame; [`forget`] retries.
fn source(path: &str) -> Result<Arc<Source>, String> {
    let key = key_of(path);
    let seen = {
        let r = lock(registry());
        if let Some(s) = r.sources.get(&key) {
            return s.clone();
        }
        r.generation(&key)
    };
    // Opened outside the lock: reading a file's index can take a while.
    // The opener gets the file the key names, resolved from the document, not the stored text.
    let open = OPENER.get().copied().unwrap_or(open_movie);
    let opened = open(&key).map(|decoder| {
        let info = decoder.info();
        Arc::new(Source { decoder: Mutex::new(decoder), info })
    });
    keep_opened(&key, seen, opened)
}

/// Keep `opened` as the decoder for `path` unless the path was registered again or forgotten
/// since generation `seen`: then it is handed back without being kept, so the next use opens
/// the file again. Another open that finished first wins.
fn keep_opened(key: &str, seen: u64, opened: Result<Arc<Source>, String>) -> Result<Arc<Source>, String> {
    let mut r = lock(registry());
    if r.generation(key) != seen {
        return opened;
    }
    r.sources.entry(key.to_string()).or_insert(opened).clone()
}

/// Refuse to read the file at `path` for a session that may not: a footage path is document
/// data, not a request parameter, so every read rechecks it with the gate of the session the
/// work is done for, as linked smart objects do. Before any cache, so a frame decoded for a
/// trusted session is not handed to a restricted one.
fn authorized(path: &str) -> Result<(), String> {
    crate::jobs::authorize_render_command("layer.videoLayers.newVideoLayerFromFile", &serde_json::json!({"path": path}))
        .map_err(|e| format!("`{path}` may not be read here: {e}"))
}

/// What is known about the file `link` names without opening it: `Some(Ok)` when it is open,
/// `Some(Err)` with the reason it could not be read, `None` when nothing has tried it yet.
pub fn known_status(link: &FootageLink, frames_used: usize) -> Option<Result<(), String>> {
    let key = key_of(&link.path);
    let r = lock(registry());
    Some(match r.sources.get(&key)? {
        Ok(s) if s.info.frames < frames_used => Err(format!("`{}` has {} frames, but the layer shows {frames_used}", link.path, s.info.frames)),
        Ok(_) => Ok(()),
        Err(e) => Err(format!("`{}` is offline: {e}", link.path)),
    })
}

/// Open the movie at `path` and say what it holds.
pub fn probe(path: &str) -> Result<FootageInfo, String> {
    authorized(path)?;
    source(path).map(|s| s.info)
}

/// Whether `path` names a movie file the footage reader opens (by its extension).
pub fn is_movie(path: &str) -> bool {
    photocraft_io::is_movie_name(path)
}

/// A new document showing the movie at `path`, as File › Open makes one: the movie's picture
/// size, a timeline at its rate and length, and one video layer named after the file, showing
/// the first frame.
pub fn movie_document(path: &str) -> Result<Document, String> {
    let info = probe(path)?;
    if info.frames > crate::timeline_cmds::MAX_DURATION {
        return Err(format!("`{path}` has {} frames; a video layer is limited to {}", info.frames, crate::timeline_cmds::MAX_DURATION));
    }
    let stem = std::path::Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or("Video").to_string();
    let file = std::path::Path::new(path).file_name().and_then(|s| s.to_str()).unwrap_or(path).to_string();
    let mut doc = Document::new(file, photocraft_doc::Size::new(info.width, info.height), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let fmt = doc.pixel_format();
    doc.timeline = Some(photocraft_doc::Timeline::new(info.frames, info.fps));
    let mut layer = photocraft_doc::Layer::raster(stem, fmt);
    layer.video = Some(photocraft_doc::VideoData::from_footage(FootageLink::new(path.to_string(), info), fmt));
    doc.layers.push(layer);
    // The first frame must decode: a movie whose pictures can't be read isn't opened as an empty
    // layer. Later frames show empty when they fail, as when scrubbing.
    crate::video_cmds::sync_for_output(&mut doc).map_err(|e| e.to_string())?;
    Ok(doc)
}

/// Whether the file a layer links to can be read: `Err` says why not (missing, unreadable, or
/// shorter than the frames the layer shows).
pub fn status(link: &FootageLink, frames_used: usize) -> Result<(), String> {
    authorized(&link.path)?;
    let s = source(&link.path).map_err(|e| format!("`{}` is offline: {e}", link.path))?;
    if s.info.frames < frames_used {
        return Err(format!("`{}` has {} frames, but the layer shows {frames_used}", link.path, s.info.frames));
    }
    Ok(())
}

/// Footage frame `n` of `link`'s file in `format`, cropped by the link's clip and moved where the
/// link places it in the document. `Err` names the file and why the frame can't be read.
pub fn frame(link: &FootageLink, n: usize, format: PixelFormat) -> Result<Surface, String> {
    authorized(&link.path)?;
    let decoded = decoded(&link.path, n, format)?;
    Ok(place(decoded, link))
}

fn decoded(path: &str, n: usize, format: PixelFormat) -> Result<Surface, String> {
    let file = key_of(path);
    let key = Key { path: file.clone(), frame: n, format };
    if let Some(hit) = lock(registry()).cache.get(&key) {
        return Ok(hit);
    }
    let src = source(path)?;
    if n >= src.info.frames {
        return Err(format!("`{path}` has no frame {n} (it has {})", src.info.frames));
    }
    let doc = lock(&src.decoder).decode(n)?;
    let surface = crate::file_cmds::flattened(&doc, format);
    // The decode ran outside the registry lock: a file registered again or forgotten meanwhile
    // must not get this frame in its cache, so keep it only while the same decoder is current.
    let mut r = lock(registry());
    let current = r.sources.get(&file).and_then(|s| s.as_ref().ok()).is_some_and(|s| Arc::ptr_eq(s, &src));
    if current {
        r.cache.put(key, surface.clone());
    }
    Ok(surface)
}

/// Crop `s` (footage pixels) by `link.clip` and map it into the document by `link.placement`:
/// exactly for whole-pixel moves and quarter turns, resampled otherwise.
fn place(s: Surface, link: &FootageLink) -> Surface {
    let frame = Rect::from_xywh(0, 0, link.info.width, link.info.height);
    let (s, keep) = match &link.clip {
        Some(polygon) => keep_inside(&s, polygon, frame),
        None => (s, frame),
    };
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
    let warped = photocraft_algo::transform::warp_surface(&s, keep, &h, interp).unwrap_or_else(|_| Surface::new(s.format()));
    // Resampling spreads the kept edge over the pixels around it: hold the result to the crop as
    // it lands in the document, as the painted frames were cropped there.
    match &link.clip {
        Some(polygon) => {
            let landed: Vec<[f64; 2]> = polygon
                .iter()
                .map(|&[x, y]| {
                    let p = a.apply(photocraft_geom::Point::new(x, y));
                    [p.x, p.y]
                })
                .collect();
            let bounds = warped.content_bounds();
            keep_inside(&warped, &landed, bounds).0
        }
        None => warped,
    }
}

/// Change `link` as a whole-image map `a` (document pixel → new document pixel) changes the
/// layer's stored pixels.
pub(crate) fn transform(link: &mut FootageLink, a: &Affine) {
    link.placement = a.mul(&link.placement);
}

/// Keep only the part of the footage that lands inside `r` (document pixels): Delete Cropped
/// Pixels on a layer whose frames are decoded. The crop's corners are mapped back into footage
/// pixels, so the kept part is exactly the crop whatever turn or scale the footage has, and a
/// later crop narrows it further.
pub(crate) fn clip(link: &mut FootageLink, r: Rect) {
    let corners = [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)];
    let quad: Vec<[f64; 2]> = match link.placement.inverse() {
        Some(inv) => corners
            .iter()
            .map(|&(x, y)| {
                let p = inv.apply(photocraft_geom::Point::new(f64::from(x), f64::from(y)));
                [p.x, p.y]
            })
            .collect(),
        // A placement that collapses the footage shows nothing; nothing of it survives a crop.
        None => Vec::new(),
    };
    let quad = if quad.iter().flatten().all(|v| v.is_finite()) { quad } else { Vec::new() };
    let kept = match &link.clip {
        Some(old) => intersect_convex(old, &quad),
        None => quad,
    };
    link.clip = Some(kept);
}

/// Twice the signed area of `p` (positive when its corners run counter-clockwise in a y-up frame).
fn signed_area2(p: &[[f64; 2]]) -> f64 {
    (0..p.len())
        .map(|i| {
            let ([x0, y0], [x1, y1]) = (p[i], p[(i + 1) % p.len()]);
            x0 * y1 - x1 * y0
        })
        .sum()
}

/// The part of convex polygon `subject` inside convex polygon `clip` (Sutherland-Hodgman), in
/// either winding. Empty when they don't overlap or either is degenerate.
fn intersect_convex(subject: &[[f64; 2]], clip: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let area = signed_area2(clip);
    if clip.len() < 3 || area == 0.0 {
        return Vec::new();
    }
    let side = area.signum();
    let mut out = subject.to_vec();
    for i in 0..clip.len() {
        let (a, b) = (clip[i], clip[(i + 1) % clip.len()]);
        // Positive on the polygon's inner side of edge a→b.
        let inside = |p: [f64; 2]| side * ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]));
        let input = std::mem::take(&mut out);
        for j in 0..input.len() {
            let (p, q) = (input[j], input[(j + 1) % input.len()]);
            let (dp, dq) = (inside(p), inside(q));
            if dp >= 0.0 {
                out.push(p);
            }
            if (dp >= 0.0) != (dq >= 0.0) {
                let t = dp / (dp - dq);
                out.push([p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])]);
            }
        }
        if out.is_empty() {
            break;
        }
    }
    out
}

/// The x range `[from, to)` of pixels on row `y` whose centres lie inside convex `polygon`.
fn row_span(polygon: &[[f64; 2]], y: i32) -> Option<(i32, i32)> {
    let cy = f64::from(y) + 0.5;
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for i in 0..polygon.len() {
        let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
        if (a[1] <= cy) != (b[1] <= cy) {
            let x = a[0] + (cy - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    // Pixel x is kept when its centre x + 0.5 is in [lo, hi).
    let (from, to) = ((lo - 0.5).ceil(), (hi - 0.5).ceil());
    (from < to && from.is_finite() && to.is_finite()).then(|| (from.max(f64::from(i32::MIN)) as i32, to.min(f64::from(i32::MAX)) as i32))
}

/// `s` with only the pixels of `frame` whose centres are inside convex `polygon`, and the
/// rectangle that holds them.
fn keep_inside(s: &Surface, polygon: &[[f64; 2]], frame: Rect) -> (Surface, Rect) {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for &[x, y] in polygon {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    let bounds = if polygon.len() < 3 || !(x0 < x1 && y0 < y1) {
        Rect::EMPTY
    } else {
        Rect::new(x0.floor() as i32, y0.floor() as i32, x1.ceil() as i32, y1.ceil() as i32).intersect(&frame)
    };
    if bounds.is_empty() {
        return (Surface::with_default(s.format(), &s.default_pixel()), Rect::EMPTY);
    }
    let mut out = photocraft_algo::resample::crop_surface(s, bounds);
    // An axis-aligned crop on whole pixels is already exact; only slanted edges need each row.
    let aligned = polygon.len() == 4 && polygon.iter().all(|[x, y]| x.fract() == 0.0 && y.fract() == 0.0) && {
        let xs: std::collections::BTreeSet<i64> = polygon.iter().map(|p| p[0] as i64).collect();
        let ys: std::collections::BTreeSet<i64> = polygon.iter().map(|p| p[1] as i64).collect();
        xs.len() <= 2 && ys.len() <= 2
    };
    if !aligned {
        let clear = s.default_pixel();
        for y in bounds.y0..bounds.y1 {
            match row_span(polygon, y) {
                Some((from, to)) => {
                    let (from, to) = (from.clamp(bounds.x0, bounds.x1), to.clamp(bounds.x0, bounds.x1));
                    out.fill_rect(Rect::new(bounds.x0, y, from, y + 1), &clear);
                    out.fill_rect(Rect::new(to, y, bounds.x1, y + 1), &clear);
                }
                None => out.fill_rect(Rect::new(bounds.x0, y, bounds.x1, y + 1), &clear),
            }
        }
    }
    (out, bounds)
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
        /// A frame whose decode fails, as a damaged file's would.
        pub fail_at: Option<usize>,
        /// Runs at the start of every decode, while the decoder is busy.
        pub during_decode: Option<Box<dyn Fn() + Send>>,
    }

    impl Synthetic {
        pub fn new(width: u32, height: u32, frames: usize, fps: f32) -> Self {
            Synthetic { info: FootageInfo { width, height, frames, fps }, decodes: Arc::default(), fail_at: None, during_decode: None }
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
            if let Some(f) = &self.during_decode {
                f();
            }
            if self.fail_at == Some(n) {
                return Err(format!("frame {n} is damaged"));
            }
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

#[cfg(test)]
mod tests {
    use super::testing::Synthetic;
    use super::*;

    fn opened(frames: usize) -> Result<Arc<Source>, String> {
        let decoder = Synthetic::new(8, 6, frames, 24.0);
        let info = decoder.info();
        Ok(Arc::new(Source { decoder: Mutex::new(Box::new(decoder)), info }))
    }

    /// An open of a file that was forgotten while it ran is handed back but not kept, so the next
    /// use opens the file again; without anything in between, the opened decoder is kept.
    #[test]
    fn an_open_overtaken_by_forget_is_not_kept() {
        let path = format!("test-footage://{}/overtaken.mp4", std::process::id());
        let seen = lock(registry()).generation(&path);
        // Reload Frame closes the file while the open is still reading its index.
        forget(&path);
        assert!(keep_opened(&path, seen, opened(2)).is_ok(), "the caller still gets what it opened");
        assert!(!lock(registry()).sources.contains_key(&path), "but it isn't kept");

        let seen = lock(registry()).generation(&path);
        assert!(keep_opened(&path, seen, opened(2)).is_ok());
        assert!(lock(registry()).sources.get(&path).is_some_and(Result::is_ok), "an open nothing overtook is kept");
    }
}
