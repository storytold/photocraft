//! The Home screen's recent files: the framework start screen's thumbnail source and its actions.
//!
//! The start screen draws the cards and rows. This module answers what it asks of the app: each recent file's
//! picture (the live composite of an open document, or a bounded decode of the file on disk as a background job),
//! the version of that picture, and what each action does.

use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

#[cfg(not(target_arch = "wasm32"))]
use craft::ui::primitives::{LoadedPicture, ThumbJob};
use craft::ui::primitives::{Revision, ThumbRender, ThumbRequest, ThumbSource, ThumbnailOptions, Thumbnails};
use craft::ui::shell::{RecentFile, StartAction, ThumbKey};
use egui::ColorImage;

use crate::PhotocraftApp;

/// Minimum time between two live-document thumbnail compositions for the same path: composing runs on the UI
/// thread, so an edited document's preview moves at most once per interval.
#[cfg(all(not(target_arch = "wasm32"), not(test)))]
const OPEN_DOC_REGEN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
#[cfg(all(not(target_arch = "wasm32"), test))]
const OPEN_DOC_REGEN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// How long a stat of a file on disk stays valid: frames in between never touch the disk.
#[cfg(all(not(target_arch = "wasm32"), not(test)))]
const RECHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);
#[cfg(all(not(target_arch = "wasm32"), test))]
const RECHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);

#[cfg(not(target_arch = "wasm32"))]
fn load_thumbnail_catching_panic(load: impl FnOnce() -> Option<ColorImage>) -> Option<ColorImage> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(load)).ok().flatten()
}

/// What a disk file looked like at its last stat: its modified time and size, or `None` when it was missing.
#[cfg(not(target_arch = "wasm32"))]
struct DiskStamp {
    seen: Option<(Option<std::time::SystemTime>, u64)>,
    checked: std::time::Instant,
}

/// What the revision of one path is built from.
#[derive(Default)]
struct Stamp {
    /// Bumped by `invalidate` and `clear`: a new generation is a new revision, so the picture is made again.
    generation: u64,
    /// The open document's revision the preview follows, and when it last moved.
    #[cfg(not(target_arch = "wasm32"))]
    open: Option<(u64, std::time::Instant)>,
    /// The last stat of the file on disk.
    #[cfg(not(target_arch = "wasm32"))]
    disk: Option<DiskStamp>,
}

impl Stamp {
    /// The document revision the preview follows: the document's own, but moving at most once per
    /// `OPEN_DOC_REGEN_INTERVAL`. While it is held a repaint is scheduled, so the last revision is still picked up.
    #[cfg(not(target_arch = "wasm32"))]
    fn held_revision(&mut self, revision: u64, ctx: &egui::Context) -> u64 {
        let now = std::time::Instant::now();
        match self.open {
            Some((held, _)) if held == revision => held,
            Some((held, moved)) if now.duration_since(moved) < OPEN_DOC_REGEN_INTERVAL => {
                ctx.request_repaint_after(OPEN_DOC_REGEN_INTERVAL.saturating_sub(now.duration_since(moved)));
                held
            }
            _ => {
                self.open = Some((revision, now));
                revision
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn held_revision(&mut self, revision: u64, _ctx: &egui::Context) -> u64 {
        revision
    }

    /// The revision of the file on disk, statted again only when the window gained focus or the last stat is older
    /// than `RECHECK_INTERVAL`.
    #[cfg(not(target_arch = "wasm32"))]
    fn disk_revision(&mut self, path: &str, focus_gained: bool) -> Revision {
        let due = focus_gained || self.disk.as_ref().is_none_or(|d| d.checked.elapsed() >= RECHECK_INTERVAL);
        if due {
            let seen = std::fs::metadata(path).ok().map(|m| (m.modified().ok(), m.len()));
            self.disk = Some(DiskStamp { seen, checked: std::time::Instant::now() });
        }
        match self.disk.as_ref().and_then(|d| d.seen) {
            Some((modified, len)) => Revision::of(&(self.generation, modified, len)),
            None => Revision::of(&(self.generation, "missing")),
        }
    }

    /// The web has no files to stat: the picture only follows its generation.
    #[cfg(target_arch = "wasm32")]
    fn disk_revision(&mut self, _path: &str, _focus_gained: bool) -> Revision {
        Revision::of(&(self.generation, "web"))
    }
}

/// The versions of the recent files' pictures. Shared with the app (`PhotocraftApp::thumbnail_cache`), which
/// invalidates a path when its file is saved, removed from the list or pushed out of it.
#[derive(Default)]
pub(crate) struct ThumbnailCache {
    stamps: HashMap<String, Stamp>,
}

impl ThumbnailCache {
    /// Invalidate the cached thumbnail for a single path (e.g. after save, open, or remove).
    pub(crate) fn invalidate(&mut self, path: &str) {
        let stamp = self.stamps.entry(path.to_string()).or_default();
        // The generation survives: it is what makes the next revision differ from the one the store holds.
        *stamp = Stamp { generation: stamp.generation + 1, ..Stamp::default() };
    }

    /// Clear all cached thumbnails (e.g. on Clear Recent Files).
    pub(crate) fn clear(&mut self) {
        for stamp in self.stamps.values_mut() {
            *stamp = Stamp { generation: stamp.generation + 1, ..Stamp::default() };
        }
    }
}

fn lock(cache: &Mutex<ThumbnailCache>) -> MutexGuard<'_, ThumbnailCache> {
    cache.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The recent files' thumbnail source for one pass: the open documents (their live pictures), the shared version
/// cache and whether the window gained focus this pass.
pub(crate) struct RecentThumbs<'a> {
    docs: &'a [craft::ui::contract::Tracked<photocraft_engine::DocState>],
    cache: &'a Mutex<ThumbnailCache>,
    ctx: egui::Context,
    focus_gained: bool,
}

/// The thumbnail source of this pass. The focus gain is the frame's own `WindowFocused(true)` event, which egui
/// gives only to the frame's first pass: a second pass sees no gain and applies the stats the first pass wrote (a
/// path is statted at most once per `RECHECK_INTERVAL`, or once on a focus gain).
pub(crate) fn source<'a>(app: &'a PhotocraftApp, ctx: &egui::Context) -> RecentThumbs<'a> {
    let focus_gained = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(true))));
    RecentThumbs { docs: app.session.documents(), cache: &app.thumbnail_cache, ctx: ctx.clone(), focus_gained }
}

/// The thumbnails the Home screen stores its pictures in.
pub(crate) fn thumbnails(ctx: &egui::Context) -> Thumbnails<ThumbKey> {
    Thumbnails::in_context(ctx, "home-recent", ThumbnailOptions::new())
}

impl RecentThumbs<'_> {
    /// The open document whose path is `path`.
    fn open_doc(&self, path: &str) -> Option<&photocraft_engine::DocState> {
        self.docs.iter().find(|d| d.path.as_deref().is_some_and(|p| paths_match(p, path))).map(|st| &**st)
    }
}

impl ThumbSource<ThumbKey> for RecentThumbs<'_> {
    fn revision(&self, key: &ThumbKey, _request: &ThumbRequest) -> Revision {
        // The template previews are not PhotoCraft's: it lists none.
        let ThumbKey::Recent(path) = key else { return Revision::new(0) };
        let path = path.to_string_lossy();
        let mut cache = lock(self.cache);
        let stamp = cache.stamps.entry(path.to_string()).or_default();
        // An open document's live picture wins over the file on disk.
        if let Some(st) = self.open_doc(&path) {
            let held = stamp.held_revision(st.revision, &self.ctx);
            return Revision::of(&(stamp.generation, held));
        }
        stamp.disk_revision(&path, self.focus_gained)
    }

    fn render(&self, key: &ThumbKey, request: &ThumbRequest) -> ThumbRender {
        let ThumbKey::Recent(path) = key else { return ThumbRender::Blank };
        let path = path.to_string_lossy();
        if let Some(st) = self.open_doc(&path) {
            let rgba = photocraft_compose::thumbnail(&st.doc, request.side);
            return ThumbRender::Ready(ColorImage::from_rgba_unmultiplied([rgba.width as usize, rgba.height as usize], &rgba.pixels));
        }
        // The size comes from the stat the revision was made from: the job decodes the file that stat saw. A missing
        // file has none, and the web has no files to decode.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(len) = lock(self.cache).stamps.get(&*path).and_then(|s| s.disk.as_ref()).and_then(|d| d.seen).map(|(_, len)| len) {
            let path = path.into_owned();
            let job: ThumbJob = Box::new(move |_cancel| {
                Ok(match load_thumbnail_catching_panic(|| load_thumbnail_image_bounded(&path, len)) {
                    Some(img) => LoadedPicture::Image(img),
                    None => LoadedPicture::Blank,
                })
            });
            return ThumbRender::Later(job);
        }
        ThumbRender::Blank
    }
}

/// The first `cap` recent files, newest first as the app keeps them. PhotoCraft stores no opened or created
/// times, so the start screen's dates stay empty.
pub(crate) fn recent_files(app: &PhotocraftApp, cap: usize) -> Vec<RecentFile> {
    app.ui.recent_files.iter().take(cap).map(|p| RecentFile::new(PathBuf::from(p), crate::file_open::display_name(p))).collect()
}

/// Run the start screen's `action` when it is a recent-file or Open action, and report whether it was one. New,
/// templates and pinned folders are the start screen's owner's.
pub(crate) fn act(app: &mut PhotocraftApp, action: &StartAction) -> bool {
    match action {
        StartAction::OpenRecent(path) => {
            let path = path.to_string_lossy();
            if let Err(e) = open_recent_gate(app, &path).and_then(|()| app.open_path(&path)) {
                app.open_failed(&path, &e);
            }
            true
        }
        StartAction::RemoveRecent(path) => {
            app.remove_recent(&path.to_string_lossy());
            true
        }
        StartAction::ClearRecent => {
            app.clear_recent();
            true
        }
        StartAction::Open => {
            let _ = app.open_dialog_file();
            true
        }
        _ => false,
    }
}

/// Check if two path strings match, tolerating slash direction and Windows case differences.
fn paths_match(a: &str, b: &str) -> bool {
    #[cfg(windows)]
    {
        a == b || a.replace('\\', "/").eq_ignore_ascii_case(&b.replace('\\', "/"))
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

/// Downscale an RGBA8 buffer to fit within `max_dim` on its longest side.
#[cfg(not(target_arch = "wasm32"))]
fn downsample_rgba(w: u32, h: u32, src_data: &[u8], max_dim: u32) -> Option<ColorImage> {
    if w == 0 || h == 0 || src_data.len() != (w as usize) * (h as usize) * 4 {
        return None;
    }
    let (tw, th) = if w > max_dim || h > max_dim {
        let scale = (max_dim as f32) / (w.max(h) as f32);
        (((w as f32) * scale).round().max(1.0) as u32, ((h as f32) * scale).round().max(1.0) as u32)
    } else {
        (w, h)
    };
    if tw == w && th == h {
        Some(ColorImage::from_rgba_unmultiplied([w as usize, h as usize], src_data))
    } else {
        let mut thumb_px = vec![0u8; (tw as usize) * (th as usize) * 4];
        for y in 0..th {
            let sy = ((y as f32 / th as f32) * h as f32) as usize;
            for x in 0..tw {
                let sx = ((x as f32 / tw as f32) * w as f32) as usize;
                let src_idx = (sy * w as usize + sx) * 4;
                let dst_idx = (y as usize * tw as usize + x as usize) * 4;
                if src_idx + 4 <= src_data.len()
                    && dst_idx + 4 <= thumb_px.len()
                    && let Some(src) = src_data.get(src_idx..src_idx + 4)
                    && let Some(dst) = thumb_px.get_mut(dst_idx..dst_idx + 4)
                {
                    dst.copy_from_slice(src);
                }
            }
        }
        Some(ColorImage::from_rgba_unmultiplied([tw as usize, th as usize], &thumb_px))
    }
}

/// Read a PSD/PSB thumbnail preview by streaming only the header and image resources section.
///
/// Bounded range reader: inspects the 26-byte header and reads only the resource blocks
/// section (up to 32 MB), extracting resource 1036 (JFIF JPEG) or 1033 without reading
/// the layer records, channel data, or multi-gigabyte merged payload. If no embedded thumbnail
/// exists, fallback composite decoding is only permitted if header dimensions are <= 2048x2048
/// and the total file size on disk is <= 64 MB.
#[cfg(not(target_arch = "wasm32"))]
fn read_psd_thumbnail_bounded(path: &Path, file_len: u64) -> Option<ColorImage> {
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom};

    let mut file = File::open(path).ok()?;

    // 1. Read 26-byte header
    let mut header = [0u8; 26];
    file.read_exact(&mut header).ok()?;

    if &header[0..4] != b"8BPS" {
        return None;
    }
    let version = u16::from_be_bytes([header[4], header[5]]);
    if version != 1 && version != 2 {
        return None;
    }
    let channels = u16::from_be_bytes([header[12], header[13]]);
    let height = u32::from_be_bytes([header[14], header[15], header[16], header[17]]);
    let width = u32::from_be_bytes([header[18], header[19], header[20], header[21]]);
    let depth = u16::from_be_bytes([header[22], header[23]]);

    // 2. Read color mode data length (4 bytes)
    let mut cmd_len_buf = [0u8; 4];
    file.read_exact(&mut cmd_len_buf).ok()?;
    let cmd_len = u32::from_be_bytes(cmd_len_buf);
    if cmd_len > 16 * 1024 * 1024 {
        return None;
    }
    file.seek(SeekFrom::Current(cmd_len as i64)).ok()?;

    // 3. Read image resources section length (4 bytes)
    let mut res_len_buf = [0u8; 4];
    file.read_exact(&mut res_len_buf).ok()?;
    let res_len = u32::from_be_bytes(res_len_buf);

    // 4. Read only the resources section (bounded up to 32 MB and within file length)
    if res_len > 0 && res_len <= 32 * 1024 * 1024 && (res_len as u64) <= file_len {
        let mut res_bytes = vec![0u8; res_len as usize];
        if file.read_exact(&mut res_bytes).is_ok()
            && let Ok((resources, _)) = craft_psd::tiff::resources_from_bytes(&res_bytes)
        {
            let decode_opts = photocraft_codecs::DecodeOptions {
                limits: photocraft_codecs::Limits { max_width: 4096, max_height: 4096, max_pixels: 4096 * 4096, max_alloc: 4096 * 4096 * 4 },
                ..Default::default()
            };
            for r in &resources {
                if (r.id == craft_psd::resources::ids::THUMBNAIL || r.id == craft_psd::resources::ids::THUMBNAIL_PS4)
                    && let Some(jfif) = r.data.get(28..)
                    && let Ok(img) = photocraft_codecs::decode_with(jfif, &decode_opts)
                {
                    let rgba = img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
                    if rgba.data().len() == rgba.width() as usize * rgba.height() as usize * 4 {
                        return downsample_rgba(rgba.width(), rgba.height(), rgba.data(), 256);
                    }
                }
            }
        }
    }

    // 5. Fallback for PSDs without embedded JFIF preview:
    // First, check header dimensions: must be <= 2048x2048 before reading any more of the file!
    if width == 0 || height == 0 || width > 2048 || height > 2048 {
        return None;
    }

    // Second, enforce input budget: if the file size on disk exceeds 64 MB, do NOT allocate/read the whole document
    const MAX_FALLBACK_FILE_SIZE: u64 = 64 * 1024 * 1024;
    if file_len > MAX_FALLBACK_FILE_SIZE {
        return None;
    }

    // Third, guard against uncompressed memory spikes / bombs from high channel counts or 32-bit depth:
    let sample_bytes: u64 = match depth {
        1 | 8 => 1,
        16 => 2,
        32 => 4,
        _ => return None,
    };
    const MAX_FALLBACK_DECODED_BYTES: u64 = 64 * 1024 * 1024;
    let decoded_bytes = (width as u64).checked_mul(height as u64)?.checked_mul(channels as u64)?.checked_mul(sample_bytes)?;
    if decoded_bytes > MAX_FALLBACK_DECODED_BYTES {
        return None;
    }

    // Fourth, within budget and safe dimensions: read file and decode merged composite
    let bytes = std::fs::read(path).ok()?;
    if let Ok(file) = craft_psd::PsdFile::from_bytes(&bytes)
        && let Ok(comp) = photocraft_io::merged_composite(&file)
    {
        let w = file.header.width;
        let h = file.header.height;
        if w > 0 && h > 0 && comp.len() == (w as usize) * (h as usize) {
            let mut raw_bytes = Vec::with_capacity(comp.len() * 4);
            for &[r, g, b, a] in comp.iter() {
                raw_bytes.push((r.clamp(0.0, 1.0) * 255.0).round() as u8);
                raw_bytes.push((g.clamp(0.0, 1.0) * 255.0).round() as u8);
                raw_bytes.push((b.clamp(0.0, 1.0) * 255.0).round() as u8);
                raw_bytes.push((a.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            return downsample_rgba(w, h, &raw_bytes, 256);
        }
    }

    None
}

/// Load thumbnail image with bounded streaming I/O and strict size limits.
#[cfg(not(target_arch = "wasm32"))]
fn load_thumbnail_image_bounded(path: &str, file_len: u64) -> Option<ColorImage> {
    let std_path = Path::new(path);
    let ext = std_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    // 1. PSD / PSB: read header and resources with bounded range reading
    if ext.eq_ignore_ascii_case("psd") || ext.eq_ignore_ascii_case("psb") {
        return read_psd_thumbnail_bounded(std_path, file_len);
    }

    // Enforce 64 MB input budget for all other file types (.pcraft, PNG, JPEG, TIFF, etc.)
    const MAX_THUMBNAIL_FILE_SIZE: u64 = 64 * 1024 * 1024;
    if file_len > MAX_THUMBNAIL_FILE_SIZE {
        return None;
    }

    // Guard against decompression bombs: bound decoder allocation and dimensions up front
    let decode_opts = photocraft_codecs::DecodeOptions {
        limits: photocraft_codecs::Limits { max_width: 4096, max_height: 4096, max_pixels: 4096 * 4096, max_alloc: 4096 * 4096 * 4 },
        ..Default::default()
    };

    // 2. If .pcraft, read the embedded thumbnail PNG
    if ext.eq_ignore_ascii_case("pcraft") {
        if std_path.is_dir() {
            let thumb_path = std_path.join("thumb.png");
            if let Ok(thumb_meta) = std::fs::metadata(&thumb_path)
                && thumb_meta.len() <= MAX_THUMBNAIL_FILE_SIZE
                && let Ok(bytes) = std::fs::read(&thumb_path)
                && let Ok(img) = photocraft_codecs::decode_with(&bytes, &decode_opts)
            {
                let rgba = img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
                if rgba.data().len() == rgba.width() as usize * rgba.height() as usize * 4 {
                    return downsample_rgba(rgba.width(), rgba.height(), rgba.data(), 256);
                }
            }
        } else if let Ok(bytes) = std::fs::read(std_path)
            && let Ok(Some(png_bytes)) = photocraft_format::read_thumbnail(&bytes)
            && let Ok(img) = photocraft_codecs::decode_with(&png_bytes, &decode_opts)
        {
            let rgba = img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
            if rgba.data().len() == rgba.width() as usize * rgba.height() as usize * 4 {
                return downsample_rgba(rgba.width(), rgba.height(), rgba.data(), 256);
            }
        }
        return None;
    }

    // 3. Flat raster formats (PNG, JPG, WebP, TIFF, GIF, BMP, etc.)
    if let Ok(bytes) = std::fs::read(std_path)
        && let Ok(img) = photocraft_codecs::decode_with(&bytes, &decode_opts)
    {
        let (w, h) = img.dimensions();
        // Guard against decompression bombs: don't expand > 4096 images to gigabytes of RGBA
        if w > 0 && h > 0 && w <= 4096 && h <= 4096 {
            let rgba = img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
            return downsample_rgba(w, h, rgba.data(), 256);
        }
    }

    None
}

/// Judge a click that opens the recent file `path` as File › Open Recent's item for it, as the
/// menu does: synthetic input from a client confined to its automation roots can't plant a recent
/// file outside them and click its Home card open. A user's own click is always allowed.
fn open_recent_gate(app: &PhotocraftApp, path: &str) -> Result<(), String> {
    let Some(authorize) = app.services.automation_command.as_ref() else { return Ok(()) };
    if !app.automation_input {
        return Ok(());
    }
    // The Home list is File › Open Recent's first entries, so the card's index there is its id.
    let i = app.ui.recent_files.iter().position(|p| p == path).ok_or_else(|| "no such recent file".to_string())?;
    authorize(&format!("file.openRecent.{i}"), &serde_json::json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_engine::color_cmds::ColorExt;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn panicking_thumbnail_load_returns_no_preview() {
        let loaded = load_thumbnail_catching_panic(|| panic!("simulated decoder panic"));
        assert!(loaded.is_none());
    }

    /// Drive the real Home source and store with one explicitly completed background lane.
    #[cfg(not(target_arch = "wasm32"))]
    fn recent_thumbnail_pass(app: &PhotocraftApp, frames: &mut craft::ui::testing::Frames, key: &ThumbKey) -> craft::ui::primitives::TextureState {
        let ctx = app.ctx().clone();
        let mut state = None;
        frames.frame(Vec::new(), |_| {
            state = Some(thumbnails(&ctx).state(&ctx, &source(app, &ctx), key, &ThumbRequest::for_cell(64.0, 1.0), craft::ui::primitives::Priority::VISIBLE));
        });
        state.expect("the Home thumbnail caller ran")
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn manual_recent_thumbnails(app: &PhotocraftApp) -> (craft::ui::testing::Frames, std::sync::Arc<craft::host::ManualJobs>) {
        let ctx = app.ctx().clone();
        let jobs = std::sync::Arc::new(craft::host::ManualJobs::new());
        // The production accessor uses this same salt; its later options keep this first-created loader.
        Thumbnails::<ThumbKey>::in_context(&ctx, "home-recent", ThumbnailOptions::new().lanes(1).jobs(jobs.clone()));
        let mut frames = craft::ui::testing::Frames::over(ctx);
        frames.record_paint();
        (frames, jobs)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn assert_recent_picture(frames: &mut craft::ui::testing::Frames, state: craft::ui::primitives::TextureState, rgba: [u8; 4]) {
        let craft::ui::primitives::TextureState::Ready(texture) = state else { panic!("the completed decoder must supply a ready picture") };
        assert_eq!((texture.size.width, texture.size.height), (4, 4));
        let mut paint = frames.take_paint().expect("texture uploads were recorded");
        let delta = paint.textures_delta.set.get(&texture.id).and_then(|changes| changes.last()).expect("the real decoder's texture was uploaded");
        let egui::ImageData::Color(image) = &delta.image;
        assert_eq!(image.size, [4, 4]);
        assert!(image.pixels.iter().all(|pixel| pixel.to_array() == rgba), "the picture contains the decoded file's pixels");
        // The headless test inspected the uploads instead of rendering them.
        paint.textures_delta.clear();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_loads_in_background_when_background_jobs_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("background.png");
        std::fs::write(&path, flat_png([20, 60, 100, 255])).unwrap();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = true;
        app.push_recent(&path.to_string_lossy());
        let key = ThumbKey::Recent(path);
        let (mut frames, jobs) = manual_recent_thumbnails(&app);

        assert!(matches!(recent_thumbnail_pass(&app, &mut frames, &key), craft::ui::primitives::TextureState::Loading { stand_in: None, .. }));
        assert_eq!(jobs.waiting(), 1, "the real source returned a background decoder, rather than decoding inline");
        assert!(jobs.run_next(), "the queued decoder actually ran to completion");
        assert_eq!(jobs.waiting(), 0, "no worker remains deferred");
        let state = recent_thumbnail_pass(&app, &mut frames, &key);
        assert_recent_picture(&mut frames, state, [20, 60, 100, 255]);
        assert!(!jobs.run_next(), "the ready picture starts no duplicate job");
        assert!(thumbnails(app.ctx()).shutdown(std::time::Duration::from_secs(2)), "the completed loader has no running work");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_discards_background_result_if_invalidated_while_loading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalidated.png");
        std::fs::write(&path, flat_png([100, 20, 60, 255])).unwrap();
        let path_string = path.to_string_lossy().into_owned();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = true;
        app.push_recent(&path_string);
        let key = ThumbKey::Recent(path);
        let (mut frames, jobs) = manual_recent_thumbnails(&app);

        assert!(matches!(recent_thumbnail_pass(&app, &mut frames, &key), craft::ui::primitives::TextureState::Loading { .. }));
        let before = revision_of(&app, &path_string);
        assert!(jobs.run_next(), "finish the actual decoder before invalidation and before its result is uploaded");
        assert_eq!(jobs.waiting(), 0);
        lock(&app.thumbnail_cache).invalidate(&path_string);
        assert_ne!(before, revision_of(&app, &path_string), "invalidation must change the real source's revision");
        // Check invalidation with unchanged metadata first, then make stale and fresh decoder pixels distinct.
        std::fs::write(&path_string, flat_png([20, 100, 60, 255])).unwrap();

        // The former picture may be a stand-in, but it cannot satisfy the invalidated revision as Ready.
        assert!(
            matches!(recent_thumbnail_pass(&app, &mut frames, &key), craft::ui::primitives::TextureState::Loading { .. }),
            "the completed old result is not current"
        );
        assert_eq!(jobs.waiting(), 1, "the production source requests a replacement decoder");
        assert!(jobs.run_next(), "the replacement decoder completes");
        let state = recent_thumbnail_pass(&app, &mut frames, &key);
        assert_recent_picture(&mut frames, state, [20, 100, 60, 255]);
        assert_eq!(jobs.waiting(), 0);
        assert!(!jobs.run_next());
        assert!(thumbnails(app.ctx()).shutdown(std::time::Duration::from_secs(2)), "both decoder jobs have terminated");
    }

    /// The thumbnail source of a fresh pass over `app`.
    #[cfg(not(target_arch = "wasm32"))]
    fn revision_of(app: &PhotocraftApp, path: &str) -> Revision {
        let ctx = app.ctx().clone();
        source(app, &ctx).revision(&ThumbKey::Recent(PathBuf::from(path)), &ThumbRequest::for_cell(64.0, 1.0))
    }

    /// An app with a 64 x 64 document open at `path`, at document revision 0.
    #[cfg(not(target_arch = "wasm32"))]
    fn app_with_open_document(path: &str) -> Box<PhotocraftApp> {
        use photocraft_doc::{ColorMode, Document, SampleType, Size};

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.push_recent(path);
        let doc = Document::new("open_doc.psd", Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
        app.session.open_document(doc, Some(path.to_string()));
        if let Some(st) = app.session.active_mut() {
            st.path = Some(path.to_string());
            st.revision = 0;
        }
        app
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn set_document_revision(app: &mut PhotocraftApp, revision: u64) {
        if let Some(st) = app.session.active_mut() {
            st.revision = revision;
        }
    }

    /// A 4 x 4 PNG of one colour.
    #[cfg(not(target_arch = "wasm32"))]
    fn flat_png(rgba: [u8; 4]) -> Vec<u8> {
        let img = photocraft_codecs::Image::from_raw(4, 4, photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8, rgba.repeat(16)).unwrap();
        photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap()
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_updates_when_open_document_revision_changes() {
        let path = "/tmp/photocraft-test/open_doc.psd";
        let mut app = app_with_open_document(path);
        let first = revision_of(&app, path);

        // An edit to the open document, once the hold interval has passed, moves the revision.
        set_document_revision(&mut app, 1);
        std::thread::sleep(OPEN_DOC_REGEN_INTERVAL + std::time::Duration::from_millis(20));
        let second = revision_of(&app, path);
        assert_ne!(first, second, "an edited open document must get a new picture");
        assert_eq!(second, revision_of(&app, path), "the same document revision keeps its revision");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn open_document_thumbnail_regeneration_is_rate_limited() {
        let path = "/tmp/photocraft-test/open_doc_throttle.psd";
        let mut app = app_with_open_document(path);
        let first = revision_of(&app, path);

        // Rapid edits: the previous revision is served instead of composing every frame.
        for rev in 1..=5 {
            set_document_revision(&mut app, rev);
            assert_eq!(revision_of(&app, path), first, "revision {rev} within the interval keeps the held one");
        }

        // Once the interval has passed the latest revision is taken.
        std::thread::sleep(OPEN_DOC_REGEN_INTERVAL + std::time::Duration::from_millis(20));
        let latest = revision_of(&app, path);
        assert_ne!(latest, first);
        set_document_revision(&mut app, 6);
        assert_eq!(revision_of(&app, path), latest, "a new edit right after the move is held again");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_regenerates_when_disk_file_changes() {
        let temp_dir = std::env::temp_dir().join(format!("photocraft_thumb_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("test_file.png");
        let path_str = file_path.to_string_lossy().into_owned();
        std::fs::write(&file_path, flat_png([255, 0, 0, 255])).unwrap();

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.push_recent(&path_str);
        let before = revision_of(&app, &path_str);

        // A larger multi-colour PNG changes the size on disk.
        let mut px = Vec::with_capacity(32 * 32 * 4);
        for i in 0..(32 * 32) {
            px.extend_from_slice(&[(i % 256) as u8, ((i * 7) % 256) as u8, ((i * 13) % 256) as u8, 255]);
        }
        let img = photocraft_codecs::Image::from_raw(32, 32, photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8, px).unwrap();
        let bigger = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap();
        std::fs::write(&file_path, bigger).unwrap();

        // Let the recheck interval elapse (10 ms in tests).
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_ne!(before, revision_of(&app, &path_str), "the revision must follow the file's new size");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_recheck_interval_avoids_per_frame_disk_io() {
        let temp_dir = std::env::temp_dir().join(format!("photocraft_fast_thumb_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("test_file.png");
        let path_str = file_path.to_string_lossy().into_owned();
        std::fs::write(&file_path, flat_png([0, 255, 0, 255])).unwrap();

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.push_recent(&path_str);
        let first = revision_of(&app, &path_str);

        // A stat would see the missing file and change the revision, so an unchanged one inside the interval
        // (read by a second pass over the same app) shows the disk was not read.
        std::fs::remove_file(&file_path).unwrap();
        assert_eq!(revision_of(&app, &path_str), first, "a pass within the recheck interval must not touch the disk");

        std::thread::sleep(RECHECK_INTERVAL + std::time::Duration::from_millis(10));
        assert_ne!(revision_of(&app, &path_str), first, "after the interval the missing file is seen");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_skips_composite_for_oversized_psd_without_preview() {
        let temp_dir = std::env::temp_dir().join(format!("craft_psd_bound_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("oversized.psd");
        let path_str = file_path.to_string_lossy().into_owned();

        // Create an oversized PSD (3000 x 3000) with no thumbnail resource
        let header = craft_psd::Header::new(craft_psd::Version::Psd, 3000, 3000, 3, 8, craft_psd::ColorMode::Rgb);
        let file = craft_psd::PsdFile {
            header,
            color_mode_data: Vec::new(),
            resources: Vec::new(),
            layer_info: None,
            layer_info_placement: craft_psd::LayerInfoPlacement::Section,
            global_layer_mask: None,
            global_blocks: Vec::new(),
            layer_mask_trailing: Vec::new(),
            image_data: craft_psd::ImageData { compression: craft_psd::Compression::Raw, data: Vec::new(), stored: None },
            write_as_psb: false,
        };
        let mut bytes = file.to_bytes().unwrap();
        // Append a 2 MB dummy/corrupt image payload to simulate a real multi-megabyte payload
        bytes.extend_from_slice(&vec![0xAA; 2 * 1024 * 1024]);
        std::fs::write(&file_path, &bytes).unwrap();

        // load_thumbnail_image_bounded should return None due to <= 2048 bound checked from 26-byte header,
        // without attempting to read or parse the 2 MB payload.
        let thumb = load_thumbnail_image_bounded(&path_str, bytes.len() as u64);
        assert!(thumb.is_none(), "oversized PSD with payload must skip composite fallback");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_skips_composite_for_high_channel_psd_without_preview() {
        let temp_dir = std::env::temp_dir().join(format!("craft_psd_channels_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("high_channels.psd");
        let path_str = file_path.to_string_lossy().into_owned();

        // 2000 x 2000 with 56 channels: decoded uncompressed size = 2000 * 2000 * 56 = 224 MB (> 64 MB cap)
        let header = craft_psd::Header::new(craft_psd::Version::Psd, 2000, 2000, 56, 8, craft_psd::ColorMode::Rgb);
        let file = craft_psd::PsdFile {
            header,
            color_mode_data: Vec::new(),
            resources: Vec::new(),
            layer_info: None,
            layer_info_placement: craft_psd::LayerInfoPlacement::Section,
            global_layer_mask: None,
            global_blocks: Vec::new(),
            layer_mask_trailing: Vec::new(),
            image_data: craft_psd::ImageData { compression: craft_psd::Compression::Raw, data: Vec::new(), stored: None },
            write_as_psb: false,
        };
        let bytes = file.to_bytes().unwrap();
        std::fs::write(&file_path, &bytes).unwrap();

        let thumb = load_thumbnail_image_bounded(&path_str, bytes.len() as u64);
        assert!(thumb.is_none(), "high-channel PSD exceeding decoded memory budget must skip composite fallback");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_extracts_embedded_preview_from_large_psd_without_reading_payload() {
        let temp_dir = std::env::temp_dir().join(format!("craft_psd_large_preview_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("large_payload.psd");
        let path_str = file_path.to_string_lossy().into_owned();

        let img = photocraft_codecs::Image::from_raw(
            512,
            512,
            photocraft_codecs::ChannelLayout::Rgb,
            photocraft_codecs::SampleType::U8,
            [255, 0, 0].repeat(512 * 512),
        )
        .unwrap();
        let jfif = photocraft_codecs::encode(&img, photocraft_codecs::Format::Jpeg, &photocraft_codecs::EncodeOptions::default()).unwrap();
        let mut res_data = vec![0u8; 28];
        res_data[0..4].copy_from_slice(&1u32.to_be_bytes()); // format: 1 (kJpegRGB)
        res_data[4..8].copy_from_slice(&512u32.to_be_bytes()); // width: 512
        res_data[8..12].copy_from_slice(&512u32.to_be_bytes()); // height: 512
        res_data.extend_from_slice(&jfif);
        let resource = craft_psd::resources::ImageResource::new(craft_psd::resources::ids::THUMBNAIL, res_data);

        let header = craft_psd::Header::new(craft_psd::Version::Psd, 400, 400, 3, 8, craft_psd::ColorMode::Rgb);
        let file = craft_psd::PsdFile {
            header,
            color_mode_data: Vec::new(),
            resources: vec![resource],
            layer_info: None,
            layer_info_placement: craft_psd::LayerInfoPlacement::Section,
            global_layer_mask: None,
            global_blocks: Vec::new(),
            layer_mask_trailing: Vec::new(),
            image_data: craft_psd::ImageData { compression: craft_psd::Compression::Raw, data: Vec::new(), stored: None },
            write_as_psb: false,
        };
        let mut bytes = file.to_bytes().unwrap();
        // Append 3 MB of corrupt payload bytes after the resources section
        bytes.extend_from_slice(&vec![0xEE; 3 * 1024 * 1024]);
        std::fs::write(&file_path, &bytes).unwrap();

        // The bounded streaming range reader must extract the embedded 1036 JFIF preview
        // from the initial resources section without erroring on or allocating the 3 MB trailing payload.
        let thumb = load_thumbnail_image_bounded(&path_str, bytes.len() as u64);
        assert!(thumb.is_some(), "must extract embedded JFIF preview despite trailing multi-megabyte payload");
        let thumb_img = thumb.unwrap();
        assert_eq!(thumb_img.size, [256, 256], "embedded previews must be downsampled before texture creation");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_skips_flat_image_larger_than_4k_dimensions() {
        let temp_dir = std::env::temp_dir().join(format!("photocraft_huge_img_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("huge_img.png");
        let path_str = file_path.to_string_lossy().into_owned();

        // Write a PNG with 4097x1 dimensions (exceeds 4096 cap on longest side)
        let img = photocraft_codecs::Image::from_raw(4097, 1, photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8, vec![0u8; 4097 * 4])
            .unwrap();
        let png = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap();
        std::fs::write(&file_path, &png).unwrap();

        let thumb = load_thumbnail_image_bounded(&path_str, png.len() as u64);
        assert!(thumb.is_none(), "flat images exceeding 4096 on longest side must be skipped");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_skips_oversized_thumb_in_pcraft_dir() {
        let temp_dir = std::env::temp_dir().join(format!("photocraft_oversized_pcraft_test_{}.pcraft", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let thumb_path = temp_dir.join("thumb.png");
        let path_str = temp_dir.to_string_lossy().into_owned();

        // Create a 65 MiB thumb.png (exceeds 64 MiB input budget) using set_len (instant sparse file)
        let f = std::fs::File::create(&thumb_path).unwrap();
        f.set_len(65 * 1024 * 1024).unwrap();
        drop(f);

        // Directory metadata len is small (typically 0 or 4096), but the inner thumb.png exceeds budget
        let dir_len = std::fs::metadata(&temp_dir).map(|m| m.len()).unwrap_or(0);
        let thumb = load_thumbnail_image_bounded(&path_str, dir_len);
        assert!(thumb.is_none(), "oversized thumb.png in .pcraft directory must be rejected before reading");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
