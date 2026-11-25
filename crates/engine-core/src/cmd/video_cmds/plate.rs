//! Reading a video layer's [`Plate`]: an image sequence's unpainted frames stay in their files and
//! are read, through the plate's cache, when the playhead reaches them. A frame is read exactly as
//! File › Open Image Sequence read it at open ([`super::decode_frame`]), and only while its file
//! is the one the plate was opened from (same length and modification time).

use std::cell::RefCell;
use std::path::Path;

use photocraft_doc::{Plate, PlateFile, VideoData, VideoFrame};
use photocraft_raster::Surface;

/// Whether File › Open Image Sequence leaves frames of this format in their files: formats whose
/// import has no setting or approval that changes or refuses pixels. Camera raw, PDF, vector and
/// layered formats (PSD and the like) load whole, as before.
pub(super) fn streams(format: Option<photocraft_formats::Format>) -> bool {
    use photocraft_formats::Format::*;
    matches!(format, Some(Png | Jpeg | Bmp | Tiff | Tga | OpenExr | Hdr | Dpx | Cineon | Qoi | Pnm | WebP))
}

/// `path`'s length and modification time, as a plate records them.
pub(super) fn plate_file(path: &Path) -> std::io::Result<PlateFile> {
    let meta = std::fs::metadata(path)?;
    let modified_ns = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).and_then(|d| u64::try_from(d.as_nanos()).ok());
    Ok(PlateFile { path: path.to_string_lossy().into_owned(), len: meta.len(), modified_ns })
}

/// Why `file` can't be read as the plate frame it was: missing, or changed since.
fn unchanged(file: &PlateFile) -> std::result::Result<(), String> {
    let now = plate_file(Path::new(&file.path)).map_err(|e| format!("`{}` can't be read: {e}", file.path))?;
    if (now.len, now.modified_ns) != (file.len, file.modified_ns) {
        return Err(format!("`{}` has changed since the plate was opened; Reload Frame reads the plate again", file.path));
    }
    Ok(())
}

/// Frame `index` of `plate`: from its cache, or read from its file and kept in the cache.
pub(crate) fn plate_frame(plate: &Plate, index: usize) -> std::result::Result<Surface, String> {
    // Memory Usage may have changed since the plate opened.
    plate.set_cache_budget(super::plate_cache_budget());
    if let Some(frame) = plate.cached(index) {
        plate.trim_around(index);
        return Ok(frame);
    }
    let file = plate.files.get(index).ok_or_else(|| format!("the plate has no frame {}", index + 1))?;
    // A saved document's paths are followed as every path a document stores is.
    if plate.from_saved_document {
        crate::doc_path::resolve(&file.path).map_err(|why| crate::doc_path::refusal("the plate frame", &file.path, why).to_string())?;
    }
    unchanged(file)?;
    let path = Path::new(&file.path);
    let frame = super::read_frame(path, plate.format, &photocraft_io::ImportOptions::default(), Some(plate.size)).map_err(|e| e.to_string())?;
    // A file rewritten while it was read is not the frame either.
    unchanged(file)?;
    plate.keep(index, frame.clone());
    Ok(frame)
}

thread_local! {
    /// Plate frames an edit failed to read into the document: a count, and the last reason.
    static FAILURES: RefCell<(u64, Option<String>)> = const { RefCell::new((0, None)) };
}

fn fail(why: String) {
    FAILURES.with(|f| {
        let mut f = f.borrow_mut();
        f.0 += 1;
        f.1 = Some(why);
    });
}

/// A mark to pass to [`check_since`] after an edit.
pub(crate) fn failure_generation() -> u64 {
    FAILURES.with(|f| f.borrow().0)
}

/// `Err` when [`hold_plate_frames`] failed on this thread since `generation`: the edit missed
/// frames it had to change, so it must not be kept.
pub(crate) fn check_since(generation: u64) -> std::result::Result<(), String> {
    FAILURES.with(|f| {
        let f = f.borrow();
        if f.0 == generation { Ok(()) } else { Err(f.1.clone().unwrap_or_else(|| "a plate frame could not be read".into())) }
    })
}

/// Bytes of frame pixels one whole-plate read may bring into the document.
fn hold_limit() -> u64 {
    super::footage_byte_limit()
}

/// Read every plate frame of `v` into the document, so an edit that changes every frame (a mode or
/// depth change, a resize, crop or canvas turn) reaches them too. Frames that can't be read, or a
/// plate too large to hold, are recorded for [`check_since`] and left as they are; the edit fails.
pub(crate) fn hold_plate_frames(v: &mut VideoData) {
    let Some(plate) = v.plate.clone() else { return };
    let wanted: std::collections::BTreeSet<usize> = v.all_frames().filter_map(VideoFrame::plate_index).collect();
    if wanted.is_empty() {
        return;
    }
    let canvas = photocraft_geom::Rect::from_xywh(0, 0, plate.size.width, plate.size.height);
    let per_frame = photocraft_raster::tile_storage_bytes(canvas, &plate.format);
    let need = per_frame.saturating_mul(wanted.len() as u64);
    if need > hold_limit() {
        fail(format!(
            "this edit changes every frame, which would bring {} plate frames ({} MiB) into memory, more than the {} MiB limit; render the video or work on a shorter plate",
            wanted.len(),
            need >> 20,
            hold_limit() >> 20
        ));
        return;
    }
    let mut read = std::collections::HashMap::new();
    for i in wanted {
        match plate_frame(&plate, i) {
            Ok(s) => {
                read.insert(i, s);
            }
            Err(e) => {
                fail(format!("plate frame {}: {e}", i + 1));
                return;
            }
        }
    }
    for f in v.all_frames_mut() {
        if let Some(s) = f.plate_index().and_then(|i| read.get(&i)) {
            *f = VideoFrame::Pixels(s.clone());
        }
    }
    v.plate = None;
    v.shown = None;
}
