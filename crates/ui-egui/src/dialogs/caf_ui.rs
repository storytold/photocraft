//! Edit › Content-Aware Fill…: Photoshop's Content-Aware Fill workspace, hosted on the app's dialog host.
//!
//! The framework's [`ContentAwareFillWorkspace`] draws the tools, the options bar, the panels and the preview, and
//! keeps the session (the sampling option, brush strokes and lasso edits) with its history and the fill settings,
//! which it remembers itself. This module is the app's side of it:
//!
//! * [`CafApp`] holds what the workspace reads from the document: the proxy-resolution sampling and fill-area
//!   planes, the composite shown behind them and the preview picture.
//! * [`PcFillSource`] is what the workspace reads each pass ([`FillSource`]); its preview job runs the engine's
//!   plan (`content_aware_plan`), so the preview is exactly what OK and Apply write.
//! * [`take`] acts on what the workspace reported: a stroke paints the planes, a lasso edit or a reset re-plans,
//!   Apply and OK run `edit.contentAwareFill`, the overlay colour opens a colour picker.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use craft::behavior::kurbo;
use craft::imaging::ui::fill::{ContentAwareFillWorkspace, FillCommand, FillPreviewRequest, FillSession, FillSource, SamplingPlane, SamplingStroke};
use craft::imaging::ui::select_mask::EditMode;
use craft::imaging::ui::wide_angle::PreviewDone;
use craft::ui::canvas::RasterSource;
use craft::ui::dialog::DialogId;
use craft::ui::dialog::HostedWorkspace;
use craft::ui::runtime::memory::SaveKey;
use craft::ui::runtime::pending::Job;
use photocraft_algo::content_aware::{SamplingStroke as ProxyStroke, paint_strokes};
use photocraft_doc::{DocId, Document, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Interrupt;

use crate::PhotocraftApp;

use crate::dialogs::shared_picture::SharedPicture;

mod lifecycle;
mod preview;

pub use lifecycle::open;
pub(crate) use lifecycle::{describe, on_job_event, recorded_from_workspace, recorded_to_workspace, take};
use preview::{Base, compute_preview, plan_params};

/// Longest side of the document proxy shown in the view (px).
const PROXY_SIDE: usize = 1600;
/// The saved slot of the workspace's settings.
const REMEMBERED: SaveKey = SaveKey::new("photocraft.edit.contentAwareFill.workspace");
const CMD: &str = "edit.contentAwareFill";
/// What a preview job that was superseded answers.
const CANCELLED: &str = "cancelled";

/// The hosted workspace as the dialog host stores it.
pub(crate) type Hosted = HostedWorkspace<ContentAwareFillWorkspace>;

/// The app's side of the open workspace.
pub(crate) struct CafApp {
    /// The hosted workspace's id on the dialog host.
    id: DialogId,
    /// The document and layer the workspace was opened on.
    document: DocId,
    layer: LayerId,
    canvas: Rect,
    /// The document revision the image, the base and the planes were made from.
    revision: u64,
    doc: Arc<Document>,
    /// Document px per proxy px, and the proxy size.
    k: usize,
    pw: usize,
    ph: usize,
    /// The plan without strokes: the window, the fill area and the sampling option's area.
    base: Base,
    /// Per proxy pixel: the sampling option's area (before the strokes), the fill area, and the sampling area after
    /// the strokes so far (painted in order, as the engine's `paint_strokes` paints the full-resolution one).
    base_px: Vec<bool>,
    hole_px: Vec<bool>,
    mask: Vec<bool>,
    /// The session the planes were made for; undo and redo change the workspace's session without an event.
    known: FillSession,
    /// The Sample All Layers setting the base was planned for.
    sample_all: bool,
    /// The last point painted of the stroke in progress.
    last: Option<[f64; 2]>,
    /// The last Apply or OK error, or the plan's refusal of the last edit.
    error: Option<String>,
    /// The planes or the document changed: the next [`take`] hands the workspace a new source.
    dirty: bool,
    /// The visible layers composited at the proxy size; moves when the document does.
    image: Arc<SharedPicture>,
    /// The last ready preview, written by the preview jobs.
    picture: Arc<SharedPicture>,
    /// Why the last preview failed; `None` after a good one.
    preview_error: Arc<Mutex<Option<String>>>,
    /// How many preview jobs started.
    preview_runs: Arc<AtomicUsize>,
}

impl CafApp {
    /// The proxy, in proxy px.
    fn proxy_area(&self) -> Rect {
        Rect::new(0, 0, self.pw as i32, self.ph as i32)
    }

    /// `points` in proxy coordinates: proxy pixel `(px, py)` is centred on document point
    /// `canvas.x0 + (px + 0.5) k`, so a stroke's points and size scale by `1 / k`.
    fn proxy_stroke(&self, add: bool, size: f64, points: &[[f64; 2]]) -> ProxyStroke {
        let k = self.k as f64;
        let (ox, oy) = (f64::from(self.canvas.x0), f64::from(self.canvas.y0));
        ProxyStroke { add, size: size / k, points: points.iter().map(|p| [(p[0] - ox) / k, (p[1] - oy) / k]).collect() }
    }

    /// Paints `points` of a stroke into the proxy's sampling area: only the pixels within the brush of its path
    /// are visited, whatever came before.
    fn paint(&mut self, add: bool, size: f64, points: &[[f64; 2]]) {
        let (stroke, area) = (self.proxy_stroke(add, size, points), self.proxy_area());
        paint_strokes(&mut self.mask, area, &[stroke]);
        self.dirty = true;
    }

    /// The press: the stroke's first point.
    fn begin_stroke(&mut self, stroke: &SamplingStroke) {
        self.error = None;
        self.paint(stroke.mode == EditMode::Add, stroke.size, &stroke.points);
        self.last = stroke.points.last().copied();
    }

    /// New points: the segment from the last painted point through them.
    fn extend_stroke(&mut self, stroke: &SamplingStroke) {
        let points: Vec<[f64; 2]> = self.last.iter().chain(&stroke.points).copied().collect();
        self.paint(stroke.mode == EditMode::Add, stroke.size, &points);
        self.last = stroke.points.last().copied().or(self.last);
    }

    /// The release. A stroke recorded without a drag (a control request) was never painted: paint it whole.
    fn commit_stroke(&mut self, stroke: &SamplingStroke) {
        self.error = None;
        if self.last.take().is_none() {
            self.paint(stroke.mode == EditMode::Add, stroke.size, &stroke.points);
        }
    }

    /// Takes a new base: its option's area and fill area at the proxy's pixel centres, then `strokes` over it.
    fn set_base(&mut self, base: Base, strokes: &[SamplingStroke]) {
        let k = self.k as f64;
        for py in 0..self.ph {
            for px in 0..self.pw {
                let (x, y) = (f64::from(self.canvas.x0) + (px as f64 + 0.5) * k, f64::from(self.canvas.y0) + (py as f64 + 0.5) * k);
                let i = base.index(x, y);
                let at = |v: &[bool]| i.is_some_and(|i| v.get(i).copied().unwrap_or(false));
                self.base_px[py * self.pw + px] = at(&base.base);
                self.hole_px[py * self.pw + px] = at(&base.hole);
            }
        }
        self.base = base;
        self.restate_all(strokes);
    }

    /// Rebuilds the proxy's sampling area: the base painted with every stroke in order. Each stroke costs its own
    /// pixels, so this is linear in the strokes, not strokes × pixels.
    fn restate_all(&mut self, strokes: &[SamplingStroke]) {
        self.last = None;
        self.mask.clone_from(&self.base_px);
        let strokes: Vec<ProxyStroke> = strokes.iter().map(|s| self.proxy_stroke(s.mode == EditMode::Add, s.size, &s.points)).collect();
        let area = self.proxy_area();
        paint_strokes(&mut self.mask, area, &strokes);
        self.dirty = true;
    }

    /// What the workspace reads this pass, from the planes as they stand.
    fn source(&self, texture_limit: usize) -> Arc<dyn FillSource> {
        let plane = |bits: &[bool]| SamplingPlane::new([self.canvas.x0, self.canvas.y0], [self.pw as u32, self.ph as u32], self.k as u32, bits.to_vec());
        let w = self.base.window;
        Arc::new(PcFillSource {
            doc: Arc::clone(&self.doc),
            layer: self.layer,
            texture_limit,
            image: Arc::clone(&self.image),
            preview: Arc::clone(&self.picture),
            preview_error: Arc::clone(&self.preview_error),
            preview_runs: Arc::clone(&self.preview_runs),
            sampling: plane(&self.mask),
            fill_area: plane(&self.hole_px),
            window: kurbo::Rect::new(f64::from(w.x0), f64::from(w.y0), f64::from(w.x1), f64::from(w.y1)),
        })
    }
}

/// What the workspace reads from the app each pass.
pub(crate) struct PcFillSource {
    doc: Arc<Document>,
    layer: LayerId,
    /// The device's texture side limit the preview is reduced to.
    texture_limit: usize,
    image: Arc<SharedPicture>,
    preview: Arc<SharedPicture>,
    preview_error: Arc<Mutex<Option<String>>>,
    preview_runs: Arc<AtomicUsize>,
    sampling: SamplingPlane,
    fill_area: SamplingPlane,
    window: kurbo::Rect,
}

impl FillSource for PcFillSource {
    fn image(&self) -> &dyn RasterSource {
        &*self.image
    }

    fn sampling(&self) -> Option<&SamplingPlane> {
        Some(&self.sampling)
    }

    fn fill_area(&self) -> Option<&SamplingPlane> {
        Some(&self.fill_area)
    }

    fn sampling_window(&self) -> Option<kurbo::Rect> {
        Some(self.window)
    }

    fn preview(&self) -> Option<&dyn RasterSource> {
        self.preview.is_ready().then_some(&*self.preview as &dyn RasterSource)
    }

    fn preview_job(&self, request: &FillPreviewRequest) -> Job<PreviewDone, String> {
        let (doc, layer, limit) = (Arc::clone(&self.doc), self.layer, self.texture_limit);
        let params = plan_params(&FillCommand::new(&request.session, &request.settings), layer);
        let (picture, shown_error, runs) = (Arc::clone(&self.preview), Arc::clone(&self.preview_error), Arc::clone(&self.preview_runs));
        Job::new(move |progress, cancel| {
            runs.fetch_add(1, Ordering::Relaxed);
            let started = Instant::now();
            let is_cancelled = || cancel.is_cancelled();
            let report = |fraction: f32| progress.report(fraction);
            let interrupt = Interrupt::new(&is_cancelled, &report);
            let filled = compute_preview(&doc, layer, &params, &interrupt, limit);
            // A superseded job leaves the picture and the error of the request that replaced it.
            if cancel.is_cancelled() {
                return Err(CANCELLED.to_string());
            }
            let mut shown = shown_error.lock().unwrap_or_else(PoisonError::into_inner);
            match filled {
                Ok((rect, image, factor)) => {
                    picture.replace_at([rect.x0, rect.y0], image, factor);
                    *shown = None;
                    Ok(PreviewDone::new(started.elapsed()))
                }
                Err(e) => {
                    *shown = Some(e.clone());
                    Err(e)
                }
            }
        })
    }
}

#[cfg(test)]
mod custom_alert_tests;
#[cfg(test)]
mod tests;
