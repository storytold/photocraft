//! Camera raws developed by the shared develop engine (`craft-develop`, LightCraft's pipeline)
//! for the open-time Camera Raw dialog, instead of `photocraft-raw`'s develop ([`crate::raw`]).
//!
//! A raw opened for the dialog ([`crate::ImportOptions::raw_develop`]) opens as
//! [`import_raw_developed`] renders it with the default settings, the dialog previews it from a
//! [`DevelopSource`], and its Open renders the whole frame again with the dialog's settings. The
//! settings stay PhotoCraft's [`CameraRaw`] and are converted by [`crate::develop_bridge`], on the
//! white-balance scale the file's [`SourceInfo`] gives ([`basis`]).
//!
//! The engine develops to linear Rec.2020; renders are asked for in ProPhoto RGB at 16 bits, whose
//! encoding (ROMM primaries, D50 white with Bradford adaptation, a pure 1.8 gamma) is that of
//! `photocraft_cms::Builtin::ProPhotoCompat`, the profile the document carries, so no conversion
//! follows. A file the engine can't develop opens through [`crate::raw`] as before: a variant it
//! only knows the embedded preview of ([`Loaded::from_preview`]) or that fails to load.

use std::sync::{Arc, OnceLock};

use craft_develop::load::{LoadOptions, Loaded, load_raw};
use craft_develop::raster::Rgb32f;
use craft_develop::{DeepSamples, OutputDepth, OutputSpace, Quality, RenderRequest, SourceInfo, StageCache, render_cached};
use craft_develop_model::DevelopSettings;
use photocraft_algo::camera_raw::CameraRaw;
use photocraft_codecs::{self as codecs, ChannelLayout, Image};

use crate::camera_raw_xmp::Crop;
use crate::develop_bridge::{DevelopEdit, WbBasis, to_develop};

use crate::flat::image_to_document;
use crate::raw::DEVELOPED_NOTE;
use crate::{ImportResult, IoError};
/// The base profiles a panel lists (`craft_develop::profiles`).
pub use craft_develop::profiles::{PROFILES, ProfileInfo, profile, profile_groups};

/// Names the engine in a developed raw's import note, after [`DEVELOPED_NOTE`]: what tells the
/// dialog a document came from this module, so it previews with the same engine.
pub const DEVELOP_ENGINE: &str = "craft-develop";

/// Opens a note of a raw [`import_raw_developed`] could not develop with the engine (it was
/// developed by [`crate::raw`] instead).
pub const NOT_DEVELOPED: &str = "not developed with craft-develop";

/// Whether [`import_raw_developed`]'s `warnings` say it fell back to [`crate::raw`]'s develop.
pub fn fell_back(warnings: &[String]) -> bool {
    warnings.iter().any(|w| w.contains(NOT_DEVELOPED))
}

/// Whether a raw's import `warnings` say this module developed it ([`DEVELOP_ENGINE`]).
pub fn developed_here(warnings: &[String]) -> bool {
    warnings.iter().any(|w| w.contains(DEVELOPED_NOTE) && w.contains(DEVELOP_ENGINE))
}

/// The white-balance scale `info`'s settings are edited on: Kelvin about the as-shot white for a
/// raw with a camera colour model, else the relative scale (a fitted look, a rendered source).
pub fn basis(info: &SourceInfo) -> WbBasis {
    if info.raw && !info.relative_wb { WbBasis::Kelvin { as_shot_temp: info.as_shot_temp, as_shot_tint: info.as_shot_tint } } else { WbBasis::Relative }
}

/// The develop settings of Camera Raw settings `cr` for a source described by `info`: a raw's
/// defaults ([`DevelopSettings::for_raw`]) with every control Camera Raw has taken from `cr`.
pub fn settings(info: &SourceInfo, cr: &CameraRaw) -> DevelopSettings {
    to_develop(cr, basis(info), &DevelopSettings::for_raw(info.as_shot_temp, info.as_shot_tint))
}

fn load_options(max_edge: usize) -> LoadOptions<'static> {
    // Remove Hot Pixels, which `DevelopSettings::for_raw` turns on, works on the mosaic at load.
    LoadOptions { max_edge, hot_pixels: Some(craft_develop::sensor::HOT_PIXEL_MAX_SHARE), ..LoadOptions::default() }
}

fn engine_error(e: impl std::fmt::Display) -> IoError {
    IoError::Raw(photocraft_raw::RawError::Malformed(format!("{DEVELOP_ENGINE}: {e}")))
}

/// Loads `bytes` for rendering, at most `max_edge` on the long edge. `Err` when the engine only
/// has the embedded preview of the file (the caller opens it as [`crate::raw`] does).
fn load(bytes: &[u8], max_edge: usize) -> Result<Loaded, IoError> {
    let loaded = load_raw(bytes, &load_options(max_edge)).map_err(engine_error)?;
    if let Some(why) = &loaded.from_preview {
        return Err(engine_error(format!("only the embedded preview: {why}")));
    }
    Ok(loaded)
}

/// `s` rendered from `src` at exactly `w`×`h` (16-bit ProPhoto), as interleaved RGB.
fn render_rgb16(src: &Arc<Rgb32f>, info: &SourceInfo, s: &DevelopSettings, (w, h): (usize, usize), cache: &StageCache) -> Result<Vec<u16>, IoError> {
    craft_develop::check_output_size(w, h).map_err(engine_error)?;
    let req = RenderRequest { space: OutputSpace::ProPhoto, depth: OutputDepth::U16, quality: Quality::Full, ..RenderRequest::exact(w, h) };
    let out = render_cached(src, info, s, &req, cache);
    let deep = out.deep.ok_or_else(|| engine_error("the render has no 16-bit result"))?;
    if (deep.width, deep.height) != (w, h) {
        return Err(engine_error(format!("rendered {}×{}, not the {w}×{h} asked for", deep.width, deep.height)));
    }
    match deep.samples {
        DeepSamples::U16(v) if v.len() == w * h * 3 => Ok(v),
        _ => Err(engine_error("the render's samples are not 16-bit RGB of its size")),
    }
}

/// Interleaved 16-bit RGB as the dialog's straight RGBA floats.
fn rgba(rgb: &[u16]) -> Vec<[f32; 4]> {
    rgb.as_chunks::<3>().0.iter().map(|p| [f32::from(p[0]) / 65535.0, f32::from(p[1]) / 65535.0, f32::from(p[2]) / 65535.0, 1.0]).collect()
}

/// A raw decoded once for the open-time dialog: a proxy-sized copy for the interactive preview and
/// thumbnails, and the whole frame on first use for the 1:1 preview. Each keeps its own
/// [`StageCache`], so a slider drag re-runs only the stages it changes. Shared between the dialog
/// and its workers (`Send + Sync`).
pub struct DevelopSource {
    bytes: Arc<Vec<u8>>,
    proxy: Arc<Rgb32f>,
    info: SourceInfo,
    proxy_cache: StageCache,
    /// For the mask overlay's renders, which differ from the preview's in depth and overlay.
    overlay_cache: StageCache,
    full: OnceLock<Result<(Arc<Rgb32f>, SourceInfo), String>>,
    full_cache: StageCache,
}

impl DevelopSource {
    /// Decodes `bytes` and develops a copy of at most `proxy_edge` pixels on the long edge;
    /// `full_long` is the long edge of the whole frame (noise reduction sizes its radii by it).
    pub fn new(bytes: Arc<Vec<u8>>, proxy_edge: usize, full_long: usize) -> Result<DevelopSource, IoError> {
        let Loaded { image, mut info, .. } = load(&bytes, proxy_edge.max(1))?;
        info.native_long = Some(full_long.max(image.width.max(image.height)));
        Ok(DevelopSource {
            bytes,
            proxy: Arc::new(image),
            info,
            proxy_cache: StageCache::default(),
            overlay_cache: StageCache::default(),
            full: OnceLock::new(),
            full_cache: StageCache::default(),
        })
    }

    /// The scale white balance is edited on ([`basis`]).
    pub fn basis(&self) -> WbBasis {
        basis(&self.info)
    }

    /// The proxy's size as developed (the preview renders it at the size it asks for).
    pub fn proxy_size(&self) -> (usize, usize) {
        (self.proxy.width, self.proxy.height)
    }

    /// What the proxy was decoded with: the file's as-shot white, its white-balance scale.
    pub fn info(&self) -> &SourceInfo {
        &self.info
    }

    /// The develop settings of Camera Raw settings `cr` for this raw ([`settings`]).
    pub fn settings(&self, cr: &CameraRaw) -> DevelopSettings {
        settings(&self.info, cr)
    }

    /// The settings the raw opens with (Camera Raw's defaults on this raw's defaults): what the
    /// developed document shows.
    pub fn defaults(&self) -> DevelopSettings {
        self.settings(&CameraRaw::default())
    }

    /// Develop settings edited directly, starting from Camera Raw settings `cr` (a sidecar's, the
    /// defaults).
    pub fn edit(&self, cr: &CameraRaw) -> DevelopEdit {
        DevelopEdit::new(cr, self.basis(), &DevelopSettings::for_raw(self.info.as_shot_temp, self.info.as_shot_tint))
    }

    /// Auto's tone settings for the proxy under `s`'s white balance.
    pub fn auto_tone(&self, s: &DevelopSettings) -> craft_develop::auto::AutoTone {
        craft_develop::auto::auto_tone(&self.proxy, &self.info, s)
    }

    /// The white (temperature, tint) white-balance `mode` gives this raw.
    pub fn white_balance_for(&self, mode: craft_develop_model::WbMode) -> (f64, f64) {
        craft_develop::auto::white_balance_for(mode, &self.proxy, &self.info)
    }

    /// `cr` rendered from the proxy at exactly `w`×`h`, as straight RGBA in the document's encoding
    /// (ProPhoto, gamma 1.8).
    pub fn develop_proxy(&self, cr: &CameraRaw, w: usize, h: usize) -> Result<Vec<[f32; 4]>, IoError> {
        self.develop_proxy_with(&self.settings(cr), w, h)
    }

    /// Develop settings `s` rendered from the proxy at exactly `w`×`h` ([`Self::develop_proxy`]).
    pub fn develop_proxy_with(&self, s: &DevelopSettings, w: usize, h: usize) -> Result<Vec<[f32; 4]>, IoError> {
        render_rgb16(&self.proxy, &self.info, s, (w, h), &self.proxy_cache).map(|rgb| rgba(&rgb))
    }

    /// The alpha of mask `id` of `s` (0..255 per pixel, row by row) where the proxy renders at
    /// exactly `w`×`h`: what the mask overlay shows. `Err` for a mask `s` doesn't have (or one
    /// without components).
    pub fn mask_alpha(&self, s: &DevelopSettings, id: u32, w: usize, h: usize) -> Result<Vec<u8>, IoError> {
        craft_develop::check_output_size(w, h).map_err(engine_error)?;
        let id16 = u16::try_from(id).map_err(engine_error)?;
        if !s.masks.iter().any(|m| m.id == id && !m.components.is_empty()) {
            return Err(engine_error(format!("no mask {id} to show")));
        }
        let overlay = craft_develop::Overlay::Mask { id: id16, view: craft_develop::MaskView::WhiteOnBlack, color: [255, 255, 255], opacity: 100 };
        let req = RenderRequest { overlay, quality: Quality::Full, ..RenderRequest::exact(w, h) };
        // the mask alone, not the picture under it: a brush stroke being painted re-renders this
        // on every pointer move
        let alpha = craft_develop::mask_alpha_cached(&self.proxy, &self.info, s, &req, &self.overlay_cache, id)
            .ok_or_else(|| engine_error(format!("no mask {id} to show")))?;
        if (alpha.width, alpha.height) != (w, h) {
            return Err(engine_error(format!("rendered {}×{}, not the {w}×{h} asked for", alpha.width, alpha.height)));
        }
        Ok(alpha.data.iter().map(|a| (255.0 * a.clamp(0.0, 1.0)).round() as u8).collect())
    }

    /// Where the proxy's render of `s` at `w`×`h` comes from ([`ViewMap`]): how a point on the
    /// preview maps to mask, spot and red-eye coordinates.
    pub fn view_map(&self, s: &DevelopSettings, w: usize, h: usize) -> ViewMap {
        ViewMap { frame: craft_develop::frame_for(&self.proxy, &self.info, s, true), w: w.max(1), h: h.max(1) }
    }

    /// The colour a colour-range mask samples at `p` (normalized coordinates of the transformed
    /// image, [`ViewMap::to_norm`]) under `s`; `None` outside the image.
    pub fn color_range_sample(&self, s: &DevelopSettings, p: craft_geom::Point, w: usize, h: usize) -> Option<[f64; 3]> {
        craft_develop::check_output_size(w, h).ok()?;
        craft_develop::color_range_sample(&self.proxy, &self.info, s, &RenderRequest::exact(w, h), p)
    }

    /// `cr` rendered from the whole frame at exactly `w`×`h` (the document's size), decoding the
    /// whole frame the first time.
    pub fn develop_full(&self, cr: &CameraRaw, w: usize, h: usize) -> Result<Vec<[f32; 4]>, IoError> {
        self.develop_full_with(&self.settings(cr), w, h)
    }

    /// Develop settings `s` rendered from the whole frame ([`Self::develop_full`]).
    pub fn develop_full_with(&self, s: &DevelopSettings, w: usize, h: usize) -> Result<Vec<[f32; 4]>, IoError> {
        let full = self.full.get_or_init(|| load(&self.bytes, usize::MAX).map(|l| (Arc::new(l.image), l.info)).map_err(|e| e.to_string()));
        let (src, info) = full.as_ref().map_err(engine_error)?;
        render_rgb16(src, info, s, (w, h), &self.full_cache).map(|rgb| rgba(&rgb))
    }
}

/// How the preview of a develop maps to the coordinates its settings store: points on the shown
/// frame (0..1 across its width and height) to a mask's coordinates ([`craft_develop_model::Mask::on_photo`]:
/// on the photo before lens corrections, Upright and Transform, or an old mask's on the corrected
/// frame) and to the normalized coordinates spots and red eyes use, and back.
#[derive(Clone)]
pub struct ViewMap {
    frame: craft_develop::geometry::Frame,
    w: usize,
    h: usize,
}

impl ViewMap {
    fn px(&self, p: craft_geom::Point) -> craft_geom::Point {
        craft_geom::Point::new(p.x * self.w as f64, p.y * self.h as f64)
    }

    fn unit(&self, o: craft_geom::Point) -> craft_geom::Point {
        craft_geom::Point::new(o.x / self.w as f64, o.y / self.h as f64)
    }

    /// The mask coordinates at `p` on the shown frame.
    pub fn to_mask(&self, on_photo: bool, p: craft_geom::Point) -> craft_geom::Point {
        self.frame.for_mask(on_photo).out_to_mask(self.w, self.h).apply(self.px(p))
    }

    /// Where mask coordinates `n` show on the frame; `None` where the warp can't place them.
    pub fn from_mask(&self, on_photo: bool, n: craft_geom::Point) -> Option<craft_geom::Point> {
        self.frame.for_mask(on_photo).mask_to_out(self.w, self.h)(n).map(|o| self.unit(o))
    }

    /// The normalized coordinates (spots, red eyes, colour samples) at `p` on the shown frame.
    pub fn to_norm(&self, p: craft_geom::Point) -> craft_geom::Point {
        self.frame.out_to_norm(self.w, self.h).apply(self.px(p))
    }

    /// Where normalized coordinates `n` show on the frame.
    pub fn from_norm(&self, n: craft_geom::Point) -> craft_geom::Point {
        self.unit(self.frame.norm_to_out(self.w, self.h).apply(n))
    }

    /// The frame's long edge in shown pixels per unit of the image's long edge (radii and brush
    /// sizes are fractions of that edge).
    pub fn px_per_long(&self) -> f64 {
        self.frame.px_per_long(self.w)
    }

    /// The oriented image's sides over its long side `(ow / l, oh / l)`: normalized ↔ long-edge units.
    pub fn long_scale(&self) -> (f64, f64) {
        let l = self.frame.ow.max(self.frame.oh).max(1e-9);
        (self.frame.ow / l, self.frame.oh / l)
    }

    /// The shown frame's size in pixels.
    pub fn size(&self) -> (usize, usize) {
        (self.w, self.h)
    }
}

/// Crops interleaved RGB `w`×`h` to `crop` (fractions of the frame), as [`crate::raw`] crops a
/// develop. Returns the new size.
pub(crate) fn crop_rgb(rgb: &mut Vec<u16>, (w, h): (usize, usize), crop: Crop) -> Result<(usize, usize), IoError> {
    let bad = || engine_error(format!("the developed {w}x{h} frame holds fewer samples than its size"));
    let Some((x0, y0, x1, y1)) = crop.pixels(w, h) else { return Ok((w, h)) };
    let mut out = Vec::with_capacity((x1 - x0) * (y1 - y0) * 3);
    for y in y0..y1 {
        let row = y.checked_mul(w).ok_or_else(bad)?;
        out.extend_from_slice(rgb.get((row + x0) * 3..(row + x1) * 3).ok_or_else(bad)?);
    }
    *rgb = out;
    Ok((x1 - x0, y1 - y0))
}

/// Develops `bytes` with Camera Raw settings `cr` and opens it as a 16-bit ProPhoto document (the
/// same document [`crate::raw::import_raw_tuned`] makes), cropped to the sidecar's `crop` (in the
/// stored orientation) once upright. With the default settings and no crop its note carries
/// [`DEVELOPED_NOTE`] (an open for the dialog). The second value is whether white balance was
/// applied (always: the engine has a white for every raw it develops).
///
/// A raw the engine can't develop opens as [`crate::raw::import_raw_tuned`] opens it, its note
/// saying so.
pub fn import_raw_developed(name: &str, bytes: &[u8], cr: &CameraRaw, crop: Option<Crop>) -> Result<(ImportResult, bool), IoError> {
    import_raw_developed_with(name, bytes, cr, None, crop)
}

/// [`import_raw_developed`] rendering `develop` when given (the dialog's develop settings, edited
/// directly), else `cr` through the bridge. `cr` is what a raw the engine can't develop opens with.
pub fn import_raw_developed_with(
    name: &str,
    bytes: &[u8],
    cr: &CameraRaw,
    develop: Option<&DevelopSettings>,
    crop: Option<Crop>,
) -> Result<(ImportResult, bool), IoError> {
    let format = photocraft_raw::identify(bytes).map(|f| f.name()).unwrap_or("camera raw");
    let loaded = match load(bytes, usize::MAX) {
        Ok(l) => l,
        Err(e) => return fallback(name, bytes, cr, crop, &e),
    };
    let camera = craft_raw::probe_info(bytes).map(|i| crate::raw::camera_name(i.camera.make.as_deref(), i.camera.model.as_deref())).unwrap_or_default();
    let (w, h) = (loaded.image.width, loaded.image.height);
    let src = Arc::new(loaded.image);
    let from_cr = settings(&loaded.info, cr);
    let s = develop.unwrap_or(&from_cr);
    let mut rgb = render_rgb16(&src, &loaded.info, s, (w, h), &StageCache::default())?;
    let (w, h) = match crop {
        Some(c) => crop_rgb(&mut rgb, (w, h), c.oriented(photocraft_raw::orientation(bytes).unwrap_or(1)))?,
        None => (w, h),
    };
    let default = crop.is_none() && *s == settings(&loaded.info, &CameraRaw::default());
    let summary = if default {
        format!("{DEVELOPED_NOTE} ({DEVELOP_ENGINE}, as-shot white balance)")
    } else {
        format!("developed in RAW Develop ({DEVELOP_ENGINE}{})", if crop.is_some() { format!(", cropped to {w}×{h}") } else { String::new() })
    };
    Ok((document(name, bytes, format, &camera, (w, h), &rgb, &summary)?, true))
}

/// [`import_raw_developed`] for a raw the engine can't develop: [`crate::raw`]'s develop, with a
/// note saying why.
fn fallback(name: &str, bytes: &[u8], cr: &CameraRaw, crop: Option<Crop>, why: &IoError) -> Result<(ImportResult, bool), IoError> {
    let (mut r, wb) = if cr.is_identity() && crop.is_none() {
        (crate::raw::import_raw(name, bytes)?, true)
    } else {
        let tuning = crate::raw::RawTuning {
            temperature: cr.temperature,
            tint: cr.tint,
            exposure: cr.exposure,
            shadows: cr.shadows,
            highlights: cr.highlights,
            clarity: cr.clarity,
            texture: cr.texture,
            dehaze: cr.dehaze,
            crop,
            ..Default::default()
        };
        crate::raw::import_raw_tuned(name, bytes, &tuning)?
    };
    // (a file opened as its embedded preview keeps the preview's own note, as before)
    if r.warnings.iter().any(|w| w.contains(photocraft_raw::OUTPUT_SPACE)) {
        r.warnings.push(format!("{NOT_DEVELOPED} ({why}); developed as before"));
    }
    Ok((r, wb))
}

fn document(name: &str, bytes: &[u8], format: &str, camera: &str, (w, h): (usize, usize), rgb: &[u16], summary: &str) -> Result<ImportResult, IoError> {
    let (w32, h32) = (u32::try_from(w).map_err(engine_error)?, u32::try_from(h).map_err(engine_error)?);
    let mut img = Image::from_u16(w32, h32, ChannelLayout::Rgb, rgb)?;
    // The raw's own EXIF (camera, lens, exposure, date) for File Info; no orientation: upright.
    img.meta.exif = codecs::raw_exif(bytes);
    let mut r = image_to_document(name, &img)?;
    r.document.icc_profile = Some(Arc::new(photocraft_cms::Builtin::ProPhotoCompat.profile().to_bytes().to_vec()));
    r.warnings.push(format!(
        "{format}{} {summary} into 16-bit {}",
        if camera.is_empty() { String::new() } else { format!(" from {camera}") },
        photocraft_raw::OUTPUT_SPACE
    ));
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};

    fn dng() -> Vec<u8> {
        let (w, h) = (96, 64);
        let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 60000));
        spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
        spec.build()
    }

    #[test]
    fn the_default_open_is_a_16_bit_prophoto_document_with_the_engines_note() {
        let (r, wb) = import_raw_developed("t.dng", &dng(), &CameraRaw::default(), None).unwrap();
        assert!(wb);
        assert!(developed_here(&r.warnings), "{:?}", r.warnings);
        assert_eq!(r.document.icc_profile.as_deref().map(Vec::as_slice), Some(&photocraft_cms::Builtin::ProPhotoCompat.profile().to_bytes()[..]));
        assert_eq!((r.document.size.width, r.document.size.height), (96, 64));
        assert!(!fell_back(&r.warnings), "{:?}", r.warnings);
    }

    #[test]
    fn settings_change_the_render_and_drop_the_default_note() {
        let bytes = dng();
        let (base, _) = import_raw_developed("t.dng", &bytes, &CameraRaw::default(), None).unwrap();
        let (bright, _) = import_raw_developed("t.dng", &bytes, &CameraRaw { exposure: 1.0, ..Default::default() }, None).unwrap();
        assert!(!developed_here(&bright.warnings) && bright.warnings.iter().any(|w| w.contains(DEVELOP_ENGINE)));
        let mean = |r: &ImportResult| {
            let s = r.document.layers.iter().find_map(|l| l.surface()).unwrap();
            let mut px = vec![[0.0f32; 4]; 96 * 64];
            s.read_rgba_into(photocraft_geom::Rect::new(0, 0, 96, 64), &mut px);
            px.iter().map(|p| p[1]).sum::<f32>() / px.len() as f32
        };
        assert!(mean(&bright) > mean(&base) + 0.02, "{} vs {}", mean(&bright), mean(&base));
    }

    #[test]
    fn a_crop_is_applied_after_the_render() {
        let crop = Crop { top: 0.25, left: 0.25, bottom: 0.75, right: 0.75 };
        let (r, _) = import_raw_developed("t.dng", &dng(), &CameraRaw::default(), Some(crop)).unwrap();
        assert_eq!((r.document.size.width, r.document.size.height), (48, 32));
    }

    #[test]
    fn the_proxy_renders_at_the_size_asked_for_and_matches_the_full_frame() {
        let bytes = Arc::new(dng());
        let source = DevelopSource::new(bytes.clone(), 48, 96).unwrap();
        let proxy = source.develop_proxy(&CameraRaw::default(), 48, 32).unwrap();
        assert_eq!(proxy.len(), 48 * 32);
        let full = source.develop_full(&CameraRaw::default(), 96, 64).unwrap();
        assert_eq!(full.len(), 96 * 64);
        // a 2×2 average of the full frame is close to the proxy
        let at = |x: usize, y: usize| full[y * 96 + x][1];
        let avg = |x: usize, y: usize| (at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1)) / 4.0;
        let err = (0..32).flat_map(|y| (0..48).map(move |x| (x, y))).map(|(x, y)| (avg(x, y) - proxy[y * 48 + x][1]).abs()).sum::<f32>() / (48.0 * 32.0);
        assert!(err < 0.03, "mean difference {err}");
    }

    #[test]
    fn not_a_raw_falls_back_and_says_so() {
        assert!(import_raw_developed("x.dng", b"not a raw", &CameraRaw::default(), None).is_err());
        assert!(DevelopSource::new(Arc::new(b"not a raw".to_vec()), 64, 64).is_err());
    }

    #[test]
    fn the_white_balance_basis_follows_the_source() {
        let mut info = SourceInfo { raw: true, as_shot_temp: 5000.0, as_shot_tint: 3.0, ..SourceInfo::default() };
        assert_eq!(basis(&info), WbBasis::Kelvin { as_shot_temp: 5000.0, as_shot_tint: 3.0 });
        info.relative_wb = true;
        assert_eq!(basis(&info), WbBasis::Relative);
        assert_eq!(basis(&SourceInfo::default()), WbBasis::Relative);
    }

    /// The mask overlay is the alpha of the mask the render evaluates, and the view map takes a
    /// point of the shown frame to the mask's coordinates and back.
    #[test]
    fn a_masks_alpha_and_the_view_map() {
        use craft_develop_model::{Mask, MaskComponent, MaskOp, MaskShape};
        let source = DevelopSource::new(Arc::new(dng()), 96, 96).unwrap();
        let (w, h) = source.proxy_size();
        let mut s = source.defaults();
        let radial = MaskShape::Radial { center: craft_geom::Point::new(0.5, 0.5), rx: 0.2, ry: 0.2, angle: 0.0, feather: 10.0, invert: false };
        s.masks.push(Mask { id: 7, components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape: radial }], ..Mask::default() });
        let a = source.mask_alpha(&s, 7, w, h).unwrap();
        assert_eq!(a.len(), w * h);
        assert!(a[(h / 2) * w + w / 2] > 240, "inside: {}", a[(h / 2) * w + w / 2]);
        assert!(a[0] < 10, "outside: {}", a[0]);
        assert!(source.mask_alpha(&s, 8, w, h).is_err(), "no such mask");
        let map = source.view_map(&s, w, h);
        let p = craft_geom::Point::new(0.25, 0.75);
        let n = map.to_mask(true, p);
        assert!((n.x - 0.25).abs() < 1e-6 && (n.y - 0.75).abs() < 1e-6, "no crop or warp: the same point {n:?}");
        let back = map.from_mask(true, n).unwrap();
        assert!((back.x - p.x).abs() < 1e-6 && (back.y - p.y).abs() < 1e-6);
        let q = map.from_norm(map.to_norm(p));
        assert!((q.x - p.x).abs() < 1e-6 && (q.y - p.y).abs() < 1e-6);
        assert!(map.px_per_long() > 0.0);
        assert!(source.color_range_sample(&s, craft_geom::Point::new(0.5, 0.5), w, h).is_some());
    }

    /// Per-move cost of a long Auto Mask brush stroke at the dialog's preview size: what the
    /// develop view's worker renders for each pointer move (the develop and the overlay's alpha).
    /// `PHOTOCRAFT_LC_RAW=<raw> cargo test --release -p photocraft-io --lib auto_mask_stroke_timing -- --ignored --nocapture`
    #[test]
    #[ignore = "timing on a real raw (PHOTOCRAFT_LC_RAW)"]
    fn auto_mask_stroke_timing() {
        use craft_develop_model::{BrushStroke, Mask, MaskComponent, MaskOp, MaskShape};
        let Ok(path) = std::env::var("PHOTOCRAFT_LC_RAW") else { return };
        let source = DevelopSource::new(Arc::new(std::fs::read(path).unwrap()), 900, 6000).unwrap();
        let (w, h) = source.proxy_size();
        let mut s = source.defaults();
        let auto = std::env::var("PHOTOCRAFT_AUTO_MASK").map_or(true, |v| v != "0");
        let stroke = |n: usize| {
            // a zigzag over the frame's middle, a dab spacing (a quarter of the radius) apart
            let points = (0..n)
                .map(|i| {
                    let t = i as f64 * 0.0075;
                    let x = 0.35 + (t % 0.3);
                    let y = 0.35 + 0.05 * (t / 0.3).floor();
                    craft_geom::Point::new(x, y)
                })
                .collect();
            BrushStroke { points, size: 0.03, auto_mask: auto, ..BrushStroke::default() }
        };
        let adjust = craft_develop_model::LocalAdjustments { exposure: 1.0, ..Default::default() };
        s.masks.push(Mask {
            id: 1,
            adjust,
            components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape: MaskShape::Brush { strokes: vec![] } }],
            ..Mask::default()
        });
        let _ = source.develop_proxy_with(&s, w, h).unwrap();
        let _ = source.mask_alpha(&s, 1, w, h);
        eprintln!("preview {w}x{h}, auto mask {auto}");
        let (mut dev, mut ov) = (Vec::new(), Vec::new());
        for n in 1..=240 {
            s.masks[0].components[0].shape = MaskShape::Brush { strokes: vec![stroke(n)] };
            let t = std::time::Instant::now();
            let _ = source.develop_proxy_with(&s, w, h).unwrap();
            let d = t.elapsed().as_secs_f64() * 1e3;
            let t = std::time::Instant::now();
            let _ = source.mask_alpha(&s, 1, w, h).unwrap();
            let o = t.elapsed().as_secs_f64() * 1e3;
            dev.push(d);
            ov.push(o);
            if n % 40 == 0 {
                let k = n - 40..n;
                let mean = |v: &[f64]| v[k.clone()].iter().sum::<f64>() / 40.0;
                eprintln!("moves {:>3}..{n}: develop {:.1} ms, overlay {:.1} ms per move", n - 39, mean(&dev), mean(&ov));
            }
        }
    }
}
