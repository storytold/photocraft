//! Camera raw files via `photocraft-raw`: developed to a 16-bit RGB document
//! in ProPhoto RGB (the embedded profile is the built-in ProPhoto-compatible
//! profile), or, for raw variants not decoded yet, the camera's embedded
//! JPEG preview with a warning.
//!
//! Files without colour calibration (everything but DNG) get their colour
//! from the camera's own embedded JPEG: `photocraft-raw` cannot decode JPEG,
//! so the preview is decoded here and handed to [`photocraft_raw::fit_look`],
//! which fits a matrix and tone curve and keeps the neutral fallback when the
//! fit is not clearly better.

use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use photocraft_cms::{Builtin, Intent, Profile, Transform};
use photocraft_codecs::{self as codecs, ChannelLayout, Format, Image, SampleType};
pub use photocraft_raw::Demosaic;
use photocraft_raw::{CameraLook, DevelopOptions, Limits, PreviewColorSpace, RawError, Reference, Sensor, WhiteBalance};
pub use photocraft_raw::{EXTENSIONS, Tone, retone};

use crate::flat::image_to_document;
use crate::{ImportResult, IoError};

/// `true` if `bytes` are a camera raw file `photocraft-raw` recognises.
pub fn is_raw(bytes: &[u8]) -> bool {
    photocraft_raw::is_raw(bytes)
}

/// The decode limits shared with the flat codecs.
fn limits() -> Limits {
    let l = codecs::Limits::default();
    Limits { max_width: l.max_width, max_height: l.max_height, max_pixels: l.max_pixels, max_alloc: l.max_alloc }
}

/// A raw file's embedded JPEG preview, turned upright. Its own EXIF orientation wins when it
/// has one; otherwise the raw's IFD0 orientation applies (TIFF-based raws record it there, and
/// their previews are usually stored as the sensor reads out). Developed raws are oriented by
/// `photocraft-raw` and their EXIF carries no orientation, so nothing is turned twice.
fn upright_preview(raw: &[u8], jpeg: &[u8], limits: &photocraft_raw::Limits) -> Result<Image, IoError> {
    // Decode the embedded preview under the caller's limits, not the codec defaults: an
    // embedded JPEG can declare far more decoded pixels than the raw development the caller
    // authorized, so the preview fallback must honor the same policy.
    let codec_limits =
        codecs::Limits { max_width: limits.max_width, max_height: limits.max_height, max_pixels: limits.max_pixels, max_alloc: limits.max_alloc };
    let img = codecs::decode_as_with(Format::Jpeg, jpeg, &codecs::DecodeOptions { keep_orientation: true, limits: codec_limits })?;
    let own = img.meta.exif.as_deref().map_or(1, codecs::exif_orientation);
    let o = if own != 1 { own } else { photocraft_raw::orientation(raw).unwrap_or_else(|| codecs::exif_orientation(raw)) };
    Ok(img.oriented(o)?)
}

/// Camera Raw's Temperature, Tint, Exposure, Shadows, Highlights, Clarity, Texture and Dehaze,
/// applied while developing the sensor data (the open-time Camera Raw dialog) instead of to the
/// developed RGB pixels, and the demosaic method.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RawTuning {
    /// −100…100, as `photocraft_algo::camera_raw::CameraRaw::temperature`.
    pub temperature: f32,
    /// −100…100, as `CameraRaw::tint`.
    pub tint: f32,
    /// EV, as `CameraRaw::exposure`.
    pub exposure: f32,
    pub demosaic: Demosaic,
    /// −100…100, as `CameraRaw::shadows`: in linear light before the tone curve, as Camera Raw
    /// does for raw files ([`photocraft_raw::Tone`]).
    pub shadows: f32,
    /// −100…100, as `CameraRaw::highlights`.
    pub highlights: f32,
    /// −100…100, as `CameraRaw::clarity`.
    pub clarity: f32,
    /// −100…100, as `CameraRaw::texture`.
    pub texture: f32,
    /// −100…100, as `CameraRaw::dehaze`.
    pub dehaze: f32,
}

/// Develops a raw file with `tuning` applied at the raw stage: Exposure as develop exposure,
/// Shadows, Highlights, Clarity, Texture and Dehaze in linear light before the tone curve, Temperature / Tint as gains on the
/// camera's as-shot white-balance multipliers (the same gains the Camera Raw filter applies to
/// RGB). The second value is `false` when the file has
/// no as-shot white balance: Temperature / Tint were then not applied, and the caller keeps
/// them as RGB adjustments.
pub fn import_raw_tuned(name: &str, bytes: &[u8], tuning: &RawTuning) -> Result<(ImportResult, bool), IoError> {
    import_raw_tuned_inner(name, bytes, tuning, None)
}

/// [`import_raw_tuned`] from what the open already learned about `bytes` ([`take_kept_decode`],
/// [`RawDecode::new`]): the same pixels without decoding the file and fitting its look again.
/// A `decode` of another file is ignored.
pub fn import_raw_tuned_from(name: &str, bytes: &[u8], tuning: &RawTuning, decode: &RawDecode) -> Result<(ImportResult, bool), IoError> {
    import_raw_tuned_inner(name, bytes, tuning, Some(decode))
}

fn import_raw_tuned_inner(name: &str, bytes: &[u8], tuning: &RawTuning, decode: Option<&RawDecode>) -> Result<(ImportResult, bool), IoError> {
    let format = photocraft_raw::identify(bytes).map(|f| f.name()).unwrap_or("camera raw");
    let exposure = f64::from(tuning.exposure);
    if !exposure.is_finite() || !(-10.0..=10.0).contains(&exposure) {
        return Err(IoError::Raw(RawError::Malformed(format!("exposure {} EV is out of range", tuning.exposure))));
    }
    let fresh;
    let decode = match decode.filter(|d| d.is_of(bytes)) {
        Some(d) => d,
        None => {
            fresh = RawDecode::new(bytes)?;
            &fresh
        }
    };
    let sensor = &decode.sensor;
    let slider = |v: f32| if v.is_finite() { v.clamp(-100.0, 100.0) } else { 0.0 };
    let tone = photocraft_raw::Tone {
        shadows: slider(tuning.shadows),
        highlights: slider(tuning.highlights),
        clarity: slider(tuning.clarity),
        texture: slider(tuning.texture),
        dehaze: slider(tuning.dehaze),
    };
    // The colour rendering the default open used (a look fitted to the camera's JPEG for files
    // without colour calibration), so re-developing changes only what the sliders change.
    let camera_look = decode.look.clone();
    let mut opts = DevelopOptions { limits: limits(), exposure, demosaic: tuning.demosaic, tone, camera_look, ..Default::default() };
    let wb_applied = match sensor.camera_wb {
        Some(m) => {
            let g = wb_gains(tuning.temperature, tuning.tint);
            opts.white_balance = WhiteBalance::Multipliers([m[0] * g[0], m[1] * g[1], m[2] * g[2]]);
            true
        }
        None => false,
    };
    let dev = photocraft_raw::develop_sensor(sensor, &opts).map_err(IoError::Raw)?;
    let wb = if wb_applied {
        format!("as-shot white balance, temperature {:+.0}, tint {:+.0}", tuning.temperature, tuning.tint)
    } else {
        "estimated white balance".into()
    };
    let tone = [("shadows", tone.shadows), ("highlights", tone.highlights), ("clarity", tone.clarity), ("texture", tone.texture), ("dehaze", tone.dehaze)]
        .iter()
        .filter(|(_, v)| *v != 0.0)
        .map(|(k, v)| format!(", {k} {v:+.0}"))
        .collect::<String>();
    let summary = format!("developed in Camera Raw (exposure {:+.2} EV{tone}, {wb}, {} demosaic)", tuning.exposure, tuning.demosaic.id());
    Ok((developed_document(name, bytes, format, dev, &summary)?, wb_applied))
}

/// The tone curve `bytes` develop with by default: the fitted look's for a file without colour
/// calibration (see [`import_raw_with`]), else the camera profile's (empty: none, or not a raw
/// this crate decodes). For previews of the open-time raw-stage sliders
/// ([`photocraft_raw::retone`]), in linear-light knots.
pub fn tone_curve(bytes: &[u8]) -> Vec<[f32; 2]> {
    RawDecode::new(bytes).map(|d| d.tone_curve).unwrap_or_default()
}

/// A raw file decoded once for the open-time Camera Raw dialog: the sensor data, the look fitted
/// to the camera's JPEG (for a file without colour calibration, see [`default_look`]) and the
/// tone curve the default develop uses. The dialog's preview and its Open re-develop
/// ([`import_raw_tuned_from`]) share one instead of decoding and fitting the file again each
/// (the sensor is 2 bytes per pixel: drop it when the dialog closes).
pub struct RawDecode {
    fingerprint: Fingerprint,
    sensor: Sensor,
    look: Option<CameraLook>,
    tone_curve: Vec<[f32; 2]>,
}

impl std::fmt::Debug for RawDecode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Not the sensor data: tens of millions of samples.
        f.debug_struct("RawDecode").field("fingerprint", &self.fingerprint).field("look", &self.look.is_some()).finish_non_exhaustive()
    }
}

impl RawDecode {
    /// Decodes `bytes` and fits its look, as a default open does.
    pub fn new(bytes: &[u8]) -> Result<RawDecode, IoError> {
        let sensor = decode_sensor(bytes)?;
        let look = default_look(bytes, &sensor);
        Ok(RawDecode::from_parts(bytes, sensor, look))
    }

    fn from_parts(bytes: &[u8], sensor: Sensor, look: Option<CameraLook>) -> RawDecode {
        let tone_curve = match &look {
            Some(look) => look.tone.as_ref().map(look_knots).unwrap_or_default(),
            None => sensor.tone_curve.clone(),
        };
        RawDecode { fingerprint: Fingerprint::of(bytes), sensor, look, tone_curve }
    }

    /// The tone curve a default develop uses, as [`tone_curve`] returns it.
    pub fn tone_curve(&self) -> &[[f32; 2]] {
        &self.tone_curve
    }

    /// `true` if this was decoded from `bytes`.
    fn is_of(&self, bytes: &[u8]) -> bool {
        self.fingerprint == Fingerprint::of(bytes)
    }
}

/// The sensor data of `bytes` under the shared limits.
fn decode_sensor(bytes: &[u8]) -> Result<Sensor, IoError> {
    DECODES.with(|n| n.set(n.get() + 1));
    photocraft_raw::decode(bytes, &limits()).map_err(IoError::Raw)
}

thread_local! {
    static DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many raw files this thread has decoded so far (for tests that count decodes).
#[doc(hidden)]
pub fn decodes_on_this_thread() -> usize {
    DECODES.with(std::cell::Cell::get)
}

/// A file's length and a 64-bit digest of its bytes: tells the file a [`RawDecode`] was made
/// from from any other (not a cryptographic hash).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fingerprint(usize, u64);

impl Fingerprint {
    fn of(bytes: &[u8]) -> Fingerprint {
        const K: u64 = 0x9E37_79B9_7F4A_7C15;
        let mix = |lane: u64, word: u64| (lane ^ word).wrapping_mul(K).rotate_left(29);
        let mut lanes = [0x243F_6A88_85A3_08D3u64, 0x1319_8A2E_0370_7344, 0xA409_3822_299F_31D0, 0x082E_FA98_EC4E_6C89];
        let (blocks, rest) = bytes.as_chunks::<32>();
        let mut absorb = |block: &[u8; 32]| {
            for (lane, word) in lanes.iter_mut().zip(block.as_chunks::<8>().0) {
                *lane = mix(*lane, u64::from_le_bytes(*word));
            }
        };
        blocks.iter().for_each(&mut absorb);
        let mut tail = [0u8; 32];
        tail[..rest.len()].copy_from_slice(rest);
        absorb(&tail);
        let digest = lanes.iter().fold(bytes.len() as u64, |acc, &lane| mix(acc, lane));
        Fingerprint(bytes.len(), digest)
    }
}

/// The decodes of the last opens that await their Camera Raw dialog (see
/// [`ImportOptions::keep_raw`](crate::ImportOptions::keep_raw)): at most two, so several raws
/// opened at once each find theirs, and the oldest goes first. The dialog takes its own out.
static KEPT: Mutex<Vec<Arc<RawDecode>>> = Mutex::new(Vec::new());

const KEPT_MAX: usize = 2;

fn kept() -> std::sync::MutexGuard<'static, Vec<Arc<RawDecode>>> {
    KEPT.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Hands over (removes) the decode a default open of exactly `bytes` kept for its dialog, if it
/// is still there. Whoever takes it owns the sensor data: dropping it frees it.
pub fn take_kept_decode(bytes: &[u8]) -> Option<Arc<RawDecode>> {
    let want = Fingerprint::of(bytes);
    let mut kept = kept();
    let at = kept.iter().position(|d| d.fingerprint == want)?;
    Some(kept.remove(at))
}

/// A fitted look's tone curve as knots for [`photocraft_raw::retone`] (which reads them
/// log-linearly): sampled densely enough that the two agree within a fraction of a code value.
fn look_knots(t: &photocraft_raw::ToneCurve) -> Vec<[f32; 2]> {
    const N: usize = 256;
    (1..=N).map(|i| (i as f64 / N as f64).powi(2)).map(|x| [x as f32, t.eval(x) as f32]).collect()
}

/// The look a default open develops `bytes` with: fitted to the embedded JPEG when the file has
/// no colour calibration, is not monochrome and the fit clearly helps (as [`import_raw_with`]
/// does), else `None`. It is fitted at the camera's own white balance and exposure, so a
/// re-develop's sliders are not fitted away.
fn default_look(bytes: &[u8], sensor: &Sensor) -> Option<CameraLook> {
    if !sensor.color.calibrations.is_empty() || sensor.is_monochrome() {
        return None;
    }
    fit_preview_look(bytes, sensor, &DevelopOptions { limits: limits(), ..Default::default() }).ok()
}

/// "Canon" + "Canon EOS 80D" and "NIKON CORPORATION" + "NIKON D3200" read as the model alone:
/// the model already starts with the make's first word.
fn camera_name(make: Option<&str>, model: Option<&str>) -> String {
    match (make, model) {
        (Some(make), Some(model))
            if make.split_whitespace().next().is_some_and(|brand| model.to_ascii_lowercase().starts_with(&brand.to_ascii_lowercase())) =>
        {
            model.to_string()
        }
        (make, model) => [make, model].into_iter().flatten().collect::<Vec<_>>().join(" "),
    }
}

/// Camera Raw's white-balance gains for Temperature / Tint (−100…100), the same the Camera
/// Raw filter applies ([`photocraft_algo::camera_raw::white_balance_gains`]).
fn wb_gains(temperature: f32, tint: f32) -> [f64; 3] {
    photocraft_algo::camera_raw::white_balance_gains(temperature.clamp(-100.0, 100.0), tint.clamp(-100.0, 100.0)).map(f64::from)
}

/// A developed raw as a 16-bit ProPhoto document, with a note saying how it was developed.
fn developed_document(name: &str, bytes: &[u8], format: &str, dev: photocraft_raw::Developed, summary: &str) -> Result<ImportResult, IoError> {
    let mut img = Image::from_u16(dev.width, dev.height, ChannelLayout::Rgb, &dev.rgb)?;
    // The raw's own EXIF (camera, lens, exposure, date), for File Info's Camera Data. It carries
    // no orientation: the developed image is already upright.
    img.meta.exif = codecs::raw_exif(bytes);
    let mut r = image_to_document(name, &img)?;
    r.document.icc_profile = Some(Arc::new(photocraft_cms::Builtin::ProPhotoCompat.profile().to_bytes().to_vec()));
    let camera = camera_name(dev.info.make.as_deref(), dev.info.model.as_deref());
    r.warnings.push(format!(
        "{format}{} {summary} into 16-bit {}",
        if camera.is_empty() { String::new() } else { format!(" from {camera}") },
        photocraft_raw::OUTPUT_SPACE
    ));
    r.warnings.extend(dev.warnings);
    Ok(r)
}

/// Develops a raw file with the default settings.
pub fn import_raw(name: &str, bytes: &[u8]) -> Result<ImportResult, IoError> {
    import_raw_demosaic(name, bytes, Demosaic::default())
}

/// Develops a raw file with the default settings and the `demosaic` method.
pub fn import_raw_demosaic(name: &str, bytes: &[u8], demosaic: Demosaic) -> Result<ImportResult, IoError> {
    import_raw_with(name, bytes, &DevelopOptions { limits: limits(), demosaic, ..Default::default() })
}

/// [`import_raw_demosaic`] that also keeps the sensor data and the fitted look for the file's
/// Camera Raw dialog ([`take_kept_decode`]).
pub fn import_raw_demosaic_keeping(name: &str, bytes: &[u8], demosaic: Demosaic) -> Result<ImportResult, IoError> {
    import_raw_with_inner(name, bytes, &DevelopOptions { limits: limits(), demosaic, ..Default::default() }, true)
}

/// Smallest preview long edge preferred for the colour fit (a smaller preview decodes faster;
/// the fit grid is at most `PROXY_EDGE` cells and wants about two pixels per cell).
const FIT_PREVIEW_EDGE: u32 = 2 * photocraft_raw::PROXY_EDGE as u32;

/// The colour space of a raw's decoded preview when it is not sRGB: its own ICC profile, else the
/// one the raw's EXIF states (a Sony body set to Adobe RGB writes no profile into the JPEG).
/// `None` means sRGB.
fn preview_profile(raw: &[u8], icc: Option<&[u8]>) -> Result<Option<Cow<'static, Profile>>, String> {
    let src = match icc {
        Some(icc) => Cow::Owned(Profile::parse(icc).map_err(|e| format!("the preview's ICC profile does not parse: {e}"))?),
        None if photocraft_raw::embedded_preview_color_space(raw) == Some(PreviewColorSpace::AdobeRgb) => Cow::Borrowed(Builtin::AdobeRgbCompat.profile()),
        None => return Ok(None),
    };
    Ok((!src.same_colors(Builtin::Srgb.profile())).then_some(src))
}

/// Fits the colour of a decoded raw to the camera's embedded JPEG (see
/// [`photocraft_raw::fit_look`]): picks the smallest preview at least
/// `FIT_PREVIEW_EDGE` pixels long (else the largest), decodes it, turns it upright and
/// converts it to sRGB when it is in another colour space ([`preview_profile`]). `Err` says
/// why there is no fit.
pub fn fit_preview_look(raw: &[u8], sensor: &Sensor, opts: &DevelopOptions) -> Result<CameraLook, String> {
    let previews = photocraft_raw::embedded_previews(raw);
    let long = |p: &photocraft_raw::Preview| p.width.max(p.height);
    let pick = previews
        .iter()
        .filter(|p| long(p) >= FIT_PREVIEW_EDGE)
        .min_by_key(|p| long(p))
        .or_else(|| previews.iter().max_by_key(|p| long(p)))
        .ok_or("no embedded JPEG preview")?;
    let img = upright_preview(raw, pick.jpeg, &opts.limits).map_err(|e| format!("the embedded preview does not decode: {e}"))?;
    if img.layout() != ChannelLayout::Rgb || img.sample_type() != SampleType::U8 {
        return Err("the embedded preview is not 8-bit RGB".into());
    }
    let (w, h) = img.dimensions();
    let mut rgb: Cow<[u8]> = Cow::Borrowed(img.data());
    if let Some(src) = preview_profile(raw, img.icc.as_deref())? {
        let t = Transform::new(&src, Builtin::Srgb.profile(), Intent::RelativeColorimetric, false).map_err(|e| e.to_string())?;
        let mut out = vec![0u8; rgb.len()];
        t.convert_u8(&rgb, 3, &mut out, 3, false);
        rgb = Cow::Owned(out);
    }
    photocraft_raw::fit_look(sensor, opts, &Reference { width: w, height: h, rgb: &rgb }).map_err(|e| e.to_string())
}

/// A sensor and the look fitted to it, as [`develop_fitted`] hands them back.
type Kept = (Sensor, Option<CameraLook>);

/// Decodes and develops; a file without colour calibration gets a look fitted to its preview.
/// With `keep`, also hands back the sensor and the look it fitted (the default look, when
/// `opts` named none) instead of freeing them.
fn develop_fitted(bytes: &[u8], opts: &DevelopOptions, keep: bool) -> Result<(photocraft_raw::Developed, Option<Kept>), RawError> {
    // The sensor (and the look fitted to it) the preview stages of this open already made.
    let (sensor, predecoded_look) = match take_predecoded(bytes, opts) {
        Some((sensor, look)) => (sensor, Some(look)),
        None => {
            let sensor = decode_sensor(bytes).map_err(|e| match e {
                IoError::Raw(e) => e,
                e => RawError::Malformed(e.to_string()),
            })?;
            (sensor, None)
        }
    };
    let mut opts = Cow::Borrowed(opts);
    let named_look = opts.camera_look.is_some();
    if opts.camera_look.is_none() {
        let look = match predecoded_look {
            Some(look) => look,
            None => fitted_look(bytes, &sensor, &opts),
        };
        if let Some(look) = look {
            opts.to_mut().camera_look = Some(look);
        }
    }
    let dev = photocraft_raw::develop_sensor(&sensor, &opts)?;
    let kept = (keep && !named_look).then(|| (sensor, opts.into_owned().camera_look));
    Ok((dev, kept))
}

/// The look [`develop_fitted`] fits for a file without colour calibration (none when the file
/// has calibration, is monochrome or the fit does not hold).
fn fitted_look(bytes: &[u8], sensor: &Sensor, opts: &DevelopOptions) -> Option<CameraLook> {
    if !sensor.color.calibrations.is_empty() || sensor.is_monochrome() {
        return None;
    }
    fit_preview_look(bytes, sensor, opts).ok()
}

/// In the import note of a raw developed from its sensor data with the default settings (not of
/// a preview fallback): what the open-time Camera Raw dialog looks for.
pub const DEVELOPED_NOTE: &str = "developed with default settings";

/// The demosaic method a default open developed with, read back from its [`DEVELOPED_NOTE`]
/// warning: the open-time dialog starts from it, whatever the preference says by the time the
/// (background) open lands.
pub fn developed_demosaic(warnings: &[String]) -> Option<Demosaic> {
    let note = warnings.iter().find(|w| w.contains(DEVELOPED_NOTE))?;
    let after = &note[note.find(DEVELOPED_NOTE)? + DEVELOPED_NOTE.len()..];
    let id = after.trim_start().strip_prefix('(')?.split(" demosaic").next()?;
    Demosaic::from_id(id)
}

/// Develops a raw file with explicit settings. Without `opts.camera_look`, a file without
/// colour calibration gets one fitted to its embedded JPEG when that clearly helps.
pub fn import_raw_with(name: &str, bytes: &[u8], opts: &DevelopOptions) -> Result<ImportResult, IoError> {
    import_raw_with_inner(name, bytes, opts, false)
}

fn import_raw_with_inner(name: &str, bytes: &[u8], opts: &DevelopOptions, keep: bool) -> Result<ImportResult, IoError> {
    let format = photocraft_raw::identify(bytes).map(|f| f.name()).unwrap_or("camera raw");
    match develop_fitted(bytes, opts, keep) {
        Ok((dev, kept_parts)) => {
            if let Some((sensor, look)) = kept_parts {
                let mut kept = kept();
                if kept.len() == KEPT_MAX {
                    kept.remove(0);
                }
                kept.push(Arc::new(RawDecode::from_parts(bytes, sensor, look)));
            }
            let summary = format!("{DEVELOPED_NOTE} ({} demosaic, as-shot white balance)", opts.demosaic.id());
            developed_document(name, bytes, format, dev, &summary)
        }
        Err(RawError::Unsupported(reason)) => match photocraft_raw::embedded_preview(bytes) {
            Some(p) => {
                let img = upright_preview(bytes, p.jpeg, &opts.limits)?;
                let mut r = image_to_document(name, &img)?;
                r.warnings.insert(
                    0,
                    format!(
                        "{format}: {reason} is not supported yet; opened the camera's embedded {}x{} JPEG preview instead (8-bit, not the raw sensor data)",
                        p.width, p.height
                    ),
                );
                Ok(r)
            }
            None => Err(IoError::Raw(RawError::Unsupported(reason))),
        },
        Err(e) => Err(IoError::Raw(e)),
    }
}

// ------------------------------------------------------------------------------ progressive open

/// What the preview stages of an interactive open learned, for the full develop that follows on
/// the same thread: the file, the demosaic method it fits with, the sensor and the fitted look.
struct Predecoded {
    fingerprint: Fingerprint,
    demosaic: Demosaic,
    sensor: Sensor,
    look: Option<CameraLook>,
}

thread_local! {
    static PREDECODED: std::cell::RefCell<Option<Predecoded>> = const { std::cell::RefCell::new(None) };
}

/// Hands the full develop the sensor and look the preview stages made for `bytes`, once. Another
/// file or demosaic method, or `opts` that name a look, finds nothing (and drops the stash).
fn take_predecoded(bytes: &[u8], opts: &DevelopOptions) -> Option<(Sensor, Option<CameraLook>)> {
    let p = PREDECODED.with(|p| p.borrow_mut().take())?;
    (p.demosaic == opts.demosaic && opts.camera_look.is_none() && p.fingerprint == Fingerprint::of(bytes)).then_some((p.sensor, p.look))
}

/// Long edge a preview JPEG should have at least to be worth showing: the smallest such preview
/// decodes fastest, else the largest.
const FIRST_PAINT_EDGE: u32 = 1920;

/// The camera's embedded JPEG as a document, upright and tagged with its own colour space (8-bit
/// sRGB or Adobe RGB, unlike the 16-bit ProPhoto of a develop). `None` without a decodable one.
fn jpeg_preview(name: &str, bytes: &[u8]) -> Option<ImportResult> {
    let previews = photocraft_raw::embedded_previews(bytes);
    let long = |p: &photocraft_raw::Preview| p.width.max(p.height);
    let pick = previews.iter().filter(|p| long(p) >= FIRST_PAINT_EDGE).min_by_key(|p| long(p)).or_else(|| previews.iter().max_by_key(|p| long(p)))?;
    let img = upright_preview(bytes, pick.jpeg, &limits()).ok()?;
    let profile = preview_profile(bytes, img.icc.as_deref()).ok()?;
    let mut r = image_to_document(name, &img).ok()?;
    if let (Some(p), None) = (profile, &img.icc) {
        r.document.icc_profile = Some(Arc::new(p.to_bytes().to_vec()));
    }
    Some(r)
}

/// The half-size develop of `bytes` as a document ([`photocraft_raw::develop_half`]), with the
/// colour rendering the full develop will use. The decoded sensor and look stay behind for that
/// full develop ([`develop_fitted`]) on this thread, so the file is decoded once.
fn half_preview(name: &str, bytes: &[u8], demosaic: Demosaic) -> Option<ImportResult> {
    let opts = DevelopOptions { limits: limits(), demosaic, ..Default::default() };
    let sensor = decode_sensor(bytes).ok()?;
    let look = fitted_look(bytes, &sensor, &opts);
    let half = photocraft_raw::develop_half(&sensor, &DevelopOptions { camera_look: look.clone(), ..opts });
    let format = photocraft_raw::identify(bytes).map(|f| f.name()).unwrap_or("camera raw");
    let r = half.ok().and_then(|dev| developed_document(name, bytes, format, dev, "half-size preview").ok());
    PREDECODED.with(|p| *p.borrow_mut() = Some(Predecoded { fingerprint: Fingerprint::of(bytes), demosaic, sensor, look }));
    r
}

/// The first-paint stages of opening a raw, each handed to `publish` as it is ready: the camera's
/// embedded JPEG (when there is one), then the half-size develop. A stage that fails (no JPEG, a
/// corrupt one, a CFA the half-size develop does not support) is skipped silently; the full
/// develop ([`import_raw_demosaic`]) reports real errors. Call it on the thread that then runs
/// the full develop: that one reuses the sensor decoded here. `cancelled` is polled between stages.
pub fn publish_previews(name: &str, bytes: &[u8], demosaic: Demosaic, publish: &mut dyn FnMut(ImportResult), cancelled: &dyn Fn() -> bool) {
    if let Some(r) = jpeg_preview(name, bytes) {
        publish(r);
    }
    if !cancelled()
        && let Some(r) = half_preview(name, bytes, demosaic)
    {
        publish(r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_developed_note_says_which_demosaic_method_was_used() {
        let note = format!("NEF: {DEVELOPED_NOTE} (mhc demosaic, as-shot white balance) into 16-bit ProPhoto RGB");
        assert_eq!(developed_demosaic(&["unrelated".to_string(), note]), Some(Demosaic::Mhc));
        assert_eq!(developed_demosaic(&[format!("{DEVELOPED_NOTE} (amaze demosaic)")]), None);
        assert_eq!(developed_demosaic(&["opened the embedded preview".to_string()]), None);
    }

    #[test]
    fn camera_names_drop_a_repeated_make() {
        assert_eq!(camera_name(Some("Canon"), Some("Canon EOS 80D")), "Canon EOS 80D");
        assert_eq!(camera_name(Some("NIKON CORPORATION"), Some("NIKON D3200")), "NIKON D3200");
        assert_eq!(camera_name(Some("SONY"), Some("ILCE-7M3")), "SONY ILCE-7M3");
        assert_eq!(camera_name(None, Some("X")), "X");
        assert_eq!(camera_name(Some("Panasonic"), Some("DC-G9")), "Panasonic DC-G9");
    }

    /// A Sony body set to Adobe RGB says so only in the raw's EXIF (ColorSpace Uncalibrated, InteropIndex "R03"):
    /// its preview, which has no ICC profile, is read as Adobe RGB for the colour fit, not as sRGB.
    #[test]
    fn a_preview_without_a_profile_takes_the_raws_exif_colour_space() {
        use photocraft_raw::testgen::{TiffBuilder, Val};
        let raw = |color_space: u16, index: &str| {
            let mut t = TiffBuilder::default();
            let interop = t.ifd(vec![(0x0001, Val::Ascii(index.into()))]);
            let exif = t.ifd(vec![(0xA001, Val::Short(vec![color_space])), (0xA005, Val::Ifds(vec![interop]))]);
            t.chain = vec![t.ifd(vec![(271, Val::Ascii("SONY".into())), (34665, Val::Ifds(vec![exif]))])];
            t.build()
        };
        let adobe = raw(0xFFFF, "R03");
        let srgb = raw(1, "R98");
        assert!(preview_profile(&adobe, None).unwrap().unwrap().same_colors(Builtin::AdobeRgbCompat.profile()));
        assert!(preview_profile(&srgb, None).unwrap().is_none());
        assert!(preview_profile(b"not a raw", None).unwrap().is_none(), "unstated: sRGB");
        // the preview's own profile wins over the raw's EXIF
        let srgb_icc = Builtin::Srgb.profile().to_bytes().to_vec();
        assert!(preview_profile(&adobe, Some(srgb_icc.as_slice())).unwrap().is_none());
        let p3_icc = Builtin::DisplayP3.profile().to_bytes().to_vec();
        assert!(preview_profile(&srgb, Some(p3_icc.as_slice())).unwrap().unwrap().same_colors(Builtin::DisplayP3.profile()));
        assert!(preview_profile(&adobe, Some(b"not an ICC profile")).is_err());
    }

    // ---- progressive open

    use photocraft_raw::testgen::{DngSpec, RafSpec, Val, XTRANS, baseline_jpeg, mosaic, mosaic_cfa, scene};

    fn dng_bytes() -> Vec<u8> {
        let (w, h) = (48, 32);
        DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000)).build()
    }

    /// The DNG with a real JPEG (`jw` x `jh`) appended and referenced as IFD0's JPEGInterchangeFormat.
    fn dng_with_jpeg(jw: usize, jh: usize) -> Vec<u8> {
        let (w, h) = (48, 32);
        let jpeg = baseline_jpeg(&vec![90u8; jw * jh * 3], jw, jh, 3);
        let spec = |at: u32| {
            let mut s = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000));
            s.ifd0_extra = vec![(513, Val::Long(vec![at])), (514, Val::Long(vec![jpeg.len() as u32]))];
            s
        };
        let at = spec(0).build().len() as u32;
        let mut bytes = spec(at).build();
        assert_eq!(bytes.len() as u32, at);
        bytes.extend_from_slice(&jpeg);
        bytes
    }

    fn previews_of(bytes: &[u8]) -> Vec<ImportResult> {
        let mut out = Vec::new();
        publish_previews("p.dng", bytes, Demosaic::default(), &mut |r| out.push(r), &|| false);
        out
    }

    fn size(r: &ImportResult) -> (u32, u32) {
        (r.document.size.width, r.document.size.height)
    }

    fn png(r: &ImportResult) -> Vec<u8> {
        crate::export(&r.document, "x.png", &Default::default()).unwrap().bytes
    }

    #[test]
    fn a_raw_without_a_jpeg_shows_the_half_size_develop_first() {
        let p = previews_of(&dng_bytes());
        assert_eq!(p.len(), 1);
        assert_eq!(size(&p[0]), (24, 16));
        assert!(p[0].document.icc_profile.is_some(), "16-bit ProPhoto, tagged");
        assert!(!p[0].warnings.iter().any(|w| w.contains(DEVELOPED_NOTE)), "a preview is not a default open");
    }

    #[test]
    fn a_raw_with_a_jpeg_shows_it_first_then_the_half_size_develop() {
        let p = previews_of(&dng_with_jpeg(40, 30));
        assert_eq!(p.iter().map(size).collect::<Vec<_>>(), vec![(40, 30), (24, 16)]);
    }

    #[test]
    fn a_corrupt_jpeg_falls_through_to_the_next_stage() {
        let mut bytes = dng_with_jpeg(40, 30);
        let n = bytes.len();
        // Keep the header (so it is found) and destroy the entropy-coded data.
        bytes[n - 60..n - 2].fill(0xAB);
        let p = previews_of(&bytes);
        assert_eq!(p.iter().map(size).collect::<Vec<_>>(), vec![(24, 16)]);
    }

    #[test]
    fn x_trans_previews_at_a_third_and_a_cfa_the_half_size_develop_cannot_do_falls_through() {
        let (w, h) = (30, 18);
        let raf = RafSpec::new(w, h, mosaic_cfa(&scene(w, h), w, &XTRANS, 6, 256, 16383), XTRANS.to_vec()).build();
        let p = previews_of(&raf);
        // The RAF's tiny stand-in JPEG does not decode: only the 3x3-binned develop is left.
        assert_eq!(p.iter().map(size).collect::<Vec<_>>(), vec![(10, 6)]);
        // A repeat the half-size develop does not support is an error there; the preview stage skips it.
        let mut s = photocraft_raw::decode(&raf, &limits()).unwrap();
        s.cfa.as_mut().unwrap().height = 3;
        s.cfa.as_mut().unwrap().colors.truncate(18);
        assert!(matches!(photocraft_raw::develop_half(&s, &DevelopOptions::default()), Err(RawError::Unsupported(_))));
    }

    #[test]
    fn the_full_develop_after_the_previews_is_identical_and_decodes_once() {
        for bytes in [dng_bytes(), dng_with_jpeg(40, 30)] {
            let plain = import_raw_demosaic("a.dng", &bytes, Demosaic::default()).unwrap();
            let before = decodes_on_this_thread();
            drop(previews_of(&bytes));
            let full = import_raw_demosaic("a.dng", &bytes, Demosaic::default()).unwrap();
            assert_eq!(decodes_on_this_thread() - before, 1, "the preview's decode is reused");
            assert_eq!(png(&plain), png(&full));
            assert_eq!(plain.warnings, full.warnings);
            // The stash is used up: a later open decodes afresh.
            let before = decodes_on_this_thread();
            import_raw_demosaic("a.dng", &bytes, Demosaic::default()).unwrap();
            assert_eq!(decodes_on_this_thread() - before, 1);
        }
    }

    #[test]
    fn the_stash_is_not_used_for_another_file_or_method() {
        let (a, b) = (dng_bytes(), dng_with_jpeg(40, 30));
        drop(previews_of(&a));
        let before = decodes_on_this_thread();
        let full = import_raw_demosaic("b.dng", &b, Demosaic::default()).unwrap();
        assert_eq!(decodes_on_this_thread() - before, 1);
        assert_eq!(png(&full), png(&import_raw_demosaic("b.dng", &b, Demosaic::default()).unwrap()));
    }
}
