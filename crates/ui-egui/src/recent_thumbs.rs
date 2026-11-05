//! What the Home screen's Recent files know about their files: whether each still exists, how big
//! it is, when it last changed, and a thumbnail.
//!
//! Nothing here touches the disk on the frame thread: a network drive can hang a `stat` for
//! minutes. The frame asks ([`RecentView::want`]) for the cards it shows and a small, bounded
//! pool of worker threads answers ([`disk::probe`]): it stats the file, then finds a thumbnail
//! in this order: the on-disk cache (keyed by path, modified time and size), the preview a
//! `.pcraft` or PSD/PSB file embeds, and, for files under [`MAX_DECODE_BYTES`], a full decode
//! through the normal import followed by the compositor's thumbnail. A file with none of those
//! shows "No preview". A worker that panics or cannot start degrades to the same.
//!
//! A path whose `stat` says "not found" is [`FileStat::Missing`]; any other failure is
//! [`FileStat::Unavailable`] (the file may well be there); one that has not been statted yet has no
//! status. The screen must not treat either of the last two as missing.
//!
//! A file taken off the list is forgotten: [`Shared`] is the one place the workers ask before they
//! write to the cache, so nothing is cached for it afterwards, and its cached thumbnails are
//! deleted. Every request carries the identity of the entry that made it, so the answer to a
//! request of an entry that has since been removed (and perhaps added again) is ignored.

#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};

use photocraft_doc::Document;
use photocraft_raster::Rgba8Image;

#[cfg(not(target_arch = "wasm32"))]
mod disk;
#[cfg(test)]
mod tests;

/// The longest side, in pixels, of the thumbnails made here.
pub const THUMB_SIDE: u32 = 320;

/// The largest file read whole to make a thumbnail (a full decode, or the preview inside a
/// `.pcraft`, which is a zip that has to be read to be opened). A larger file shows its embedded
/// PSD/PSB preview, which is read without reading the file, or "No preview".
pub const MAX_DECODE_BYTES: u64 = 64 << 20;

/// Thumbnails held as textures at once; the least recently shown is dropped past this and comes
/// back from the on-disk cache when it is shown again.
pub const MAX_TEXTURES: usize = 64;

/// Thumbnail files kept in the on-disk cache; the oldest are deleted past this.
pub const MAX_CACHE_FILES: usize = 400;

/// Worker threads.
const WORKERS: usize = 2;

/// Requests waiting for, or being worked on by, the workers.
const QUEUE: usize = 8;

/// How long (seconds) a file's status is trusted while the Home screen stays up.
const STALE_SECS: f64 = 30.0;

/// The key a file that is missing has: its thumbnail, if any, is the cached one.
const MISSING_KEY: u64 = u64::MAX;

/// What a `stat` of a recent file found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStat {
    /// It is there: its length (0 for a folder) and modified time (nanoseconds since the epoch,
    /// as precise as the file system keeps it).
    Present { size: u64, modified: Option<u64> },
    /// The file was not found: it was moved, renamed or deleted.
    Missing,
    /// The `stat` failed for another reason (no permission, a drive that errors): the file may
    /// exist, so opening it is tried and reports the real error.
    Unavailable,
}

/// What the worker found for a thumbnail.
pub(crate) enum Outcome {
    /// Not asked for.
    Skipped,
    /// The file is the one the thumbnail already shown was made from.
    Unchanged,
    /// Nothing to show.
    NoPreview,
    /// The thumbnail.
    Image(Rgba8Image),
}

/// A worker's answer for one file.
pub(crate) struct Done {
    pub path: String,
    /// The entry the request was made for (see [`Probe::id`]).
    pub id: u64,
    /// `None`: not known (the worker failed), which is not the same as missing.
    pub stat: Option<FileStat>,
    pub key: Option<u64>,
    pub outcome: Outcome,
}

/// A request to look at one file.
pub(crate) struct Probe {
    pub path: String,
    /// Identity of the entry asking: an answer for an entry that is gone, or was replaced by a
    /// new one for the same path, is ignored.
    pub id: u64,
    /// Also make a thumbnail (unless the file is the one `known` was made from).
    pub want_thumb: bool,
    /// The cache key of the thumbnail already held, if any.
    pub known: Option<u64>,
    pub ctx: egui::Context,
}

/// What the frame and the workers agree on about the cache.
#[derive(Default)]
pub(crate) struct Shared {
    /// Paths a worker may cache a thumbnail for: those a request was made for, until forgotten.
    allowed: Mutex<HashSet<String>>,
    /// Held while a worker checks `allowed` and writes, and while thumbnails are deleted, so a
    /// deletion never overtakes a write it should have followed.
    cache: Mutex<()>,
}

impl Shared {
    /// A request for `path` is being made: it may be cached.
    pub(crate) fn allow(&self, path: &str) {
        lock(&self.allowed).insert(path.to_string());
    }

    /// `path` (or, for `None`, every file) may no longer be cached.
    fn deny(&self, path: Option<&str>) {
        let mut allowed = lock(&self.allowed);
        match path {
            Some(p) => {
                allowed.remove(p);
            }
            None => allowed.clear(),
        }
    }

    /// Run `write` (a store into the cache for `path`) unless `path` was forgotten.
    pub(crate) fn store(&self, path: &str, write: impl FnOnce()) {
        let _cache = lock(&self.cache);
        let allowed = lock(&self.allowed).contains(path);
        if allowed {
            write();
        }
    }

    /// Delete the cached thumbnails of `path`, or all of them for `None`, once any write in
    /// progress is done.
    #[cfg(not(target_arch = "wasm32"))]
    fn wipe(&self, dir: &Path, path: Option<&str>) {
        let _cache = lock(&self.cache);
        disk::forget(dir, path);
    }
}

pub(crate) enum Job {
    Probe(Probe),
    /// Cache the thumbnail of a document just opened or saved at `path`, so the Home screen has
    /// it without opening the file again.
    Seed {
        path: String,
        doc: Arc<Document>,
    },
}

/// What to draw in a card's picture area.
pub enum Preview {
    /// A thumbnail: its texture and size in points before scaling, and whether it is the
    /// grayscale one of a missing file.
    Image { id: egui::TextureId, size: [f32; 2], gray: bool },
    /// Not asked for, or on its way.
    Loading,
    /// There is nothing to show: draw the file icon and "No preview".
    None,
}

enum Thumb {
    /// Not made, or dropped to free memory: made the next time it is shown.
    Unrequested,
    Ready {
        tex: egui::TextureHandle,
        size: [f32; 2],
        gray: bool,
    },
    NoPreview,
}

struct Entry {
    /// See [`Probe::id`]; 0 for an entry made outside the view (tests).
    id: u64,
    stat: Option<FileStat>,
    /// Cache key the thumbnail (or the absence of one) was made for.
    key: Option<u64>,
    thumb: Thumb,
    in_flight: bool,
    /// The refresh generation the status is from.
    fresh: u64,
    /// Last frame the thumbnail was shown (for dropping the least recently used).
    used: u64,
}

impl Entry {
    fn new() -> Self {
        Entry { id: 0, stat: None, key: None, thumb: Thumb::Unrequested, in_flight: false, fresh: 0, used: 0 }
    }
}

/// Threads answering [`Probe`]s. Detached: dropping this lets them finish their current file and
/// end, so a hung drive never holds up the window closing.
struct Worker {
    jobs: SyncSender<Job>,
    done: Receiver<Done>,
}

impl Worker {
    #[cfg(not(target_arch = "wasm32"))]
    fn start(cache: Option<PathBuf>, shared: Arc<Shared>) -> Option<Self> {
        let (jobs, rx) = std::sync::mpsc::sync_channel::<Job>(QUEUE);
        let (tx, done) = std::sync::mpsc::channel::<Done>();
        let rx = Arc::new(Mutex::new(rx));
        let mut started = 0;
        for n in 0..WORKERS {
            let (rx, tx, cache, shared) = (rx.clone(), tx.clone(), cache.clone(), shared.clone());
            let spawned = std::thread::Builder::new().name(format!("recent-thumbs-{n}")).spawn(move || disk::serve(&rx, &tx, cache.as_deref(), &shared));
            if spawned.is_ok() {
                started += 1;
            }
        }
        (started > 0).then_some(Worker { jobs, done })
    }

    #[cfg(target_arch = "wasm32")]
    fn start(_: Option<PathBuf>, _: Arc<Shared>) -> Option<Self> {
        None
    }
}

/// The Home screen's Recent files: their statuses and thumbnails, and the worker that finds them.
#[derive(Default)]
pub struct RecentView {
    entries: HashMap<String, Entry>,
    /// Thumbnails the cards on screen this frame want, with when each file was opened (newest
    /// first once sorted).
    wanted: Vec<(String, u64)>,
    /// Files whose status is wanted for sorting though their cards are off screen.
    stats_wanted: Vec<String>,
    worker: Option<Worker>,
    /// Where the workers cache thumbnails (set when they start).
    cache_dir: Option<PathBuf>,
    /// What the workers may cache (see [`Shared`]).
    shared: Arc<Shared>,
    /// The identity the next entry gets.
    next_id: u64,
    /// The worker could not be started: every thumbnail is "No preview".
    worker_failed: bool,
    /// Frame counter, for [`Entry::used`].
    tick: u64,
    /// Statuses older than this generation are looked at again.
    generation: u64,
    last_frame: u64,
    last_refresh: f64,
}

impl RecentView {
    /// Start a frame of the Home screen showing `listed` (every file in the list, in any order).
    /// Forgets files no longer listed, and looks at the visible files again when the screen was
    /// away for a frame or has been up for a while: a file may have gone, or changed.
    pub fn begin(&mut self, ctx: &egui::Context, listed: &[String]) {
        let frame = ctx.cumulative_frame_nr();
        let now = ctx.input(|i| i.time);
        if frame > self.last_frame.saturating_add(1) || now - self.last_refresh > STALE_SECS {
            self.generation += 1;
            self.last_refresh = now;
        }
        self.last_frame = frame;
        self.tick += 1;
        self.wanted.clear();
        self.stats_wanted.clear();
        let keep: HashSet<&str> = listed.iter().map(String::as_str).collect();
        let gone: Vec<String> = self.entries.keys().filter(|path| !keep.contains(path.as_str())).cloned().collect();
        let cache = self.cache_dir.clone();
        for path in gone {
            self.forget(cache.as_deref(), Some(&path));
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(STALE_SECS));
    }

    /// What is known about the file at `path`; `None` until the worker has looked.
    pub fn status(&self, path: &str) -> Option<FileStat> {
        self.entries.get(path).and_then(|e| e.stat)
    }

    /// The picture for the file at `path` (see [`Preview`]).
    pub fn preview(&mut self, path: &str) -> Preview {
        let tick = self.tick;
        let Some(e) = self.entries.get_mut(path) else { return Preview::Loading };
        match &e.thumb {
            Thumb::Ready { tex, size, gray } => {
                e.used = tick;
                Preview::Image { id: tex.id(), size: *size, gray: *gray }
            }
            Thumb::NoPreview => Preview::None,
            Thumb::Unrequested => Preview::Loading,
        }
    }

    /// The card for `path` (opened at `opened`, seconds since the epoch, 0 when unknown) is on
    /// screen: its status and thumbnail are wanted, the most recently opened first.
    pub fn want(&mut self, path: &str, opened: u64) {
        self.entry(path);
        self.wanted.push((path.to_string(), opened));
    }

    /// Only the status of `path` is wanted (a sort by size needs it for files off screen).
    pub fn want_status(&mut self, path: &str) {
        self.entry(path);
        self.stats_wanted.push(path.to_string());
    }

    /// The entry for `path`, made (with a new identity) when there is none.
    fn entry(&mut self, path: &str) -> &mut Entry {
        let next = &mut self.next_id;
        self.entries.entry(path.to_string()).or_insert_with(|| {
            *next += 1;
            Entry { id: *next, ..Entry::new() }
        })
    }

    /// `path` (or, for `None`, every file) is off the Recent list: drop what is held for it, stop
    /// the workers caching anything more for it (a queued seed or a probe already running
    /// included) and delete its cached thumbnails (all of them for `None`) on a short-lived
    /// thread, so the frame never touches the disk and the deletion does not depend on the job
    /// queue or on the Home screen being shown.
    pub fn forget(&mut self, cache: Option<&Path>, path: Option<&str>) {
        match path {
            Some(p) => {
                self.entries.remove(p);
            }
            None => self.entries.clear(),
        }
        self.shared.deny(path);
        wipe(&self.shared, cache, path);
    }

    /// Whether a worker may still cache a thumbnail for `path`.
    #[cfg(test)]
    pub(crate) fn is_allowed(&self, path: &str) -> bool {
        lock(&self.shared.allowed).contains(path)
    }

    /// Cache the thumbnail of `doc`, just opened or saved at `path`, without opening the file
    /// again later. Skipped without a cache folder; dropped when the workers are busy.
    pub fn seed(&mut self, path: &str, doc: Arc<Document>, cache: Option<PathBuf>) {
        if cache.is_none() || self.worker_failed {
            return;
        }
        self.shared.allow(path);
        let Some(worker) = self.ensure_worker(cache.as_deref()) else { return };
        // Dropped when the queue is full: a thumbnail made later from the file does as well.
        let _ = worker.jobs.try_send(Job::Seed { path: path.to_string(), doc });
    }

    /// End the frame: take the workers' answers and send them the files the frame wanted.
    pub fn finish(&mut self, ctx: &egui::Context, cache: Option<&Path>) {
        self.collect(ctx);
        if self.wanted.is_empty() && self.stats_wanted.is_empty() {
            return;
        }
        if self.worker_failed || self.ensure_worker(cache).is_none() {
            // No way to look: show what a card can without, never a "Missing" it did not find.
            for (path, _) in &self.wanted {
                if let Some(e) = self.entries.get_mut(path)
                    && matches!(e.thumb, Thumb::Unrequested)
                {
                    e.thumb = Thumb::NoPreview;
                }
            }
            return;
        }
        // Newest first; a file wanted twice (grid and list never both) counts once.
        let mut wanted = std::mem::take(&mut self.wanted);
        wanted.sort_by_key(|(_, opened)| std::cmp::Reverse(*opened));
        let stats = std::mem::take(&mut self.stats_wanted);
        let generation = self.generation;
        let shared = self.shared.clone();
        let visible = wanted.iter().map(|(p, _)| (p.clone(), true)).chain(stats.into_iter().map(|p| (p, false)));
        for (path, thumb) in visible {
            let Some(worker) = self.worker.as_ref() else { break };
            let Some(e) = self.entries.get_mut(&path) else { continue };
            let fresh = e.stat.is_some() && e.fresh == generation;
            let needs_thumb = thumb && matches!(e.thumb, Thumb::Unrequested);
            if e.in_flight || (fresh && !needs_thumb) {
                continue;
            }
            shared.allow(&path);
            let probe = Probe {
                path: path.clone(),
                id: e.id,
                want_thumb: thumb,
                // A dropped or never-made thumbnail is made again whatever the file's key.
                known: if matches!(e.thumb, Thumb::Unrequested) { None } else { e.key },
                ctx: ctx.clone(),
            };
            match worker.jobs.try_send(Job::Probe(probe)) {
                Ok(()) => e.in_flight = true,
                // Full: the rest wait for the next frame. Gone: the threads ended.
                Err(TrySendError::Full(_)) => break,
                Err(TrySendError::Disconnected(_)) => {
                    self.worker = None;
                    self.worker_failed = true;
                    break;
                }
            }
        }
    }

    fn ensure_worker(&mut self, cache: Option<&Path>) -> Option<&Worker> {
        if self.worker.is_none() && !self.worker_failed {
            self.cache_dir = cache.map(Path::to_path_buf);
            self.worker = Worker::start(self.cache_dir.clone(), self.shared.clone());
            self.worker_failed = self.worker.is_none();
        }
        self.worker.as_ref()
    }

    /// Take every answer waiting and apply it.
    fn collect(&mut self, ctx: &egui::Context) {
        let mut answers = Vec::new();
        if let Some(worker) = &self.worker {
            loop {
                match worker.done.try_recv() {
                    Ok(done) => answers.push(done),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.worker = None;
                        self.worker_failed = true;
                        break;
                    }
                }
            }
        }
        for done in answers {
            self.apply(ctx, done);
        }
        self.drop_unused_textures();
    }

    fn apply(&mut self, ctx: &egui::Context, done: Done) {
        let generation = self.generation;
        let tick = self.tick;
        // Taken off the list while it was being looked at (and perhaps listed again since, as a
        // new entry): the answer is not for the entry there now. Nothing was cached for it after
        // it was forgotten (see `Shared`).
        let Some(e) = self.entries.get_mut(&done.path).filter(|e| e.id == done.id) else { return };
        e.in_flight = false;
        let changed = done.stat.is_some() && e.stat.is_some() && e.stat != done.stat;
        if done.stat.is_some() {
            e.stat = done.stat;
            e.fresh = generation;
        }
        let missing = e.stat == Some(FileStat::Missing);
        match done.outcome {
            Outcome::Skipped => {
                // The file changed, or went, since its thumbnail was made: make it again the
                // next time the card is shown (an unchanged file comes back from the cache).
                if changed {
                    e.thumb = Thumb::Unrequested;
                }
            }
            Outcome::Unchanged => {
                e.key = done.key;
                // A thumbnail shown in colour for a file that has just gone is made again, in
                // gray, from the cache.
                if missing && matches!(e.thumb, Thumb::Ready { gray: false, .. }) {
                    e.thumb = Thumb::Unrequested;
                }
                // A grayed thumbnail for a file that is not missing after all (it could not be
                // statted) comes back in colour from the cache.
                if e.stat == Some(FileStat::Unavailable) && matches!(e.thumb, Thumb::Ready { gray: true, .. }) {
                    e.key = None;
                    e.thumb = Thumb::Unrequested;
                }
            }
            Outcome::NoPreview => {
                e.key = done.key;
                e.thumb = Thumb::NoPreview;
            }
            Outcome::Image(img) => {
                e.key = done.key;
                let (w, h) = (img.width as usize, img.height as usize);
                let valid = w > 0 && h > 0 && w.checked_mul(h).and_then(|n| n.checked_mul(4)) == Some(img.pixels.len()) && img.width.max(img.height) <= 2048;
                e.thumb = if valid {
                    let mut pixels = img.pixels;
                    if missing {
                        gray_out(&mut pixels);
                    }
                    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels);
                    let tex = ctx.load_texture(format!("recent-{}", done.path), image, egui::TextureOptions::LINEAR);
                    e.used = tick;
                    Thumb::Ready { tex, size: [w as f32, h as f32], gray: missing }
                } else {
                    Thumb::NoPreview
                };
            }
        }
    }

    /// Past [`MAX_TEXTURES`], drop the thumbnails shown longest ago.
    fn drop_unused_textures(&mut self) {
        let mut ready: Vec<(u64, &String)> = self.entries.iter().filter(|(_, e)| matches!(e.thumb, Thumb::Ready { .. })).map(|(p, e)| (e.used, p)).collect();
        if ready.len() <= MAX_TEXTURES {
            return;
        }
        ready.sort();
        let excess = ready.len().saturating_sub(MAX_TEXTURES);
        let drop: Vec<String> = ready.into_iter().take(excess).map(|(_, p)| p.clone()).collect();
        for path in drop {
            if let Some(e) = self.entries.get_mut(&path) {
                e.thumb = Thumb::Unrequested;
            }
        }
    }

    /// Textures currently held (tests).
    #[cfg(test)]
    pub(crate) fn textures(&self) -> usize {
        self.entries.values().filter(|e| matches!(e.thumb, Thumb::Ready { .. })).count()
    }
}

/// Delete cached thumbnails (see [`Shared::wipe`]) on a detached thread. Failing to start one is
/// ignored: the cache is only a cache.
fn wipe(shared: &Arc<Shared>, cache: Option<&Path>, path: Option<&str>) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(dir) = cache {
        let (shared, dir, path) = (shared.clone(), dir.to_path_buf(), path.map(str::to_string));
        let _ = std::thread::Builder::new().name("recent-thumbs-forget".into()).spawn(move || shared.wipe(&dir, path.as_deref()));
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (shared, cache, path);
}

/// `rgba` (straight alpha) as gray, keeping each pixel's alpha.
fn gray_out(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let [r, g, b, _] = px;
        let y = (u32::from(*r) * 299 + u32::from(*g) * 587 + u32::from(*b) * 114) / 1000;
        let y = y.min(255) as u8;
        (*r, *g, *b) = (y, y, y);
    }
}

/// The name of a file's kind, from its extension: "PSD", "JPEG", "PCRAFT"; "File" without one.
pub fn kind_label(path: &str) -> String {
    let ext = std::path::Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_uppercase()).unwrap_or_default();
    match ext.as_str() {
        "" => tl!("File").to_string(),
        "JPG" | "JPE" | "JFIF" => "JPEG".into(),
        "TIF" => "TIFF".into(),
        _ => ext,
    }
}

/// Seconds since the Unix epoch.
pub fn unix_now() -> u64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// How long ago `opened` was, in words: "Just now", "6 minutes ago", "3 hours ago", "2 days ago".
pub fn when_label(now: u64, opened: u64) -> String {
    let minutes = now.saturating_sub(opened) / 60;
    let lang = crate::i18n::current();
    if minutes == 0 {
        tl!("Just now").to_string()
    } else if minutes < 60 {
        crate::i18n::trn(lang, minutes, "{n} minute ago", "{n} minutes ago")
    } else if minutes < 24 * 60 {
        crate::i18n::trn(lang, (minutes + 30) / 60, "{n} hour ago", "{n} hours ago")
    } else {
        crate::i18n::trn(lang, (minutes + 720) / (24 * 60), "{n} day ago", "{n} days ago")
    }
}

/// A file size as "812 KB", "184.2 MB" or "1.8 GB".
pub fn size_label(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if bytes == 0 {
        "0 KB".into()
    } else if b < KB * KB {
        format!("{} KB", (b / KB).ceil().max(1.0))
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.1} GB", b / (KB * KB * KB))
    }
}

/// A lock on `m` that ignores poisoning: a worker that panicked must not stop the others.
pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
