//! Layer › Video Layers (and Rasterize › Video). A video layer holds a stack of frames; the
//! layer's displayed raster content is the frame at the timeline playhead, kept in sync by [`sync`]
//! after any edit or navigation. Paint, fills and filters edit that displayed content, so [`store`]
//! writes it back into the playhead's frame before anything re-syncs. Headless and scriptable. Frame persistence in `.pcraft` is a
//! follow-up; a saved document keeps the current frame as the layer's content.

use photocraft_doc::{Document, Layer, LayerContent, Timeline, VideoData};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// Maximum dense RGBA staging pixels in one animated GIF frame. A GIF also keeps the
/// quantised frame and palette while rendering, so the document canvas limit is not enough.
const MAX_GIF_FRAME_PIXELS: u64 = 64 << 20;
/// Largest total an animated GIF render keeps staged before encoding: one index byte per pixel plus
/// a 768-byte palette for every frame, the same 2 GiB bound as imported footage ([`MAX_FOOTAGE_BYTES`]).
/// The per-frame cap alone lets `frames × pixels` reach terabytes (a 4000 × 4000 canvas over a
/// 1,000,000-frame timeline). 1920 × 1080 holds about 2,145 frames and 4096 × 2160 about 242.
/// This bounds the staged frames only (indices and palettes). It does not bound the encoder's output
/// buffer (LZW data, roughly 1.3-1.5 times the raw size, growing by doubling) or the per-frame
/// transients (RGBA f32, 16 bytes per pixel), so it is not a bound on peak memory.
const MAX_GIF_STAGED_BYTES: u64 = MAX_FOOTAGE_BYTES;
/// The GIF format stores each side as 16 bits.
const MAX_GIF_SIDE: u64 = 65_535;

/// Reject a GIF render whose canvas or frames-times-canvas staging would be unreasonable, before any
/// frame is rendered. All sums are checked in `u64`.
fn check_gif_budget(w: u64, h: u64, frames: u64) -> std::result::Result<(), String> {
    if w > MAX_GIF_SIDE || h > MAX_GIF_SIDE {
        return Err(format!("{w} × {h} is too large to render as an animated GIF (the GIF format allows {MAX_GIF_SIDE} pixels per side)"));
    }
    let pixels = w.checked_mul(h).filter(|p| *p <= MAX_GIF_FRAME_PIXELS);
    let Some(pixels) = pixels else {
        return Err(format!("{w} × {h} is too large to render as an animated GIF (limit {MAX_GIF_FRAME_PIXELS} pixels per frame)"));
    };
    let per_frame = pixels + 768;
    if frames.checked_mul(per_frame).is_none_or(|total| total > MAX_GIF_STAGED_BYTES) {
        return Err(format!(
            "{frames} frames of {w} × {h} are too large to render as an animated GIF (limit {} MiB staged in total); shorten the timeline or reduce the canvas",
            MAX_GIF_STAGED_BYTES >> 20
        ));
    }
    Ok(())
}

/// Set every video layer's displayed content to its frame at the playhead.
pub fn sync(doc: &mut Document) {
    let cur = doc.timeline.as_ref().map_or(0, |t| t.current);
    sync_layers(&mut doc.layers, cur);
}

fn sync_layers(layers: &mut [Layer], cur: usize) {
    for l in layers.iter_mut() {
        let frame = l.video.as_ref().and_then(|v| {
            let visible = if v.show_altered { &v.frames } else { v.original_frames.as_ref().unwrap_or(&v.frames) };
            visible.get(cur.min(visible.len().saturating_sub(1))).cloned()
        });
        if let Some(f) = frame
            && let LayerContent::Raster(s) = &mut l.content
        {
            *s = f;
        }
        if let Some(ch) = l.children_mut() {
            sync_layers(ch, cur);
        }
    }
}

/// Keep edits made to every video layer's displayed content (paint, fills, filters) by writing it
/// back into its frame at the playhead. Call before moving the playhead or re-syncing.
pub fn store(doc: &mut Document) {
    let cur = doc.timeline.as_ref().map_or(0, |t| t.current);
    store_layers(&mut doc.layers, cur);
}

fn store_layers(layers: &mut [Layer], cur: usize) {
    for l in layers.iter_mut() {
        if let (Some(v), LayerContent::Raster(s)) = (&mut l.video, &l.content) {
            let at = cur.min(v.frames.len().saturating_sub(1));
            // When showing the original, the layer's displayed surface is a preview,
            // not an edit to write over the altered frame (#3060).
            if (v.show_altered || v.original_frames.is_none())
                && let Some(f) = v.frames.get_mut(at)
            {
                *f = s.clone();
            }
        }
        if let Some(ch) = l.children_mut() {
            store_layers(ch, cur);
        }
    }
}

/// The active layer, if it's a video layer.
fn active_video(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document")?;
    match st.active_layer.and_then(|id| st.doc.layer(id)) {
        Some(l) if l.video.is_some() => Ok(()),
        _ => Err("select a video layer".into()),
    }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document".into())
}

/// Grow the timeline to at least `frames` frames (never shrink it). Only a work area that reached
/// the old end follows the new end; a narrowed one stays where the user put it (#4170).
fn grow_duration(t: &mut Timeline, frames: usize) {
    let old = t.duration;
    t.duration = t.duration.max(frames.max(1));
    if t.work_end >= old {
        t.work_end = t.duration;
    }
    t.clamp();
}

/// Edit the active video layer's [`VideoData`] at the playhead, then re-sync the displayed frame.
fn edit_video(s: &mut Session, label: &str, f: impl FnOnce(&mut VideoData, usize) -> Result<()>) -> Result<Value> {
    let count = s.edit(label, |doc, active| {
        store(doc);
        let cur = doc.timeline.as_ref().map_or(0, |t| t.current);
        let id = active.ok_or(EngineError::NoDocument)?;
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let v = l.video.as_mut().ok_or_else(|| EngineError::BadParams { cmd: label.into(), msg: "the active layer is not a video layer".into() })?;
        f(v, cur)?;
        let n = v.frames.len();
        if let Some(t) = &mut doc.timeline {
            grow_duration(t, n);
        }
        sync(doc);
        Ok(n)
    })?;
    Ok(json!({"frames": count}))
}

fn blank_of(fmt: photocraft_color::PixelFormat) -> Surface {
    Surface::new(fmt)
}

/// Both the altered and original footage stacks must have room for an added frame.
fn check_frame_growth(frames: usize, original_frames: Option<usize>) -> Result<()> {
    let limit = crate::timeline_cmds::MAX_DURATION;
    if frames.max(original_frames.unwrap_or(0)) >= limit {
        return Err(EngineError::Other(format!("a video layer is limited to {limit} frames")));
    }
    Ok(())
}

fn new_blank(s: &mut Session, _p: &Value) -> Result<Value> {
    let id = s.edit("New Blank Video Layer", |doc, active| {
        store(doc);
        let fmt = doc.pixel_format();
        if doc.timeline.is_none() {
            doc.timeline = Some(Timeline::new(1, 30.0));
        }
        let fps = doc.timeline.as_ref().map_or(30.0, |t| t.fps);
        let n = doc.timeline.as_ref().map_or(1, |t| t.duration).max(1);
        // A loaded timeline's duration bypasses the validation of timeline.create / setProps.
        if n > crate::timeline_cmds::MAX_DURATION {
            return Err(EngineError::Other(format!("the timeline is {n} frames; a video layer is limited to {} frames", crate::timeline_cmds::MAX_DURATION)));
        }
        let frames: Vec<Surface> = (0..n).map(|_| Surface::new(fmt)).collect();
        let name = doc.next_layer_name("Video Layer");
        let mut l = Layer::raster(name, fmt);
        l.video = Some(VideoData::new(frames, fps));
        let id = crate::layer_place::insert_new(doc, *active, l, crate::layer_place::Place::AutoEditable);
        *active = Some(id);
        sync(doc);
        Ok(id)
    })?;
    Ok(json!({"layer": id.0}))
}

fn insert_blank(s: &mut Session, _p: &Value) -> Result<Value> {
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    edit_video(s, "Insert Blank Frame", move |v, cur| {
        check_frame_growth(v.frames.len(), v.original_frames.as_ref().map(Vec::len))?;
        let at = cur.saturating_add(1).min(v.frames.len());
        v.frames.insert(at, blank_of(fmt));
        if let Some(original) = &mut v.original_frames {
            // The stacks stay aligned, but never index past a shorter one.
            original.insert(at.min(original.len()), blank_of(fmt));
        }
        Ok(())
    })
}

fn duplicate_frame(s: &mut Session, _p: &Value) -> Result<Value> {
    edit_video(s, "Duplicate Frame", |v, cur| {
        if let Some(f) = v.frames.get(cur) {
            check_frame_growth(v.frames.len(), v.original_frames.as_ref().map(Vec::len))?;
            let f = f.clone();
            if let Some(original) = &mut v.original_frames {
                let orig = original.get(cur).cloned().unwrap_or_else(|| blank_of(f.format()));
                original.insert((cur + 1).min(original.len()), orig);
            }
            v.frames.insert(cur + 1, f);
        }
        Ok(())
    })
}

fn delete_frame(s: &mut Session, _p: &Value) -> Result<Value> {
    edit_video(s, "Delete Frame", |v, cur| {
        if v.frames.len() > 1 && cur < v.frames.len() {
            v.frames.remove(cur);
            if let Some(original) = &mut v.original_frames
                && cur < original.len()
            {
                original.remove(cur);
            }
        }
        Ok(())
    })
}

fn restore_frame(s: &mut Session, _p: &Value) -> Result<Value> {
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    edit_video(s, "Restore Frame", move |v, cur| {
        let original = v.original_frames.as_ref().and_then(|frames| frames.get(cur)).cloned();
        if let Some(f) = v.frames.get_mut(cur) {
            *f = original.unwrap_or_else(|| blank_of(fmt));
        }
        Ok(())
    })
}

fn restore_all(s: &mut Session, _p: &Value) -> Result<Value> {
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    edit_video(s, "Restore All Frames", move |v, _| {
        for (i, f) in v.frames.iter_mut().enumerate() {
            *f = v.original_frames.as_ref().and_then(|frames| frames.get(i)).cloned().unwrap_or_else(|| blank_of(fmt));
        }
        Ok(())
    })
}

fn show_altered(s: &mut Session, _p: &Value) -> Result<Value> {
    edit_video(s, "Show Altered Video", |v, _| {
        v.show_altered = !v.show_altered;
        Ok(())
    })
}

/// Rasterize: drop the frame stack, keeping the current frame as a normal pixel layer.
fn rasterize(s: &mut Session, _p: &Value) -> Result<Value> {
    s.edit("Rasterize Video", |doc, active| {
        let id = active.ok_or(EngineError::NoDocument)?;
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        // The content already shows the current frame (kept in sync); just drop the stack.
        l.video = None;
        Ok(())
    })?;
    Ok(json!({"rasterized": true}))
}

// ------------------------------------------------------------------ file-based footage

/// Frames loaded from a file or an image sequence.
struct Footage {
    frames: Vec<Surface>,
    /// The frame rate the footage itself implies (a single animated GIF's or WebP's delays); `None` for
    /// stills and sequences, which carry none.
    fps: Option<f32>,
    /// Decoder warnings that survived salvaging, e.g. a GIF or WebP truncated mid-animation
    /// (#11113): the frames are real but incomplete, and the caller must disclose that.
    warnings: Vec<String>,
}

/// The frame rate for a GIF's frame delays (milliseconds): that of the most common delay, the
/// first one seen on a tie. GIF has a delay per frame but the timeline one rate. Delays under
/// 20 ms, which includes the 0 and 10 ms many encoders write, are shown at 100 ms, as browsers
/// do, since they would otherwise play at an unintended 50-100 fps.
fn fps_from_delays(delays: &[u32]) -> Option<f32> {
    let clean = |d: u32| if d < 20 { 100 } else { d };
    // Count delays with a map keyed on the cleaned delay, recording first-seen order
    // separately. A linear scan per frame (iter_mut().find) was O(n^2) for a GIF with many
    // distinct delays. order[] preserves the tie-break: the earliest-seen delay wins.
    let mut counts: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    let mut order: Vec<u32> = Vec::new();
    for d in delays.iter().copied().map(clean) {
        let n = counts.entry(d).or_insert_with(|| {
            order.push(d);
            0
        });
        *n += 1;
    }
    let best = order.iter().fold(None::<(u32, usize)>, |best, &d| {
        let n = counts[&d];
        match best {
            Some((_, bn)) if bn >= n => best,
            _ => Some((d, n)),
        }
    });
    best.map(|(d, _)| 1000.0 / d as f32)
}

/// `photocraft_io::import_gif_frames` / `import_webp_frames`.
type ImportFrames = fn(
    &str,
    &[u8],
    &mut dyn FnMut(photocraft_doc::Document, u32) -> std::result::Result<(), photocraft_io::IoError>,
) -> std::result::Result<photocraft_io::FramesImport, photocraft_io::IoError>;

/// Frame storage one footage import may keep (whole tiles, all frames together).
const MAX_FOOTAGE_BYTES: u64 = 2 << 30;

/// The footage frame limit this import applies: the production cap, or a test scope's lowered
/// cap so a regression can reach the refusal without keeping gibibytes of frames.
fn footage_byte_limit() -> u64 {
    #[cfg(test)]
    if let Some(limit) = limits::max_footage_bytes() {
        return limit;
    }
    MAX_FOOTAGE_BYTES
}

/// Import options for footage files. A test scope may lower the PSD/layered-TIFF memory limit,
/// which the user's preferences do not expose, to reach an admission refusal on a small file.
fn footage_import_options(s: &Session) -> photocraft_io::ImportOptions {
    #[cfg(test)]
    if let Some(limit) = limits::psd_max_pixel_bytes() {
        return photocraft_io::ImportOptions { psd_max_pixel_bytes: limit, ..crate::jobs::import_options(s.prefs()) };
    }
    crate::jobs::import_options(s.prefs())
}

/// Per-thread test overrides for the footage import limits. A scope restores the previous values
/// on drop, and the thread-local keeps parallel tests from lowering each other's limits.
#[cfg(test)]
mod limits {
    use std::cell::Cell;

    #[derive(Clone, Copy, Default)]
    struct Overrides {
        psd_max_pixel_bytes: Option<u64>,
        max_footage_bytes: Option<u64>,
    }

    thread_local! {
        static CURRENT: Cell<Overrides> = const { Cell::new(Overrides { psd_max_pixel_bytes: None, max_footage_bytes: None }) };
    }

    /// Restores the previous overrides when dropped.
    pub(crate) struct Scoped(Overrides);

    impl Scoped {
        /// Lowers the PSD/layered-TIFF memory admission limit for the current thread.
        pub(crate) fn psd_max_pixel_bytes(limit: u64) -> Self {
            Self::set(Overrides { psd_max_pixel_bytes: Some(limit), ..Default::default() })
        }

        /// Lowers the retained footage-frame limit for the current thread.
        pub(crate) fn max_footage_bytes(limit: u64) -> Self {
            Self::set(Overrides { max_footage_bytes: Some(limit), ..Default::default() })
        }

        fn set(next: Overrides) -> Self {
            Self(CURRENT.with(|current| current.replace(next)))
        }
    }

    impl Drop for Scoped {
        fn drop(&mut self) {
            CURRENT.with(|current| current.set(self.0));
        }
    }

    pub(super) fn psd_max_pixel_bytes() -> Option<u64> {
        CURRENT.with(|current| current.get().psd_max_pixel_bytes)
    }

    pub(super) fn max_footage_bytes() -> Option<u64> {
        CURRENT.with(|current| current.get().max_footage_bytes)
    }
}

/// The engine error for a failed page of a multi-page TIFF. A memory refusal keeps its typed form,
/// so the UI, control channel and agents still get the required and limit fields they need to
/// offer approval or explain the hard limit; any other failure keeps the page's text.
fn tiff_page_error(display: &std::path::Path, name: &str, page: usize, error: photocraft_io::IoError) -> EngineError {
    match error {
        photocraft_io::IoError::PsdApprovalRequired { .. } | photocraft_io::IoError::PsdImportBudget { .. } => crate::error::io_open_error(name, error),
        error => EngineError::Other(format!("`{}` page {}: {error}", display.display(), page + 1)),
    }
}

/// Load frames from `path`: a single image (1 frame, or every frame of an animated GIF or WebP), or a
/// directory of images (sorted = an image sequence). Each becomes a full raster frame in the
/// document's format.
fn load_frames(path: &str, fmt: photocraft_color::PixelFormat, opts: &photocraft_io::ImportOptions) -> Result<Footage> {
    let p = std::path::Path::new(path);
    let is_img = |f: &std::path::Path| {
        matches!(
            f.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
            Some("png" | "jpg" | "jpeg" | "tif" | "tiff" | "webp" | "bmp" | "gif" | "tga" | "exr" | "pcraft" | "psd")
        )
    };
    let files: Vec<std::path::PathBuf> = if p.is_dir() {
        let mut v: Vec<_> = std::fs::read_dir(p)
            .map_err(|e| EngineError::Other(format!("read dir `{path}`: {e}")))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|f| is_img(f))
            .collect();
        v.sort();
        v
    } else {
        vec![p.to_path_buf()]
    };
    if files.is_empty() {
        return Err(EngineError::Other(format!("no images in `{path}`")));
    }
    let mut frames: Vec<Surface> = Vec::with_capacity(files.len());
    let mut delays = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    // Each frame stores whole tiles however little it draws, so count the tiles kept so far.
    let mut kept_bytes = 0u64;
    let max_footage_bytes = footage_byte_limit();
    let mut keep = |frames: &mut Vec<Surface>, surface: Surface| -> std::result::Result<(), String> {
        // A frame of fully transparent pixels stores no tiles, so the count needs its own limit.
        if frames.len() >= crate::timeline_cmds::MAX_DURATION {
            return Err(format!("the footage has more than {} frames", crate::timeline_cmds::MAX_DURATION));
        }
        let tile = u64::from(photocraft_geom::TILE_SIZE.unsigned_abs()).pow(2) * surface.format().bytes_per_pixel() as u64;
        kept_bytes = kept_bytes.saturating_add(tile.saturating_mul(surface.tile_count() as u64));
        if kept_bytes > max_footage_bytes {
            return Err(format!("the footage needs more than {} MiB of frames", max_footage_bytes >> 20));
        }
        frames.push(surface);
        Ok(())
    };
    for f in &files {
        let bytes = crate::allocation::read_file_within(f, max_footage_bytes).map_err(|e| EngineError::Other(format!("read `{}`: {e}", f.display())))?;
        let name = f.to_string_lossy();
        // A GIF (a still one is its only frame) or an animated WebP brings every frame.
        let animation = match photocraft_formats::detect(&name, &bytes) {
            Some(photocraft_formats::Format::Gif) => Some(photocraft_io::import_gif_frames as ImportFrames),
            Some(photocraft_formats::Format::WebP) if photocraft_codecs::webp_is_animated(&bytes) => Some(photocraft_io::import_webp_frames as ImportFrames),
            _ => None,
        };
        if let Some(import) = animation {
            let mut over = None;
            let done = import(&name, &bytes, &mut |doc, delay_ms| {
                if let Err(msg) = keep(&mut frames, crate::file_cmds::flattened(&doc, fmt)) {
                    over = Some(msg);
                    return Err(photocraft_io::IoError::Cancelled);
                }
                delays.push(delay_ms);
                Ok(())
            });
            if let Some(msg) = over {
                return Err(EngineError::Other(format!("`{}`: {msg}", f.display())));
            }
            let done = done.map_err(|e| EngineError::Other(format!("`{}`: {e}", f.display())))?;
            // A file that decoded with salvage (a truncation warning, #11113) must still be
            // disclosed even though the frames themselves came through.
            for w in done.warnings {
                let w = format!("`{}`: {w}", f.display());
                if !warnings.contains(&w) {
                    warnings.push(w);
                }
            }
            continue;
        }
        // A multi-page TIFF: every full-resolution page of the main chain, in order, as the
        // GIF's frames are (#6379). Thumbnails, reduced-resolution copies and masks are left out.
        if photocraft_formats::detect(&name, &bytes) == Some(photocraft_formats::Format::Tiff) {
            let (pages, complete) = tiff_pages(&bytes);
            if pages.len() > 1 {
                // A damaged or looping page chain, or one past the 10,000-directory limit, lists
                // only its first pages: refuse it rather than silently import part of the file.
                if !complete {
                    return Err(EngineError::Other(format!(
                        "`{}`: only the first {} pages could be read (its page list is damaged or too long), so not every page would be imported",
                        f.display(),
                        pages.len()
                    )));
                }
                for page in pages {
                    // Same admission policy, caller limit and import settings as any other TIFF
                    // page import: a multi-page TIFF is not a way around the memory budget.
                    let doc = photocraft_io::import_tiff_page_with_options(&name, &bytes, Some(page), opts, &photocraft_raster::Interrupt::NONE)
                        .map_err(|e| tiff_page_error(f, &name, page, e))?
                        .document;
                    // The page is flattened before admission of the next one, so the retained
                    // frames of every page count against the footage limit, as GIF and WebP
                    // frames do.
                    keep(&mut frames, crate::file_cmds::flattened(&doc, fmt)).map_err(|msg| EngineError::Other(format!("`{}`: {msg}", f.display())))?;
                }
                continue;
            }
        }
        let mut doc = crate::file_cmds::import_with_options(&name, &bytes, opts)?;
        // A Targa's alpha opens as a channel (as in Photoshop); a frame's alpha is transparency.
        if f.extension().is_some_and(|e| e.eq_ignore_ascii_case("tga")) {
            photocraft_io::tga_alpha_channel_to_transparency(&mut doc);
        }
        keep(&mut frames, crate::file_cmds::flattened_checked(&doc, fmt, "layer.videoLayers.newVideoLayerFromFile")?)
            .map_err(|msg| EngineError::Other(format!("`{}`: {msg}", f.display())))?;
    }
    // Only a lone GIF or WebP says how fast it plays; a sequence mixing files has no single rate.
    let fps = if files.len() == 1 { fps_from_delays(&delays) } else { None };
    Ok(Footage { frames, fps, warnings })
}

fn new_from_file(s: &mut Session, p: &Value) -> Result<Value> {
    let path = p
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::BadParams { cmd: "layer.videoLayers.newVideoLayerFromFile".into(), msg: "need `path`".into() })?
        .to_string();
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    let Footage { frames, fps: footage_fps, warnings } = load_frames(&path, fmt, &footage_import_options(s))?;
    // An explicit `fps` wins, then the rate the footage implies, then 30.
    let fps = crate::timeline_cmds::fps_param("layer.videoLayers.newVideoLayerFromFile", p)?.or(footage_fps).unwrap_or(30.0);
    let n = frames.len();
    let id = s.edit("New Video Layer from File", |doc, active| {
        store(doc);
        if doc.timeline.is_none() {
            doc.timeline = Some(Timeline::new(n, fps));
        } else if let Some(t) = &mut doc.timeline {
            grow_duration(t, n);
        }
        let name = doc.next_layer_name(std::path::Path::new(&path).file_stem().and_then(|s| s.to_str()).unwrap_or("Video"));
        let mut l = Layer::raster(name, fmt);
        let mut v = VideoData::new(frames, fps);
        v.original_frames = Some(v.frames.clone());
        v.source = photocraft_doc::VideoSource::File { path: path.clone() };
        // The first frame is the layer's content before it is placed: an active artboard takes
        // the layer only when its content is on the board, as `frames_to_layers` does.
        if let (Some(first), LayerContent::Raster(surf)) = (v.frames.first(), &mut l.content) {
            *surf = first.clone();
        }
        l.video = Some(v);
        let id = crate::layer_place::insert_new(doc, *active, l, crate::layer_place::Place::AutoEditable);
        *active = Some(id);
        sync(doc);
        Ok(id)
    })?;
    Ok(json!({"layer": id.0, "frames": n, "warnings": warnings}))
}

fn replace_footage(s: &mut Session, p: &Value) -> Result<Value> {
    let path = p
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::BadParams { cmd: "layer.videoLayers.replaceFootage".into(), msg: "need `path`".into() })?
        .to_string();
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    let Footage { frames, warnings, .. } = load_frames(&path, fmt, &footage_import_options(s))?;
    let mut out = edit_video(s, "Replace Footage", move |v, _| {
        v.original_frames = Some(frames.clone());
        v.frames = frames;
        v.source = photocraft_doc::VideoSource::File { path: path.clone() };
        Ok(())
    })?;
    if !warnings.is_empty() {
        out["warnings"] = Value::from(warnings);
    }
    Ok(out)
}

fn interpret_footage(s: &mut Session, p: &Value) -> Result<Value> {
    let fps = crate::timeline_cmds::fps_param("layer.videoLayers.interpretFootage", p)?;
    edit_video(s, "Interpret Footage", move |v, _| {
        if let Some(f) = fps {
            v.fps = f;
        }
        Ok(())
    })
}

fn reload_frame(s: &mut Session, _p: &Value) -> Result<Value> {
    // Reload the whole stack from its file source (what a frame reload needs for a sequence).
    let (src, fmt) = {
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let id = st.active_layer.ok_or(EngineError::NoDocument)?;
        let v = st
            .doc
            .layer(id)
            .and_then(|l| l.video.as_ref())
            .ok_or_else(|| EngineError::BadParams { cmd: "layer.videoLayers.reloadFrame".into(), msg: "not a video layer".into() })?;
        (v.source.clone(), st.doc.pixel_format())
    };
    let photocraft_doc::VideoSource::File { path } = src else {
        return Err(EngineError::BadParams { cmd: "layer.videoLayers.reloadFrame".into(), msg: "this layer has no file source to reload".into() });
    };
    // The footage path is stored in the document.
    let file = crate::doc_path::resolve(&path).map_err(|why| crate::doc_path::refusal("the footage", &path, why))?;
    let Footage { frames, warnings, .. } = load_frames(&file.to_string_lossy(), fmt, &footage_import_options(s))?;
    let mut out = edit_video(s, "Reload Frame", move |v, _| {
        v.original_frames = Some(frames.clone());
        v.frames = frames;
        Ok(())
    })?;
    if !warnings.is_empty() {
        out["warnings"] = Value::from(warnings);
    }
    Ok(out)
}

/// The pages of a TIFF that are pages of the document: full-resolution directories of the main
/// chain (not SubIFDs, thumbnails or masks), by index into [`photocraft_codecs::tiff_info`]'s
/// pages, and whether the walk listed every directory. Empty when the file can't be listed.
fn tiff_pages(bytes: &[u8]) -> (Vec<usize>, bool) {
    let Ok(info) = photocraft_codecs::tiff_info(bytes) else { return (Vec::new(), true) };
    let pages = info.pages.iter().enumerate().filter(|(_, p)| p.parent.is_none() && p.kind == photocraft_codecs::TiffPageKind::Page).map(|(i, _)| i).collect();
    (pages, info.complete)
}

/// Import › Video Frames to Layers: load frames from a file/sequence and add each as its own layer.
fn frames_to_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let path = p
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::BadParams { cmd: "file.import.videoFramesToLayers".into(), msg: "need `path`".into() })?
        .to_string();
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    let Footage { frames, warnings, .. } = load_frames(&path, fmt, &footage_import_options(s))?;
    let n = frames.len();
    s.edit("Video Frames to Layers", |doc, active| {
        let mut above = *active;
        let mut place = crate::layer_place::Place::AutoEditable;
        for (i, f) in frames.into_iter().enumerate() {
            let mut l = Layer::raster(format!("Frame {}", i + 1), fmt);
            if let LayerContent::Raster(surf) = &mut l.content {
                *surf = f;
            }
            above = Some(crate::layer_place::insert_new(doc, above, l, place));
            place = crate::layer_place::Place::Above;
        }
        *active = above;
        Ok(())
    })?;
    Ok(json!({"layers": n, "warnings": warnings}))
}

/// The document's name without its extension, as a file name: a native document keeps whatever
/// name its file declares, parent-directory components included, and the frames are written
/// beside the chosen folder, not wherever the name leads.
fn safe_stem(name: &str) -> String {
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = leaf.rsplit_once('.').map_or(leaf, |(a, _)| a).trim();
    if stem.is_empty() || stem.chars().all(|c| c == '.') { "video".to_string() } else { stem.to_string() }
}

/// Export › Render Video: composite the whole document at each timeline frame and write an image
/// sequence (`{dir}/{stem}_NNNN.{format}`). Clean-room; no proprietary codec.
fn render_video(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = p.get("dir").and_then(Value::as_str).ok_or_else(|| EngineError::BadParams { cmd: "file.export.renderVideo".into(), msg: "need `dir`".into() })?;
    std::fs::create_dir_all(dir).map_err(|e| EngineError::Other(format!("mkdir `{dir}`: {e}")))?;
    render_video_with(s, p, |name, bytes| crate::file_cmds::write_file(&format!("{dir}/{name}"), bytes), |_, _| true)
}

/// Render through a caller-owned writer. The writer receives one filename, never an ambient
/// directory. Returning false from progress stops at a frame boundary; the caller owns cleanup.
pub fn render_video_with(
    s: &Session,
    p: &Value,
    mut write: impl FnMut(&str, &[u8]) -> Result<()>,
    mut progress: impl FnMut(usize, usize) -> bool,
) -> Result<Value> {
    let dir = p.get("dir").and_then(Value::as_str).unwrap_or("");
    let format = p.get("format").and_then(Value::as_str).unwrap_or("png");
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let mut base = (*st.doc).clone();
    store(&mut base);
    let duration = base.timeline.as_ref().map_or(1, |t| t.duration).max(1);
    // Loaded timelines bypass the duration validation used by timeline.create/setProps.
    if duration > crate::timeline_cmds::MAX_DURATION {
        return Err(EngineError::Other(format!("the timeline is {duration} frames; Render Video is limited to {} frames", crate::timeline_cmds::MAX_DURATION)));
    }
    let range = render_range(p, base.timeline.as_ref())?;
    // Rates under 1 fps are valid timelines, so only an unusable loaded rate falls back to 30.
    let timeline_fps = base.timeline.as_ref().map(|t| t.fps).filter(|f| f.is_finite() && *f > 0.0).unwrap_or(30.0);
    let plan = render_plan(p, range, timeline_fps, (base.size.width, base.size.height))?;
    // Every rendered frame, in order; `frames` counts them.
    let frames = plan.frames.len();
    let fps = plan.fps;
    let stem = safe_stem(&base.name);
    let fmt = base.pixel_format();
    let bounds = base.bounds();
    let out = Rect::from_xywh(0, 0, plan.size.0, plan.size.1);
    let (w, h) = (out.width() as usize, out.height() as usize);

    let cancelled = || EngineError::Other("render cancelled".into());
    if !progress(0, frames) {
        return Err(cancelled());
    }

    // Animated GIF: one file, every frame quantised to its own palette.
    if format.eq_ignore_ascii_case("gif") {
        // Each frame is staged as f32 RGBA and every encoded frame is kept until the file is written;
        // reject over-budget sizes before an infallible allocation can abort the process.
        check_gif_budget(w as u64, h as u64, frames as u64).map_err(EngineError::Other)?;
        let delay = (100.0 / fps).round().clamp(1.0, 65535.0) as u16;
        let mut gframes = Vec::with_capacity(frames);
        for (i, &f) in plan.frames.iter().enumerate() {
            let mut doc = base.clone();
            if let Some(t) = &mut doc.timeline {
                t.current = f;
            }
            sync(&mut doc);
            let surf = plan.scaled(crate::file_cmds::flattened(&doc, fmt), bounds);
            let mut px = vec![[0.0f32; 4]; w * h];
            surf.read_rgba_into(out, &mut px);
            gframes.push(gif_frame(px, w, delay)?);
            // The final report follows encoding/writing, so total means the output exists.
            if !progress((i + 1).min(frames.saturating_sub(1)), frames) {
                return Err(cancelled());
            }
        }
        let gif = photocraft_codecs::web::encode_gif_animated(w as u32, h as u32, &gframes, true).map_err(|e| EngineError::Other(e.to_string()))?;
        let name = format!("{stem}.gif");
        write(&name, &gif)?;
        if !progress(frames, frames) {
            return Err(cancelled());
        }
        let path = format!("{dir}/{name}");
        return Ok(json!({"frames": frames, "file": path}));
    }

    // A movie: one file, every frame encoded in turn.
    if is_movie_format(format) {
        // As in the reference application, a document without a timeline has only image sequences.
        if base.timeline.is_none() {
            return Err(EngineError::BadParams {
                cmd: "file.export.renderVideo".into(),
                msg: "A movie needs a video timeline; render an image sequence instead.".into(),
            });
        }
        return render_movie(p, format, &base, &plan, write, progress);
    }

    // Image sequence.
    // PSD / PSB frames would embed the files linked smart objects name: not for a restricted session.
    let opts = photocraft_io::ExportOptions { read_linked_files: s.authorize.is_none(), ..Default::default() };
    let mut files = Vec::new();
    for (i, &f) in plan.frames.iter().enumerate() {
        let mut doc = base.clone();
        if let Some(t) = &mut doc.timeline {
            t.current = f;
        }
        sync(&mut doc);
        // Another size renders the flattened frame, resampled, as a one-layer document.
        if plan.size != (base.size.width, base.size.height) {
            doc = plan.scaled_document(&doc, fmt, bounds);
        }
        // Files are numbered from the first rendered frame, whatever frame the range starts at.
        let path = format!("{stem}_{i:04}.{format}");
        let bytes = photocraft_io::export(&doc, format, &opts).map(|r| r.bytes).map_err(|e| EngineError::Other(format!("render frame {f}: {e}")))?;
        write(&path, &bytes)?;
        files.push(path);
        if !progress(i + 1, frames) {
            return Err(cancelled());
        }
    }
    Ok(json!({"frames": files.len(), "dir": dir}))
}

/// Whether Render Video's `format` names a movie file (one file for every frame) rather than an
/// image format.
fn is_movie_format(format: &str) -> bool {
    format.eq_ignore_ascii_case("mov") || format.eq_ignore_ascii_case("mp4")
}

/// The movie codec Render Video's `preset` (default the format's first) asks for in `format`.
#[cfg(not(target_arch = "wasm32"))]
fn movie_codec(p: &Value, format: &str) -> Result<photocraft_io::movie_out::MovieCodec> {
    use photocraft_io::movie_out::MovieCodec;
    let bad = |msg: String| EngineError::BadParams { cmd: "file.export.renderVideo".into(), msg };
    let preset = match p.get("preset").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => Some(v.as_str().ok_or_else(|| bad("`preset`: A preset name is required.".into()))?),
    };
    let codec = if format.eq_ignore_ascii_case("mov") {
        // The reference application's MOV codec wasn't measured (#14165): the Animation presets
        // write ProRes, which keeps the picture near-losslessly at any size.
        match preset.unwrap_or("animationHigh") {
            "animationHigh" => MovieCodec::ProRes4444,
            "animationMedium" => MovieCodec::ProRes422Hq,
            "uncompressed" => MovieCodec::Uncompressed,
            other => return Err(bad(format!("`preset`: `{other}` isn't a MOV preset (animationHigh, animationMedium or uncompressed)."))),
        }
    } else {
        match preset.unwrap_or("highQuality") {
            "highQuality" => MovieCodec::H264,
            // Their bit rates weren't measured (#14165).
            "mediumQuality" | "lowQuality" => {
                return Err(EngineError::Other(format!("the H.264 preset `{}` isn't available yet; use highQuality", preset.unwrap_or_default())));
            }
            other => return Err(bad(format!("`preset`: `{other}` isn't an H.264 preset (highQuality)."))),
        }
    };
    if !codec.available() {
        return Err(EngineError::Other(format!(
            "{} movies can't be written by this build (whether PhotoCraft ships the encoder is still open, #14165); render a MOV or an image sequence instead",
            codec.name()
        )));
    }
    Ok(codec)
}

/// Render Video's movie output: every frame of `plan`, encoded in turn into one file named after
/// the document.
#[cfg(not(target_arch = "wasm32"))]
fn render_movie(
    p: &Value,
    format: &str,
    base: &Document,
    plan: &RenderPlan,
    mut write: impl FnMut(&str, &[u8]) -> Result<()>,
    mut progress: impl FnMut(usize, usize) -> bool,
) -> Result<Value> {
    let codec = movie_codec(p, format)?;
    let frames = plan.frames.len();
    let fmt = base.pixel_format();
    let bounds = base.bounds();
    let out = Rect::from_xywh(0, 0, plan.size.0, plan.size.1);
    let failed = |e: photocraft_io::movie_out::MovieError| EngineError::Other(format!("render the movie: {e}"));
    let cancelled = || EngineError::Other("render cancelled".into());
    let mut movie = photocraft_io::movie_out::MovieWriter::new(codec, plan.size.0, plan.size.1, plan.fps).map_err(failed)?;
    let mut px = vec![[0.0f32; 4]; out.width() as usize * out.height() as usize];
    for (i, &f) in plan.frames.iter().enumerate() {
        let mut doc = base.clone();
        if let Some(t) = &mut doc.timeline {
            t.current = f;
        }
        sync(&mut doc);
        plan.scaled(crate::file_cmds::flattened(&doc, fmt), bounds).read_rgba_into(out, &mut px);
        movie.push(&px).map_err(|e| EngineError::Other(format!("render frame {f}: {e}")))?;
        // The final report follows writing, so total means the file exists.
        if !progress((i + 1).min(frames.saturating_sub(1)), frames) {
            return Err(cancelled());
        }
    }
    let bytes = movie.finish().map_err(failed)?;
    let name = format!("{}.{}", safe_stem(&base.name), codec.extension());
    write(&name, &bytes)?;
    if !progress(frames, frames) {
        return Err(cancelled());
    }
    let dir = p.get("dir").and_then(Value::as_str).unwrap_or("");
    Ok(json!({"frames": frames, "file": format!("{dir}/{name}"), "codec": codec.name()}))
}

/// The web build writes no movie files: its workspace has no movie encoder.
#[cfg(target_arch = "wasm32")]
fn render_movie(
    _: &Value,
    _: &str,
    _: &Document,
    _: &RenderPlan,
    _: impl FnMut(&str, &[u8]) -> Result<()>,
    _: impl FnMut(usize, usize) -> bool,
) -> Result<Value> {
    Err(EngineError::Other("movie files can't be written in the web build; render an image sequence instead".into()))
}

/// Most pixels a side Render Video's `width` and `height` take, and most frames a second its
/// `fps` takes, as in the reference application.
const MAX_RENDER_SIDE: u32 = 5000;
const MAX_RENDER_FPS: f64 = 5000.0;

/// What Render Video writes: for each output frame, the timeline frame it shows; the output size;
/// and the output rate.
struct RenderPlan {
    frames: Vec<usize>,
    size: (u32, u32),
    fps: f32,
}

impl RenderPlan {
    /// `frame` (document pixels in `bounds`) at the output size: as it is when that is the
    /// document's size, resampled otherwise.
    fn scaled(&self, frame: Surface, bounds: Rect) -> Surface {
        if (bounds.width(), bounds.height()) == self.size {
            return frame;
        }
        let (sx, sy) = (f64::from(self.size.0) / f64::from(bounds.width().max(1)), f64::from(self.size.1) / f64::from(bounds.height().max(1)));
        photocraft_algo::resample::resize_surface_in_canvas(&frame, sx, sy, photocraft_algo::resample::Resample::BicubicAutomatic, bounds)
    }

    /// `doc` flattened, at the output size, as a one-layer document of its mode, depth, profile
    /// and resolution.
    fn scaled_document(&self, doc: &Document, fmt: photocraft_color::PixelFormat, bounds: Rect) -> Document {
        let mut out = Document::new(doc.name.clone(), photocraft_doc::Size::new(self.size.0, self.size.1), doc.mode, doc.depth);
        out.icc_profile = doc.icc_profile.clone();
        out.resolution_dpi = doc.resolution_dpi;
        let mut layer = Layer::raster("Layer 1", fmt);
        layer.content = LayerContent::Raster(self.scaled(crate::file_cmds::flattened(doc, fmt), bounds));
        out.layers.push(layer);
        out
    }
}

/// Render Video's plan from its params: the frames of `range` (timeline frames at `timeline_fps`)
/// sampled at `fps` (default the timeline's) over the same span of time, at `width` × `height`
/// (default `document`'s size).
fn render_plan(p: &Value, range: std::ops::RangeInclusive<usize>, timeline_fps: f32, document: (u32, u32)) -> Result<RenderPlan> {
    let bad = |msg: String| EngineError::BadParams { cmd: "file.export.renderVideo".into(), msg };
    let side = |key: &str, default: u32| -> Result<u32> {
        match p.get(key).filter(|v| !v.is_null()) {
            None => Ok(default),
            Some(v) => v
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| (1..=MAX_RENDER_SIDE).contains(n))
                .ok_or_else(|| bad(format!("`{key}`: An integer between 1 and {MAX_RENDER_SIDE} is required."))),
        }
    };
    let size = (side("width", document.0)?, side("height", document.1)?);
    let fps = match p.get("fps").filter(|v| !v.is_null()) {
        None => f64::from(timeline_fps),
        Some(v) => v
            .as_f64()
            .filter(|f| f.is_finite() && (1.0..=MAX_RENDER_FPS).contains(f))
            .ok_or_else(|| bad(format!("`fps`: A number between 1.000 and {MAX_RENDER_FPS:.3} is required.")))?,
    };
    let (start, len) = (*range.start(), range.end() - range.start() + 1);
    let source = f64::from(timeline_fps);
    let frames = if (fps - source).abs() <= f64::EPSILON * source.max(1.0) {
        range.collect()
    } else {
        // The same span of time at the new rate: output frame k shows the timeline frame on screen
        // at time k / fps.
        let count = ((len as f64) * fps / source).round().max(1.0);
        if count > crate::timeline_cmds::MAX_DURATION as f64 {
            return Err(bad(format!("{len} frames at {fps} fps make more than {} frames", crate::timeline_cmds::MAX_DURATION)));
        }
        (0..count as usize).map(|k| start + ((k as f64 * source / fps + 1e-9).floor() as usize).min(len - 1)).collect()
    };
    Ok(RenderPlan { frames, size, fps: fps as f32 })
}

/// The timeline frames Render Video writes, first to last, from its `range` param: `"all"` (or no
/// `range`) for every frame, `"workArea"` for the timeline's work area, or `{"start": N, "end": M}`
/// for frames N to M, both included. A document without a timeline has one frame, 0.
fn render_range(p: &Value, timeline: Option<&Timeline>) -> Result<std::ops::RangeInclusive<usize>> {
    let bad = |msg: String| EngineError::BadParams { cmd: "file.export.renderVideo".into(), msg };
    let last = timeline.map_or(1, |t| t.duration).max(1) - 1;
    match p.get("range") {
        None | Some(Value::Null) => Ok(0..=last),
        Some(Value::String(s)) if s == "all" => Ok(0..=last),
        Some(Value::String(s)) if s == "workArea" => {
            let mut t = timeline.cloned().ok_or_else(|| bad("`workArea` needs a timeline: the document has one frame".into()))?;
            // A loaded work area may be out of step with the duration.
            t.clamp();
            Ok(t.work_start..=t.work_end - 1)
        }
        Some(Value::Object(o)) => {
            if let Some(key) = o.keys().find(|k| !matches!(k.as_str(), "start" | "end")) {
                return Err(bad(format!("`range` takes `start` and `end`, not `{key}`")));
            }
            let frame = |key: &str| -> Result<usize> {
                o.get(key)
                    .and_then(Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| *n <= last)
                    .ok_or_else(|| bad(format!("`{key}`: An integer between 0 and {last} is required.")))
            };
            let (start, end) = (frame("start")?, frame("end")?);
            if end < start {
                return Err(bad("The range of frames is incorrect.".into()));
            }
            Ok(start..=end)
        }
        Some(v) => Err(bad(format!("`range` must be \"all\", \"workArea\" or {{\"start\": N, \"end\": M}}, not {v}"))),
    }
}

/// One animated GIF frame from straight RGBA pixels, as Save for Web's GIF does (#3061): pixels
/// under 50 % opacity get the frame's transparent index, the rest are drawn opaque, and the
/// palette is built from the visible pixels with one entry kept for transparency.
fn gif_frame(mut px: Vec<[f32; 4]>, w: usize, delay_cs: u16) -> Result<photocraft_codecs::web::GifFrame> {
    use photocraft_algo::quantize::{self, Dither, Forced, PaletteKind};
    let transparent = px.iter().any(|p| p[3] < 0.5);
    let visible: Vec<[f32; 4]> = px.iter().filter(|p| p[3] >= 0.5).map(|p| [p[0], p[1], p[2], 1.0]).collect();
    // A fully transparent frame still needs a palette.
    let src = if visible.is_empty() { &px } else { &visible };
    let mut pal = quantize::build_palette(src, PaletteKind::Adaptive, if transparent { 255 } else { 256 }, Forced::None).map_err(EngineError::Other)?;
    let t = transparent.then(|| {
        pal.truncate(255);
        pal.push(pal.first().copied().unwrap_or([0, 0, 0]));
        pal.len() - 1
    });
    let indices = quantize::quantize(&mut px, w, &pal, Dither::None, 1.0, t);
    Ok(photocraft_codecs::web::GifFrame { indices, palette: pal, transparent: t.map(|i| i as u8), delay_cs })
}

macro_rules! spec {
    ($id:expr, $label:expr, $menu:expr, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: $menu, shortcut: None, params: "{} → {}", enabled: $en, journal: true, run: $run }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    let vl = &["Layer", "Video Layers"] as &[&str];
    vec![
        spec!("layer.videoLayers.newBlankVideoLayer", "New Blank Video Layer", vl, has_doc, |s, p| new_blank(s, p)),
        spec!("layer.videoLayers.insertBlankFrame", "Insert Blank Frame", vl, active_video, |s, p| insert_blank(s, p)),
        spec!("layer.videoLayers.duplicateFrame", "Duplicate Frame", vl, active_video, |s, p| duplicate_frame(s, p)),
        spec!("layer.videoLayers.deleteFrame", "Delete Frame", vl, active_video, |s, p| delete_frame(s, p)),
        spec!("layer.videoLayers.restoreFrame", "Restore Frame", vl, active_video, |s, p| restore_frame(s, p)),
        spec!("layer.videoLayers.restoreAllFrames", "Restore All Frames", vl, active_video, |s, p| restore_all(s, p)),
        spec!("layer.videoLayers.showAlteredVideo", "Show Altered Video", vl, active_video, |s, p| show_altered(s, p)),
        spec!("layer.videoLayers.rasterize", "Rasterize", vl, active_video, |s, p| rasterize(s, p)),
        spec!("layer.rasterize.video", "Video", &["Layer", "Rasterize"], active_video, |s, p| rasterize(s, p)),
        spec!("layer.videoLayers.newVideoLayerFromFile", "New Video Layer from File…", vl, has_doc, |s, p| new_from_file(s, p)),
        spec!("layer.videoLayers.replaceFootage", "Replace Footage…", vl, active_video, |s, p| replace_footage(s, p)),
        spec!("layer.videoLayers.interpretFootage", "Interpret Footage…", vl, active_video, |s, p| interpret_footage(s, p)),
        spec!("layer.videoLayers.reloadFrame", "Reload Frame", vl, active_video, |s, p| reload_frame(s, p)),
        spec!("file.import.videoFramesToLayers", "Video Frames to Layers…", &["File", "Import"], has_doc, |s, p| frames_to_layers(s, p)),
        spec!("file.export.renderVideo", "Render Video…", &["File", "Export"], has_doc, |s, p| render_video(s, p)),
    ]
}

#[cfg(test)]
mod render_video_movie_tests;
#[cfg(test)]
mod render_video_range_tests;
#[cfg(test)]
mod render_video_size_rate_tests;
#[cfg(test)]
mod tests;
