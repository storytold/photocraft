//! A video layer's plate: the numbered image files an image sequence was opened from. Frames
//! nobody has painted stay on disk and are read when the playhead reaches them, so a long plate
//! need not fit in memory. The engine reads them (this crate does no file I/O); a [`Plate`] keeps
//! what it read in a small cache of the frames nearest the one last read.

use std::collections::BTreeMap;
use std::sync::Mutex;

use photocraft_color::PixelFormat;
use photocraft_geom::Size;
use photocraft_raster::Surface;
use serde::{Deserialize, Serialize};

/// How much decoded plate a [`Plate`] keeps by default: 1 GiB, about seven 4K float frames or
/// 25 2K ones.
pub const DEFAULT_PLATE_CACHE_BYTES: u64 = 1 << 30;

/// One file of a plate, and what it was when the plate was read: a file whose length or
/// modification time differs has changed, and is not read as this frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, craft_ui_macros::PlainData)]
pub struct PlateFile {
    pub path: String,
    /// Length in bytes.
    pub len: u64,
    /// Modification time in nanoseconds since the Unix epoch, where the file system has one.
    pub modified_ns: Option<u64>,
}

/// The files of an image sequence a video layer reads its unpainted frames from, one frame per
/// file, each flattened to `format` at canvas size `size`. Shared (`Arc`) by every copy of the
/// document that refers to it, history steps included.
#[derive(Debug, craft_ui_macros::PlainData)]
pub struct Plate {
    pub files: Vec<PlateFile>,
    pub format: PixelFormat,
    pub size: Size,
    /// Read from a saved document: its paths are the document's, followed only where the engine
    /// follows paths a document stores. A plate opened in this session reads the files it listed.
    pub from_saved_document: bool,
    #[plain_data(assume = "a memo of decoded files: what a frame reads never changes, as a changed file is refused rather than read")]
    cache: PlateCache,
}

impl Plate {
    pub fn new(files: Vec<PlateFile>, format: PixelFormat, size: Size) -> Self {
        Plate { files, format, size, from_saved_document: false, cache: PlateCache::new(DEFAULT_PLATE_CACHE_BYTES) }
    }

    /// Number of frames (files).
    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Frame `index`, if it is in the cache.
    pub fn cached(&self, index: usize) -> Option<Surface> {
        self.cache.lock().frames.get(&index).cloned()
    }

    /// Keep frame `index`, just read, in the cache, then drop the frames farthest from it until the
    /// cache is within its budget. The frame just kept always stays.
    pub fn keep(&self, index: usize, frame: Surface) {
        let bytes = photocraft_raster::stored_tiles_bytes(frame.tile_count() as u64, &frame.format());
        let mut c = self.cache.lock();
        if let Some(old) = c.frames.insert(index, frame) {
            let old = photocraft_raster::stored_tiles_bytes(old.tile_count() as u64, &old.format());
            c.bytes = c.bytes.saturating_sub(old);
        }
        c.bytes = c.bytes.saturating_add(bytes);
        while c.bytes > c.budget {
            let farthest = c.frames.keys().copied().filter(|&i| i != index).max_by_key(|&i| i.abs_diff(index));
            let Some(far) = farthest else { break };
            if let Some(f) = c.frames.remove(&far) {
                c.bytes = c.bytes.saturating_sub(photocraft_raster::stored_tiles_bytes(f.tile_count() as u64, &f.format()));
            }
        }
    }

    /// Set how many bytes of decoded frames the cache keeps, dropping what no longer fits the next
    /// time a frame is kept.
    pub fn set_cache_budget(&self, bytes: u64) {
        self.cache.lock().budget = bytes;
    }

    /// The frames in the cache, in order.
    pub fn cached_frames(&self) -> Vec<usize> {
        self.cache.lock().frames.keys().copied().collect()
    }

    /// Bytes of decoded frames in the cache (whole tiles).
    pub fn cached_bytes(&self) -> u64 {
        self.cache.lock().bytes
    }
}

/// The decoded frames a [`Plate`] keeps.
#[derive(Debug)]
struct PlateCache(Mutex<CacheInner>);

#[derive(Debug)]
struct CacheInner {
    budget: u64,
    bytes: u64,
    frames: BTreeMap<usize, Surface>,
}

impl PlateCache {
    fn new(budget: u64) -> Self {
        PlateCache(Mutex::new(CacheInner { budget, bytes: 0, frames: BTreeMap::new() }))
    }

    /// The cache, whether or not a thread panicked holding it: it only ever holds whole entries.
    fn lock(&self) -> std::sync::MutexGuard<'_, CacheInner> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};

    fn frame(fmt: PixelFormat) -> Surface {
        let mut s = Surface::new(fmt);
        s.write_pixel(0, 0, &[1.0, 0.0, 0.0, 1.0]);
        s
    }

    /// Over budget, the cache drops the frames farthest from the one just read, never that one.
    #[test]
    fn the_cache_keeps_the_frames_nearest_the_last_read() {
        let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
        let one = photocraft_raster::stored_tiles_bytes(1, &fmt);
        let plate = Plate::new(Vec::new(), fmt, Size::new(8, 8));
        plate.set_cache_budget(3 * one);
        for i in 0..10 {
            plate.keep(i, frame(fmt));
        }
        let before = plate.cached_frames();
        assert!(plate.cached_bytes() <= 3 * one && before.contains(&9), "{before:?}");
        assert!(before.iter().all(|&i| i + before.len() >= 10), "the frames just read stay: {before:?}");
        plate.keep(5, frame(fmt));
        let after = plate.cached_frames();
        assert!(after.contains(&5) && plate.cached_bytes() <= 3 * one, "{after:?}");
        let dropped: Vec<usize> = before.iter().copied().filter(|i| !after.contains(i)).collect();
        assert!(!dropped.is_empty(), "{after:?}");
        assert!(dropped.iter().all(|d| after.iter().all(|k| k.abs_diff(5) <= d.abs_diff(5))), "the farthest went: {dropped:?} {after:?}");
        plate.set_cache_budget(0);
        plate.keep(9, frame(fmt));
        assert_eq!(plate.cached_frames(), [9], "the frame just read stays, whatever the budget");
    }
}
