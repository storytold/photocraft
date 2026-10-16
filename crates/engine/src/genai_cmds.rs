//! Edit › Generative Fill (#41): fill the selection from a prompt through the image provider the
//! app configured ([`Session::set_image_provider`]). With no provider the command is disabled and
//! nothing is ever sent anywhere.
//!
//! The selection's bounds plus some surrounding context are rendered (all visible layers, in
//! sRGB) and sent with a mask of the selection, once per variation (three by default, as
//! Photoshop makes). The results land in a new group masked by the selection, one layer per
//! variation with the first showing, so nothing under it changes and one undo removes it.
//!
//! Two modes: `fill` (the default) blurs the selected area out before sending, so the provider
//! fills a hole (an empty prompt removes what was there); `edit` sends it as it is, so the provider
//! changes only what the prompt asks for and keeps the rest (Photoshop's Prompt to Edit). Either
//! can carry reference images (files or open documents) for the provider to follow.
//! `layer.generative.selectVariation` flips between them and `layer.generative.more` asks for
//! more. The network calls run as one cancellable background job.
//!
//! Fills don't lock the document (this fork): each one's job edits it with
//! [`crate::jobs::run_on`], so while one is generating the user selects the next area and starts
//! another. Up to [`MAX_PARALLEL`] talk to the provider at once and the rest wait their turn
//! (`queue`). A started fill is remembered until its layer lands (above the layer that was active
//! when it started, without making it active) or it is discarded; one that failed waits for
//! `layer.generative.retry` or `layer.generative.discard`, and `layer.generative.fills` lists them
//! for the shell's progress. `area` fills one of the selection's separate areas (`areas`), so
//! several objects selected together are generated side by side.
//!
//! Pixel interchange follows #730's approach (composite through the document's profile to sRGB,
//! and back into the document's mode and depth on the way in).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use photocraft_cms::{Builtin, Intent, Transform};
use photocraft_codecs::{self as codecs, ChannelLayout, Format, Image, SampleType as CodecSample};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{DocId, Document, Layer, LayerId, LayerMask};
use photocraft_genai::{EditMode, EditRequest, ImageProvider, ImageSize, ReferenceImage};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde::Serialize;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::jobs::{JobCtx, JobId};
use crate::{EngineError, Result, Session};

mod areas;
mod queue;

pub use areas::MAX_AREAS;
pub use queue::MAX_PARALLEL;

pub const FILL: &str = "edit.generativeFill";
pub const SELECT_VARIATION: &str = "layer.generative.selectVariation";
pub const MORE: &str = "layer.generative.more";
pub const RETRY: &str = "layer.generative.retry";
pub const DISCARD: &str = "layer.generative.discard";
pub const FILLS: &str = "layer.generative.fills";
pub const AREAS: &str = "layer.generative.areas";
const LABEL: &str = "Generative Fill";
/// Longest side sent to the provider; larger areas are scaled down for the request and the
/// result is scaled back up.
const UPLOAD_SIDE: u32 = 2048;
const DEFAULT_VARIATIONS: usize = 3;
const MAX_VARIATIONS: u64 = 4;
/// Largest result accepted, per side.
const RESULT_SIDE: u32 = 8192;
/// Longest side of a reference image as sent: enough to show an object or a design, small enough
/// that a few of them add little to each request.
const REFERENCE_SIDE: u32 = 1024;

/// A configured provider, shared with the background job that calls it.
pub type SharedProvider = Arc<dyn ImageProvider>;

impl Session {
    /// Install (or with `None`, remove) the generative image provider. The app builds it from the
    /// user's settings and key; headless and test sessions have none unless they set one.
    pub fn set_image_provider(&mut self, provider: Option<SharedProvider>) {
        self.genai = provider;
    }

    pub fn image_provider(&self) -> Option<&SharedProvider> {
        self.genai.as_ref()
    }

    /// The fills started in this session that haven't landed: generating, waiting their turn, or
    /// failed (waiting for Retry or Discard), oldest first.
    pub fn generative_fills(&self) -> Vec<FillInfo> {
        self.genai_state.fills.iter().filter_map(|f| f.info(self)).collect()
    }

    /// The selection fill `id` fills (its shape, for the shell to draw while it generates).
    pub fn generative_fill_mask(&self, id: u64) -> Option<&Surface> {
        self.genai_state.fills.iter().find(|f| f.id == id).map(|f| &f.input.sel)
    }

    /// Forget fills whose job ended without a result or an error (cancelled) and fills on closed
    /// documents. The shell calls it when a job ends.
    pub fn tidy_generative_fills(&mut self) {
        let fills = std::mem::take(&mut self.genai_state.fills);
        let (keep, gone): (Vec<_>, Vec<_>) = fills.into_iter().partition(|f| f.info(self).is_some());
        self.genai_state.fills = keep;
        for f in gone {
            if let Some(job) = f.job {
                self.cancel_job(job);
            }
        }
    }
}

/// The session's generative state (this fork).
#[derive(Default)]
pub(crate) struct GenState {
    /// What each generative layer made in this session was generated from.
    sources: std::collections::HashMap<LayerId, Arc<Source>>,
    /// Fills that haven't landed.
    fills: Vec<Fill>,
    next_fill: u64,
    /// Turns at the provider, shared by every fill and Generate More.
    turns: Arc<queue::Turns>,
}

/// A fill that hasn't landed.
struct Fill {
    id: u64,
    /// Its current job (none yet, or the one that failed).
    job: Option<JobId>,
    doc: DocId,
    /// The layer its result goes above: the active layer when it started.
    anchor: Option<LayerId>,
    status: Arc<FillStatus>,
    input: Arc<FillInput>,
}

/// What a fill's job reports back while it runs.
#[derive(Default)]
struct FillStatus {
    /// Place in line for a turn at the provider (1 = next), 0 when not waiting.
    queued: AtomicUsize,
    /// Why it failed, when it did (it waits for Retry or Discard then).
    error: Mutex<Option<String>>,
}

impl FillStatus {
    fn error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn set_error(&self, e: Option<String>) {
        *self.error.lock().unwrap_or_else(PoisonError::into_inner) = e;
    }
}

/// Everything a fill's job needs, kept so Retry can send it again.
struct FillInput {
    /// The document as it was when the fill started: what is sent comes from it.
    doc: Arc<Document>,
    /// The selection (or the area of it) filled; the result is masked by it.
    sel: Surface,
    /// The canvas when it started: the result lands only on a canvas of that size.
    canvas: Rect,
    area: Rect,
    size: (u32, u32),
    mode: EditMode,
    prompt: String,
    name: String,
    references: Vec<ReferenceSource>,
    count: usize,
    /// Make the result the active layer: run inline (scripts, the CLI), the next command works
    /// on it. In the background the user has moved on, so the active layer stays.
    select: bool,
}

/// A fill that hasn't landed, as `layer.generative.fills` and the shell see it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FillInfo {
    pub id: u64,
    pub job: Option<JobId>,
    pub document: DocId,
    /// The layer its result goes above (`None`, or a layer since deleted: above the active one).
    pub anchor: Option<LayerId>,
    /// The name its layer gets (from the prompt).
    pub name: String,
    pub prompt: String,
    /// The selection's bounds, in document pixels: `[x0, y0, x1, y1]`.
    pub bounds: [i32; 4],
    /// `queued` (waiting for a turn), `running` or `failed`.
    pub state: &'static str,
    /// Place in line when queued (1 = next).
    pub queue_position: usize,
    pub progress: f32,
    pub elapsed_ms: f64,
    pub error: Option<String>,
}

impl Fill {
    /// How it stands; `None` once there is nothing to show (it was cancelled, or its document
    /// closed).
    fn info(&self, s: &Session) -> Option<FillInfo> {
        s.documents().iter().find(|st| st.doc.id == self.doc)?;
        let error = self.status.error();
        let job = self.job.and_then(|j| s.job(j));
        if job.is_none() && error.is_none() {
            return None;
        }
        let queued = self.status.queued.load(Ordering::Relaxed);
        let state = if error.is_some() {
            "failed"
        } else if queued > 0 {
            "queued"
        } else {
            "running"
        };
        let b = self.input.sel.content_bounds().intersect(&self.input.canvas);
        Some(FillInfo {
            id: self.id,
            job: self.job,
            document: self.doc,
            anchor: self.anchor,
            name: self.input.name.clone(),
            prompt: self.input.prompt.clone(),
            bounds: [b.x0, b.y0, b.x1, b.y1],
            state,
            queue_position: if error.is_some() { 0 } else { queued },
            progress: job.as_ref().map_or(0.0, |j| j.progress),
            elapsed_ms: job.as_ref().map_or(0.0, |j| j.elapsed_ms),
            error,
        })
    }
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: FILL.into(), msg: msg.into() }
}

fn other(e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(e.to_string())
}

fn fill_enabled(s: &Session) -> std::result::Result<(), String> {
    let p = s.image_provider().ok_or("set up a generative AI provider first (Generative AI Settings)")?;
    if !p.capabilities().edit {
        return Err(format!("{} can't fill selections", p.info().name));
    }
    let st = s.active().ok_or("no document open")?;
    let sel = st.doc.selection.as_ref().ok_or("no selection")?;
    if sel.content_bounds().intersect(&st.doc.bounds()).is_empty() {
        return Err("the selection is empty".into());
    }
    Ok(())
}

/// The area sent: the selection's bounds with a margin of context around them, grown toward the
/// provider's output shape so the result isn't stretched, inside the canvas.
pub(crate) fn context_area(selected: Rect, canvas: Rect, aspect: impl Fn(u32, u32) -> (u32, u32)) -> Rect {
    let side = selected.width().max(selected.height());
    // Twice the selection's size on each side, so the model can read a pattern or trend that
    // runs through it (a row of panels stepping down) and not just what touches its edge. Half
    // the size was tried first: Gemini saw two panels of seven and carried the pattern under
    // the selection straight up instead of continuing the row's curve.
    let margin = side.saturating_mul(2).clamp(64, 2048) as i32;
    let r = Rect::new(selected.x0 - margin, selected.y0 - margin, selected.x1.saturating_add(margin), selected.y1.saturating_add(margin)).intersect(&canvas);
    let (a, b) = aspect(r.width(), r.height());
    if a == 0 || b == 0 || r.is_empty() {
        return r;
    }
    let (w, h) = (u64::from(r.width()), u64::from(r.height()));
    let (want_w, want_h) =
        if w * u64::from(b) < h * u64::from(a) { ((h * u64::from(a)).div_ceil(u64::from(b)), h) } else { (w, (w * u64::from(b)).div_ceil(u64::from(a))) };
    let grow = |lo: i32, hi: i32, want: u64, min: i32, max: i32| -> (i32, i32) {
        let want = want.min((max - min) as u64) as i32;
        let extra = want - (hi - lo);
        let lo = (lo - extra / 2).max(min);
        let hi = (lo + want).min(max);
        (hi - want, hi)
    };
    let (x0, x1) = grow(r.x0, r.x1, want_w, canvas.x0, canvas.x1);
    let (y0, y1) = grow(r.y0, r.y1, want_h, canvas.y0, canvas.y1);
    Rect::new(x0, y0, x1, y1)
}

/// The visible image over `area`, in 8-bit sRGB.
fn srgb_composite(doc: &Document, area: Rect) -> Result<Surface> {
    let rgb = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let mut surface = Surface::new(rgb);
    let mut data = Vec::new();
    photocraft_compose::render_bands(doc, area, 0, |band| -> Result<()> {
        data.clear();
        data.resize(band.px.len() * 4, 0.0);
        for (p, out) in band.px.iter().zip(data.as_chunks_mut::<4>().0.iter_mut()) {
            photocraft_raster::from_rgba_into(&rgb, *p, out);
        }
        surface.write_region(band.rect, &data);
        Ok(())
    })?;
    let t = Transform::new(&crate::color_cmds::composite_profile(doc), Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).map_err(other)?;
    Ok(crate::color_cmds::convert_surface(&surface, ColorMode::Rgb, rgb, &t))
}

/// sRGB pixels into the document's mode, depth and profile.
fn to_document(doc: &Document, s: &Surface) -> Result<Surface> {
    let fmt = PixelFormat::new(doc.mode, doc.depth, true);
    let t = Transform::new(Builtin::Srgb.profile(), &crate::color_cmds::document_profile(doc), Intent::RelativeColorimetric, true).map_err(other)?;
    let converted = crate::color_cmds::convert_surface(s, ColorMode::Rgb, fmt, &t);
    Ok(if converted.format() == fmt { converted } else { converted.convert(fmt) })
}

/// `area` of `s` scaled to `w`×`h`, at the origin.
fn scaled(s: &Surface, area: Rect, w: u32, h: u32) -> Surface {
    let mut local = Surface::new(s.format());
    local.write_region(Rect::from_xywh(0, 0, area.width(), area.height()), &s.read_region(area));
    if (w, h) == (area.width(), area.height()) {
        return local;
    }
    let sx = f64::from(w) / f64::from(area.width());
    let sy = f64::from(h) / f64::from(area.height());
    photocraft_algo::resample::resize_surface(&local, sx, sy, photocraft_algo::resample::Resample::Bicubic)
}

/// Largest mean difference (0..1) around the selection still treated as colour drift.
const MAX_DRIFT: f64 = 0.25;
/// Mean difference past which a pixel outside the selection counts as changed by the model, not
/// just relit.
const CHANGED: f32 = 0.25;
/// Difference in fine detail past which a pixel outside the selection counts as changed.
const DETAIL: f32 = 0.03;
/// Levels in the fitted tone curves.
const BINS: usize = 32;
/// Grey that transparent parts of the image are shown on when sent.
const HOLE: f32 = 128.0;
/// How bright the placeholder over the selection is, as a share of the colours around it.
/// At full brightness, bright content running into the selection (lit panels below an empty
/// sky) smears through it and reads as that content carrying on to the far edge; dimmed, it
/// reads as a gap to fill. Dimming didn't make Gemini draw dark shapes into bright scenes.
const PLACEHOLDER: f32 = 0.35;

fn encode_rgba(px: Vec<u8>, w: u32, h: u32) -> Result<Vec<u8>> {
    let img = Image::from_u8(w, h, ChannelLayout::Rgba, px).map_err(other)?;
    codecs::encode(&img, Format::Png, &Default::default()).map_err(other)
}

/// The image and mask sent, both `w`×`h`: `image` (sRGB, already scaled) and an opaque
/// black-and-white PNG of the selection (white = fill). In [`EditMode::Fill`] the selection is
/// replaced by a dimmed soft blur of the colours around it: the provider never sees what is being
/// replaced, as Photoshop's fill doesn't, so it can't paint it back. A flat grey hole was tried
/// first: Gemini read it as an object (a grey box, a lamp) and drew it in. In [`EditMode::Edit`]
/// it is sent as it is, since the provider is to change what is there.
fn request_images(image: &Surface, sel: &Surface, area: Rect, w: u32, h: u32, mode: EditMode) -> Result<(Vec<u8>, Vec<u8>, Reference)> {
    let r = Rect::from_xywh(0, 0, w, h);
    let rgba = image.convert(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true)).to_interleaved(r);
    let coverage: Vec<f32> = scaled(sel, area, w, h).read_region(r).into_iter().map(|v| v.clamp(0.0, 1.0)).collect();
    let mut original = Vec::with_capacity(coverage.len() * 3);
    for px in rgba.as_chunks::<4>().0 {
        let a = f32::from(px[3]) / 255.0;
        // Transparent areas read as mid grey, not black.
        for &c in &px[..3] {
            original.push((f32::from(c) * a + HOLE * (1.0 - a)) / 255.0);
        }
    }
    let fill = (mode == EditMode::Fill).then(|| surroundings_fill(&original, &coverage, w as usize, h as usize));
    let mut cleared = Vec::with_capacity(rgba.len());
    let mut mask = Vec::with_capacity(rgba.len());
    for (i, &m) in coverage.iter().enumerate() {
        for c in 0..3 {
            let v = match &fill {
                Some(fill) => original[i * 3 + c] * (1.0 - m) + fill[i * 3 + c] * PLACEHOLDER * m,
                None => original[i * 3 + c],
            };
            cleared.push((v * 255.0).round().clamp(0.0, 255.0) as u8);
        }
        cleared.push(255);
        let g = (m * 255.0).round() as u8;
        mask.extend_from_slice(&[g, g, g, 255]);
    }
    let reference = Reference { original, coverage, w, h };
    Ok((encode_rgba(cleared, w, h)?, encode_rgba(mask, w, h)?, reference))
}

/// Colours for the selected area taken from around it (RGB, 0..1, `w`×`h`), by push-pull: the
/// unselected pixels are averaged down a pyramid of halving sizes until every cell has some, then
/// the averages are brought back up with bilinear interpolation, each level keeping its own known
/// pixels. The result keeps the edge colours and turns smooth toward the middle, with no rings
/// or steps a model could read as shapes.
fn surroundings_fill(original: &[f32], coverage: &[f32], w: usize, h: usize) -> Vec<f32> {
    // Each level: (width, height, weight 0..1, colour).
    let mut levels: Vec<(usize, usize, Vec<f32>, Vec<f32>)> = vec![(w, h, coverage.iter().map(|c| 1.0 - c).collect(), original.to_vec())];
    while let Some((lw, lh, lwt, lc)) = levels.last()
        && (*lw > 1 || *lh > 1)
    {
        let (nw, nh) = (lw.div_ceil(2), lh.div_ceil(2));
        let (mut wt, mut col) = (vec![0.0f32; nw * nh], vec![0.0f32; nw * nh * 3]);
        for y in 0..nh {
            for x in 0..nw {
                let (mut sum, mut acc) = (0.0f32, [0.0f32; 3]);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = (2 * x + dx, 2 * y + dy);
                    if sx < *lw && sy < *lh {
                        let i = sy * lw + sx;
                        sum += lwt[i];
                        for c in 0..3 {
                            acc[c] += lwt[i] * lc[i * 3 + c];
                        }
                    }
                }
                let i = y * nw + x;
                if sum > 0.0 {
                    for c in 0..3 {
                        col[i * 3 + c] = acc[c] / sum;
                    }
                }
                wt[i] = sum.min(1.0);
            }
        }
        levels.push((nw, nh, wt, col));
    }
    // Bring the averages back up: unknown parts of each level take the level above, interpolated.
    for l in (0..levels.len() - 1).rev() {
        let (cw, ch, _, coarse) = levels[l + 1].clone();
        let (lw, lh, wt, col) = &mut levels[l];
        for y in 0..*lh {
            let fy = ((y as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (ch - 1) as f32);
            let (y0, ty) = (fy.floor() as usize, fy.fract());
            let y1 = (y0 + 1).min(ch - 1);
            for x in 0..*lw {
                let fx = ((x as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (cw - 1) as f32);
                let (x0, tx) = (fx.floor() as usize, fx.fract());
                let x1 = (x0 + 1).min(cw - 1);
                let i = y * *lw + x;
                for c in 0..3 {
                    let at = |xx: usize, yy: usize| coarse[(yy * cw + xx) * 3 + c];
                    let up = (at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx) * (1.0 - ty) + (at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx) * ty;
                    col[i * 3 + c] = wt[i] * col[i * 3 + c] + (1.0 - wt[i]) * up;
                }
            }
        }
    }
    levels.swap_remove(0).3
}

/// What a result is matched against: the area as it was before the hole was cleared (RGB, 0..1)
/// and the selection's coverage, both at the request size.
pub(crate) struct Reference {
    original: Vec<f32>,
    coverage: Vec<f32>,
    w: u32,
    h: u32,
}

/// Box blur of a `w`×`h` plane with radius `r`, edges clamped.
fn box_blur(v: &mut [f32], w: usize, h: usize, r: usize) {
    let pass = |line: &mut [f32], tmp: &mut Vec<f32>| {
        let n = line.len();
        tmp.clear();
        tmp.extend_from_slice(line);
        let mut sum: f32 = (0..=r.min(n - 1)).map(|i| tmp[i]).sum();
        let mut count = r.min(n - 1) + 1;
        for i in 0..n {
            line[i] = sum / count as f32;
            if i + r + 1 < n {
                sum += tmp[i + r + 1];
                count += 1;
            }
            if i >= r {
                sum -= tmp[i - r];
                count -= 1;
            }
        }
    };
    let mut tmp = Vec::new();
    let mut col = vec![0.0; h];
    for row in v.chunks_mut(w) {
        pass(row, &mut tmp);
    }
    for x in 0..w {
        for y in 0..h {
            col[y] = v[y * w + x];
        }
        pass(&mut col, &mut tmp);
        for y in 0..h {
            v[y * w + x] = col[y];
        }
    }
}

/// Brightness of an RGBA or RGB pixel (0..1), Rec. 709 weights.
fn luma(p: &[f32]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

/// A tone curve taking brightness `from` to brightness `to`, fitted over the pixels with weight
/// `wt`: the mean change at each of [`BINS`] levels, so a model that lifts the shadows and leaves
/// the highlights is undone level by level. Levels without enough samples take their neighbours'
/// change; the curve never turns downward.
fn fit_curve(from: &[f32], to: &[f32], wt: &[f32]) -> [f32; BINS] {
    let (mut n, mut d) = ([0.0f64; BINS], [0.0f64; BINS]);
    for (i, &o) in wt.iter().enumerate() {
        let x = from[i].clamp(0.0, 1.0);
        let b = ((x * BINS as f32) as usize).min(BINS - 1);
        n[b] += f64::from(o);
        d[b] += f64::from(o) * f64::from(to[i] - x);
    }
    let known: Vec<usize> = (0..BINS).filter(|&b| n[b] >= 32.0).collect();
    let at = |k: usize| (d[k] / n[k]) as f32;
    let mut curve = [0.0f32; BINS];
    for (b, v) in curve.iter_mut().enumerate() {
        *v = match (known.iter().rev().find(|&&k| k <= b), known.iter().find(|&&k| k >= b)) {
            (Some(&lo), Some(&hi)) if lo != hi => at(lo) + (at(hi) - at(lo)) * (b - lo) as f32 / (hi - lo) as f32,
            (Some(&k), _) | (None, Some(&k)) => at(k),
            (None, None) => 0.0,
        };
    }
    // Rising: each level's output at least the one below it.
    for b in 1..BINS {
        curve[b] = curve[b].max(curve[b - 1] - 1.0 / BINS as f32);
    }
    curve
}

/// `v` through a curve from [`fit_curve`] (the change at each level's centre, interpolated).
fn apply_curve(curve: &[f32; BINS], v: f32) -> f32 {
    let t = (v.clamp(0.0, 1.0) * BINS as f32 - 0.5).clamp(0.0, (BINS - 1) as f32);
    let (b, f) = (t as usize, t.fract());
    v + curve[b] + (curve[(b + 1).min(BINS - 1)] - curve[b]) * f
}

/// An RGBA pixel's brightness moved along `curve`, its hue and saturation kept: scaled while
/// bright enough to scale, shifted near black.
fn tone(curve: &[f32; BINS], p: &mut [f32]) {
    let y = luma(p);
    let target = apply_curve(curve, y);
    for v in &mut p[..3] {
        *v = if y > 0.05 { *v * (target / y) } else { *v + (target - y) };
    }
}

/// Take out the colour drift a model adds when it redraws the whole image, so the fill meets the
/// photo around it without a seam. Everything outside the selection should have come back
/// unchanged; comparing it with the original measures the drift. First a tone curve on brightness
/// (models relight: shadows lifted, highlights kept), hue untouched, fitted again without the pixels the model
/// actually changed (an object it moved), then
/// what is left at the edge carried smoothly inward by push-pull, as Photoshop's fills blend into
/// their surroundings. `px` is RGBA (0..1) at the reference size.
fn match_surroundings(px: &mut [f32], r: &Reference) -> String {
    let (w, h) = (r.w as usize, r.h as usize);
    let mut outside: Vec<f32> = r.coverage.iter().map(|c| 1.0 - c.clamp(0.0, 1.0)).collect();
    let weight: f64 = outside.iter().map(|&o| f64::from(o)).sum();
    if weight < 16.0 || px.len() != w * h * 4 {
        return "not matched: too little outside the selection".into();
    }
    // A result that doesn't resemble the original around the selection (reframed, restyled) is
    // left alone rather than forced onto it.
    let diff: f64 = (0..w * h).map(|i| f64::from(outside[i]) * (0..3).map(|c| f64::from((px[i * 4 + c] - r.original[i * 3 + c]).abs())).sum::<f64>()).sum();
    let drift = diff / (weight * 3.0);
    if drift > MAX_DRIFT {
        return format!("not matched: mean difference outside the selection {drift:.3} > {MAX_DRIFT}");
    }
    let target: Vec<f32> = r.original.as_chunks::<3>().0.iter().map(|p| luma(p)).collect();
    let fit = |px: &[f32], wt: &[f32]| fit_curve(&px.as_chunks::<4>().0.iter().map(|p| luma(p)).collect::<Vec<_>>(), &target, wt);
    // What the model changed rather than relit: different detail (an edge moved, an object
    // redrawn), or a different colour by far more than relighting explains.
    let first = fit(px, &outside);
    let detail = |mut v: Vec<f32>| {
        let mut smooth = v.clone();
        box_blur(&mut smooth, w, h, 2);
        box_blur(&mut smooth, w, h, 2);
        v.iter_mut().zip(&smooth).for_each(|(a, b)| *a -= b);
        v
    };
    let (got, want) = (detail(px.as_chunks::<4>().0.iter().map(|p| luma(p)).collect()), detail(target.clone()));
    let mut differs: Vec<f32> = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).collect();
    box_blur(&mut differs, w, h, 4);
    let mut changed = 0.0f64;
    for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
        let mut q = *p;
        tone(&first, &mut q);
        let far = (0..3).map(|c| (q[c] - r.original[i * 3 + c]).abs()).sum::<f32>() / 3.0 > CHANGED;
        if outside[i] > 0.0 && (far || differs[i] > DETAIL) {
            changed += f64::from(outside[i]);
            outside[i] = 0.0;
        }
    }
    let curve = if outside.iter().map(|&o| f64::from(o)).sum::<f64>() >= 16.0 { fit(px, &outside) } else { first };
    let stats = format!(
        "mean difference outside the selection {drift:.3}; {:.0}% of it changed; brightness change in shadows/midtones/highlights: {:+.3}/{:+.3}/{:+.3}",
        100.0 * changed / weight,
        curve[BINS / 8],
        curve[BINS / 2],
        curve[BINS * 7 / 8]
    );
    for p in px.as_chunks_mut::<4>().0 {
        tone(&curve, p);
    }
    // What the curve leaves, known only where the result kept the original, spread over the rest.
    let residual: Vec<f32> = (0..w * h).flat_map(|i| [0, 1, 2].map(|c| if outside[i] > 0.0 { r.original[i * 3 + c] - px[i * 4 + c] } else { 0.0 })).collect();
    let unknown: Vec<f32> = outside.iter().map(|o| 1.0 - o).collect();
    let spread = surroundings_fill(&residual, &unknown, w, h);
    for i in 0..w * h {
        for c in 0..3 {
            let v = &mut px[i * 4 + c];
            if r.coverage[i] > 0.0 {
                *v += spread[i * 3 + c];
            }
            *v = v.clamp(0.0, 1.0);
        }
    }
    stats
}

/// Decode a provider's PNG or JPEG into 8-bit sRGB, with its size.
fn decode(bytes: &[u8]) -> Result<(Surface, Rect)> {
    let opts = codecs::DecodeOptions {
        limits: codecs::Limits { max_width: RESULT_SIDE, max_height: RESULT_SIDE, max_pixels: 40_000_000, max_alloc: 512 * 1024 * 1024 },
        keep_orientation: false,
    };
    let format = codecs::detect(bytes)
        .filter(|f| matches!(f, Format::Png | Format::Jpeg | Format::WebP))
        .ok_or_else(|| other("the image service returned an unreadable image"))?;
    let img = codecs::decode_as_with(format, bytes, &opts).map_err(|e| other(format!("the image service returned an unreadable image ({e})")))?;
    let img = img.convert(ChannelLayout::Rgba, CodecSample::U8);
    let r = Rect::from_xywh(0, 0, img.width(), img.height());
    if r.is_empty() {
        return Err(other("the image service returned an empty image"));
    }
    let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    s.write_interleaved(r, img.data());
    Ok((s, r))
}

/// A file's image as a document, for use as a reference image (any format File › Open reads).
pub fn reference_document(name: &str, bytes: &[u8]) -> Result<Document> {
    let imported = photocraft_io::import(name, bytes).map_err(|e| other(format!("{name}: {e}")))?;
    if imported.preview_only {
        return Err(other(format!("{name}: only this file's embedded preview could be read")));
    }
    Ok(imported.document)
}

/// A document's composite reduced to at most `side` pixels on its long side, in 8-bit sRGB matted
/// over white: (width, height, RGB). What a reference image is made of, and its preview.
pub fn reference_pixels(doc: &Document, side: u32) -> Result<(u32, u32, Vec<u8>)> {
    let mut buf = photocraft_compose::thumbnail_buffer(doc, side.max(1));
    let (w, h) = (buf.rect.width(), buf.rect.height());
    if w == 0 || h == 0 || buf.px.len() != (w as usize) * (h as usize) {
        return Err(other("the reference image is empty"));
    }
    let t = Transform::new(&crate::color_cmds::composite_profile(doc), Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).map_err(other)?;
    t.apply(buf.px.as_flattened_mut(), 4);
    let finite = |v: f32| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    let mut rgb = Vec::with_capacity(buf.px.len() * 3);
    for p in &buf.px {
        let a = finite(p[3]);
        for &c in &p[..3] {
            // Over white: a cut-out product reads as one on a plain backdrop.
            rgb.push(((finite(c) * a + (1.0 - a)) * 255.0).round() as u8);
        }
    }
    Ok((w, h, rgb))
}

/// A document as a reference image: at most [`REFERENCE_SIDE`] pixels, sRGB, JPEG.
pub fn reference_image(doc: &Document) -> Result<ReferenceImage> {
    let (w, h, rgb) = reference_pixels(doc, REFERENCE_SIDE)?;
    let img = Image::from_raw(w, h, ChannelLayout::Rgb, CodecSample::U8, rgb).map_err(other)?;
    let opts = codecs::EncodeOptions { jpeg_quality: 90, embed_icc: false, embed_metadata: false, ..Default::default() };
    Ok(ReferenceImage { bytes: codecs::encode(&img, Format::Jpeg, &opts).map_err(other)?, mime: "image/jpeg".into() })
}

/// Where a reference image comes from: a file (read when the job runs) or an open document.
#[derive(Clone)]
enum ReferenceSource {
    File(String),
    Document(Arc<Document>),
}

impl ReferenceSource {
    fn image(&self) -> Result<ReferenceImage> {
        match self {
            ReferenceSource::File(path) => {
                let bytes = crate::file_cmds::read_file(path)?;
                reference_image(&reference_document(path, &bytes)?)
            }
            ReferenceSource::Document(doc) => reference_image(doc),
        }
    }
}

/// `references`: a list of file paths (or `{"path": …}`) and open documents (`{"document": id}`),
/// at most `max`.
fn references_param(s: &Session, p: &Value, max: usize, provider: &str) -> Result<Vec<ReferenceSource>> {
    const WHAT: &str = "`references` must be a list of file paths and {\"document\": id} entries";
    let list = match p.get("references") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(list)) => list,
        Some(_) => return Err(bad(WHAT)),
    };
    if list.is_empty() {
        return Ok(Vec::new());
    }
    if max == 0 {
        return Err(bad(format!("{provider} can't use reference images")));
    }
    if list.len() > max {
        return Err(bad(format!("at most {max} reference images can be sent")));
    }
    let file = |path: &str| if path.trim().is_empty() { Err(bad(WHAT)) } else { Ok(ReferenceSource::File(path.to_owned())) };
    list.iter()
        .map(|v| match v {
            Value::String(path) => file(path),
            Value::Object(o) => match (o.get("path"), o.get("document")) {
                (Some(Value::String(path)), None) => file(path),
                (None, Some(id)) => {
                    let id = id.as_u64().ok_or_else(|| bad(WHAT))?;
                    let st = s.documents().iter().find(|d| d.doc.id.0 == id).ok_or_else(|| bad(format!("no open document has id {id}")))?;
                    Ok(ReferenceSource::Document(st.doc.clone()))
                }
                _ => Err(bad(WHAT)),
            },
            _ => Err(bad(WHAT)),
        })
        .collect()
}

fn mode_param(p: &Value) -> Result<EditMode> {
    match p.get("mode") {
        None | Some(Value::Null) => Ok(EditMode::Fill),
        Some(Value::String(m)) if m == "fill" => Ok(EditMode::Fill),
        Some(Value::String(m)) if m == "edit" => Ok(EditMode::Edit),
        Some(_) => Err(bad("`mode` must be \"fill\" or \"edit\"")),
    }
}

/// The new layer's name: Photoshop names a generative layer after its prompt.
fn layer_name(prompt: &str) -> String {
    let p = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if p.is_empty() {
        return LABEL.into();
    }
    match p.char_indices().nth(40) {
        Some((i, _)) => format!("{}…", &p[..i]),
        None => p,
    }
}

/// What a generative layer was made from (the image and mask sent, the prompt and where the
/// result goes), so Generate More can ask the provider again without re-rendering a document that
/// now holds the earlier results. Kept in the session only: a reopened file's generative layer is
/// an ordinary group.
pub(crate) struct Source {
    prompt: String,
    area: Rect,
    request: EditRequest,
    reference: Reference,
}

fn variations_param(p: &Value, key: &str) -> Result<usize> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(DEFAULT_VARIATIONS),
        Some(v) => match v.as_u64() {
            Some(n @ 1..=MAX_VARIATIONS) => Ok(n as usize),
            _ => Err(bad(format!("`{key}` must be a whole number from 1 to {MAX_VARIATIONS}"))),
        },
    }
}

/// Ask for `count` results of one request. The calls run side by side (each is a slow network
/// round trip); any that fail are dropped as long as one succeeds.
fn request_all(ctx: &crate::jobs::JobCtx, provider: &dyn ImageProvider, request: &EditRequest, count: usize) -> Result<Vec<Vec<u8>>> {
    let name = provider.info().name;
    let msg = if count == 1 { format!("Waiting for {name}") } else { format!("Waiting for {name} ({count} variations)") };
    ctx.progress(0.15, &msg);
    #[cfg(not(target_arch = "wasm32"))]
    let results: Vec<_> = if count == 1 {
        vec![queue::edit(ctx, provider, request)]
    } else {
        std::thread::scope(|scope| {
            let calls: Vec<_> = (0..count).map(|_| scope.spawn(|| queue::edit(ctx, provider, request))).collect();
            calls.into_iter().map(|c| c.join().unwrap_or(Err(photocraft_genai::GenError::BadResponse("the request failed")))).collect()
        })
    };
    #[cfg(target_arch = "wasm32")]
    let results: Vec<_> = (0..count).map(|_| queue::edit(ctx, provider, request)).collect();
    ctx.check()?;
    let mut first_error = None;
    let mut images = Vec::new();
    for r in results {
        match r {
            Ok(img) => images.push(img.bytes),
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    match (images.is_empty(), first_error) {
        (true, Some(e)) => Err(other(e)),
        (true, None) => Err(other("the image service returned nothing")),
        _ => Ok(images),
    }
}

/// A provider's image, colour-matched to the original around the selection, scaled into `area`
/// in document coordinates.
fn place(bytes: &[u8], area: Rect, reference: &Reference) -> Result<(Surface, String)> {
    let (got, full) = decode(bytes)?;
    let r = Rect::from_xywh(0, 0, reference.w, reference.h);
    let mut at_request = scaled(&got, full, reference.w, reference.h);
    let mut px = at_request.read_region(r);
    let stats = match_surroundings(&mut px, reference);
    at_request.write_region(r, &px);
    let fitted = scaled(&at_request, r, area.width(), area.height());
    let mut placed = Surface::new(fitted.format());
    placed.write_region(area, &fitted.read_region(Rect::from_xywh(0, 0, area.width(), area.height())));
    Ok((placed, stats))
}

/// Debugging aid: with `PHOTOCRAFT_GENAI_DUMP` set to a folder, each request's image, mask, mode,
/// reference images, raw results and colour-matching figures are written to a new subfolder of it.
/// Nothing is written unless the variable is set.
fn dump(request: &EditRequest, results: &[Vec<u8>], stats: &[String]) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(root) = std::env::var_os("PHOTOCRAFT_GENAI_DUMP") else { return };
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let dir = std::path::Path::new(&root).join(format!("fill-{stamp}"));
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let _ = std::fs::write(dir.join("sent-image.png"), &request.image);
        let _ = std::fs::write(dir.join("sent-mask.png"), &request.mask);
        let _ = std::fs::write(dir.join("prompt.txt"), &request.prompt);
        let _ = std::fs::write(dir.join("mode.txt"), format!("{:?}", request.mode));
        for (i, r) in request.references.iter().enumerate() {
            let ext = if r.mime == "image/png" { "png" } else { "jpg" };
            let _ = std::fs::write(dir.join(format!("sent-reference-{}.{ext}", i + 1)), &r.bytes);
        }
        for (i, bytes) in results.iter().enumerate() {
            let ext = if bytes.starts_with(b"\x89PNG") { "png" } else { "jpg" };
            let _ = std::fs::write(dir.join(format!("result-{}.{ext}", i + 1)), bytes);
        }
        let _ = std::fs::write(dir.join("color-match.txt"), stats.iter().enumerate().map(|(i, s)| format!("result {}: {s}\n", i + 1)).collect::<String>());
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (request, results, stats);
}

/// Place every result, then (when asked) dump the request and results for debugging.
fn place_all(request: &EditRequest, results: &[Vec<u8>], area: Rect, reference: &Reference) -> Result<Vec<Surface>> {
    let placed = results.iter().map(|b| place(b, area, reference)).collect::<Result<Vec<_>>>()?;
    let stats: Vec<String> = placed.iter().map(|(_, s)| s.clone()).collect();
    dump(request, results, &stats);
    Ok(placed.into_iter().map(|(p, _)| p).collect())
}

fn variation_layer(doc: &Document, pixels: &Surface, n: usize, visible: bool) -> Result<Layer> {
    let converted = to_document(doc, pixels)?;
    let mut layer = Layer::raster(format!("Variation {n}"), converted.format());
    *layer.surface_mut().ok_or_else(|| other("could not create the layer"))? = converted;
    layer.visible = visible;
    Ok(layer)
}

/// `area`: which of the selection's separate areas to fill (0-based), if not all of it.
fn area_param(p: &Value) -> Result<Option<usize>> {
    match p.get("area") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(|n| Some(n as usize)).ok_or_else(|| bad("`area` must be an area number from 0 (see layer.generative.areas)")),
    }
}

fn fill(s: &mut Session, p: &Value) -> Result<Value> {
    let prompt = match p.get("prompt") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(t)) => t.trim().to_owned(),
        Some(_) => return Err(bad("`prompt` must be text")),
    };
    photocraft_genai::check_prompt(&prompt, false).map_err(|e| bad(e.to_string()))?;
    let count = variations_param(p, "variations")?;
    let mode = mode_param(p)?;
    let area_index = area_param(p)?;
    if mode == EditMode::Edit && prompt.is_empty() {
        return Err(bad("Edit needs a prompt: say what to change in the selection"));
    }
    fill_enabled(s).map_err(bad)?;
    let provider = s.image_provider().cloned().ok_or_else(|| bad("no generative AI provider"))?;
    let (caps, provider_name) = (provider.capabilities(), provider.info().name);
    if mode == EditMode::Edit && !caps.edit_in_place {
        return Err(bad(format!("{provider_name} can't edit what is in the selection; use Fill")));
    }
    let references = references_param(s, p, caps.max_references.min(photocraft_genai::MAX_REFERENCES), provider_name)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = st.doc.clone();
    let mut sel = doc.selection.clone().ok_or_else(|| bad("no selection"))?;
    let canvas = doc.bounds();
    if let Some(k) = area_index {
        let areas = areas::Areas::find(&sel, canvas);
        let n = areas.len();
        sel = areas.mask(&sel, k).ok_or_else(|| bad(format!("the selection has {n} separate areas; `area` counts them from 0")))?;
    }
    let area = context_area(sel.content_bounds().intersect(&canvas), canvas, |w, h| provider.edit_aspect(w, h));
    let k = (f64::from(UPLOAD_SIDE) / f64::from(area.width().max(area.height()))).min(1.0);
    let (w, h) = (((f64::from(area.width()) * k).round() as u32).max(1), ((f64::from(area.height()) * k).round() as u32).max(1));
    let (doc_id, anchor) = (doc.id, st.active_layer);
    let input = FillInput {
        name: layer_name(&prompt),
        doc,
        sel,
        canvas,
        area,
        size: (w, h),
        mode,
        prompt,
        references,
        count,
        select: !s.spawning(),
    };
    let g = &mut s.genai_state;
    g.next_fill += 1;
    let id = g.next_fill;
    g.fills.push(Fill { id, job: None, doc: doc_id, anchor, status: Arc::default(), input: Arc::new(input) });
    start(s, id)
}

/// Run fill `id`'s job: in the background (it stays listed until it lands), or inline.
fn start(s: &mut Session, id: u64) -> Result<Value> {
    let provider = s.image_provider().cloned().ok_or_else(|| bad("no generative AI provider"))?;
    let fill = s.genai_state.fills.iter().find(|f| f.id == id).ok_or_else(|| bad(format!("no Generative Fill {id} is waiting")))?;
    let (doc, input, status, turns) = (fill.doc, Arc::clone(&fill.input), Arc::clone(&fill.status), Arc::clone(&s.genai_state.turns));
    status.set_error(None);
    let spawning = s.spawning();
    let worker = Arc::clone(&status);
    let r = crate::jobs::run_on(
        s,
        LABEL,
        doc,
        move |ctx| {
            let r = fill_work(ctx, &input, provider.as_ref(), &turns, &worker);
            if let Err(e) = &r
                && !matches!(e, EngineError::Cancelled)
            {
                worker.set_error(Some(e.to_string()));
            }
            r
        },
        move |s, (placed, source)| land(s, id, placed, source),
    );
    let started = match &r {
        Ok(v) if spawning => v.get("job").and_then(Value::as_u64).map(|j| (v.clone(), JobId(j))),
        _ => None,
    };
    match started {
        Some((mut v, job)) => {
            if let Some(f) = s.genai_state.fills.iter_mut().find(|f| f.id == id) {
                f.job = Some(job);
            }
            if let Some(o) = v.as_object_mut() {
                o.insert("fill".into(), json!(id));
            }
            Ok(v)
        }
        // Inline it landed (and was forgotten) or failed; a job that couldn't start is
        // forgotten too.
        None => {
            s.genai_state.fills.retain(|f| f.id != id);
            r
        }
    }
}

/// A fill's network part: what is sent, a turn at the provider, the results placed.
fn fill_work(ctx: &JobCtx, input: &FillInput, provider: &dyn ImageProvider, turns: &queue::Turns, status: &FillStatus) -> Result<(Vec<Surface>, Source)> {
    let (w, h) = input.size;
    ctx.progress(0.05, "Preparing image");
    let (image, mask, reference) = request_images(&scaled(&srgb_composite(&input.doc, input.area)?, input.area, w, h), &input.sel, input.area, w, h, input.mode)?;
    ctx.check()?;
    if !input.references.is_empty() {
        ctx.progress(0.1, "Preparing reference images");
    }
    let references = input.references.iter().map(ReferenceSource::image).collect::<Result<Vec<_>>>()?;
    ctx.check()?;
    let request = EditRequest { image, mask, prompt: input.prompt.clone(), size: ImageSize::for_long_side(w.max(h)), mode: input.mode, references };
    let _turn = turns.take(ctx, &status.queued)?;
    let results = request_all(ctx, provider, &request, input.count)?;
    ctx.progress(0.9, "Placing the results");
    let placed = place_all(&request, &results, input.area, &reference)?;
    Ok((placed, Source { prompt: request.prompt.clone(), area: input.area, request, reference }))
}

/// Fill `id`'s results arrive (on its document): a new group masked by the selection, one layer
/// per variation with the first showing, above the layer that was active when it started. One
/// undo step.
fn land(s: &mut Session, id: u64, placed: Vec<Surface>, source: Source) -> Result<Value> {
    // Discarded meanwhile.
    let i = s.genai_state.fills.iter().position(|f| f.id == id).ok_or(EngineError::Cancelled)?;
    let fill = s.genai_state.fills.remove(i);
    let input = &fill.input;
    let canvas = s.active().map(|st| st.doc.bounds()).ok_or(EngineError::NoDocument)?;
    if canvas != input.canvas {
        return Err(other("the canvas changed size while Generative Fill ran; its result was discarded"));
    }
    let (group, ids) = s.edit(LABEL, |doc, active| {
        // Photoshop's generative layer holds its variations; here that is a group masked by
        // the selection with one layer per variation, the first one showing.
        let children = placed.iter().enumerate().map(|(i, px)| variation_layer(doc, px, i + 1, i == 0)).collect::<Result<Vec<_>>>()?;
        let ids: Vec<u64> = children.iter().map(|l| l.id.0).collect();
        let mut group = Layer::group(input.name.clone(), children);
        group.mask = Some(LayerMask { surface: input.sel.clone(), ..LayerMask::reveal_all() });
        let above = fill.anchor.filter(|a| doc.layer(*a).is_some()).or(*active);
        let id = doc.insert_above(above, group);
        crate::layer_multi_cmds::check_group_depth(doc, LABEL)?;
        if input.select {
            *active = Some(id);
        }
        Ok((id, ids))
    })?;
    let area = source.area;
    s.genai_state.sources.insert(group, Arc::new(source));
    Ok(json!({"layer": group.0, "variations": ids, "bounds": [area.x0, area.y0, area.width(), area.height()], "fill": id}))
}

fn fill_param(p: &Value, cmd: &str) -> Result<u64> {
    p.get("fill").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: "`fill` must be a fill id (see layer.generative.fills)".into() })
}

fn retry_enabled(s: &Session) -> std::result::Result<(), String> {
    s.image_provider().map(|_| ()).ok_or_else(|| "set up a generative AI provider first (Generative AI Settings)".into())
}

/// Send a failed fill again, as it was started.
fn retry(s: &mut Session, p: &Value) -> Result<Value> {
    let id = fill_param(p, RETRY)?;
    let bad = |msg: String| EngineError::BadParams { cmd: RETRY.into(), msg };
    let fill = s.genai_state.fills.iter().find(|f| f.id == id).ok_or_else(|| bad(format!("no Generative Fill {id} is waiting")))?;
    if fill.status.error().is_none() {
        return Err(bad(format!("Generative Fill {id} hasn't failed")));
    }
    // Its old job has ended: the error is set as it returns.
    if let Some(job) = fill.job {
        s.cancel_job(job);
    }
    start(s, id)
}

/// Stop a fill that is generating or waiting its turn, or forget one that failed.
fn discard(s: &mut Session, p: &Value) -> Result<Value> {
    let id = fill_param(p, DISCARD)?;
    let i = s
        .genai_state
        .fills
        .iter()
        .position(|f| f.id == id)
        .ok_or_else(|| EngineError::BadParams { cmd: DISCARD.into(), msg: format!("no Generative Fill {id} is waiting") })?;
    let fill = s.genai_state.fills.remove(i);
    if let Some(job) = fill.job {
        s.cancel_job(job);
    }
    Ok(json!({"discarded": id}))
}

fn fills(s: &mut Session, _: &Value) -> Result<Value> {
    s.tidy_generative_fills();
    Ok(json!({"fills": s.generative_fills(), "parallel": MAX_PARALLEL}))
}

/// The separate areas of the active document's selection, top to bottom: `[x0, y0, x1, y1]` each.
pub fn selection_areas(s: &Session) -> Vec<Rect> {
    let Some(st) = s.active() else { return Vec::new() };
    let Some(sel) = st.doc.selection.as_ref() else { return Vec::new() };
    areas::Areas::find(sel, st.doc.bounds()).rects().to_vec()
}

fn areas_enabled(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    st.doc.selection.as_ref().map(|_| ()).ok_or_else(|| "no selection".into())
}

fn areas(s: &mut Session, _: &Value) -> Result<Value> {
    let rects: Vec<[i32; 4]> = selection_areas(s).iter().map(|r| [r.x0, r.y0, r.x1, r.y1]).collect();
    Ok(json!({"areas": rects}))
}

/// The generative layer `p["layer"]` names, or the active one: the group itself or one of its
/// variations.
fn generative_target(s: &Session, p: &Value) -> Option<LayerId> {
    let st = s.active()?;
    let id = match p.get("layer").and_then(Value::as_u64) {
        Some(id) => LayerId(id),
        None => st.active_layer?,
    };
    let is_generative = |id: LayerId| s.genai_state.sources.contains_key(&id) && st.doc.layer(id).is_some_and(Layer::is_group);
    if is_generative(id) {
        return Some(id);
    }
    let path = st.doc.path_of(id)?;
    let parent = st.doc.layer_at(path.get(..path.len().checked_sub(1)?)?)?.id;
    is_generative(parent).then_some(parent)
}

impl Session {
    /// The active generative layer (a Generative Fill group, or the active variation's group), for
    /// the Properties panel's variations.
    pub fn generative_layer(&self) -> Option<LayerId> {
        generative_target(self, &Value::Null)
    }

    /// The prompt a generative layer was made with.
    pub fn generative_prompt(&self, id: LayerId) -> Option<&str> {
        self.genai_state.sources.get(&id).map(|s| s.prompt.as_str())
    }
}

fn select_enabled(s: &Session) -> std::result::Result<(), String> {
    generative_target(s, &Value::Null).map(|_| ()).ok_or_else(|| "select a generative layer first".into())
}

fn select_variation(s: &mut Session, p: &Value) -> Result<Value> {
    let bad = |msg: &str| EngineError::BadParams { cmd: SELECT_VARIATION.into(), msg: msg.into() };
    let group = generative_target(s, p).ok_or_else(|| bad("select a generative layer first"))?;
    let index = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad("`index` must be a variation number from 0"))? as usize;
    let children = s.active().and_then(|st| st.doc.layer(group)).and_then(Layer::children).map_or(0, <[Layer]>::len);
    if index >= children {
        return Err(bad(&format!("this generative layer has {children} variations")));
    }
    s.edit("Generative Variation", |doc, _| {
        let layers = doc.layer_mut(group).and_then(Layer::children_mut).ok_or_else(|| other("the generative layer is gone"))?;
        for (i, l) in layers.iter_mut().enumerate() {
            l.visible = i == index;
        }
        Ok(())
    })?;
    Ok(json!({"layer": group.0, "index": index}))
}

fn more_enabled(s: &Session) -> std::result::Result<(), String> {
    let p = s.image_provider().ok_or("set up a generative AI provider first (Generative AI Settings)")?;
    if !p.capabilities().edit {
        return Err(format!("{} can't fill selections", p.info().name));
    }
    select_enabled(s)
}

fn more(s: &mut Session, p: &Value) -> Result<Value> {
    let bad = |msg: String| EngineError::BadParams { cmd: MORE.into(), msg };
    let count = variations_param(p, "count").map_err(|_| bad(format!("`count` must be a whole number from 1 to {MAX_VARIATIONS}")))?;
    more_enabled(s).map_err(bad)?;
    let group = generative_target(s, p).ok_or_else(|| bad("select a generative layer first".into()))?;
    let provider = s.image_provider().cloned().ok_or_else(|| bad("no generative AI provider".into()))?;
    let source = s.genai_state.sources.get(&group).cloned().ok_or_else(|| bad("select a generative layer first".into()))?;
    let doc = s.active().map(|st| st.doc.id).ok_or(EngineError::NoDocument)?;
    let turns = Arc::clone(&s.genai_state.turns);
    // Like a fill, it doesn't lock the document: its variations join the group when they come.
    crate::jobs::run_on(
        s,
        LABEL,
        doc,
        move |ctx| {
            let _turn = turns.take(ctx, &AtomicUsize::new(0))?;
            let results = request_all(ctx, provider.as_ref(), &source.request, count)?;
            ctx.progress(0.9, "Placing the results");
            place_all(&source.request, &results, source.area, &source.reference)
        },
        move |s, placed| {
            let ids = s.edit("Generate More", |doc, _| {
                let have = doc.layer(group).and_then(Layer::children).map(<[Layer]>::len).ok_or_else(|| other("the generative layer is gone"))?;
                let new = placed.iter().enumerate().map(|(i, px)| variation_layer(doc, px, have + i + 1, i == 0)).collect::<Result<Vec<_>>>()?;
                let ids: Vec<u64> = new.iter().map(|l| l.id.0).collect();
                let layers = doc.layer_mut(group).and_then(Layer::children_mut).ok_or_else(|| other("the generative layer is gone"))?;
                // The first new variation shows, as when the layer was made.
                for l in layers.iter_mut() {
                    l.visible = false;
                }
                layers.extend(new);
                Ok(ids)
            })?;
            Ok(json!({"layer": group.0, "variations": ids}))
        },
    )
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: FILL,
            label: "Generative Fill…",
            menu: &["Edit"],
            shortcut: None,
            params: r#"{"prompt":str?,"variations":1..4=3,"mode":"fill"|"edit"="fill","references":[path|{"path":str}|{"document":id}]?,"area":uint?} fills the selection through the configured provider: a new group masked by the selection holds one layer per variation, the first showing. `fill` blurs the selected area out first, so the provider fills a hole (an empty prompt continues the surrounding image through it, removing what was there); `edit` sends it as it is and changes only what the prompt asks for (a prompt is required). `references` (at most 4) are images from files or open documents the provider should follow. `area` fills only that one of the selection's separate areas (0 = the first, see layer.generative.areas). Started in the background it doesn't lock the document: it returns {job, fill} at once, at most 5 fills talk to the provider at a time (the rest wait their turn), and the group lands above the layer that was active when it started, without becoming active (layer.generative.fills lists those still running or failed). Sends the selected area, its surroundings and any reference images to that provider, once per variation."#,
            enabled: fill_enabled,
            run: fill,
            journal: true,
        },
        CommandSpec {
            id: SELECT_VARIATION,
            label: "Generative Variation",
            menu: &[],
            shortcut: None,
            params: r#"{"index":uint,"layer":id?} shows variation `index` (0 = the first) of the active generative layer and hides the others."#,
            enabled: select_enabled,
            run: select_variation,
            journal: true,
        },
        CommandSpec {
            id: MORE,
            label: "Generate More",
            menu: &[],
            shortcut: None,
            params: r#"{"count":1..4=3,"layer":id?} asks the provider for more variations of the active generative layer, with the same image, mask and prompt, and adds them to it. Sends that area to the provider again."#,
            enabled: more_enabled,
            run: more,
            journal: true,
        },
        CommandSpec {
            id: RETRY,
            label: "Retry Generative Fill",
            menu: &[],
            shortcut: None,
            params: r#"{"fill":id} sends a Generative Fill that failed again, as it was started (the same area, prompt and references, from the document as it was then). Sends that area to the provider again."#,
            enabled: retry_enabled,
            run: retry,
            journal: false,
        },
        CommandSpec {
            id: DISCARD,
            label: "Discard Generative Fill",
            menu: &[],
            shortcut: None,
            params: r#"{"fill":id} stops a Generative Fill that is generating or waiting its turn (its result is dropped), or forgets one that failed."#,
            enabled: |_| Ok(()),
            run: discard,
            journal: false,
        },
        CommandSpec {
            id: FILLS,
            label: "Generative Fills",
            menu: &[],
            shortcut: None,
            params: r#"{} lists the Generative Fills that haven't landed: {fills:[{id, job, document, anchor, name, prompt, bounds:[x0,y0,x1,y1], state:"queued"|"running"|"failed", queue_position, progress, elapsed_ms, error}], parallel}."#,
            enabled: |_| Ok(()),
            run: fills,
            journal: false,
        },
        CommandSpec {
            id: AREAS,
            label: "Selection Areas",
            menu: &[],
            shortcut: None,
            params: r#"{} the separate areas of the selection, top to bottom: {areas:[[x0,y0,x1,y1]]}. Generative Fill's `area` fills one of them."#,
            enabled: areas_enabled,
            run: areas,
            journal: false,
        },
    ]
}

#[cfg(test)]
#[path = "genai_cmds_tests.rs"]
mod tests;
